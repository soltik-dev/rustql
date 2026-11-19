//! Exportación de tablas: a un fichero .sql, a una carpeta (un fichero por
//! tabla), a otra BBDD del mismo servidor o a otro servidor.
//!
//! Cada motor sabe describir sus tablas como sentencias (`Source`, en
//! export/mysql.rs y export/postgres.rs); aquí está lo común: a dónde van
//! esas sentencias (`Sink`), el orden en que se ejecutan y el progreso.

mod mysql;
mod postgres;

use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};

use super::{ConnectionParams, DbResult, Engine};

// ── Tipos públicos ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum ExportDestination {
    SqlFile {
        path: String,
    },
    Folder {
        path: String,
    },
    SameServerDb {
        target_db: String,
    },
    RemoteServer {
        params: ConnectionParams,
        target_db: String,
    },
}

#[derive(Debug, Clone)]
pub struct ExportOptions {
    pub include_create: bool,
    pub include_data: bool,
    pub drop_if_exists: bool,
    pub batch_size: usize,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            include_create: true,
            include_data: true,
            drop_if_exists: false,
            batch_size: 500,
        }
    }
}

pub enum ExportEvent {
    TableStarted(String),
    TableDone {
        table: String,
        rows: u64,
    },
    TableError {
        table: String,
        error: String,
    },
    Finished {
        total_tables: usize,
        total_rows: u64,
        elapsed_ms: u64,
    },
    Cancelled,
    /// Filas copiadas desde el aviso anterior: las tablas se copian por
    /// bloques, y una grande tarda.
    RowsCopied(u64),
    /// Error fatal que aborta toda la exportación (conexión, fichero, etc.).
    Failed(String),
}

// ── Función pública ────────────────────────────────────────────────────────────

/// Lanza el export en un hilo separado y devuelve un Receiver de eventos de progreso.
/// No toma el backend principal: abre su propia conexión usando `params`.
pub fn spawn_export(
    params: ConnectionParams,
    source_db: String,
    tables: Vec<String>,
    destination: ExportDestination,
    options: ExportOptions,
    cancel: Arc<AtomicBool>,
) -> Receiver<ExportEvent> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let start = std::time::Instant::now();
        let job = Job {
            source_db: &source_db,
            tables: &tables,
            options: &options,
            cancel: &cancel,
            tx: &tx,
        };
        let result = match params.engine {
            Engine::Mysql => mysql::run(&params, &destination, &job),
            Engine::Postgres => postgres::run(&params, &destination, &job),
        };
        let _ = tx.send(match result {
            _ if cancel.load(Ordering::Relaxed) => ExportEvent::Cancelled,
            Ok((total_tables, total_rows)) => ExportEvent::Finished {
                total_tables,
                total_rows,
                elapsed_ms: start.elapsed().as_millis() as u64,
            },
            Err(e) => ExportEvent::Failed(e),
        });
    });
    rx
}

// ── Lo común a todos los motores ───────────────────────────────────────────────

/// Tablas exportadas y filas copiadas.
type Totals = (usize, u64);

const SAME_TARGET_ERROR: &str = "El destino es la misma base de datos del mismo servidor que el origen: \
     la copia borraría o duplicaría sus datos. Elige otra base de datos destino.";

/// Lo que se exporta y cómo se cuenta.
struct Job<'a> {
    source_db: &'a str,
    tables: &'a [String],
    options: &'a ExportOptions,
    cancel: &'a AtomicBool,
    tx: &'a Sender<ExportEvent>,
}

impl Job<'_> {
    fn send(&self, event: ExportEvent) {
        let _ = self.tx.send(event);
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// Recorre las tablas avisando del progreso; `export` exporta una y
    /// devuelve sus filas. Se detiene al cancelar.
    fn for_each_table(&self, mut export: impl FnMut(&str) -> DbResult<u64>) -> Totals {
        let (mut done, mut total_rows) = (0, 0);
        for table in self.tables {
            if self.cancelled() {
                break;
            }
            self.send(ExportEvent::TableStarted(table.clone()));
            let table = table.clone();
            match export(&table) {
                Ok(rows) => {
                    total_rows += rows;
                    done += 1;
                    self.send(ExportEvent::TableDone { table, rows });
                }
                Err(error) => self.send(ExportEvent::TableError { table, error }),
            }
        }
        (done, total_rows)
    }
}

/// Lo que hay que ejecutar, y en qué momento, para recrear una tabla.
#[derive(Default)]
struct TableScript {
    /// Antes de sus datos: crearla, con lo que necesite (esquema, tipos…).
    create: Vec<String>,
    /// Sus datos; `None` si no los tiene (una vista).
    data: Option<TableData>,
    /// Tras sus datos: claves, índices y el valor de sus secuencias.
    after_data: Vec<String>,
    /// Cuando ya están todas las tablas: lo que depende de otras (claves
    /// foráneas, vistas).
    finish: Vec<String>,
}

struct TableData {
    /// Consulta que lee las filas en el origen.
    select: String,
    /// `INSERT INTO tabla`, sin columnas ni valores.
    insert_into: String,
    /// Lo que va entre la lista de columnas y la de valores.
    values_keyword: &'static str,
}

/// Origen de una exportación: una BBDD de un motor concreto.
trait Source {
    fn engine(&self) -> Engine;
    /// Sentencias que dejan una sesión (o un fichero) lista para la carga.
    fn prelude(&self) -> Vec<String>;
    /// Sentencias que eliminan `tables` del destino antes de crearlas, si
    /// el motor necesita eliminarlas juntas. Si no, cada tabla se elimina
    /// en su propio `TableScript::create`.
    fn drop_together(&self, tables: &[String]) -> DbResult<Vec<String>>;
    fn script(&self, table: &str, options: &ExportOptions) -> DbResult<TableScript>;
    /// Lee las filas de `select` sin traerlas todas a memoria: las entrega
    /// a `each` en bloques de hasta `batch`, con los nombres de las columnas
    /// y cada valor ya escrito como literal SQL. Si `each` falla, deja de
    /// leer y devuelve su error.
    fn read_rows(&self, select: &str, batch: usize, each: &mut RowBatch<'_>) -> DbResult<()>;
}

/// Lo que recibe cada bloque de filas: columnas y valores como literales.
type RowBatch<'a> = dyn FnMut(&[String], Vec<Vec<String>>) -> DbResult<()> + 'a;

/// Destino de las sentencias: un fichero o una conexión.
trait Sink {
    fn run(&mut self, sql: &str) -> DbResult<()>;

    /// Texto para quien lea el destino; una conexión no lo necesita.
    fn comment(&mut self, _text: &str) -> DbResult<()> {
        Ok(())
    }

    fn run_all(&mut self, statements: &[String]) -> DbResult<()> {
        statements.iter().try_for_each(|sql| self.run(sql))
    }
}

/// Fichero .sql: cada sentencia, terminada en punto y coma.
struct FileSink(BufWriter<std::fs::File>);

impl FileSink {
    fn create(path: &Path, source_db: &str) -> DbResult<Self> {
        let file = std::fs::File::create(path)
            .map_err(|e| format!("No se pudo crear el fichero '{}': {e}", path.display()))?;
        let mut sink = Self(BufWriter::new(file));
        let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
        sink.comment(&format!("RustQL Export — Base de datos: {source_db}"))?;
        sink.comment(&format!("Generado: {now}"))?;
        sink.comment(&"─".repeat(70))?;
        writeln!(sink.0).map_err(write_error)?;
        Ok(sink)
    }

    fn finish(mut self) -> DbResult<()> {
        self.0.flush().map_err(write_error)
    }
}

impl Sink for FileSink {
    fn run(&mut self, sql: &str) -> DbResult<()> {
        writeln!(self.0, "{sql};").map_err(write_error)
    }

    fn comment(&mut self, text: &str) -> DbResult<()> {
        writeln!(self.0, "-- {text}").map_err(write_error)
    }
}

fn write_error(e: std::io::Error) -> String {
    format!("Error escribiendo fichero: {e}")
}

/// Cabecera de una parte del volcado, para quien lo lea.
fn section(sink: &mut dyn Sink, title: &str) -> DbResult<()> {
    sink.comment("")?;
    sink.comment(&format!(
        "── {title} {}",
        "─".repeat(60usize.saturating_sub(title.len()))
    ))?;
    sink.comment("")
}

/// Crea la tabla en `sink` y copia sus datos, bloque a bloque. Devuelve las
/// filas copiadas y lo que queda por ejecutar cuando estén todas las tablas.
fn export_table(
    source: &dyn Source,
    sink: &mut dyn Sink,
    table: &str,
    job: &Job,
) -> DbResult<(u64, Vec<String>)> {
    let options = job.options;
    let script = source.script(table, options)?;
    // Sin nada que escribir ahora (una vista, que va al final), sin cabecera
    if !script.create.is_empty() || script.data.is_some() || !script.after_data.is_empty() {
        section(sink, &format!("Tabla: {table}"))?;
    }
    sink.run_all(&script.create)?;

    let mut total_rows = 0;
    if let Some(data) = script.data.filter(|_| options.include_data) {
        let engine = source.engine();
        let mut header = None;
        let mut write_batch = |columns: &[String], rows: Vec<Vec<String>>| {
            // Una tabla grande son muchos bloques: se puede cancelar entre ellos
            if job.cancelled() {
                return Err("Exportación cancelada.".to_string());
            }
            let header = header.get_or_insert_with(|| {
                let columns: Vec<String> = columns.iter().map(|c| engine.quote_ident(c)).collect();
                format!(
                    "{} ({}) {}",
                    data.insert_into,
                    columns.join(", "),
                    data.values_keyword
                )
            });
            let values: Vec<String> = rows
                .iter()
                .map(|row| format!("({})", row.join(", ")))
                .collect();
            sink.run(&format!("{header}\n{}", values.join(",\n")))
                .map_err(|e| format!("INSERT: {e}"))?;
            total_rows += rows.len() as u64;
            job.send(ExportEvent::RowsCopied(rows.len() as u64));
            Ok(())
        };
        source.read_rows(&data.select, options.batch_size.max(1), &mut write_batch)?;
    }
    sink.run_all(&script.after_data)?;
    Ok((total_rows, script.finish))
}

/// Exporta todas las tablas del trabajo a un mismo destino.
fn export_into(source: &dyn Source, sink: &mut dyn Sink, job: &Job) -> DbResult<Totals> {
    sink.run_all(&source.prelude())?;
    if job.options.drop_if_exists {
        // Si algo del destino que no se exporta depende de ellas (una clave
        // foránea, una vista), el servidor se niega: no se fuerza, porque
        // eso lo eliminaría sin avisar
        let drops = source.drop_together(job.tables)?;
        sink.run_all(&drops).map_err(|e| {
            format!(
                "No se pudieron eliminar del destino las tablas que ya existen: {e}\n\n\
                 Si otras tablas o vistas del destino dependen de ellas, expórtalas también \
                 o elimínalas antes."
            )
        })?;
    }

    // Las tablas con algo pendiente para el final no se dan por hechas
    // hasta entonces: ahí también pueden fallar
    let mut pending: Vec<(String, u64, Vec<String>)> = Vec::new();
    let (mut done, mut total_rows) = (0, 0);
    for table in job.tables {
        if job.cancelled() {
            return Ok((done, total_rows));
        }
        job.send(ExportEvent::TableStarted(table.clone()));
        let table = table.clone();
        match export_table(source, sink, &table, job) {
            Ok((rows, finish)) if finish.is_empty() => {
                done += 1;
                total_rows += rows;
                job.send(ExportEvent::TableDone { table, rows });
            }
            Ok((rows, finish)) => pending.push((table, rows, finish)),
            Err(error) => job.send(ExportEvent::TableError { table, error }),
        }
    }
    if !pending.is_empty() {
        section(sink, "Vistas y claves foráneas")?;
    }
    for (table, rows, finish) in pending {
        match sink.run_all(&finish) {
            Ok(()) => {
                done += 1;
                total_rows += rows;
                job.send(ExportEvent::TableDone { table, rows });
            }
            Err(error) => job.send(ExportEvent::TableError { table, error }),
        }
    }
    Ok((done, total_rows))
}

fn export_to_file(source: &dyn Source, path: &str, job: &Job) -> DbResult<Totals> {
    let mut sink = FileSink::create(Path::new(path), job.source_db)?;
    let totals = export_into(source, &mut sink, job)?;
    sink.finish()?;
    Ok(totals)
}

/// Un fichero por tabla, cada uno con todo lo suyo (también lo que en un
/// único destino se deja para el final).
fn export_to_folder(source: &dyn Source, folder: &str, job: &Job) -> DbResult<Totals> {
    std::fs::create_dir_all(folder)
        .map_err(|e| format!("No se pudo crear la carpeta '{folder}': {e}"))?;
    Ok(job.for_each_table(|table| {
        let path = Path::new(folder).join(format!("{}.sql", file_name_for(table)));
        let mut sink = FileSink::create(&path, job.source_db)?;
        sink.run_all(&source.prelude())?;
        if job.options.drop_if_exists {
            sink.run_all(&source.drop_together(&[table.to_string()])?)?;
        }
        let (rows, finish) = export_table(source, &mut sink, table, job)?;
        sink.run_all(&finish)?;
        sink.finish()?;
        Ok(rows)
    }))
}

/// Nombre de fichero para una tabla, válido en cualquier sistema: sin
/// separadores de ruta (una tabla «../x» escribiría fuera de la carpeta) ni
/// caracteres o nombres que Windows no admite, y sin empezar por punto.
fn file_name_for(table: &str) -> String {
    let name: String = table
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    const WINDOWS_RESERVED: &[&str] = &[
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    if name.is_empty()
        || name.starts_with('.')
        || WINDOWS_RESERVED.contains(&name.to_ascii_uppercase().as_str())
    {
        format!("_{name}")
    } else {
        name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nombres_de_fichero_seguros() {
        assert_eq!(file_name_for("clientes"), "clientes");
        assert_eq!(file_name_for("../x"), "_.._x");
        assert_eq!(file_name_for("a:b*c"), "a_b_c");
        assert_eq!(file_name_for("con"), "_con");
        assert_eq!(file_name_for("año"), "año");
        // Una tabla de otro esquema de PostgreSQL
        assert_eq!(file_name_for("ventas.pedidos"), "ventas.pedidos");
        assert_eq!(file_name_for("\"a.b\".t"), "_a.b_.t");
    }
}

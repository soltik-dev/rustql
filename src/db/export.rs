//! Table export: to a .sql file, to a folder (one file per table), to
//! another database on the same server or to another server.
//!
//! Each engine knows how to describe its tables as statements (`Source`,
//! in export/mysql.rs and export/postgres.rs); what they share lives here:
//! where those statements go (`Sink`), the order they run in and the
//! progress.

mod mysql;
mod postgres;

use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};

use super::{ConnectionParams, DbResult, Engine};

// ── Public types ───────────────────────────────────────────────────────────────

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
    /// Rows copied since the previous notification: tables are copied in
    /// batches, and a big one takes a while.
    RowsCopied(u64),
    /// Fatal error that aborts the whole export (connection, file, etc.).
    Failed(String),
}

// ── Public function ────────────────────────────────────────────────────────────

/// Runs the export on a separate thread and returns a Receiver of progress events.
/// It does not take the main backend: it opens its own connection with `params`.
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

// ── Shared by every engine ─────────────────────────────────────────────────────

/// Tables exported and rows copied.
type Totals = (usize, u64);

const SAME_TARGET_ERROR: &str = "El destino es la misma base de datos del mismo servidor que el origen: \
     la copia borraría o duplicaría sus datos. Elige otra base de datos destino.";

/// What gets exported and how it is counted.
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

    /// Walks the tables reporting progress; `export` exports one and returns
    /// its rows. Stops when cancelled.
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

/// What has to run, and when, to recreate a table.
#[derive(Default)]
struct TableScript {
    /// Before its data: create it, with whatever it needs (schema, types…).
    create: Vec<String>,
    /// Its data; `None` if it has none (a view).
    data: Option<TableData>,
    /// After its data: keys, indexes and the value of its sequences.
    after_data: Vec<String>,
    /// Once every table exists: whatever depends on others (foreign keys,
    /// views).
    finish: Vec<String>,
}

struct TableData {
    /// Query that reads the rows at the source.
    select: String,
    /// `INSERT INTO table`, without columns or values.
    insert_into: String,
    /// What goes between the column list and the values list.
    values_keyword: &'static str,
}

/// Source of an export: a database of a specific engine.
trait Source {
    fn engine(&self) -> Engine;
    /// Statements that get a session (or a file) ready for loading.
    fn prelude(&self) -> Vec<String>;
    /// Statements that drop `tables` at the destination before creating them,
    /// if the engine needs to drop them together. Otherwise each table is
    /// dropped in its own `TableScript::create`.
    fn drop_together(&self, tables: &[String]) -> DbResult<Vec<String>>;
    fn script(&self, table: &str, options: &ExportOptions) -> DbResult<TableScript>;
    /// Reads the rows of `select` without loading them all into memory: hands
    /// them to `each` in batches of up to `batch`, with the column names and
    /// every value already written as a SQL literal. If `each` fails, stops
    /// reading and returns its error.
    fn read_rows(&self, select: &str, batch: usize, each: &mut RowBatch<'_>) -> DbResult<()>;
}

/// What each batch of rows receives: columns and values as literals.
type RowBatch<'a> = dyn FnMut(&[String], Vec<Vec<String>>) -> DbResult<()> + 'a;

/// Destination of the statements: a file or a connection.
trait Sink {
    fn run(&mut self, sql: &str) -> DbResult<()>;

    /// Text for whoever reads the destination; a connection does not need it.
    fn comment(&mut self, _text: &str) -> DbResult<()> {
        Ok(())
    }

    fn run_all(&mut self, statements: &[String]) -> DbResult<()> {
        statements.iter().try_for_each(|sql| self.run(sql))
    }
}

/// .sql file: every statement, ending in a semicolon.
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

/// Header of a part of the dump, for whoever reads it.
fn section(sink: &mut dyn Sink, title: &str) -> DbResult<()> {
    sink.comment("")?;
    sink.comment(&format!(
        "── {title} {}",
        "─".repeat(60usize.saturating_sub(title.len()))
    ))?;
    sink.comment("")
}

/// Creates the table in `sink` and copies its data, batch by batch. Returns
/// the rows copied and what is left to run once every table exists.
fn export_table(
    source: &dyn Source,
    sink: &mut dyn Sink,
    table: &str,
    job: &Job,
) -> DbResult<(u64, Vec<String>)> {
    let options = job.options;
    let script = source.script(table, options)?;
    // Nothing to write now (a view, which goes at the end): no header
    if !script.create.is_empty() || script.data.is_some() || !script.after_data.is_empty() {
        section(sink, &format!("Tabla: {table}"))?;
    }
    sink.run_all(&script.create)?;

    let mut total_rows = 0;
    if let Some(data) = script.data.filter(|_| options.include_data) {
        let engine = source.engine();
        let mut header = None;
        let mut write_batch = |columns: &[String], rows: Vec<Vec<String>>| {
            // A big table is many batches: it can be cancelled between them
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

/// Exports every table of the job to a single destination.
fn export_into(source: &dyn Source, sink: &mut dyn Sink, job: &Job) -> DbResult<Totals> {
    sink.run_all(&source.prelude())?;
    if job.options.drop_if_exists {
        // If something at the destination that is not being exported depends
        // on them (a foreign key, a view), the server refuses: it is not
        // forced, because that would drop it without warning
        let drops = source.drop_together(job.tables)?;
        sink.run_all(&drops).map_err(|e| {
            format!(
                "No se pudieron eliminar del destino las tablas que ya existen: {e}\n\n\
                 Si otras tablas o vistas del destino dependen de ellas, expórtalas también \
                 o elimínalas antes."
            )
        })?;
    }

    // Tables with something pending for the end are not counted as done
    // until then: they can still fail there
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

/// One file per table, each with everything it needs (including what a
/// single destination leaves for the end).
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

/// File name for a table, valid on any system: no path separators (a
/// table "../x" would write outside the folder), no characters or names
/// that Windows rejects, and not starting with a dot.
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
        // A table of another PostgreSQL schema
        assert_eq!(file_name_for("ventas.pedidos"), "ventas.pedidos");
        assert_eq!(file_name_for("\"a.b\".t"), "_a.b_.t");
    }
}

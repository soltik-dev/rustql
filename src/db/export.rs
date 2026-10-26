use std::io::{BufWriter, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};

use mysql::PooledConn;
use mysql::prelude::Queryable;

use super::DbBackend;
use super::mysql::MysqlBackend;
use super::{ConnectionParams, escape_sql, qualified, quote_ident};

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
        run_export(params, source_db, tables, destination, options, cancel, tx);
    });
    rx
}

// ── Implementación interna ─────────────────────────────────────────────────────

fn run_export(
    params: ConnectionParams,
    source_db: String,
    tables: Vec<String>,
    destination: ExportDestination,
    options: ExportOptions,
    cancel: Arc<AtomicBool>,
    tx: Sender<ExportEvent>,
) {
    let start = std::time::Instant::now();

    if let ExportDestination::SameServerDb { target_db } = &destination
        && *target_db == source_db
    {
        let _ = tx.send(ExportEvent::Failed(SAME_TARGET_ERROR.into()));
        return;
    }

    // Conectar a la fuente
    let backend = match MysqlBackend::connect_with_progress(&params, |_| {}) {
        Ok(b) => b,
        Err(e) => {
            let _ = tx.send(ExportEvent::Failed(format!("Conexión al origen: {e}")));
            return;
        }
    };

    let result = match destination {
        ExportDestination::SqlFile { ref path } => {
            export_to_file(&backend, &source_db, &tables, &options, path, &cancel, &tx)
        }
        ExportDestination::Folder { ref path } => {
            export_to_folder(&backend, &source_db, &tables, &options, path, &cancel, &tx)
        }
        ExportDestination::SameServerDb { ref target_db } => export_to_same_server_db(
            &backend, &source_db, &tables, &options, target_db, &cancel, &tx,
        ),
        ExportDestination::RemoteServer {
            ref params,
            ref target_db,
        } => {
            let dest_backend = match MysqlBackend::connect_with_progress(params, |_| {}) {
                Ok(b) => b,
                Err(e) => {
                    let _ = tx.send(ExportEvent::Failed(format!("Conexión al destino: {e}")));
                    return;
                }
            };
            // Otro perfil (u otro nombre de host) puede llevar al mismo servidor:
            // copiar una BBDD sobre sí misma borra (DROP) o duplica sus datos.
            if *target_db == source_db {
                match same_server(&backend, &dest_backend) {
                    Ok(false) => {}
                    Ok(true) => {
                        let _ = tx.send(ExportEvent::Failed(SAME_TARGET_ERROR.into()));
                        return;
                    }
                    Err(e) => {
                        let _ = tx.send(ExportEvent::Failed(format!(
                            "No se pudo comprobar si el destino es el mismo servidor: {e}"
                        )));
                        return;
                    }
                }
            }
            export_to_remote(
                &backend,
                &dest_backend,
                &source_db,
                &tables,
                &options,
                target_db,
                &cancel,
                &tx,
            )
        }
    };

    if cancel.load(Ordering::Relaxed) {
        let _ = tx.send(ExportEvent::Cancelled);
        return;
    }

    match result {
        Ok((total_tables, total_rows)) => {
            let elapsed_ms = start.elapsed().as_millis() as u64;
            let _ = tx.send(ExportEvent::Finished {
                total_tables,
                total_rows,
                elapsed_ms,
            });
        }
        Err(e) => {
            let _ = tx.send(ExportEvent::Failed(e));
        }
    }
}

/// Recorre `tables` avisando del progreso; `export` exporta una y devuelve
/// sus filas. Se detiene al cancelar. Devuelve (tablas exportadas, filas).
fn for_each_table(
    tables: &[String],
    cancel: &AtomicBool,
    tx: &Sender<ExportEvent>,
    mut export: impl FnMut(&str) -> Result<u64, String>,
) -> (usize, u64) {
    let mut total_rows = 0u64;
    let mut done = 0usize;
    for table in tables {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let _ = tx.send(ExportEvent::TableStarted(table.clone()));
        let table = table.clone();
        match export(&table) {
            Ok(rows) => {
                total_rows += rows;
                done += 1;
                let _ = tx.send(ExportEvent::TableDone { table, rows });
            }
            Err(error) => {
                let _ = tx.send(ExportEvent::TableError { table, error });
            }
        }
    }
    (done, total_rows)
}

fn write_error(e: std::io::Error) -> String {
    format!("Error escribiendo fichero: {e}")
}

// ── Exportar a un único fichero .sql ──────────────────────────────────────────

fn export_to_file(
    backend: &MysqlBackend,
    source_db: &str,
    tables: &[String],
    options: &ExportOptions,
    path: &str,
    cancel: &Arc<AtomicBool>,
    tx: &Sender<ExportEvent>,
) -> Result<(usize, u64), String> {
    let file = std::fs::File::create(path)
        .map_err(|e| format!("No se pudo crear el fichero '{path}': {e}"))?;
    let mut out = BufWriter::new(file);
    write_sql_header(&mut out, source_db)?;
    let totals = for_each_table(tables, cancel, tx, |table| {
        export_table_to_writer(&mut out, backend, source_db, table, options)
    });
    out.flush().map_err(write_error)?;
    Ok(totals)
}

fn export_to_folder(
    backend: &MysqlBackend,
    source_db: &str,
    tables: &[String],
    options: &ExportOptions,
    folder: &str,
    cancel: &Arc<AtomicBool>,
    tx: &Sender<ExportEvent>,
) -> Result<(usize, u64), String> {
    std::fs::create_dir_all(folder)
        .map_err(|e| format!("No se pudo crear la carpeta '{folder}': {e}"))?;
    Ok(for_each_table(tables, cancel, tx, |table| {
        let file_path = format!("{folder}/{table}.sql");
        let file = std::fs::File::create(&file_path)
            .map_err(|e| format!("No se pudo crear '{file_path}': {e}"))?;
        let mut out = BufWriter::new(file);
        write_sql_header(&mut out, source_db)?;
        let rows = export_table_to_writer(&mut out, backend, source_db, table, options)?;
        out.flush().map_err(write_error)?;
        Ok(rows)
    }))
}

// ── Exportar a otra BBDD en el mismo servidor ──────────────────────────────────

fn export_to_same_server_db(
    backend: &MysqlBackend,
    source_db: &str,
    tables: &[String],
    options: &ExportOptions,
    target_db: &str,
    cancel: &Arc<AtomicBool>,
    tx: &Sender<ExportEvent>,
) -> Result<(usize, u64), String> {
    // Crear la BBDD destino si no existe
    let create_db_sql = format!("CREATE DATABASE IF NOT EXISTS {}", quote_ident(target_db));
    backend
        .execute_query(&create_db_sql)
        .map_err(|e| format!("No se pudo crear la base de datos '{target_db}': {e}"))?;

    // Una única conexión para toda la copia: FOREIGN_KEY_CHECKS es de sesión
    // (el pool puede resetearla entre queries) y sin ella el orden de las
    // tablas con claves foráneas hace fallar CREATE/INSERT.
    let mut conn = backend.conn()?;
    conn.query_drop("SET FOREIGN_KEY_CHECKS = 0")
        .map_err(|e| format!("SET FOREIGN_KEY_CHECKS: {e}"))?;

    Ok(for_each_table(tables, cancel, tx, |table| {
        copy_table_same_server(&mut conn, source_db, table, target_db, options)
    }))
}

fn copy_table_same_server(
    conn: &mut PooledConn,
    source_db: &str,
    table: &str,
    target_db: &str,
    options: &ExportOptions,
) -> Result<u64, String> {
    let src = qualified(source_db, table);
    let dst = qualified(target_db, table);

    if options.drop_if_exists {
        conn.query_drop(format!("DROP TABLE IF EXISTS {dst}"))
            .map_err(|e| format!("DROP TABLE: {e}"))?;
    }

    if options.include_create {
        conn.query_drop(format!("CREATE TABLE {dst} LIKE {src}"))
            .map_err(|e| format!("CREATE TABLE LIKE: {e}"))?;
    }

    if !options.include_data {
        return Ok(0);
    }

    conn.query_drop(format!("INSERT INTO {dst} SELECT * FROM {src}"))
        .map_err(|e| format!("INSERT INTO ... SELECT: {e}"))?;
    Ok(conn.affected_rows())
}

// ── Exportar a otro servidor ───────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn export_to_remote(
    src_backend: &MysqlBackend,
    dst_backend: &MysqlBackend,
    source_db: &str,
    tables: &[String],
    options: &ExportOptions,
    target_db: &str,
    cancel: &Arc<AtomicBool>,
    tx: &Sender<ExportEvent>,
) -> Result<(usize, u64), String> {
    // Crear BBDD destino si no existe
    dst_backend
        .execute_query(&format!(
            "CREATE DATABASE IF NOT EXISTS {}",
            quote_ident(target_db)
        ))
        .map_err(|e| format!("No se pudo crear la BBDD destino '{target_db}': {e}"))?;

    // Sesión estable en destino con FOREIGN_KEY_CHECKS desactivado (ver
    // export_to_same_server_db).
    let mut dst_conn = dst_backend.conn()?;
    dst_conn
        .query_drop("SET FOREIGN_KEY_CHECKS = 0")
        .map_err(|e| format!("SET FOREIGN_KEY_CHECKS: {e}"))?;

    Ok(for_each_table(tables, cancel, tx, |table| {
        copy_table_remote(
            src_backend,
            &mut dst_conn,
            source_db,
            table,
            target_db,
            options,
        )
    }))
}

fn copy_table_remote(
    src: &MysqlBackend,
    dst_conn: &mut PooledConn,
    source_db: &str,
    table: &str,
    target_db: &str,
    options: &ExportOptions,
) -> Result<u64, String> {
    let dst_ref = qualified(target_db, table);

    if options.drop_if_exists {
        dst_conn
            .query_drop(format!("DROP TABLE IF EXISTS {dst_ref}"))
            .map_err(|e| format!("DROP TABLE: {e}"))?;
    }

    if options.include_create {
        let ddl = get_create_table_ddl(src, source_db, table)?;
        // Sustituir referencia de BBDD en el DDL y ejecutar en destino
        let dst_ddl = ddl.replace(&quote_ident(table), &qualified(target_db, table));
        dst_conn
            .query_drop(dst_ddl)
            .map_err(|e| format!("CREATE TABLE en destino: {e}"))?;
    }

    if !options.include_data {
        return Ok(0);
    }

    // SELECT en origen, INSERT en destino en batches
    let data = src
        .execute_query(&format!("SELECT * FROM {}", qualified(source_db, table)))
        .map_err(|e| format!("SELECT: {e}"))?;

    if data.rows.is_empty() {
        return Ok(0);
    }

    let col_list = data
        .columns
        .iter()
        .map(|c| quote_ident(c))
        .collect::<Vec<_>>()
        .join(", ");

    let total_rows = data.rows.len() as u64;

    for batch in data.rows.chunks(options.batch_size) {
        let values_list = batch
            .iter()
            .map(|row| {
                let vals = row
                    .iter()
                    .map(|v| match v {
                        None => "NULL".to_string(),
                        Some(s) => format!("'{}'", escape_sql(s)),
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("({vals})")
            })
            .collect::<Vec<_>>()
            .join(", ");

        let insert_sql = format!("INSERT INTO {dst_ref} ({col_list}) VALUES {values_list}");
        dst_conn
            .query_drop(insert_sql)
            .map_err(|e| format!("INSERT batch: {e}"))?;
    }

    Ok(total_rows)
}

// ── Helpers ───────────────────────────────────────────────────────────────────

const SAME_TARGET_ERROR: &str = "El destino es la misma base de datos del mismo servidor que el origen: \
     la copia borraría o duplicaría sus datos. Elige otra base de datos destino.";

/// Si dos conexiones llegan al mismo servidor, según el propio servidor
/// (host, puerto y directorio de datos), no según los parámetros usados.
fn same_server(a: &MysqlBackend, b: &MysqlBackend) -> Result<bool, String> {
    let identity = |backend: &MysqlBackend| {
        backend
            .execute_query("SELECT @@hostname, @@port, @@datadir")
            .map(|r| r.rows.into_iter().next())
    };
    let (id_a, id_b) = (identity(a)?, identity(b)?);
    Ok(id_a.is_some() && id_a == id_b)
}

/// Obtiene el DDL de una tabla con SHOW CREATE TABLE.
fn get_create_table_ddl(
    backend: &MysqlBackend,
    source_db: &str,
    table: &str,
) -> Result<String, String> {
    let result = backend
        .execute_query(&format!(
            "SHOW CREATE TABLE {}",
            qualified(source_db, table)
        ))
        .map_err(|e| format!("SHOW CREATE TABLE: {e}"))?;

    result
        .rows
        .first()
        .and_then(|r| r.get(1))
        .and_then(|v| v.clone())
        .ok_or_else(|| format!("No se obtuvo DDL para la tabla '{table}'"))
}

/// Escribe el encabezado estándar de un fichero SQL de exportación.
fn write_sql_header(file: &mut impl Write, source_db: &str) -> Result<(), String> {
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    writeln!(file, "-- RustQL Export — Base de datos: {source_db}").map_err(write_error)?;
    writeln!(file, "-- Generado: {now}").map_err(write_error)?;
    writeln!(
        file,
        "-- ──────────────────────────────────────────────────────────────────────\n"
    )
    .map_err(write_error)?;
    writeln!(file, "SET NAMES utf8mb4;").map_err(write_error)?;
    writeln!(file, "SET FOREIGN_KEY_CHECKS = 0;\n").map_err(write_error)?;
    Ok(())
}

/// Exporta una tabla al writer proporcionado generando CREATE TABLE + INSERTs.
fn export_table_to_writer(
    file: &mut impl Write,
    backend: &MysqlBackend,
    source_db: &str,
    table: &str,
    options: &ExportOptions,
) -> Result<u64, String> {
    writeln!(
        file,
        "\n-- ── Tabla: `{table}` ─────────────────────────────────────────────────────\n"
    )
    .map_err(write_error)?;

    if options.drop_if_exists {
        writeln!(file, "DROP TABLE IF EXISTS {};", quote_ident(table)).map_err(write_error)?;
    }

    if options.include_create {
        let ddl = get_create_table_ddl(backend, source_db, table)?;
        writeln!(file, "{ddl};\n").map_err(write_error)?;
    }

    if !options.include_data {
        return Ok(0);
    }

    let data = backend
        .execute_query(&format!("SELECT * FROM {}", qualified(source_db, table)))
        .map_err(|e| format!("SELECT: {e}"))?;

    if data.rows.is_empty() {
        return Ok(0);
    }

    let col_list = data
        .columns
        .iter()
        .map(|c| quote_ident(c))
        .collect::<Vec<_>>()
        .join(", ");

    let total_rows = data.rows.len() as u64;

    for batch in data.rows.chunks(options.batch_size) {
        let values_list = batch
            .iter()
            .map(|row| {
                let vals = row
                    .iter()
                    .map(|v| match v {
                        None => "NULL".to_string(),
                        Some(s) => format!("'{}'", escape_sql(s)),
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("({vals})")
            })
            .collect::<Vec<_>>()
            .join(",\n");

        writeln!(
            file,
            "INSERT INTO {} ({col_list}) VALUES\n{values_list};",
            quote_ident(table)
        )
        .map_err(write_error)?;
    }

    Ok(total_rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_server;

    #[test]
    #[ignore = "necesita RUSTQL_TEST_MYSQL"]
    fn no_copia_una_bbdd_sobre_si_misma() {
        let b = test_server::backend();
        b.execute_write("DROP DATABASE IF EXISTS rustql_test_export")
            .unwrap();
        b.execute_write("CREATE DATABASE rustql_test_export")
            .unwrap();
        b.execute_write("CREATE TABLE rustql_test_export.t (id INT PRIMARY KEY)")
            .unwrap();
        b.execute_write("INSERT INTO rustql_test_export.t VALUES (1)")
            .unwrap();

        // Mismo servidor a través de un perfil «remoto», con DROP IF EXISTS
        let rx = spawn_export(
            test_server::params(),
            "rustql_test_export".into(),
            vec!["t".into()],
            ExportDestination::RemoteServer {
                params: test_server::params(),
                target_db: "rustql_test_export".into(),
            },
            ExportOptions {
                drop_if_exists: true,
                ..Default::default()
            },
            Arc::new(AtomicBool::new(false)),
        );
        let events: Vec<ExportEvent> = rx.iter().collect();
        let rows = b
            .execute_query("SELECT COUNT(*) FROM rustql_test_export.t")
            .map(|r| r.rows[0][0].clone());
        b.execute_write("DROP DATABASE rustql_test_export").unwrap();

        assert!(
            matches!(events.as_slice(), [ExportEvent::Failed(e)] if e == SAME_TARGET_ERROR),
            "debe abortar antes de tocar nada"
        );
        assert_eq!(
            rows,
            Ok(Some("1".into())),
            "la tabla de origen sigue intacta"
        );
    }

    #[test]
    #[ignore = "necesita RUSTQL_TEST_MYSQL"]
    fn misma_conexion_es_el_mismo_servidor() {
        let (a, b) = (test_server::backend(), test_server::backend());
        assert!(same_server(&a, &b).unwrap());
    }

    #[test]
    #[ignore = "necesita RUSTQL_TEST_MYSQL"]
    fn el_fichero_sql_se_puede_volver_a_cargar() {
        use mysql::prelude::Queryable;

        let b = test_server::backend();
        for sql in [
            "DROP DATABASE IF EXISTS rustql_test_dump",
            "DROP DATABASE IF EXISTS rustql_test_dump2",
            "CREATE DATABASE rustql_test_dump",
            "CREATE DATABASE rustql_test_dump2",
            "CREATE TABLE rustql_test_dump.t (id INT PRIMARY KEY, v TEXT)",
            "INSERT INTO rustql_test_dump.t VALUES \
             (1, 'it''s'), (2, 'barra \\\\ y salto\\n'), (3, NULL), (4, '')",
        ] {
            b.execute_write(sql).unwrap();
        }
        let path = std::env::temp_dir().join(format!("rustql-test-{}.sql", std::process::id()));
        let rx = spawn_export(
            test_server::params(),
            "rustql_test_dump".into(),
            vec!["t".into()],
            ExportDestination::SqlFile {
                path: path.to_string_lossy().into_owned(),
            },
            ExportOptions {
                batch_size: 2,
                ..Default::default()
            },
            Arc::new(AtomicBool::new(false)),
        );
        let events: Vec<ExportEvent> = rx.iter().collect();
        let dump = std::fs::read_to_string(&path).unwrap();
        let _ = std::fs::remove_file(&path);

        // Cargarlo en otra BBDD, como haría el cliente mysql
        let mut conn = b.conn().unwrap();
        let loaded = conn.query_drop(format!("USE rustql_test_dump2; {dump}"));
        let select = "SELECT id, v FROM t ORDER BY id";
        let copy: Vec<(i32, Option<String>)> = conn.query(select).unwrap();
        let mut conn = b.conn().unwrap();
        conn.query_drop("USE rustql_test_dump").unwrap();
        let source: Vec<(i32, Option<String>)> = conn.query(select).unwrap();
        b.execute_write("DROP DATABASE rustql_test_dump").unwrap();
        b.execute_write("DROP DATABASE rustql_test_dump2").unwrap();

        assert!(
            matches!(
                events.last(),
                Some(ExportEvent::Finished { total_rows: 4, .. })
            ),
            "{:?}",
            events
                .iter()
                .map(|e| match e {
                    ExportEvent::Failed(e) | ExportEvent::TableError { error: e, .. } => e.clone(),
                    _ => String::new(),
                })
                .collect::<Vec<_>>()
        );
        loaded.unwrap();
        assert_eq!(copy, source);
        assert_eq!(source[1].1.as_deref(), Some("barra \\ y salto\n"));
    }
}

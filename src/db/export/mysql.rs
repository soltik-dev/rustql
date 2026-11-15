//! Exportación desde MySQL / MariaDB. La estructura de cada tabla es la que
//! da `SHOW CREATE TABLE`, que ya trae sus claves e índices.

use std::cell::RefCell;

use mysql::PooledConn;
use mysql::prelude::Queryable;

use super::{
    ExportDestination, ExportEvent, ExportOptions, Job, RowBatch, SAME_TARGET_ERROR, Sink, Source,
    TableData, TableScript, Totals, export_into, export_to_file, export_to_folder,
};
use crate::db::mysql::{MysqlBackend, literal, qualified, quote_ident, sql_literal};
use crate::db::{ConnectionParams, DbBackend, DbResult, Engine};

pub(super) fn run(
    params: &ConnectionParams,
    destination: &ExportDestination,
    job: &Job,
) -> DbResult<Totals> {
    if let ExportDestination::SameServerDb { target_db } = destination
        && target_db == job.source_db
    {
        return Err(SAME_TARGET_ERROR.into());
    }
    let backend = MysqlBackend::connect_with_progress(params, |_| {})
        .map_err(|e| format!("Conexión al origen: {e}"))?;
    let source = MysqlSource::open(&backend, job.source_db)?;
    match destination {
        ExportDestination::SqlFile { path } => export_to_file(&source, path, job),
        ExportDestination::Folder { path } => export_to_folder(&source, path, job),
        ExportDestination::SameServerDb { target_db } => {
            copy_within_server(&source, target_db, job)
        }
        ExportDestination::RemoteServer { params, target_db } => {
            let target = MysqlBackend::connect_with_progress(params, |_| {})
                .map_err(|e| format!("Conexión al destino: {e}"))?;
            // Otro perfil (u otro nombre de host) puede llevar al mismo servidor:
            // copiar una BBDD sobre sí misma borra (DROP) o duplica sus datos.
            if target_db == job.source_db
                && same_server(&backend, &target).map_err(|e| {
                    format!("No se pudo comprobar si el destino es el mismo servidor: {e}")
                })?
            {
                return Err(SAME_TARGET_ERROR.into());
            }
            let mut sink = ConnSink::open(&target, target_db)?;
            export_into(&source, &mut sink, job)
        }
    }
}

struct MysqlSource<'a> {
    backend: &'a MysqlBackend,
    db: &'a str,
    /// Conexión con la BBDD de origen en uso, para pedir el DDL: el de una
    /// vista nombra la vista y sus tablas con su BBDD salvo que sea la que
    /// está en uso, y así crearía la vista en el origen y no en el destino.
    ddl_conn: RefCell<PooledConn>,
}

impl<'a> MysqlSource<'a> {
    fn open(backend: &'a MysqlBackend, db: &'a str) -> DbResult<Self> {
        let mut conn = backend.conn()?;
        conn.query_drop(format!("USE {}", quote_ident(db)))
            .map_err(|e| format!("Conexión al origen: {e}"))?;
        Ok(Self {
            backend,
            db,
            ddl_conn: RefCell::new(conn),
        })
    }

    fn is_view(&self, table: &str) -> DbResult<bool> {
        let sql = format!(
            "SELECT TABLE_TYPE FROM information_schema.TABLES \
             WHERE TABLE_SCHEMA = {} AND TABLE_NAME = {}",
            literal(self.db),
            literal(table)
        );
        let result = self.backend.execute_query(None, &sql)?;
        Ok(result.rows.first().and_then(|r| r[0].as_deref()) == Some("VIEW"))
    }

    /// DDL de una tabla o de una vista, como lo da `SHOW CREATE TABLE`.
    fn ddl(&self, table: &str) -> DbResult<String> {
        let sql = format!("SHOW CREATE TABLE {}", quote_ident(table));
        let row: Option<mysql::Row> = self
            .ddl_conn
            .borrow_mut()
            .query_first(sql)
            .map_err(|e| format!("SHOW CREATE TABLE: {e}"))?;
        row.and_then(|row| row.get::<String, _>(1))
            .ok_or_else(|| format!("No se obtuvo DDL para la tabla '{table}'"))
    }
}

impl Source for MysqlSource<'_> {
    fn engine(&self) -> Engine {
        Engine::Mysql
    }

    /// Sin comprobar las claves foráneas: el orden de las tablas no importa.
    fn prelude(&self) -> Vec<String> {
        vec![
            "SET NAMES utf8mb4".into(),
            "SET FOREIGN_KEY_CHECKS = 0".into(),
        ]
    }

    fn drop_together(&self, _: &[String]) -> DbResult<Vec<String>> {
        Ok(Vec::new())
    }

    fn script(&self, table: &str, options: &ExportOptions) -> DbResult<TableScript> {
        let name = quote_ident(table);
        let mut script = TableScript::default();
        // Una vista no tiene datos propios (insertar en ella duplicaría los
        // de sus tablas) y se crea al final, cuando ya existen
        if self.is_view(table)? {
            if options.drop_if_exists {
                script.finish.push(format!("DROP VIEW IF EXISTS {name}"));
            }
            if options.include_create {
                script.finish.push(self.ddl(table)?);
            }
            return Ok(script);
        }
        if options.drop_if_exists {
            script.create.push(format!("DROP TABLE IF EXISTS {name}"));
        }
        if options.include_create {
            script.create.push(self.ddl(table)?);
        }
        script.data = Some(TableData {
            select: format!("SELECT * FROM {}", qualified(self.db, table)),
            insert_into: format!("INSERT INTO {name}"),
            values_keyword: "VALUES",
        });
        Ok(script)
    }

    fn read_rows(&self, select: &str, batch: usize, each: &mut RowBatch<'_>) -> DbResult<()> {
        // En su propia conexión, ocupada mientras dure la lectura: el crate
        // va leyendo las filas del servidor según se le piden
        let mut conn = self.backend.conn()?;
        let mut set = conn
            .query_iter(select)
            .map_err(|e| format!("SELECT: {e}"))?;
        let columns: Vec<String> = set
            .columns()
            .as_ref()
            .iter()
            .map(|c| c.name_str().into_owned())
            .collect();
        let mut rows = Vec::with_capacity(batch);
        for row in set.by_ref() {
            let mut row = row.map_err(|e| format!("SELECT: {e}"))?;
            rows.push(
                (0..row.len())
                    .map(|i| sql_literal(row.take(i).unwrap_or(mysql::Value::NULL)))
                    .collect(),
            );
            if rows.len() == batch {
                each(&columns, std::mem::take(&mut rows))?;
            }
        }
        if !rows.is_empty() {
            each(&columns, rows)?;
        }
        Ok(())
    }
}

/// Conexión al servidor de destino, con la BBDD de destino en uso. Una sola
/// para toda la copia: FOREIGN_KEY_CHECKS es de sesión, y el pool la
/// resetea al devolverla.
struct ConnSink(PooledConn);

impl ConnSink {
    fn open(backend: &MysqlBackend, target_db: &str) -> DbResult<Self> {
        let mut conn = backend.conn()?;
        let target = quote_ident(target_db);
        conn.query_drop(format!("CREATE DATABASE IF NOT EXISTS {target}"))
            .and_then(|()| conn.query_drop(format!("USE {target}")))
            .map_err(|e| format!("No se pudo crear la BBDD destino '{target_db}': {e}"))?;
        Ok(Self(conn))
    }
}

impl Sink for ConnSink {
    fn run(&mut self, sql: &str) -> DbResult<()> {
        self.0.query_drop(sql).map_err(|e| e.to_string())
    }
}

/// A otra BBDD del mismo servidor: las tablas las copia el propio servidor,
/// sin que los datos pasen por aquí. Las vistas se crean después, con su
/// definición.
fn copy_within_server(source: &MysqlSource, target_db: &str, job: &Job) -> DbResult<Totals> {
    let backend = source.backend;
    let mut conn = backend.conn()?;
    conn.query_drop(format!(
        "CREATE DATABASE IF NOT EXISTS {}",
        quote_ident(target_db)
    ))
    .map_err(|e| format!("No se pudo crear la base de datos '{target_db}': {e}"))?;
    // Sin ello, el orden de las tablas con claves foráneas hace fallar
    // CREATE e INSERT
    conn.query_drop("SET FOREIGN_KEY_CHECKS = 0")
        .map_err(|e| format!("SET FOREIGN_KEY_CHECKS: {e}"))?;

    // Las vistas, al final: dependen de las tablas
    let mut views = Vec::new();
    let mut tables = Vec::new();
    for table in job.tables {
        match source.is_view(table) {
            Ok(true) => views.push(table.clone()),
            _ => tables.push(table.clone()),
        }
    }
    let first_view = tables.len();
    tables.append(&mut views);
    let ordered = Job {
        tables: &tables,
        source_db: job.source_db,
        options: job.options,
        cancel: job.cancel,
        tx: job.tx,
    };

    let options = job.options;
    let mut view_sink = None;
    let mut position = 0;
    Ok(ordered.for_each_table(|table| {
        position += 1;
        if position > first_view {
            // En una conexión con la BBDD de destino en uso
            if view_sink.is_none() {
                view_sink = Some(ConnSink::open(backend, target_db)?);
            }
            let sink = view_sink.as_mut().expect("recién abierta");
            sink.run_all(&source.script(table, options)?.finish)?;
            return Ok(0);
        }
        let src = qualified(job.source_db, table);
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
        job.send(ExportEvent::RowsCopied(conn.affected_rows()));
        Ok(conn.affected_rows())
    }))
}

/// Si dos conexiones llegan al mismo servidor, según el propio servidor
/// (host, puerto y directorio de datos), no según los parámetros usados.
fn same_server(a: &MysqlBackend, b: &MysqlBackend) -> DbResult<bool> {
    let identity = |backend: &MysqlBackend| {
        backend
            .execute_query(None, "SELECT @@hostname, @@port, @@datadir")
            .map(|r| r.rows.into_iter().next())
    };
    let (id_a, id_b) = (identity(a)?, identity(b)?);
    Ok(id_a.is_some() && id_a == id_b)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    use super::super::spawn_export;
    use super::*;
    use crate::db::test_server;

    #[test]
    #[ignore = "necesita RUSTQL_TEST_MYSQL"]
    fn no_copia_una_bbdd_sobre_si_misma() {
        let b = test_server::backend();
        b.execute_write(None, "DROP DATABASE IF EXISTS rustql_test_export")
            .unwrap();
        b.execute_write(None, "CREATE DATABASE rustql_test_export")
            .unwrap();
        b.execute_write(
            None,
            "CREATE TABLE rustql_test_export.t (id INT PRIMARY KEY)",
        )
        .unwrap();
        b.execute_write(None, "INSERT INTO rustql_test_export.t VALUES (1)")
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
            .execute_query(None, "SELECT COUNT(*) FROM rustql_test_export.t")
            .map(|r| r.rows[0][0].clone());
        b.execute_write(None, "DROP DATABASE rustql_test_export")
            .unwrap();

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
            b.execute_write(None, sql).unwrap();
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
        b.execute_write(None, "DROP DATABASE rustql_test_dump")
            .unwrap();
        b.execute_write(None, "DROP DATABASE rustql_test_dump2")
            .unwrap();

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

    #[test]
    #[ignore = "necesita RUSTQL_TEST_MYSQL"]
    fn a_otro_servidor_con_claves_foraneas_y_vistas() {
        let (db, copy) = ("rustql_test_remote", "rustql_test_remote2");
        let b = test_server::backend();
        for sql in [
            "DROP DATABASE IF EXISTS rustql_test_remote",
            "DROP DATABASE IF EXISTS rustql_test_remote2",
            "DROP DATABASE IF EXISTS rustql_test_remote3",
            "CREATE DATABASE rustql_test_remote",
            "CREATE TABLE rustql_test_remote.clientes (id INT AUTO_INCREMENT PRIMARY KEY, n TEXT)",
            "CREATE TABLE rustql_test_remote.pedidos (id INT PRIMARY KEY, cliente_id INT NOT NULL, \
             FOREIGN KEY (cliente_id) REFERENCES clientes (id))",
            "CREATE VIEW rustql_test_remote.resumen AS \
             SELECT c.n, COUNT(p.id) AS pedidos FROM rustql_test_remote.clientes c \
             LEFT JOIN rustql_test_remote.pedidos p ON p.cliente_id = c.id GROUP BY c.id",
            "INSERT INTO rustql_test_remote.clientes (n) VALUES ('a'), ('b')",
            "INSERT INTO rustql_test_remote.pedidos VALUES (1, 1), (2, 1), (3, 2)",
        ] {
            b.execute_write(None, sql).unwrap();
        }
        // La vista y la tabla hija, antes que aquello de lo que dependen
        let tables = ["resumen", "pedidos", "clientes"];
        let run_to = |destination, options: ExportOptions| -> Vec<ExportEvent> {
            let rx = spawn_export(
                test_server::params(),
                db.into(),
                tables.iter().map(|t| t.to_string()).collect(),
                destination,
                options,
                Arc::new(AtomicBool::new(false)),
            );
            rx.iter().collect()
        };
        let errors = |events: &[ExportEvent]| -> Vec<String> {
            events
                .iter()
                .filter_map(|e| match e {
                    ExportEvent::Failed(e) | ExportEvent::TableError { error: e, .. } => {
                        Some(e.clone())
                    }
                    _ => None,
                })
                .collect()
        };
        let run = |options| {
            let remote = ExportDestination::RemoteServer {
                params: test_server::params(),
                target_db: copy.into(),
            };
            run_to(remote, options)
        };
        // La BBDD de destino no existe: se crea. La segunda vez, eliminando
        // lo que hay
        let first = run(ExportOptions::default());
        let again = run(ExportOptions {
            drop_if_exists: true,
            ..Default::default()
        });
        // Dentro del mismo servidor las tablas las copia el servidor, y las
        // vistas se crean igual
        let within = run_to(
            ExportDestination::SameServerDb {
                target_db: "rustql_test_remote3".into(),
            },
            ExportOptions::default(),
        );
        let within_view = b.execute_query(
            None,
            "SELECT n, pedidos FROM rustql_test_remote3.resumen ORDER BY n",
        );
        let within_kind = b.execute_query(
            None,
            "SELECT TABLE_TYPE FROM information_schema.TABLES \
             WHERE TABLE_SCHEMA = 'rustql_test_remote3' AND TABLE_NAME = 'resumen'",
        );
        let read = |sql: &str| b.execute_query(None, sql).map(|r| r.rows);
        // La vista se crea como vista: sus filas no se insertan en las tablas
        let rows = read("SELECT COUNT(*) FROM rustql_test_remote2.clientes");
        let view = read("SELECT n, pedidos FROM rustql_test_remote2.resumen ORDER BY n");
        let kind = read(
            "SELECT TABLE_TYPE FROM information_schema.TABLES \
             WHERE TABLE_SCHEMA = 'rustql_test_remote2' ORDER BY TABLE_NAME",
        );
        // La copia conserva la clave foránea y el contador
        let orphan = b.execute_write(
            None,
            "INSERT INTO rustql_test_remote2.pedidos VALUES (9, 99)",
        );
        b.execute_write(
            None,
            "INSERT INTO rustql_test_remote2.clientes (n) VALUES ('c')",
        )
        .unwrap();
        let next = read("SELECT MAX(id) FROM rustql_test_remote2.clientes");
        b.execute_write(None, "DROP DATABASE rustql_test_remote")
            .unwrap();
        b.execute_write(None, "DROP DATABASE rustql_test_remote2")
            .unwrap();

        b.execute_write(None, "DROP DATABASE rustql_test_remote3")
            .unwrap();

        for events in [&first, &again, &within] {
            assert!(
                matches!(
                    events.last(),
                    Some(ExportEvent::Finished {
                        total_tables: 3,
                        total_rows: 5,
                        ..
                    })
                ) && errors(events).is_empty(),
                "{:?}",
                errors(events)
            );
        }
        let text = |values: &[&str]| -> Vec<Option<String>> {
            values.iter().map(|v| Some(v.to_string())).collect()
        };
        assert_eq!(rows.unwrap(), [text(&["2"])]);
        assert_eq!(view.unwrap(), [text(&["a", "2"]), text(&["b", "1"])]);
        assert_eq!(
            kind.unwrap(),
            [
                text(&["BASE TABLE"]),
                text(&["BASE TABLE"]),
                text(&["VIEW"])
            ]
        );
        assert!(orphan.is_err(), "la clave foránea sigue en la copia");
        assert_eq!(
            within_view.unwrap().rows,
            [text(&["a", "2"]), text(&["b", "1"])]
        );
        assert_eq!(within_kind.unwrap().rows, [text(&["VIEW"])]);
        assert_eq!(next.unwrap(), [text(&["3"])]);
    }

    #[test]
    #[ignore = "necesita RUSTQL_TEST_MYSQL"]
    fn una_tabla_grande_por_bloques_y_los_binarios_intactos() {
        let (db, copy) = ("rustql_test_chunks", "rustql_test_chunks2");
        let b = test_server::backend();
        // 4096 filas; el binario no es texto válido en ninguna
        let rows: Vec<String> = (1..=4096)
            .map(|n| format!("({n}, 'fila {n}', UNHEX(CONCAT('00FF80', HEX({n}))))"))
            .collect();
        let insert = format!(
            "INSERT INTO rustql_test_chunks.grande VALUES {}",
            rows.join(", ")
        );
        for sql in [
            "DROP DATABASE IF EXISTS rustql_test_chunks",
            "DROP DATABASE IF EXISTS rustql_test_chunks2",
            "CREATE DATABASE rustql_test_chunks",
            "CREATE TABLE rustql_test_chunks.grande (id INT PRIMARY KEY, v TEXT, bin BLOB)",
            insert.as_str(),
            "INSERT INTO rustql_test_chunks.grande VALUES \
             (5000, 'it''s \\\\ \\n', ''), (5001, NULL, NULL)",
        ] {
            b.execute_write(None, sql).unwrap();
        }
        // A otro servidor: las filas pasan por aquí, bloque a bloque
        let rx = spawn_export(
            test_server::params(),
            db.into(),
            vec!["grande".into()],
            ExportDestination::RemoteServer {
                params: test_server::params(),
                target_db: copy.into(),
            },
            ExportOptions {
                batch_size: 500,
                ..Default::default()
            },
            Arc::new(AtomicBool::new(false)),
        );
        let events: Vec<ExportEvent> = rx.iter().collect();
        let fingerprint = |db: &str| {
            let sql = format!(
                "SELECT COUNT(*), SUM(CRC32(CONCAT_WS('|', id, IFNULL(v, 'NULL'), \
                 IFNULL(HEX(bin), 'NULL')))) FROM {db}.grande"
            );
            b.execute_query(None, &sql).map(|r| r.rows)
        };
        let (original, copied) = (fingerprint(db), fingerprint(copy));
        b.execute_write(None, "DROP DATABASE rustql_test_chunks")
            .unwrap();
        b.execute_write(None, "DROP DATABASE rustql_test_chunks2")
            .unwrap();

        let batches: Vec<u64> = events
            .iter()
            .filter_map(|e| match e {
                ExportEvent::RowsCopied(rows) => Some(*rows),
                _ => None,
            })
            .collect();
        assert_eq!(batches, [500, 500, 500, 500, 500, 500, 500, 500, 98]);
        assert!(
            matches!(
                events.last(),
                Some(ExportEvent::Finished {
                    total_rows: 4098,
                    ..
                })
            ),
            "{:?}",
            events
                .iter()
                .filter_map(|e| match e {
                    ExportEvent::Failed(e) | ExportEvent::TableError { error: e, .. } =>
                        Some(e.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
        );
        let original = original.unwrap();
        assert_eq!(original[0][0], Some("4098".to_string()));
        assert_eq!(copied.unwrap(), original);
    }
}

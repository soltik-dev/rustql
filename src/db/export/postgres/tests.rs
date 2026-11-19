//! Against a real server (`RUSTQL_TEST_POSTGRES`). Each test creates and
//! drops its own `rustql_test_*` databases.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use super::super::{ExportEvent, spawn_export};
use super::*;
use crate::db::test_server::{pg_backend, pg_params};

/// Tables of the test database, with the child before the parent: foreign
/// keys must not depend on the order.
const TABLES: [&str; 5] = [
    "ventas.pedidos",
    "clientes",
    "productos",
    "clientes_activos",
    "\"es.quema\".notas",
];

const SCHEMA: &str = "
    CREATE TYPE estado AS ENUM ('pendiente', 'enviado', 'con''comilla');
    CREATE TABLE clientes (
        id serial PRIMARY KEY,
        nombre varchar(50) NOT NULL,
        email text UNIQUE,
        activo boolean NOT NULL DEFAULT true,
        etiquetas text[],
        datos jsonb,
        foto bytea
    );
    CREATE INDEX clientes_nombre ON clientes (lower(nombre)) WHERE activo;
    CREATE TABLE productos (
        id int GENERATED ALWAYS AS IDENTITY (START WITH 100 INCREMENT BY 5) PRIMARY KEY,
        nombre text NOT NULL COLLATE \"C\",
        precio numeric(10,2) CHECK (precio >= 0),
        con_iva numeric GENERATED ALWAYS AS (precio * 1.21) STORED
    );
    CREATE SCHEMA ventas;
    CREATE TABLE ventas.pedidos (
        id bigserial PRIMARY KEY,
        cliente_id int NOT NULL REFERENCES clientes (id) ON DELETE CASCADE,
        producto_id int REFERENCES productos (id),
        fase estado NOT NULL DEFAULT 'pendiente',
        fases estado[],
        creado date DEFAULT CURRENT_DATE
    );
    CREATE VIEW clientes_activos AS
        SELECT c.id, c.nombre, count(p.id) AS pedidos
        FROM clientes c LEFT JOIN ventas.pedidos p ON p.cliente_id = c.id
        WHERE c.activo GROUP BY c.id;
    CREATE SCHEMA \"es.quema\";
    CREATE UNLOGGED TABLE \"es.quema\".notas (\"Clave Rara\" text PRIMARY KEY, v text);

    INSERT INTO clientes (nombre, email, etiquetas, datos, foto) VALUES
        ('Ana', 'ana@example.com', '{vip,\"con coma, y \\\"comillas\\\"\"}', '{\"puntos\": 120}', '\\x00ff10'),
        ('it''s \\ barra', NULL, NULL, NULL, NULL),
        ('Línea
nueva', 'l@example.com', '{}', '[1, 2]', '\\x');
    INSERT INTO productos (nombre, precio) VALUES ('Teclado', 49.90), ('Ratón', 19.50);
    INSERT INTO ventas.pedidos (cliente_id, producto_id, fase, fases) VALUES
        (1, 100, 'enviado', '{pendiente,enviado}'), (3, 105, 'con''comilla', NULL), (1, NULL, DEFAULT, '{}');
    INSERT INTO \"es.quema\".notas VALUES ('a', 'uno'), ('b', NULL);
";

/// Backend with database `db` freshly created with the test schema, and
/// the `others` databases dropped.
fn source_database(db: &str, others: &[&str]) -> PostgresBackend {
    let b = pg_backend();
    for name in others.iter().chain([&db]) {
        b.drop_database(name).ok();
    }
    b.create_database(db, "").unwrap();
    b.execute_write(Some(db), SCHEMA).unwrap();
    b
}

fn export(
    source_db: &str,
    tables: &[&str],
    destination: ExportDestination,
    options: ExportOptions,
) -> Vec<ExportEvent> {
    let rx = spawn_export(
        pg_params(),
        source_db.into(),
        tables.iter().map(|t| t.to_string()).collect(),
        destination,
        options,
        Arc::new(AtomicBool::new(false)),
    );
    rx.iter().collect()
}

/// The errors of an export, to show them if a test fails.
fn errors(events: &[ExportEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|e| match e {
            ExportEvent::Failed(error) => Some(error.clone()),
            ExportEvent::TableError { table, error } => Some(format!("{table}: {error}")),
            _ => None,
        })
        .collect()
}

fn finished(events: &[ExportEvent]) -> Option<(usize, u64)> {
    match events.last() {
        Some(ExportEvent::Finished {
            total_tables,
            total_rows,
            ..
        }) if errors(events).is_empty() => Some((*total_tables, *total_rows)),
        _ => None,
    }
}

/// What must be the same at the source and in the copy: rows, structure,
/// keys, indexes and where the sequences are.
fn fingerprint(b: &PostgresBackend, db: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut query = |title: &str, sql: &str| {
        lines.push(format!("── {title}"));
        let result = b.execute_query(Some(db), sql).unwrap();
        lines.extend(result.rows.iter().map(|row| format!("{row:?}")));
    };
    for table in [
        "clientes",
        "productos",
        "ventas.pedidos",
        "\"es.quema\".notas",
    ] {
        query(table, &format!("SELECT * FROM {table} ORDER BY 1"));
    }
    query("vista", "SELECT * FROM clientes_activos ORDER BY 1");
    query(
        "columnas",
        "SELECT c.table_schema, c.table_name, c.column_name, c.data_type, c.udt_name, \
                c.is_nullable, c.column_default, c.is_identity, c.identity_generation, \
                c.is_generated, c.generation_expression, c.collation_name \
         FROM information_schema.columns c \
         WHERE c.table_schema NOT IN ('pg_catalog', 'information_schema') \
         ORDER BY 1, 2, c.ordinal_position",
    );
    query(
        "restricciones",
        "SELECT conrelid::regclass::text, conname, pg_get_constraintdef(oid) FROM pg_constraint \
         WHERE connamespace NOT IN ('pg_catalog'::regnamespace, 'information_schema'::regnamespace) \
           AND contype <> 'n' ORDER BY 1, 2",
    );
    query(
        "índices",
        "SELECT indexdef FROM pg_indexes \
         WHERE schemaname NOT IN ('pg_catalog', 'information_schema') ORDER BY 1",
    );
    query(
        "tablas",
        "SELECT c.oid::regclass::text, c.relkind, c.relpersistence FROM pg_class c \
         JOIN pg_namespace n ON n.oid = c.relnamespace \
         WHERE n.nspname NOT IN ('pg_catalog', 'information_schema') AND n.nspname !~ '^pg_' \
           AND c.relkind IN ('r', 'v') ORDER BY 1",
    );
    // The next value each sequence will return
    query(
        "secuencias",
        "SELECT schemaname, sequencename, last_value, increment_by FROM pg_sequences ORDER BY 1, 2",
    );
    query(
        "enumerados",
        "SELECT t.typname, e.enumlabel FROM pg_enum e JOIN pg_type t ON t.oid = e.enumtypid \
         ORDER BY 1, e.enumsortorder",
    );
    lines
}

/// Compares two fingerprints, showing only what differs.
#[track_caller]
fn assert_same(copy: &[String], original: &[String]) {
    let missing: Vec<_> = original.iter().filter(|l| !copy.contains(l)).collect();
    let extra: Vec<_> = copy.iter().filter(|l| !original.contains(l)).collect();
    assert!(
        missing.is_empty() && extra.is_empty(),
        "la copia no es igual.\nFalta:\n{missing:#?}\nSobra:\n{extra:#?}"
    );
    assert_eq!(copy, original, "mismas líneas en otro orden");
}

#[test]
#[ignore = "necesita RUSTQL_TEST_POSTGRES"]
fn el_fichero_sql_recrea_la_base_de_datos() {
    let (db, copy) = ("rustql_test_dump_pg", "rustql_test_dump_pg2");
    let b = source_database(db, &[copy]);
    b.create_database(copy, "").unwrap();
    let path = std::env::temp_dir().join(format!("rustql-test-pg-{}.sql", std::process::id()));
    let events = export(
        db,
        &TABLES,
        ExportDestination::SqlFile {
            path: path.to_string_lossy().into_owned(),
        },
        ExportOptions {
            batch_size: 2,
            ..Default::default()
        },
    );
    let dump = std::fs::read_to_string(&path).unwrap();
    let _ = std::fs::remove_file(&path);

    // Load it into another database, as psql would
    let loaded = b.execute_write(Some(copy), &dump);
    let (original, restored) = (fingerprint(&b, db), fingerprint(&b, copy));
    // The copy keeps numbering where the original was
    let next = b.execute_query(
        Some(copy),
        "INSERT INTO clientes (nombre) VALUES ('nuevo') RETURNING id; \
         INSERT INTO productos (nombre) VALUES ('otro');
         SELECT max(id) FROM productos",
    );
    let next_identity = b.execute_query(Some(copy), "SELECT max(id) FROM productos");
    b.drop_database(db).unwrap();
    b.drop_database(copy).unwrap();

    assert_eq!(finished(&events), Some((5, 10)), "{:?}", errors(&events));
    loaded.unwrap_or_else(|e| panic!("{e}\n\n{dump}"));
    assert_same(&restored, &original);
    assert!(original.iter().any(|l| l.contains("FOREIGN KEY")));
    assert_eq!(next.unwrap().rows, [[Some("4".to_string())]]);
    assert_eq!(next_identity.unwrap().rows, [[Some("110".to_string())]]);
}

#[test]
#[ignore = "necesita RUSTQL_TEST_POSTGRES"]
fn copia_a_otra_bbdd_y_se_puede_repetir() {
    let (db, copy) = ("rustql_test_copy_pg", "rustql_test_copy_pg2");
    let b = source_database(db, &[copy]);
    let to_copy = || ExportDestination::SameServerDb {
        target_db: copy.into(),
    };

    // The destination database does not exist: it is created
    let first = export(db, &TABLES, to_copy(), ExportOptions::default());
    let after_first = fingerprint(&b, copy);
    // Without dropping what is there, the second time the tables already exist
    let clash = export(db, &TABLES, to_copy(), ExportOptions::default());
    // Dropping it, it can be repeated even with foreign keys between them
    let again = export(
        db,
        &TABLES,
        to_copy(),
        ExportOptions {
            drop_if_exists: true,
            ..Default::default()
        },
    );
    let after_again = fingerprint(&b, copy);
    // A table that other non-exported tables depend on is not dropped:
    // the server refuses and it is not forced
    let with_drop = || ExportOptions {
        drop_if_exists: true,
        ..Default::default()
    };
    let blocked = export(db, &["productos"], to_copy(), with_drop());
    // With the ones that depend on it, it is. First only the structure…
    let related = ["productos", "ventas.pedidos", "clientes_activos"];
    let empty = export(
        db,
        &related,
        to_copy(),
        ExportOptions {
            include_data: false,
            ..with_drop()
        },
    );
    let empty_rows = b.execute_query(Some(copy), "SELECT count(*) FROM ventas.pedidos");
    // …and then only the data, over the tables already created
    let data_only = export(
        db,
        &related,
        to_copy(),
        ExportOptions {
            include_create: false,
            ..Default::default()
        },
    );
    let after_data_only = fingerprint(&b, copy);
    let original = fingerprint(&b, db);
    b.drop_database(db).unwrap();
    b.drop_database(copy).unwrap();

    assert_eq!(finished(&first), Some((5, 10)), "{:?}", errors(&first));
    assert_same(&after_first, &original);
    assert_eq!(errors(&clash).len(), 5, "{:?}", errors(&clash));
    assert_eq!(finished(&again), Some((5, 10)), "{:?}", errors(&again));
    assert_same(&after_again, &original);
    let blocked = errors(&blocked);
    assert!(
        matches!(blocked.as_slice(), [e] if e.contains("expórtalas también")),
        "{blocked:?}"
    );
    assert_eq!(finished(&empty), Some((3, 0)), "{:?}", errors(&empty));
    assert_eq!(empty_rows.unwrap().rows, [[Some("0".to_string())]]);
    assert_eq!(
        finished(&data_only),
        Some((3, 5)),
        "{:?}",
        errors(&data_only)
    );
    assert_same(&after_data_only, &original);
}

#[test]
#[ignore = "necesita RUSTQL_TEST_POSTGRES"]
fn a_otro_servidor_y_nunca_sobre_si_misma() {
    let (db, copy) = ("rustql_test_remote_pg", "rustql_test_remote_pg2");
    let b = source_database(db, &[copy]);
    let remote = |target: &str| ExportDestination::RemoteServer {
        params: pg_params(),
        target_db: target.into(),
    };
    let options = ExportOptions {
        drop_if_exists: true,
        ..Default::default()
    };
    // The same server through a "remote" profile, and the same database
    let onto_itself = export(db, &TABLES, remote(db), options.clone());
    let same_server_db = export(
        db,
        &TABLES,
        ExportDestination::SameServerDb {
            target_db: db.into(),
        },
        options.clone(),
    );
    let untouched = b.execute_query(Some(db), "SELECT count(*) FROM clientes");
    let copied = export(db, &TABLES, remote(copy), options);
    let (original, restored) = (fingerprint(&b, db), fingerprint(&b, copy));
    b.drop_database(db).unwrap();
    b.drop_database(copy).unwrap();

    for events in [&onto_itself, &same_server_db] {
        assert!(
            matches!(events.as_slice(), [ExportEvent::Failed(e)] if e == SAME_TARGET_ERROR),
            "debe abortar antes de tocar nada: {:?}",
            errors(events)
        );
    }
    assert_eq!(untouched.unwrap().rows, [[Some("3".to_string())]]);
    assert_eq!(finished(&copied), Some((5, 10)), "{:?}", errors(&copied));
    assert_same(&restored, &original);
}

#[test]
#[ignore = "necesita RUSTQL_TEST_POSTGRES"]
fn una_carpeta_con_un_fichero_por_tabla_y_lo_que_no_se_exporta() {
    let db = "rustql_test_folder_pg";
    let b = source_database(db, &[]);
    b.execute_write(
        Some(db),
        "CREATE TABLE medidas (dia date, v int) PARTITION BY RANGE (dia);
         CREATE TABLE medidas_2026 PARTITION OF medidas
             FOR VALUES FROM ('2026-01-01') TO ('2027-01-01')",
    )
    .unwrap();
    let folder = std::env::temp_dir().join(format!("rustql-test-pg-{}", std::process::id()));
    let events = export(
        db,
        &["ventas.pedidos", "clientes_activos", "medidas", "no_existe"],
        ExportDestination::Folder {
            path: folder.to_string_lossy().into_owned(),
        },
        ExportOptions {
            drop_if_exists: true,
            ..Default::default()
        },
    );
    let read = |name: &str| std::fs::read_to_string(folder.join(name)).unwrap_or_default();
    let (table, view) = (read("ventas.pedidos.sql"), read("clientes_activos.sql"));
    let _ = std::fs::remove_dir_all(&folder);
    b.drop_database(db).unwrap();

    // The two that cannot be exported say so; the rest are exported
    let errors = errors(&events);
    assert_eq!(errors.len(), 2, "{errors:?}");
    assert!(errors[0].starts_with("medidas: ") && errors[0].contains("pg_dump"));
    assert!(errors[1].starts_with("no_existe: "), "{errors:?}");
    assert!(matches!(
        events.last(),
        Some(ExportEvent::Finished {
            total_tables: 2,
            total_rows: 3,
            ..
        })
    ));
    // Each file carries everything it needs, including what goes at the end
    for expected in [
        "DROP TABLE IF EXISTS \"ventas\".\"pedidos\";",
        "CREATE SCHEMA IF NOT EXISTS \"ventas\";",
        "CREATE TYPE public.estado AS ENUM ('pendiente', 'enviado', 'con''comilla');",
        "CREATE TABLE \"ventas\".\"pedidos\" (",
        "INSERT INTO \"ventas\".\"pedidos\" (\"id\", \"cliente_id\"",
        "FOREIGN KEY (cliente_id) REFERENCES public.clientes(id) ON DELETE CASCADE;",
        "pg_get_serial_sequence('\"ventas\".\"pedidos\"', 'id'), 3, true);",
    ] {
        assert!(table.contains(expected), "falta «{expected}» en:\n{table}");
    }
    assert!(view.contains("DROP VIEW IF EXISTS \"public\".\"clientes_activos\";"));
    assert!(view.contains("CREATE VIEW \"public\".\"clientes_activos\" AS"));
    assert!(
        !view.contains("INSERT"),
        "una vista no tiene datos:\n{view}"
    );
}

#[test]
#[ignore = "necesita RUSTQL_TEST_POSTGRES"]
fn una_tabla_grande_se_copia_por_bloques_y_se_puede_cancelar() {
    let (db, copy) = ("rustql_test_chunks_pg", "rustql_test_chunks_pg2");
    let b = pg_backend();
    for name in [db, copy] {
        b.drop_database(name).ok();
        b.create_database(name, "").unwrap();
    }
    b.execute_write(
        Some(db),
        "CREATE TABLE grande (id int PRIMARY KEY, v text);
         INSERT INTO grande SELECT n, repeat('x', 50) || n FROM generate_series(1, 10000) n;
         CREATE TABLE pequena (id int PRIMARY KEY);
         INSERT INTO pequena VALUES (1), (2)",
    )
    .unwrap();
    let target = || ExportDestination::SameServerDb {
        target_db: copy.into(),
    };
    let options = |batch_size| ExportOptions {
        batch_size,
        drop_if_exists: true,
        ..Default::default()
    };
    let batches = |events: &[ExportEvent]| -> Vec<u64> {
        events
            .iter()
            .filter_map(|e| match e {
                ExportEvent::RowsCopied(rows) => Some(*rows),
                _ => None,
            })
            .collect()
    };

    // Whole: in batches of the requested size, and it arrives complete
    let whole = export(db, &["grande", "pequena"], target(), options(1000));
    let copied = b.execute_query(
        Some(copy),
        "SELECT count(*), sum(id), count(DISTINCT v) FROM grande",
    );

    // Cancelled when the first batch arrives: it does not go on to the end
    let cancel = Arc::new(AtomicBool::new(false));
    let rx = spawn_export(
        pg_params(),
        db.into(),
        vec!["grande".into(), "pequena".into()],
        target(),
        options(100),
        Arc::clone(&cancel),
    );
    let mut cancelled = Vec::new();
    for event in rx.iter() {
        if matches!(event, ExportEvent::RowsCopied(_)) {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        cancelled.push(event);
    }

    // If the destination rejects a batch, that table fails (and stops being
    // read at the source) and the others carry on
    b.execute_write(
        Some(copy),
        "DROP TABLE IF EXISTS grande, pequena;
         CREATE TABLE grande (id int PRIMARY KEY CHECK (id < 2500), v text);
         CREATE TABLE pequena (id int PRIMARY KEY)",
    )
    .unwrap();
    let rejected = export(
        db,
        &["grande", "pequena"],
        target(),
        ExportOptions {
            batch_size: 1000,
            include_create: false,
            ..Default::default()
        },
    );
    let after_rejected = b.execute_query(Some(copy), "SELECT count(*) FROM pequena");
    b.drop_database(db).unwrap();
    b.drop_database(copy).unwrap();

    assert_eq!(finished(&whole), Some((2, 10002)), "{:?}", errors(&whole));
    let sizes = batches(&whole);
    assert_eq!(
        sizes.len(),
        11,
        "diez bloques de la grande y uno de la pequeña"
    );
    assert!(sizes[..10].iter().all(|rows| *rows == 1000), "{sizes:?}");
    assert_eq!(
        copied.unwrap().rows,
        [text(&["10000", "50005000", "10000"])]
    );

    assert!(matches!(cancelled.last(), Some(ExportEvent::Cancelled)));
    assert!(batches(&cancelled).len() < 20, "se detuvo enseguida");

    let errors = errors(&rejected);
    assert!(
        matches!(errors.as_slice(), [e] if e.starts_with("grande: INSERT: ")),
        "{errors:?}"
    );
    assert!(matches!(
        rejected.last(),
        Some(ExportEvent::Finished {
            total_tables: 1,
            total_rows: 2,
            ..
        })
    ));
    assert_eq!(after_rejected.unwrap().rows, [text(&["2"])]);
}

fn text(values: &[&str]) -> Vec<Option<String>> {
    values.iter().map(|v| Some(v.to_string())).collect()
}

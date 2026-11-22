//! Against a real server: `RUSTQL_TEST_POSTGRES=user:password@host:port
//! cargo test -- --ignored`. Each test creates and drops its `rustql_test_*` database.

use super::*;
use crate::db::test_server::{names, pg_backend, pg_params};

/// Backend with database `db` freshly created and empty.
fn fresh(db: &str) -> PostgresBackend {
    let b = pg_backend();
    b.execute_write(None, &format!("DROP DATABASE IF EXISTS {db}"))
        .unwrap();
    b.create_database(db, "").unwrap();
    b
}

fn rows(b: &PostgresBackend, db: &str, sql: &str) -> Vec<Vec<Option<String>>> {
    b.execute_query(Some(db), sql).unwrap().rows
}

fn text(values: &[&str]) -> Vec<Option<String>> {
    values.iter().map(|v| Some(v.to_string())).collect()
}

#[test]
#[ignore = "necesita RUSTQL_TEST_POSTGRES"]
fn las_bbdd_se_ven_como_en_mysql_y_los_esquemas_van_en_el_nombre() {
    let db = "rustql_test_tree";
    let b = fresh(db);
    b.execute_write(
        Some(db),
        "CREATE TABLE clientes (id serial PRIMARY KEY, nombre text);
         CREATE VIEW v_clientes AS SELECT * FROM clientes;
         CREATE SCHEMA ventas;
         CREATE TABLE ventas.pedidos (id int PRIMARY KEY);
         CREATE TABLE \"con.punto\" (id int);
         CREATE SCHEMA \"es.quema\";
         CREATE TABLE \"es.quema\".t (id int PRIMARY KEY);
         INSERT INTO clientes (nombre) VALUES ('a'), ('b'), ('c');
         ANALYZE clientes;",
    )
    .unwrap();

    let databases = b.list_databases();
    let tables = b.list_tables(db);
    let all = b.list_all_tables();
    let info = b.get_tables_info(db);
    let server = b.get_databases_info();
    // Tables of other schemas are read with the name the tree gives
    // them, also when the schema has a dot
    let other = b.execute_query(
        Some(db),
        &format!("SELECT * FROM {}", SQL.qualified(db, "ventas.pedidos")),
    );
    let dotted = b.get_columns_info(db, "\"es.quema\".t");
    let dotted_rows = b.execute_query(
        Some(db),
        &format!("SELECT * FROM {}", SQL.qualified(db, "\"es.quema\".t")),
    );
    b.drop_database(db).unwrap();

    let databases = databases.unwrap();
    assert!(databases.contains(&db.to_string()), "{databases:?}");
    // Neither templates nor those that do not allow connections
    assert!(!databases.iter().any(|d| d.starts_with("template")));
    // Those of the default schema, first and without a prefix
    let expected = [
        "clientes",
        "public.con.punto",
        "v_clientes",
        "\"es.quema\".t",
        "ventas.pedidos",
    ];
    let tables = tables.unwrap();
    // The view is told apart from the tables
    let kinds: Vec<TableKind> = tables.iter().map(|t| t.kind).collect();
    assert_eq!(
        kinds,
        [
            TableKind::Table,
            TableKind::Table,
            TableKind::View,
            TableKind::Table,
            TableKind::Table
        ]
    );
    assert_eq!(names(tables), expected);
    let all: Vec<(String, String)> = all
        .unwrap()
        .into_iter()
        .map(|(db, table)| (db, table.name))
        .collect();
    for table in expected {
        assert!(
            all.contains(&(db.to_string(), table.to_string())),
            "{table}"
        );
    }
    let info = info.unwrap();
    assert_eq!(
        info.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
        expected
    );
    assert_eq!((info[0].row_count, info[0].engine.as_str()), (3, "heap"));
    assert!(info[0].size_bytes > 0);
    assert_eq!(other.unwrap().columns, ["id"]);
    assert!(dotted.unwrap()[0].is_primary());
    assert_eq!(dotted_rows.unwrap().columns, ["id"]);

    let server = server.unwrap();
    let this = server.iter().find(|d| d.name == db).expect("en la lista");
    assert_eq!(this.charset, "UTF8");
    assert!(this.size_bytes > 0);
    // Tables are only counted in the database being asked from
    assert_eq!(this.table_count, None);
    assert!(server.iter().any(|d| d.table_count.is_some()));
}

#[test]
#[ignore = "necesita RUSTQL_TEST_POSTGRES"]
fn estructura_de_una_tabla() {
    let db = "rustql_test_columns";
    let b = fresh(db);
    b.execute_write(
        Some(db),
        "CREATE SCHEMA s;
         CREATE TABLE s.t (
            id serial PRIMARY KEY,
            ident int GENERATED ALWAYS AS IDENTITY,
            codigo varchar(10) NOT NULL UNIQUE,
            precio numeric(10,2) DEFAULT 1.5,
            total numeric GENERATED ALWAYS AS (precio * 2) STORED,
            creado timestamptz DEFAULT now(),
            notas text,
            etiquetas text[]
         );
         CREATE INDEX ON s.t (notas);
         ALTER TABLE s.t DROP COLUMN etiquetas;
         CREATE TABLE compuesta (a int, b int, v text, PRIMARY KEY (a, b));",
    )
    .unwrap();
    let columns = b.get_columns_info(db, "s.t");
    let composite = b.get_columns_info(db, "compuesta");
    b.drop_database(db).unwrap();

    let columns = columns.unwrap();
    let by_name = |name: &str| columns.iter().find(|c| c.name == name).unwrap();
    assert_eq!(
        columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
        [
            "id", "ident", "codigo", "precio", "total", "creado", "notas"
        ],
        "en orden y sin la columna eliminada"
    );
    let id = by_name("id");
    assert!(id.is_primary() && id.is_auto && !id.is_nullable);
    assert_eq!(id.col_type, "integer");
    assert!(id.default_val.as_deref().unwrap().starts_with("nextval("));
    let ident = by_name("ident");
    assert!(ident.is_auto && !ident.is_primary());
    assert_eq!(ident.extra, "identity (always)");
    let codigo = by_name("codigo");
    assert_eq!(
        (codigo.col_type.as_str(), codigo.column_key.as_str()),
        ("character varying(10)", "UNI")
    );
    assert!(!codigo.is_nullable && !codigo.is_auto);
    let precio = by_name("precio");
    assert_eq!(precio.col_type, "numeric(10,2)");
    assert_eq!(precio.default_val.as_deref(), Some("1.5"));
    assert!(precio.is_nullable && !precio.is_auto);
    assert_eq!(by_name("total").extra, "generated (stored)");
    assert!(by_name("total").is_auto);
    assert!(
        !by_name("creado").is_auto,
        "un valor por defecto se puede pisar"
    );
    assert_eq!(by_name("notas").column_key, "MUL");

    let composite = composite.unwrap();
    assert_eq!(
        composite
            .iter()
            .map(ColumnInfo::is_primary)
            .collect::<Vec<_>>(),
        [true, true, false]
    );
}

#[test]
#[ignore = "necesita RUSTQL_TEST_POSTGRES"]
fn cualquier_tipo_llega_como_texto() {
    let b = pg_backend();
    let result = b
        .execute_query(
            None,
            "SELECT 42 AS n, 1.50::numeric AS d, true AS b, NULL::int AS nada, \
                    '2026-11-11 10:00:00'::timestamp AS ts, '{\"a\": [1, 2]}'::jsonb AS j, \
                    ARRAY['x', 'y'] AS arr, '\\x00ff'::bytea AS bin, \
                    '6f1c1a1e-0000-4000-8000-000000000001'::uuid AS u, 'ñ€'::text AS t",
        )
        .unwrap();
    assert_eq!(
        result.columns,
        ["n", "d", "b", "nada", "ts", "j", "arr", "bin", "u", "t"]
    );
    assert_eq!(
        result.rows,
        [vec![
            Some("42".to_string()),
            Some("1.50".into()),
            Some("t".into()),
            None,
            Some("2026-11-11 10:00:00".into()),
            Some("{\"a\": [1, 2]}".into()),
            Some("{x,y}".into()),
            Some("\\x00ff".into()),
            Some("6f1c1a1e-0000-4000-8000-000000000001".into()),
            Some("ñ€".into()),
        ]]
    );

    // With no rows, the columns arrive anyway: the grid needs them
    let empty = b
        .execute_query(None, "SELECT 1 AS uno WHERE false")
        .unwrap();
    assert_eq!(empty.columns, ["uno"]);
    assert!(empty.rows.is_empty() && !empty.truncated);
}

#[test]
#[ignore = "necesita RUSTQL_TEST_POSTGRES"]
fn la_transaccion_convierte_los_parametros_al_tipo_de_su_columna() {
    let db = "rustql_test_tx";
    let b = fresh(db);
    b.execute_write(
        Some(db),
        "CREATE TABLE t (id int PRIMARY KEY, activo bool, dia date, precio numeric(8,2), \
                         datos jsonb, ref uuid, bin bytea, v text)",
    )
    .unwrap();
    let target = SQL.qualified(db, "t");
    let columns: Vec<String> = ["id", "activo", "dia", "precio", "datos", "ref", "bin", "v"]
        .iter()
        .map(|c| SQL.quote_ident(c))
        .collect();
    // As the user would type them in the grid: all text
    let insert = |values: &[Option<&str>]| Statement {
        sql: SQL.insert(&target, &columns),
        params: values.iter().map(|v| v.map(String::from)).collect(),
        single_row: false,
    };
    let update = |v: &str, id: &str| Statement {
        sql: format!("UPDATE {target} SET \"v\" = $1 WHERE \"id\" = $2"),
        params: vec![Some(v.into()), Some(id.into())],
        single_row: true,
    };
    b.execute_transaction(
        Some(db),
        &[
            insert(&[
                Some("1"),
                Some("true"),
                Some("2026-11-11"),
                Some("12.5"),
                Some("{\"k\": 1}"),
                Some("6f1c1a1e-0000-4000-8000-000000000001"),
                Some("\\x00ff"),
                Some("it's a \\ test"),
            ]),
            insert(&[Some("2"), None, None, None, None, None, None, None]),
            Statement {
                sql: SQL.insert(&SQL.qualified(db, "t"), &[SQL.quote_ident("id")]),
                params: vec![Some("3".into())],
                single_row: false,
            },
        ],
    )
    .unwrap();

    // A value that is not valid for its column: the server says so and nothing is left
    let bad_type = b.execute_transaction(
        Some(db),
        &[
            update("x", "1"),
            insert(&[
                Some("no es un número"),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            ]),
        ],
    );
    // Row 9 does not exist (someone else deleted it): row 1 is not saved either
    let missing = b.execute_transaction(Some(db), &[update("x", "1"), update("y", "9")]);
    // Leaving a row with the value it already had is not "not found"
    let unchanged = b.execute_transaction(Some(db), &[update("it's a \\ test", "1")]);
    // After the failures, the pool connections are still outside a transaction
    let after = rows(&b, db, "SELECT * FROM t ORDER BY id");
    b.drop_database(db).unwrap();

    let err = bad_type.unwrap_err();
    assert!(
        err.contains("ROLLBACK") && err.contains("no es un número"),
        "{err}"
    );
    let err = missing.unwrap_err();
    assert!(err.contains("found 0"), "{err}");
    unchanged.unwrap();
    assert_eq!(
        after[0],
        text(&[
            "1",
            "t",
            "2026-11-11",
            "12.50",
            "{\"k\": 1}",
            "6f1c1a1e-0000-4000-8000-000000000001",
            "\\x00ff",
            "it's a \\ test",
        ])
    );
    assert_eq!(after[1][0], Some("2".to_string()));
    assert!(after[1][1..].iter().all(Option::is_none));
    assert_eq!(after.len(), 3);
}

#[test]
#[ignore = "necesita RUSTQL_TEST_POSTGRES"]
fn sentencias_del_dialecto_que_cambian_de_un_motor_a_otro() {
    let db = "rustql_test_dialect";
    let b = fresh(db);
    b.execute_write(
        Some(db),
        "CREATE TABLE origen (id serial PRIMARY KEY, v text NOT NULL DEFAULT 'x', n int);
         CREATE INDEX ON origen (n);
         INSERT INTO origen (v, n) VALUES ('Uno', 10), ('dos', 25)",
    )
    .unwrap();
    let (src, dst) = (SQL.qualified(db, "origen"), SQL.qualified(db, "copia"));
    b.execute_write(Some(db), &SQL.create_table_like(&dst, &src))
        .unwrap();
    let copied = b.execute_write(Some(db), &format!("INSERT INTO {dst} SELECT * FROM {src}"));
    // Row with every default
    b.execute_transaction(
        Some(db),
        &[Statement {
            sql: SQL.insert(&src, &[]),
            params: Vec::new(),
            single_row: false,
        }],
    )
    .unwrap();
    let structure = b.get_columns_info(db, "copia");
    let defaults = rows(&b, db, "SELECT v, n FROM origen ORDER BY id DESC LIMIT 1");
    // LIKE on a number and case-insensitive
    let like = |column: &str, pattern: &str| {
        let sql = format!(
            "SELECT id FROM {src} WHERE {} ORDER BY id",
            SQL.like(column, pattern)
        );
        rows(&b, db, &sql)
    };
    let (by_number, by_text) = (like("n", "%5%"), like("v", "UN%"));
    // Empty and drop
    b.truncate_table(db, "copia").unwrap();
    let truncated = rows(&b, db, "SELECT count(*) FROM copia");
    b.drop_table(db, "copia", TableKind::Table).unwrap();
    let tables = b.list_tables(db);
    b.drop_database(db).unwrap();

    assert_eq!(copied.unwrap(), 2, "filas afectadas");
    let structure = structure.unwrap();
    assert!(structure[0].is_primary(), "la copia conserva la clave");
    assert_eq!(structure[2].column_key, "MUL", "y los índices");
    assert_eq!(defaults, [vec![Some("x".to_string()), None]]);
    assert_eq!(by_number, [text(&["2"])]);
    assert_eq!(by_text, [text(&["1"])]);
    assert_eq!(truncated, [text(&["0"])]);
    assert_eq!(names(tables.unwrap()), ["origen"]);
}

#[test]
#[ignore = "necesita RUSTQL_TEST_POSTGRES"]
fn un_literal_no_permite_inyectar_se_lean_como_se_lean_las_barras() {
    let b = pg_backend();
    let attack = "\\' OR 1=1 -- ";
    let run = |setting: &str, sql: &str| {
        let cancel = CancelFlag::default();
        b.execute_in_session(
            None,
            &format!("SET standard_conforming_strings = {setting}; {sql}"),
            10,
            &cancel,
        )
        .unwrap()
        .rows
    };
    for setting in ["on", "off"] {
        for value in ["it's", "a\\b", "x\\'y", "línea\nnueva", attack] {
            let back = run(setting, &format!("SELECT {}", SQL.literal(value)));
            assert_eq!(
                back,
                [text(&[value])],
                "standard_conforming_strings={setting}"
            );
        }
        let matches = run(
            setting,
            &format!(
                "SELECT count(*) FROM (VALUES ('a'), ('b')) AS t(v) WHERE v = {}",
                SQL.literal(attack)
            ),
        );
        assert_eq!(matches, [text(&["0"])], "inyección con {setting}");
    }
}

#[test]
#[ignore = "necesita RUSTQL_TEST_POSTGRES"]
fn la_sesion_del_editor_sobrevive_entre_ejecuciones() {
    let db = "rustql_test_session";
    let b = fresh(db);
    b.execute_write(Some(db), "CREATE TABLE t (id int)")
        .unwrap();
    let run = |sql: &str, max_rows: usize| {
        b.execute_in_session(Some(db), sql, max_rows, &CancelFlag::default())
    };
    let count = || rows(&b, db, "SELECT count(*) FROM t");

    // Each line is a separate run (F9)
    run("SET application_name = 'editor'", 100).unwrap();
    run("CREATE TEMP TABLE tmp (v int)", 100).unwrap();
    run("BEGIN", 100).unwrap();
    run("INSERT INTO t VALUES (1), (2), (3), (4), (5)", 100).unwrap();
    let inside = run(
        "SELECT current_setting('application_name'), count(*) FROM t, tmp",
        100,
    );
    let outside = count();
    run("ROLLBACK", 100).unwrap();
    let rolled_back = run("SELECT count(*) FROM t", 100);

    run(
        "BEGIN; INSERT INTO t VALUES (1), (2), (3), (4), (5); COMMIT",
        100,
    )
    .unwrap();
    let committed = count();
    // Row limit, and the session is still usable afterwards
    let capped = run("SELECT * FROM t ORDER BY id", 3);
    let exact = run("SELECT * FROM t", 5);
    let empty = run("SELECT id AS n FROM t WHERE false", 3);
    // With several statements, the result is that of the first one returning rows
    let several = run(
        "SET application_name = 'x'; SELECT 1 AS uno; SELECT 2 AS dos",
        100,
    );
    // An error leaves the session alive and is reported as the server reports it
    let error = run("SELECT * FROM no_existe", 100);
    let alive = run("SELECT count(*) FROM tmp", 100);
    // Another database, another session: here the temporary table does not exist
    let elsewhere = b.execute_in_session(None, "SELECT * FROM tmp", 1, &CancelFlag::default());
    b.drop_database(db).unwrap();

    assert_eq!(inside.unwrap().rows, [text(&["editor", "0"])]);
    assert_eq!(outside, [text(&["0"])], "sin COMMIT, el resto no la ve");
    assert_eq!(rolled_back.unwrap().rows, [text(&["0"])]);
    assert_eq!(committed, [text(&["5"])]);
    let capped = capped.unwrap();
    assert_eq!((capped.rows.len(), capped.truncated), (3, true));
    let exact = exact.unwrap();
    assert_eq!((exact.rows.len(), exact.truncated), (5, false));
    let empty = empty.unwrap();
    assert_eq!(empty.columns, ["n"], "columnas aunque no haya filas");
    assert!(empty.rows.is_empty());
    let several = several.unwrap();
    assert_eq!(
        (several.columns, several.rows),
        (vec!["uno".to_string()], vec![text(&["1"])])
    );
    let error = error.unwrap_err();
    assert!(error.contains("no_existe"), "{error}");
    assert_eq!(alive.unwrap().rows, [text(&["0"])]);
    assert!(elsewhere.is_err());
}

#[test]
#[ignore = "necesita RUSTQL_TEST_POSTGRES"]
fn cancelar_interrumpe_la_consulta_y_conserva_la_sesion() {
    let b = Arc::new(pg_backend());
    let run = |sql: &str, cancel: &CancelFlag| b.execute_in_session(None, sql, 100, cancel);
    run("SET application_name = 'siete'", &CancelFlag::default()).unwrap();

    // A query that takes a minute, cancelled after 300 ms
    let cancel = CancelFlag::default();
    let slow = {
        let (b, cancel) = (Arc::clone(&b), Arc::clone(&cancel));
        std::thread::spawn(move || {
            let start = std::time::Instant::now();
            let result = b.execute_in_session(None, "SELECT pg_sleep(60)", 100, &cancel);
            (result, start.elapsed())
        })
    };
    std::thread::sleep(Duration::from_millis(300));
    b.cancel_in_session(&cancel).unwrap();
    let (result, elapsed) = slow.join().unwrap();
    assert!(elapsed.as_secs() < 10, "tardó {elapsed:?}");
    assert_eq!(result.unwrap_err(), QUERY_CANCELLED);

    // The session is the same and the next query does not inherit the cancellation
    let after = run("SHOW application_name", &CancelFlag::default()).unwrap();
    assert_eq!(after.rows, [text(&["siete"])]);

    // Cancelled before starting, it never reaches the server
    let never = CancelFlag::default();
    b.cancel_in_session(&never).unwrap();
    let skipped = run("SET application_name = 'ocho'", &never);
    assert_eq!(skipped.unwrap_err(), QUERY_CANCELLED);
    let untouched = run("SHOW application_name", &CancelFlag::default()).unwrap();
    assert_eq!(untouched.rows, [text(&["siete"])]);
}

#[test]
#[ignore = "necesita RUSTQL_TEST_POSTGRES"]
fn la_conexion_limitada_a_una_bbdd_solo_ensena_esa() {
    let db = "rustql_test_only";
    let all = fresh(db);
    all.execute_write(Some(db), "CREATE TABLE t (id int)")
        .unwrap();
    let params = ConnectionParams {
        database: format!(" {db} "),
        ..pg_params()
    };
    let only = PostgresBackend::connect_with_progress(&params, |_| {}).unwrap();

    let databases = only.list_databases();
    let tables = only.list_all_tables();
    let info = only.get_databases_info();
    // Without naming a database, it runs in that one
    let unqualified = only.execute_query(None, "SELECT current_database()");
    // The form's pick button lists them all, whether one is set or not
    let picker = crate::db::list_server_databases(&params);
    let missing = PostgresBackend::connect_with_progress(
        &ConnectionParams {
            database: "rustql_test_no_existe".into(),
            ..pg_params()
        },
        |_| {},
    );
    drop(only);
    all.drop_database(db).unwrap();

    assert_eq!(databases.unwrap(), [db]);
    assert_eq!(tables.unwrap(), [(db.to_string(), "t".into())]);
    let info = info.unwrap();
    assert_eq!(info.len(), 1);
    assert_eq!((info[0].name.as_str(), info[0].table_count), (db, Some(1)));
    assert_eq!(unqualified.unwrap().rows, [text(&[db])]);
    let picker = picker.unwrap();
    assert!(picker.contains(&db.to_string()) && picker.contains(&"postgres".to_string()));
    let err = missing.err().expect("no existe");
    assert!(err.contains("rustql_test_no_existe"), "{err}");
}

#[test]
#[ignore = "necesita RUSTQL_TEST_POSTGRES"]
fn errores_de_conexion_legibles() {
    let wrong_password = PostgresBackend::connect_with_progress(
        &ConnectionParams {
            password: "mala".into(),
            ..pg_params()
        },
        |_| {},
    );
    let err = wrong_password.err().expect("wrong password");
    assert!(err.starts_with("Could not connect: "), "{err}");

    let nobody_listening = PostgresBackend::connect_with_progress(
        &ConnectionParams {
            port: 1,
            ..pg_params()
        },
        |_| {},
    );
    // With its cause, not just "error connecting to server"
    let err = nobody_listening.err().expect("nadie escucha");
    assert!(err.to_lowercase().contains("refused"), "{err}");
}

#[test]
#[ignore = "necesita RUSTQL_TEST_POSTGRES"]
fn borrar_una_bbdd_cierra_antes_las_conexiones_propias() {
    let db = "rustql_test_drop";
    let b = fresh(db);
    // Connections open to it: the pool's and the editor's
    b.execute_write(Some(db), "CREATE TABLE t (id int)")
        .unwrap();
    b.list_tables(db).unwrap();
    b.execute_in_session(Some(db), "SELECT 1", 1, &CancelFlag::default())
        .unwrap();

    b.drop_database(db).unwrap();
    assert!(!b.list_databases().unwrap().contains(&db.to_string()));
    // And it can be created again and used with the same name
    b.create_database(db, "").unwrap();
    assert!(b.list_tables(db).unwrap().is_empty());
    b.drop_database(db).unwrap();
}

#[test]
#[ignore = "necesita RUSTQL_TEST_POSTGRES"]
fn la_conexion_va_cifrada_si_el_servidor_lo_ofrece() {
    let b = pg_backend();
    let server_has_tls = b.execute_query(None, "SHOW ssl").unwrap().rows == [text(&["on"])];
    let encrypted = b
        .execute_query(
            None,
            "SELECT ssl FROM pg_stat_ssl WHERE pid = pg_backend_pid()",
        )
        .unwrap()
        .rows;
    // With a self-signed certificate too: it is not checked (like libpq
    // with sslmode=prefer). Without TLS on the server, the connection is plain
    let expected = if server_has_tls { "t" } else { "f" };
    assert_eq!(encrypted, [text(&[expected])]);
}

#[test]
#[ignore = "necesita RUSTQL_TEST_POSTGRES"]
fn una_conexion_caida_en_el_pool_no_se_nota() {
    let b = pg_backend();
    // The connection waiting in the pool, cut from the server side
    let pid = &rows(&b, "postgres", "SELECT pg_backend_pid()")[0][0];
    let pid = pid.as_deref().expect("pid");
    let admin = pg_backend();
    admin
        .execute_query(None, &format!("SELECT pg_terminate_backend({pid})"))
        .unwrap();

    // The following reads go through a new connection, without error
    for _ in 0..3 {
        let result = b.execute_query(Some("postgres"), "SELECT 1").unwrap();
        assert_eq!(result.rows, [text(&["1"])]);
    }
}

#[test]
#[ignore = "necesita RUSTQL_TEST_POSTGRES y RUSTQL_TEST_CA"]
fn niveles_de_seguridad_de_la_conexion() {
    use crate::db::test_server::with_tls;
    let connect =
        |tls, ca| PostgresBackend::connect_with_progress(&with_tls(pg_params(), tls, ca), |_| {});
    let encrypted = |b: &PostgresBackend| {
        let sql = "SELECT ssl FROM pg_stat_ssl WHERE pid = pg_backend_pid()";
        b.execute_query(None, sql).unwrap().rows == [text(&["t"])]
    };

    // Require: encrypted, without checking the certificate (from a private CA)
    let required = connect(TlsMode::Require, false).unwrap();
    assert!(encrypted(&required));
    assert_eq!(required.security(), Security::Encrypted);
    // Verify without saying which authority signs it: it is unknown
    let unknown = connect(TlsMode::Verify, false)
        .err()
        .expect("CA desconocida");
    assert!(unknown.contains("CA file"), "{unknown}");
    // With its CA, the certificate is valid (and is for 127.0.0.1)
    let verified = connect(TlsMode::Verify, true).unwrap();
    assert!(encrypted(&verified));
    // Also the connections opened later: another database, the editor and
    // the one that cancels a query
    verified.list_tables("template1").unwrap();
    let cancel = CancelFlag::default();
    verified
        .execute_in_session(None, "SELECT 1", 1, &cancel)
        .unwrap();
    verified.cancel_in_session(&cancel).unwrap();
}

#[test]
#[ignore = "necesita RUSTQL_TEST_POSTGRES"]
fn vistas_y_vistas_materializadas() {
    let db = "rustql_test_views_backend";
    let b = fresh(db);
    b.execute_write(
        Some(db),
        "CREATE TABLE t (id int PRIMARY KEY, v text);
         INSERT INTO t VALUES (1, 'a'), (2, 'b');
         CREATE SCHEMA informes;
         CREATE VIEW informes.pares AS SELECT id, v FROM t WHERE id % 2 = 0;
         CREATE MATERIALIZED VIEW guardada AS SELECT count(*) AS n FROM t",
    )
    .unwrap();
    let tables = b.list_tables(db).unwrap();
    let view = b.view_definition(db, "informes.pares");
    let materialized = b.view_definition(db, "guardada");
    let not_a_view = b.view_definition(db, "t");
    // What it returns can be run as is to change the view
    let replaced = view
        .clone()
        .and_then(|sql| b.execute_write(Some(db), &sql.replace("= 0", "= 1")));
    let after = b.execute_query(Some(db), "SELECT id FROM informes.pares");
    // Each one is dropped with its own statement
    let as_table = b.drop_table(db, "guardada", TableKind::Table);
    b.drop_table(db, "guardada", TableKind::MaterializedView)
        .unwrap();
    b.drop_table(db, "informes.pares", TableKind::View).unwrap();
    let left = b.list_tables(db);
    b.drop_database(db).unwrap();

    let kind = |name: &str| tables.iter().find(|t| t.name == name).map(|t| t.kind);
    assert_eq!(kind("t"), Some(TableKind::Table));
    assert_eq!(kind("informes.pares"), Some(TableKind::View));
    assert_eq!(kind("guardada"), Some(TableKind::MaterializedView));
    let view = view.unwrap();
    assert!(
        view.starts_with("CREATE OR REPLACE VIEW \"informes\".\"pares\" AS\n"),
        "{view}"
    );
    let materialized = materialized.unwrap();
    assert!(
        materialized.contains("\nCREATE MATERIALIZED VIEW \"public\".\"guardada\" AS\n"),
        "{materialized}"
    );
    assert!(not_a_view.is_err());
    replaced.unwrap();
    assert_eq!(after.unwrap().rows, [text(&["1"])]);
    assert!(as_table.is_err(), "DROP TABLE no elimina una vista");
    assert_eq!(names(left.unwrap()), ["t"]);
}

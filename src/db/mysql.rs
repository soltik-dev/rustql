use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use mysql::{
    Conn, Opts, OptsBuilder, Pool, PoolConstraints, PoolOpts, PooledConn, Value,
    consts::CapabilityFlags,
    prelude::{Protocol, Queryable},
};

use super::{
    CancelFlag, ColumnInfo, ConnectionParams, DatabaseInfo, DbBackend, DbResult, Engine,
    QUERY_CANCELLED, QueryResult, Statement, TableInfo, validate_charset,
};

// El dialecto de este backend, para él y para la exportación
pub(super) fn quote_ident(s: &str) -> String {
    Engine::Mysql.quote_ident(s)
}

pub(super) fn qualified(db: &str, table: &str) -> String {
    Engine::Mysql.qualified(db, table)
}

pub(super) fn literal(s: &str) -> String {
    Engine::Mysql.literal(s)
}

/// ER_QUERY_INTERRUPTED: la sentencia se interrumpió con KILL QUERY.
const ER_QUERY_INTERRUPTED: u16 = 1317;

fn is_interrupted(e: &mysql::Error) -> bool {
    matches!(e, mysql::Error::MySqlError(e) if e.code == ER_QUERY_INTERRUPTED)
}

fn value_to_string(v: Value) -> Option<String> {
    match v {
        Value::NULL => None,
        Value::Bytes(b) => Some(String::from_utf8_lossy(&b).into_owned()),
        Value::Int(i) => Some(i.to_string()),
        Value::UInt(u) => Some(u.to_string()),
        Value::Float(f) => Some(f.to_string()),
        Value::Double(d) => Some(d.to_string()),
        Value::Date(y, m, d, h, min, s, _) => {
            Some(format!("{y}-{m:02}-{d:02} {h:02}:{min:02}:{s:02}"))
        }
        Value::Time(neg, days, h, m, s, _) => {
            let sign = if neg { "-" } else { "" };
            Some(format!("{sign}{days}d {h:02}:{m:02}:{s:02}"))
        }
    }
}

/// Lee el primer conjunto de resultados: sus columnas —que vienen aunque no
/// haya ninguna fila— y, como mucho, `max_rows` filas. Al soltar `set`, el
/// crate lee y descarta lo que quede.
fn read_result<P: Protocol>(
    mut set: mysql::QueryResult<'_, '_, '_, P>,
    max_rows: usize,
) -> mysql::Result<QueryResult> {
    let columns = set
        .columns()
        .as_ref()
        .iter()
        .map(|c| c.name_str().into_owned())
        .collect();
    let mut rows = Vec::new();
    let mut truncated = false;
    for row in set.by_ref() {
        if rows.len() == max_rows {
            truncated = true;
            break;
        }
        rows.push(row_values(row?));
    }
    Ok(QueryResult {
        columns,
        rows,
        truncated,
    })
}

fn row_values(mut row: mysql::Row) -> Vec<Option<String>> {
    (0..row.len())
        .map(|i| {
            let v: Value = row.take(i).unwrap_or(Value::NULL);
            value_to_string(v)
        })
        .collect()
}

fn col(row: &[Option<String>], i: usize) -> String {
    row.get(i)
        .and_then(|v| v.as_deref())
        .unwrap_or("")
        .to_string()
}

/// Almacena el Pool en lugar de una PooledConn individual.
/// El pool detecta conexiones muertas y reconecta de forma transparente
/// al solicitar una nueva conexión mediante get_conn().
pub struct MysqlBackend {
    pool: Pool,
    opts: Opts,
    /// La conexión se limita a esta BBDD: es la única que se lista.
    only_database: Option<String>,
    /// Conexión propia del editor SQL, fuera del pool: el pool resetea cada
    /// conexión al devolverla (COM_RESET_CONNECTION), lo que deshace
    /// transacciones, variables y tablas temporales entre ejecuciones.
    /// Se abre al primer uso.
    session: Arc<Mutex<Option<Conn>>>,
    /// Ejecución del editor que está ahora mismo en el servidor: el id de su
    /// conexión (para KILL QUERY) y su señal de cancelación.
    session_running: Mutex<Option<(u32, CancelFlag)>>,
    /// Se envió un KILL QUERY a la sesión del editor. Si llegó con la
    /// sentencia ya terminada, el servidor puede aplicárselo a la siguiente:
    /// antes de ejecutarla se absorbe con una sentencia vacía.
    session_killed: AtomicBool,
}

impl MysqlBackend {
    pub fn connect_with_progress(
        params: &ConnectionParams,
        on_status: impl Fn(&str),
    ) -> DbResult<Self> {
        if params.keyring_id.is_some() && params.password.is_empty() {
            on_status("Leyendo la contraseña del llavero...");
        }
        let password = params.resolve_password()?;
        on_status("Estableciendo conexión TCP...");
        // min:0 evita crear 10 conexiones en paralelo al arrancar (default min:10).
        // max:4 deja que los trabajos en segundo plano (datos, metadatos,
        // escrituras) no esperen unos a otros; el editor SQL va aparte.
        let pool_constraints = PoolConstraints::new(0, 4).unwrap();
        let pool_opts = PoolOpts::default().with_constraints(pool_constraints);
        let opts: Opts = OptsBuilder::new()
            .ip_or_hostname(Some(params.host.as_str()))
            .tcp_port(params.port)
            .user(Some(params.user.as_str()))
            .pass(Some(password.as_str()))
            // BBDD por defecto de la conexión: si no existe o el usuario no
            // puede entrar, lo dice el servidor al conectar
            .db_name(params.only_database())
            // Filas afectadas = filas encontradas por el WHERE, aunque el
            // UPDATE las deje como estaban: así se distingue «ya tenía ese
            // valor» de «la fila no existe» (ver execute_transaction).
            .additional_capabilities(CapabilityFlags::CLIENT_FOUND_ROWS)
            .pool_opts(pool_opts)
            .into();

        let pool = Pool::new(opts.clone()).map_err(|e| format!("Error de conexión: {e}"))?;

        on_status("Autenticando...");
        pool.get_conn()
            .map_err(|e| format!("No se pudo conectar: {e}"))?;

        on_status("Conexión establecida.");
        Ok(Self {
            pool,
            opts,
            only_database: params.only_database().map(str::to_string),
            session: Arc::default(),
            session_running: Mutex::default(),
            session_killed: AtomicBool::new(false),
        })
    }

    /// Obtiene una conexión del pool. Si la conexión previa había muerto,
    /// el pool crea una nueva automáticamente.
    /// `pub(super)` para que export.rs pueda mantener una sesión estable
    /// (variables como FOREIGN_KEY_CHECKS no sobreviven al pool).
    pub(super) fn conn(&self) -> DbResult<PooledConn> {
        self.pool
            .get_conn()
            .map_err(|e| format!("Sin conexión con el servidor: {e}"))
    }

    /// Lectura con parámetros posicionales (`?`).
    fn query_with(&self, sql: &str, params: Vec<Option<String>>) -> DbResult<QueryResult> {
        let mut conn = self.conn()?;
        conn.exec_iter(sql, params)
            .and_then(|set| read_result(set, usize::MAX))
            .map_err(|e| format!("Error en query: {e}"))
    }
}

impl DbBackend for MysqlBackend {
    fn engine(&self) -> Engine {
        Engine::Mysql
    }

    fn list_databases(&self) -> DbResult<Vec<String>> {
        if let Some(only) = &self.only_database {
            return Ok(vec![only.clone()]);
        }
        self.conn()?
            .query("SHOW DATABASES")
            .map_err(|e| format!("Error en query: {e}"))
    }

    fn list_tables(&self, database: &str) -> DbResult<Vec<String>> {
        let query = format!("SHOW TABLES FROM {}", quote_ident(database));
        self.conn()?
            .query(query)
            .map_err(|e| format!("Error en query: {e}"))
    }

    fn list_all_tables(&self) -> DbResult<Vec<(String, String)>> {
        // Sin BBDD a la que limitarse, el filtro no descarta ninguna
        self.conn()?
            .exec(
                "SELECT TABLE_SCHEMA, TABLE_NAME FROM information_schema.TABLES \
                 WHERE ? IS NULL OR TABLE_SCHEMA = ? \
                 ORDER BY TABLE_SCHEMA, TABLE_NAME",
                (&self.only_database, &self.only_database),
            )
            .map_err(|e| format!("Error en query: {e}"))
    }

    fn execute_query(&self, _db: Option<&str>, sql: &str) -> DbResult<QueryResult> {
        let mut conn = self.conn()?;
        conn.query_iter(sql)
            .and_then(|set| read_result(set, usize::MAX))
            .map_err(|e| format!("Error en query: {e}"))
    }

    fn execute_in_session(
        &self,
        db: Option<&str>,
        sql: &str,
        max_rows: usize,
        cancel: &CancelFlag,
    ) -> DbResult<QueryResult> {
        // El turno se espera aquí: las ejecuciones del editor van de una en una
        let mut session = self.session.lock().unwrap_or_else(PoisonError::into_inner);
        let conn = match &mut *session {
            Some(conn) => conn,
            None => session.insert(
                Conn::new(self.opts.clone())
                    .map_err(|e| format!("No se pudo abrir la sesión del editor: {e}"))?,
            ),
        };
        if self.session_killed.swap(false, Ordering::SeqCst) {
            // Un KILL QUERY que llegó tarde se lo lleva esta sentencia
            let _ = conn.query_drop("DO 0");
        }
        let running = |value| {
            *self
                .session_running
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = value;
        };
        // Primero anunciarse y luego mirar la señal: quien cancela hace lo
        // contrario, así que al menos uno de los dos ve al otro
        running(Some((conn.connection_id(), Arc::clone(cancel))));
        let result = if cancel.load(Ordering::SeqCst) {
            running(None);
            return Err(QUERY_CANCELLED.into());
        } else {
            (|| {
                if let Some(db) = db {
                    conn.query_drop(format!("USE {}", quote_ident(db)))?;
                }
                read_result(conn.query_iter(sql)?, max_rows)
            })()
        };
        running(None);
        match result {
            Ok(data) => Ok(data),
            Err(e) if is_interrupted(&e) => Err(QUERY_CANCELLED.into()),
            Err(e) => {
                // Conexión perdida (servidor reiniciado, timeout…): la próxima
                // ejecución abre otra; el estado de la sesión ya se ha perdido.
                if conn.ping().is_err() {
                    *session = None;
                }
                Err(format!("Error en query: {e}"))
            }
        }
    }

    fn cancel_in_session(&self, cancel: &CancelFlag) -> DbResult<()> {
        cancel.store(true, Ordering::SeqCst);
        let running = self
            .session_running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        // Solo si es la que está en el servidor: KILL QUERY interrumpiría la
        // de otra pestaña. La que espera su turno verá la señal al llegar
        let Some((conn_id, _)) = running.filter(|(_, flag)| Arc::ptr_eq(flag, cancel)) else {
            return Ok(());
        };
        self.session_killed.store(true, Ordering::SeqCst);
        // Por otra conexión: la del editor está ocupada con la consulta
        self.conn()?
            .query_drop(format!("KILL QUERY {conn_id}"))
            .map_err(|e| format!("No se pudo cancelar la consulta: {e}"))
    }

    fn execute_write(&self, _db: Option<&str>, sql: &str) -> DbResult<u64> {
        let mut conn = self.conn()?;
        conn.query_drop(sql)
            .map_err(|e| format!("Error en escritura: {e}"))?;
        Ok(conn.affected_rows())
    }

    fn get_databases_info(&self) -> DbResult<Vec<DatabaseInfo>> {
        let sql = "SELECT s.SCHEMA_NAME, \
                          s.DEFAULT_CHARACTER_SET_NAME, \
                          s.DEFAULT_COLLATION_NAME, \
                          COALESCE(COUNT(t.TABLE_NAME), 0), \
                          COALESCE(SUM(t.DATA_LENGTH + COALESCE(t.INDEX_LENGTH,0)), 0) \
                   FROM information_schema.SCHEMATA s \
                   LEFT JOIN information_schema.TABLES t ON t.TABLE_SCHEMA = s.SCHEMA_NAME \
                   WHERE ? IS NULL OR s.SCHEMA_NAME = ? \
                   GROUP BY s.SCHEMA_NAME, s.DEFAULT_CHARACTER_SET_NAME, s.DEFAULT_COLLATION_NAME \
                   ORDER BY s.SCHEMA_NAME";
        let only = self.only_database.clone();
        let r = self.query_with(sql, vec![only.clone(), only])?;
        Ok(r.rows
            .iter()
            .map(|row| DatabaseInfo {
                name: col(row, 0),
                charset: col(row, 1),
                collation: col(row, 2),
                table_count: col(row, 3).parse().ok(),
                size_bytes: col(row, 4).parse().unwrap_or(0),
            })
            .collect())
    }

    fn get_tables_info(&self, db: &str) -> DbResult<Vec<TableInfo>> {
        let sql = "SELECT TABLE_NAME, \
                          COALESCE(ENGINE,''), \
                          COALESCE(TABLE_ROWS,0), \
                          COALESCE(DATA_LENGTH,0)+COALESCE(INDEX_LENGTH,0), \
                          COALESCE(TABLE_COLLATION,'') \
                   FROM information_schema.TABLES \
                   WHERE TABLE_SCHEMA = ? AND TABLE_TYPE='BASE TABLE' \
                   ORDER BY TABLE_NAME";
        let r = self.query_with(sql, vec![Some(db.into())])?;
        Ok(r.rows
            .iter()
            .map(|row| TableInfo {
                name: col(row, 0),
                engine: col(row, 1),
                row_count: col(row, 2).parse().unwrap_or(0),
                size_bytes: col(row, 3).parse().unwrap_or(0),
                collation: col(row, 4),
            })
            .collect())
    }

    fn get_columns_info(&self, db: &str, table: &str) -> DbResult<Vec<ColumnInfo>> {
        let sql = "SELECT COLUMN_NAME, COLUMN_TYPE, IS_NULLABLE, COLUMN_DEFAULT, COLUMN_KEY, EXTRA \
                   FROM information_schema.COLUMNS \
                   WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? \
                   ORDER BY ORDINAL_POSITION";
        let r = self.query_with(sql, vec![Some(db.into()), Some(table.into())])?;
        Ok(r.rows
            .iter()
            .map(|row| {
                let default_raw = col(row, 3);
                let extra = col(row, 5);
                ColumnInfo {
                    name: col(row, 0),
                    col_type: col(row, 1),
                    is_nullable: col(row, 2).eq_ignore_ascii_case("YES"),
                    default_val: if default_raw.is_empty() {
                        None
                    } else {
                        Some(default_raw)
                    },
                    column_key: col(row, 4),
                    is_auto: extra.contains("auto_increment"),
                    extra,
                }
            })
            .collect())
    }

    fn create_database(&self, name: &str, charset: &str) -> DbResult<()> {
        validate_charset(charset)?;
        let sql = format!(
            "CREATE DATABASE {} CHARACTER SET {}",
            quote_ident(name),
            charset
        );
        self.execute_write(None, &sql).map(|_| ())
    }

    fn drop_database(&self, name: &str) -> DbResult<()> {
        let sql = format!("DROP DATABASE {}", quote_ident(name));
        self.execute_write(None, &sql).map(|_| ())
    }

    fn drop_table(&self, db: &str, table: &str) -> DbResult<()> {
        let sql = format!("DROP TABLE {}", qualified(db, table));
        self.execute_write(None, &sql).map(|_| ())
    }

    fn truncate_table(&self, db: &str, table: &str) -> DbResult<()> {
        let sql = format!("TRUNCATE TABLE {}", qualified(db, table));
        self.execute_write(None, &sql).map(|_| ())
    }

    fn execute_transaction(&self, _db: Option<&str>, stmts: &[Statement]) -> DbResult<()> {
        let mut conn = self.conn()?;
        conn.query_drop("START TRANSACTION")
            .map_err(|e| format!("Error iniciando transacción: {e}"))?;
        for stmt in stmts {
            if let Err(e) = conn.exec_drop(stmt.sql.as_str(), stmt.params.clone()) {
                let _ = conn.query_drop("ROLLBACK");
                return Err(format!("Error en transacción (ROLLBACK): {e}"));
            }
            // Con CLIENT_FOUND_ROWS son las filas encontradas por el WHERE
            let found = conn.affected_rows();
            if stmt.single_row && found != 1 {
                let _ = conn.query_drop("ROLLBACK");
                return Err(format!(
                    "No se ha guardado nada: una sentencia debía cambiar una fila y \
                     encontró {found}. La fila puede haber cambiado o desaparecido en \
                     el servidor; recarga la tabla (F5) y repite los cambios.\n\n{}",
                    stmt.sql
                ));
            }
        }
        conn.query_drop("COMMIT")
            .map_err(|e| format!("Error en COMMIT: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_server;

    #[test]
    #[ignore = "necesita RUSTQL_TEST_MYSQL"]
    fn metadatos_con_parametros() {
        let b = test_server::backend();
        // Nombres con comilla y barra: con el escape a mano eran frágiles
        let db = "rustql_test_meta'\\x";
        let qdb = quote_ident(db);
        b.execute_write(None, &format!("DROP DATABASE IF EXISTS {qdb}"))
            .unwrap();
        b.execute_write(None, &format!("CREATE DATABASE {qdb}"))
            .unwrap();
        b.execute_write(
            None,
            &format!(
                "CREATE TABLE {qdb}.t (id INT AUTO_INCREMENT PRIMARY KEY, \
             v VARCHAR(10) NOT NULL DEFAULT 'x', n INT NULL)"
            ),
        )
        .unwrap();
        b.execute_write(
            None,
            &format!("INSERT INTO {qdb}.t (v) VALUES ('a'), ('b')"),
        )
        .unwrap();
        b.execute_write(None, &format!("ANALYZE TABLE {qdb}.t"))
            .unwrap();

        let tables = b.get_tables_info(db);
        let columns = b.get_columns_info(db, "t");
        let all = b.list_all_tables();
        b.execute_write(None, &format!("DROP DATABASE {qdb}"))
            .unwrap();

        let tables = tables.unwrap();
        assert_eq!(tables.len(), 1);
        assert_eq!(tables[0].name, "t");
        assert_eq!(tables[0].row_count, 2);
        let columns = columns.unwrap();
        let names: Vec<_> = columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["id", "v", "n"]);
        assert!(columns[0].extra.contains("auto_increment"));
        assert!(columns[0].is_auto && !columns[1].is_auto);
        assert!(!columns[1].is_nullable && columns[2].is_nullable);
        assert!(columns[0].is_primary() && !columns[1].is_primary());
        assert!(all.unwrap().contains(&(db.to_string(), "t".to_string())));
    }

    #[test]
    #[ignore = "necesita RUSTQL_TEST_MYSQL"]
    fn la_conexion_limitada_a_una_bbdd_solo_ensena_esa() {
        let all = test_server::backend();
        all.execute_write(None, "DROP DATABASE IF EXISTS rustql_test_only")
            .unwrap();
        all.execute_write(None, "CREATE DATABASE rustql_test_only")
            .unwrap();
        all.execute_write(None, "CREATE TABLE rustql_test_only.t (id INT)")
            .unwrap();
        let params = ConnectionParams {
            database: " rustql_test_only ".into(),
            ..test_server::params()
        };
        let only = MysqlBackend::connect_with_progress(&params, |_| {}).unwrap();

        let databases = only.list_databases();
        let tables = only.list_all_tables();
        let info = only.get_databases_info();
        // Es la BBDD por defecto: las consultas no necesitan nombrarla
        let unqualified = only.execute_query(None, "SELECT COUNT(*) FROM t");
        // El botón de elegir del formulario lista todas, haya o no una puesta
        let picker = crate::db::list_server_databases(&params);
        let missing = MysqlBackend::connect_with_progress(
            &ConnectionParams {
                database: "rustql_test_no_existe".into(),
                ..test_server::params()
            },
            |_| {},
        );
        all.execute_write(None, "DROP DATABASE rustql_test_only")
            .unwrap();

        assert_eq!(databases.unwrap(), ["rustql_test_only"]);
        assert_eq!(
            tables.unwrap(),
            [("rustql_test_only".to_string(), "t".to_string())]
        );
        let info = info.unwrap();
        assert_eq!(info.len(), 1);
        assert_eq!(
            (info[0].name.as_str(), info[0].table_count),
            ("rustql_test_only", Some(1))
        );
        assert_eq!(unqualified.unwrap().rows, [[Some("0".to_string())]]);
        let picker = picker.unwrap();
        assert!(picker.contains(&"rustql_test_only".to_string()));
        assert!(picker.contains(&"information_schema".to_string()));
        let err = missing.err().expect("no existe");
        assert!(err.contains("rustql_test_no_existe"), "{err}");
    }

    #[test]
    #[ignore = "necesita RUSTQL_TEST_MYSQL"]
    fn una_consulta_sin_filas_conserva_sus_columnas() {
        let b = test_server::backend();
        b.execute_write(None, "DROP DATABASE IF EXISTS rustql_test_empty")
            .unwrap();
        b.execute_write(None, "CREATE DATABASE rustql_test_empty")
            .unwrap();
        b.execute_write(
            None,
            "CREATE TABLE rustql_test_empty.t (id INT PRIMARY KEY, v VARCHAR(10))",
        )
        .unwrap();
        let empty = b.execute_query(None, "SELECT * FROM rustql_test_empty.t");
        let filtered = b.execute_query(None, "SELECT v FROM rustql_test_empty.t WHERE id = 1");
        b.execute_write(None, "DROP DATABASE rustql_test_empty")
            .unwrap();

        // Sin las columnas, el grid no puede mostrar la cabecera ni dejar
        // insertar la primera fila
        let empty = empty.unwrap();
        assert_eq!(empty.columns, ["id", "v"]);
        assert!(empty.rows.is_empty());
        assert!(!empty.truncated);
        assert_eq!(filtered.unwrap().columns, ["v"]);
    }

    #[test]
    #[ignore = "necesita RUSTQL_TEST_MYSQL"]
    fn la_transaccion_se_deshace_si_no_encuentra_la_fila() {
        let b = test_server::backend();
        b.execute_write(None, "DROP DATABASE IF EXISTS rustql_test_rows")
            .unwrap();
        b.execute_write(None, "CREATE DATABASE rustql_test_rows")
            .unwrap();
        b.execute_write(
            None,
            "CREATE TABLE rustql_test_rows.t (id INT PRIMARY KEY, v VARCHAR(10))",
        )
        .unwrap();
        b.execute_write(
            None,
            "INSERT INTO rustql_test_rows.t VALUES (1,'a'),(2,'b')",
        )
        .unwrap();
        let update = |v: &str, id: &str| Statement {
            sql: "UPDATE rustql_test_rows.t SET v = ? WHERE id = ?".into(),
            params: vec![Some(v.into()), Some(id.into())],
            single_row: true,
        };

        // La fila 9 no existe (otro la borró): tampoco se guarda la 1
        let missing = b.execute_transaction(None, &[update("x", "1"), update("y", "9")]);
        // Dejar una fila con el valor que ya tenía no es «no encontrada»
        let unchanged = b.execute_transaction(None, &[update("b", "2")]);
        let rows = b.execute_query(None, "SELECT v FROM rustql_test_rows.t ORDER BY id");
        b.execute_write(None, "DROP DATABASE rustql_test_rows")
            .unwrap();

        let err = missing.unwrap_err();
        assert!(err.contains("encontró 0"), "{err}");
        unchanged.unwrap();
        assert_eq!(
            rows.unwrap().rows,
            [[Some("a".to_string())], [Some("b".to_string())]]
        );
    }

    #[test]
    #[ignore = "necesita RUSTQL_TEST_MYSQL"]
    fn la_sesion_del_editor_sobrevive_entre_ejecuciones() {
        let b = test_server::backend();
        b.execute_write(None, "DROP DATABASE IF EXISTS rustql_test_session")
            .unwrap();
        b.execute_write(None, "CREATE DATABASE rustql_test_session")
            .unwrap();
        b.execute_write(None, "CREATE TABLE rustql_test_session.t (id INT)")
            .unwrap();
        let db = Some("rustql_test_session");
        let run = |sql: &str| {
            b.execute_in_session(db, sql, 100, &CancelFlag::default())
                .unwrap()
                .rows
        };
        let count = || {
            b.execute_query(None, "SELECT COUNT(*) FROM rustql_test_session.t")
                .unwrap()
                .rows[0][0]
                .clone()
        };

        // Cada línea es una ejecución (F9) distinta
        run("SET @x = 42");
        run("START TRANSACTION");
        run("INSERT INTO t VALUES (1)");
        assert_eq!(
            run("SELECT @x, COUNT(*) FROM t")[0],
            [Some("42".into()), Some("1".into())]
        );
        assert_eq!(count(), Some("0".into()), "sin COMMIT, el resto no la ve");
        run("ROLLBACK");
        assert_eq!(run("SELECT COUNT(*) FROM t")[0], [Some("0".into())]);

        run("START TRANSACTION");
        run("INSERT INTO t VALUES (2)");
        run("COMMIT");
        assert_eq!(count(), Some("1".into()));

        b.execute_write(None, "DROP DATABASE rustql_test_session")
            .unwrap();
    }

    #[test]
    #[ignore = "necesita RUSTQL_TEST_MYSQL"]
    fn el_editor_lee_como_mucho_max_rows() {
        let b = test_server::backend();
        b.execute_write(None, "DROP DATABASE IF EXISTS rustql_test_cap")
            .unwrap();
        b.execute_write(None, "CREATE DATABASE rustql_test_cap")
            .unwrap();
        b.execute_write(None, "CREATE TABLE rustql_test_cap.t (id INT)")
            .unwrap();
        b.execute_write(
            None,
            "INSERT INTO rustql_test_cap.t VALUES (1),(2),(3),(4),(5)",
        )
        .unwrap();
        let db = Some("rustql_test_cap");
        let run =
            |sql: &str, max_rows| b.execute_in_session(db, sql, max_rows, &CancelFlag::default());
        let capped = run("SELECT * FROM t ORDER BY id", 3);
        let exact = run("SELECT * FROM t", 5);
        let empty = run("SELECT id AS n FROM t WHERE 1 = 0", 3);
        // Tras recortar, la sesión sigue utilizable
        let after = run("SELECT COUNT(*) FROM t", 3);
        b.execute_write(None, "DROP DATABASE rustql_test_cap")
            .unwrap();

        let capped = capped.unwrap();
        assert_eq!(capped.rows.len(), 3);
        assert!(capped.truncated);
        let exact = exact.unwrap();
        assert_eq!(exact.rows.len(), 5);
        assert!(!exact.truncated);
        let empty = empty.unwrap();
        assert_eq!(empty.columns, ["n"], "columnas aunque no haya filas");
        assert!(empty.rows.is_empty());
        assert_eq!(after.unwrap().rows, [[Some("5".to_string())]]);
    }

    #[test]
    #[ignore = "necesita RUSTQL_TEST_MYSQL"]
    fn cancelar_interrumpe_la_consulta_y_conserva_la_sesion() {
        let b = Arc::new(test_server::backend());
        let run = |sql: &str, cancel: &CancelFlag| b.execute_in_session(None, sql, 100, cancel);
        run("SET @x = 7", &CancelFlag::default()).unwrap();

        // Una consulta de minutos, cancelada a los 300 ms
        let cancelled = |sql: &'static str| {
            let cancel = CancelFlag::default();
            let slow = {
                let (b, cancel) = (Arc::clone(&b), Arc::clone(&cancel));
                std::thread::spawn(move || {
                    let start = std::time::Instant::now();
                    (
                        b.execute_in_session(None, sql, 100, &cancel),
                        start.elapsed(),
                    )
                })
            };
            std::thread::sleep(std::time::Duration::from_millis(300));
            b.cancel_in_session(&cancel).unwrap();
            let (result, elapsed) = slow.join().unwrap();
            assert!(elapsed.as_secs() < 10, "{sql} tardó {elapsed:?}");
            result
        };
        let join = "SELECT COUNT(*) FROM information_schema.COLUMNS a, \
                    information_schema.COLUMNS b, information_schema.COLUMNS c";
        assert_eq!(cancelled(join).unwrap_err(), QUERY_CANCELLED);
        // SLEEP interrumpido no da error (devuelve 1): termina, que es lo
        // que importa, y el servidor dice lo que tenga que decir
        if let Err(e) = cancelled("SELECT SLEEP(60)") {
            assert_eq!(e, QUERY_CANCELLED);
        }

        // La sesión es la misma (conserva la variable) y la siguiente
        // consulta no hereda la cancelación
        let after = run("SELECT @x", &CancelFlag::default()).unwrap();
        assert_eq!(after.rows, [[Some("7".to_string())]]);

        // Cancelada antes de empezar, no llega al servidor
        let never = CancelFlag::default();
        b.cancel_in_session(&never).unwrap();
        assert_eq!(run("SET @x = 8", &never).unwrap_err(), QUERY_CANCELLED);
        let untouched = run("SELECT @x", &CancelFlag::default()).unwrap();
        assert_eq!(untouched.rows, [[Some("7".to_string())]]);
    }
}

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use mysql::{
    Conn, Opts, OptsBuilder, Pool, PoolConstraints, PoolOpts, PooledConn, SslOpts, Value,
    consts::CapabilityFlags,
    prelude::{Protocol, Queryable},
};

use super::{
    CancelFlag, ColumnInfo, ConnectionParams, DatabaseInfo, DbBackend, DbResult, Engine,
    QUERY_CANCELLED, QueryResult, Security, Statement, TableEntry, TableInfo, TableKind, TlsMode,
    tls, validate_charset,
};

// This backend's dialect, for it and for the export
pub(super) fn quote_ident(s: &str) -> String {
    Engine::Mysql.quote_ident(s)
}

pub(super) fn qualified(db: &str, table: &str) -> String {
    Engine::Mysql.qualified(db, table)
}

pub(super) fn literal(s: &str) -> String {
    Engine::Mysql.literal(s)
}

/// ER_QUERY_INTERRUPTED: the statement was interrupted with KILL QUERY.
const ER_QUERY_INTERRUPTED: u16 = 1317;

fn is_interrupted(e: &mysql::Error) -> bool {
    matches!(e, mysql::Error::MySqlError(e) if e.code == ER_QUERY_INTERRUPTED)
}

/// Tree table from its TABLE_TYPE: "VIEW" and "SYSTEM VIEW" are views;
/// everything else (tables, MariaDB sequences) is treated as a table.
fn table_entry(name: String, table_type: &str) -> TableEntry {
    let kind = if table_type.contains("VIEW") {
        TableKind::View
    } else {
        TableKind::Table
    };
    TableEntry { name, kind }
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

/// A value read from the server, written as a SQL literal. Anything that
/// is not valid text (the content of a BLOB) goes in hexadecimal: as a
/// string it would get corrupted.
pub(super) fn sql_literal(v: Value) -> String {
    match v {
        Value::NULL => "NULL".into(),
        Value::Bytes(bytes) => match String::from_utf8(bytes) {
            Ok(text) => literal(&text),
            Err(not_text) => {
                let hex: String = not_text
                    .as_bytes()
                    .iter()
                    .map(|byte| format!("{byte:02X}"))
                    .collect();
                format!("0x{hex}")
            }
        },
        other => value_to_string(other).map_or("NULL".into(), |text| literal(&text)),
    }
}

/// Reads the first result set: its columns (which arrive even when there
/// are no rows) and at most `max_rows` rows. When `set` is dropped, the
/// crate reads and discards whatever is left.
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

/// Stores the Pool instead of a single PooledConn.
/// The pool detects dead connections and reconnects transparently when
/// a new connection is requested through get_conn().
pub struct MysqlBackend {
    pool: Pool,
    opts: Opts,
    security: Security,
    /// File with the authorities the server is verified against.
    _authorities: Option<tls::AuthorityBundle>,
    /// The connection is limited to this database: it is the only one listed.
    only_database: Option<String>,
    /// The SQL editor's own connection, outside the pool: the pool resets
    /// each connection when it is returned (COM_RESET_CONNECTION), which
    /// undoes transactions, variables and temporary tables between runs.
    /// Opened on first use.
    session: Arc<Mutex<Option<Conn>>>,
    /// Editor run currently on the server: the id of its connection (for
    /// KILL QUERY) and its cancellation signal.
    session_running: Mutex<Option<(u32, CancelFlag)>>,
    /// A KILL QUERY was sent to the editor session. If it arrived after the
    /// statement had finished, the server may apply it to the next one: before
    /// running it, it is absorbed with an empty statement.
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
        tls::install_provider();
        let strict = params.tls != TlsMode::Prefer;
        // When verifying: the system authorities and the one in the CA file,
        // together in one file (the crate only accepts one, and it already
        // trusts the public ones). It lives as long as the connection: it is
        // read on every new connection
        let authorities = match params.tls {
            TlsMode::Verify => tls::AuthorityBundle::create(params.ca_file())?,
            _ => None,
        };
        let build = |with_tls: bool| -> Opts {
            // min:0 avoids opening 10 connections in parallel at startup (default min:10).
            // max:4 lets the background jobs (data, metadata, writes) run without
            // waiting for each other; the SQL editor has its own connection.
            let pool_constraints = PoolConstraints::new(0, 4).unwrap();
            let pool_opts = PoolOpts::default().with_constraints(pool_constraints);
            let any_certificate = params.tls.accepts_any_certificate();
            let ca_file = authorities.as_ref().map(|a| a.path().to_path_buf());
            let ssl = SslOpts::default()
                .with_danger_accept_invalid_certs(any_certificate)
                .with_danger_skip_domain_validation(any_certificate)
                .with_root_cert_path(ca_file);
            OptsBuilder::new()
                .ip_or_hostname(Some(params.host.as_str()))
                .tcp_port(params.port)
                .user(Some(params.user.as_str()))
                .pass(Some(password.as_str()))
                // Default database of the connection: if it does not exist or the
                // user cannot enter it, the server says so on connect
                .db_name(params.only_database())
                // Affected rows = rows found by the WHERE, even if the UPDATE leaves
                // them as they were: that tells "it already had that value" apart
                // from "the row does not exist" (see execute_transaction).
                .additional_capabilities(CapabilityFlags::CLIENT_FOUND_ROWS)
                .ssl_opts(with_tls.then_some(ssl))
                // Locally the crate swaps the TCP connection for the server's
                // socket, which is not encrypted (and does not need to be). If
                // TLS is required, the connection stays on the one that has it
                .prefer_socket(!strict)
                .pool_opts(pool_opts)
                .into()
        };
        // Opens the pool and checks that it can log in
        let open = |opts: Opts| -> mysql::Result<(Pool, Opts)> {
            let pool = Pool::new(opts.clone())?;
            pool.get_conn()?;
            Ok((pool, opts))
        };

        on_status("Autenticando...");
        let (pool, opts) = match open(build(true)) {
            // "Encrypt if possible": if it cannot (the server offers no TLS, or
            // only old versions), unencrypted. A server error (password,
            // privileges) already arrives over TLS: it is not an encryption one
            Err(e) if !strict && !matches!(e, mysql::Error::MySqlError(_)) => open(build(false)),
            result => result,
        }
        .map_err(|e| {
            let error = e.to_string();
            format!(
                "No se pudo conectar: {error}{}",
                tls::hint(params.tls, &error)
            )
        })?;

        // What the server says about the connection just opened (locally the
        // crate may have swapped it for the socket, without TLS)
        let cipher: Option<(String, String)> = pool
            .get_conn()
            .and_then(|mut conn| conn.query_first("SHOW SESSION STATUS LIKE 'Ssl_cipher'"))
            .unwrap_or_default();
        let encrypted = cipher.is_some_and(|(_, cipher)| !cipher.is_empty());

        on_status("Conexión establecida.");
        Ok(Self {
            pool,
            opts,
            security: Security::of(encrypted, &params.host),
            _authorities: authorities,
            only_database: params.only_database().map(str::to_string),
            session: Arc::default(),
            session_running: Mutex::default(),
            session_killed: AtomicBool::new(false),
        })
    }

    /// Gets a connection from the pool. If the previous connection had died,
    /// the pool creates a new one automatically.
    /// `pub(super)` so export.rs can keep a stable session (variables such
    /// as FOREIGN_KEY_CHECKS do not survive the pool).
    pub(super) fn conn(&self) -> DbResult<PooledConn> {
        self.pool
            .get_conn()
            .map_err(|e| format!("Sin conexión con el servidor: {e}"))
    }

    /// Read with positional parameters (`?`).
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

    fn security(&self) -> Security {
        self.security
    }

    fn list_databases(&self) -> DbResult<Vec<String>> {
        if let Some(only) = &self.only_database {
            return Ok(vec![only.clone()]);
        }
        self.conn()?
            .query("SHOW DATABASES")
            .map_err(|e| format!("Error en query: {e}"))
    }

    fn list_tables(&self, database: &str) -> DbResult<Vec<TableEntry>> {
        // With its type: "BASE TABLE", "VIEW"…
        let query = format!("SHOW FULL TABLES FROM {}", quote_ident(database));
        let rows: Vec<(String, String)> = self
            .conn()?
            .query(query)
            .map_err(|e| format!("Error en query: {e}"))?;
        Ok(rows
            .into_iter()
            .map(|(name, table_type)| table_entry(name, &table_type))
            .collect())
    }

    fn list_all_tables(&self) -> DbResult<Vec<(String, TableEntry)>> {
        // With no database to limit to, the filter discards none
        let rows: Vec<(String, String, String)> = self
            .conn()?
            .exec(
                "SELECT TABLE_SCHEMA, TABLE_NAME, TABLE_TYPE FROM information_schema.TABLES \
                 WHERE ? IS NULL OR TABLE_SCHEMA = ? \
                 ORDER BY TABLE_SCHEMA, TABLE_NAME",
                (&self.only_database, &self.only_database),
            )
            .map_err(|e| format!("Error en query: {e}"))?;
        Ok(rows
            .into_iter()
            .map(|(db, name, table_type)| (db, table_entry(name, &table_type)))
            .collect())
    }

    fn view_definition(&self, db: &str, view: &str) -> DbResult<String> {
        let sql = "SELECT VIEW_DEFINITION FROM information_schema.VIEWS \
                   WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ?";
        let result = self.query_with(sql, vec![Some(db.into()), Some(view.into())])?;
        let definition = result
            .rows
            .first()
            .and_then(|row| row[0].clone())
            .ok_or_else(|| format!("No se encuentra la definición de la vista «{view}»."))?;
        Ok(format!(
            "CREATE OR REPLACE VIEW {} AS\n{definition}",
            qualified(db, view)
        ))
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
        // The turn is awaited here: editor runs go one at a time
        let mut session = self.session.lock().unwrap_or_else(PoisonError::into_inner);
        let conn = match &mut *session {
            Some(conn) => conn,
            None => session.insert(
                Conn::new(self.opts.clone())
                    .map_err(|e| format!("No se pudo abrir la sesión del editor: {e}"))?,
            ),
        };
        if self.session_killed.swap(false, Ordering::SeqCst) {
            // A KILL QUERY that arrived late is taken by this statement
            let _ = conn.query_drop("DO 0");
        }
        let running = |value| {
            *self
                .session_running
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = value;
        };
        // Announce first and then check the signal: whoever cancels does the
        // opposite, so at least one of the two sees the other
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
                // Connection lost (server restarted, timeout…): the next run opens
                // another one; the session state is already lost.
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
        // Only if it is the one on the server: KILL QUERY would interrupt
        // another tab's. One waiting for its turn will see the signal on arrival
        let Some((conn_id, _)) = running.filter(|(_, flag)| Arc::ptr_eq(flag, cancel)) else {
            return Ok(());
        };
        self.session_killed.store(true, Ordering::SeqCst);
        // Through another connection: the editor's is busy with the query
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

    fn drop_table(&self, db: &str, table: &str, kind: TableKind) -> DbResult<()> {
        let sql = Engine::Mysql.drop_table(db, table, kind);
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
            // With CLIENT_FOUND_ROWS they are the rows found by the WHERE
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
        // Names with a quote and a backslash: escaping by hand made them fragile
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
        assert!(all.unwrap().contains(&(db.to_string(), "t".into())));
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
        // It is the default database: queries do not need to name it
        let unqualified = only.execute_query(None, "SELECT COUNT(*) FROM t");
        // The form's pick button lists them all, whether one is set or not
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
            [("rustql_test_only".to_string(), "t".into())]
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
    #[ignore = "necesita RUSTQL_TEST_MYSQL y RUSTQL_TEST_CA"]
    fn niveles_de_seguridad_de_la_conexion() {
        let connect = |tls, ca| {
            MysqlBackend::connect_with_progress(
                &test_server::with_tls(test_server::params(), tls, ca),
                |_| {},
            )
        };
        let cipher = |b: &MysqlBackend| {
            let status = b.execute_query(None, "SHOW SESSION STATUS LIKE 'Ssl_cipher'");
            status.unwrap().rows[0][1].clone().unwrap_or_default()
        };

        // Default: gets in, encrypted or not (locally the crate switches to the
        // socket, which is not encrypted but never leaves the machine either)
        let preferred = connect(TlsMode::Prefer, false).unwrap();
        assert_ne!(preferred.security(), Security::Plain);
        // Require: encrypted, without checking the certificate (from a private CA)
        let required = connect(TlsMode::Require, false).unwrap();
        assert_ne!(cipher(&required), "", "la conexión va por TLS");
        assert_eq!(required.security(), Security::Encrypted);
        // Verify without saying which authority signs it: it is unknown
        let unknown = connect(TlsMode::Verify, false)
            .err()
            .expect("CA desconocida");
        assert!(unknown.contains("fichero de CA"), "{unknown}");
        // With its CA, the certificate is valid (and is for 127.0.0.1)
        let verified = connect(TlsMode::Verify, true).unwrap();
        assert_ne!(cipher(&verified), "");
        // The editor session uses the same secure connection
        let run = verified.execute_in_session(
            None,
            "SHOW SESSION STATUS LIKE 'Ssl_cipher'",
            10,
            &CancelFlag::default(),
        );
        assert_ne!(run.unwrap().rows[0][1], Some(String::new()));
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

        // Without the columns, the grid cannot show the header or allow
        // inserting the first row
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

        // Row 9 does not exist (someone else deleted it): row 1 is not saved either
        let missing = b.execute_transaction(None, &[update("x", "1"), update("y", "9")]);
        // Leaving a row with the value it already had is not "not found"
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

        // Each line is a separate run (F9)
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
        // After truncating, the session is still usable
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

        // A query that takes minutes, cancelled after 300 ms
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
        // An interrupted SLEEP gives no error (it returns 1): it ends, which is
        // what matters, and the server says whatever it has to say
        if let Err(e) = cancelled("SELECT SLEEP(60)") {
            assert_eq!(e, QUERY_CANCELLED);
        }

        // The session is the same (it keeps the variable) and the next query
        // does not inherit the cancellation
        let after = run("SELECT @x", &CancelFlag::default()).unwrap();
        assert_eq!(after.rows, [[Some("7".to_string())]]);

        // Cancelled before starting, it never reaches the server
        let never = CancelFlag::default();
        b.cancel_in_session(&never).unwrap();
        assert_eq!(run("SET @x = 8", &never).unwrap_err(), QUERY_CANCELLED);
        let untouched = run("SELECT @x", &CancelFlag::default()).unwrap();
        assert_eq!(untouched.rows, [[Some("7".to_string())]]);
    }
}

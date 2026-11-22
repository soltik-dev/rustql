pub mod dialect;
pub mod export;
pub mod mysql;
pub mod postgres;
pub mod tls;

use crate::i18n::t;
use std::sync::Arc;

pub use dialect::{Engine, ParamList};
pub use export::{ExportDestination, ExportEvent, ExportOptions};
pub use tls::{Security, TlsMode};

#[derive(Debug, Clone)]
pub struct ConnectionParams {
    pub engine: Engine,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    /// Profile with its password in the system keyring: if `password` is
    /// empty, it is read from there when connecting.
    pub keyring_id: Option<String>,
    /// Database the connection is limited to: the tree only shows that one.
    /// Empty = every database on the server.
    pub database: String,
    /// Whether the connection is encrypted and whether the certificate is checked.
    pub tls: TlsMode,
    /// PEM file with the authority that signs the server certificate, for
    /// `TlsMode::Verify`. Empty = only the public authorities.
    pub tls_ca: String,
}

impl ConnectionParams {
    /// Stable identity of the connection ("user@host:port") to keep state per
    /// server, independent of the profile name.
    pub fn key(&self) -> String {
        format!("{}@{}:{}", self.user, self.host, self.port)
    }

    /// The CA file to verify the server with, if one was given.
    pub fn ca_file(&self) -> Option<&str> {
        let path = self.tls_ca.trim();
        (!path.is_empty()).then_some(path)
    }

    /// The database the connection is limited to, if one was given.
    pub fn only_database(&self) -> Option<&str> {
        let database = self.database.trim();
        (!database.is_empty()).then_some(database)
    }

    /// The password typed in or, if empty, the one in the keyring. May block
    /// (the keyring may ask to be unlocked): only call it from worker threads.
    pub fn resolve_password(&self) -> DbResult<String> {
        match &self.keyring_id {
            Some(id) if self.password.is_empty() => crate::secrets::get(id)
                .map_err(|e| t!("keyring-read-failed", error = e))?
                .ok_or_else(|| t!("keyring-password-gone")),
            _ => Ok(self.password.clone()),
        }
    }
}

impl Default for ConnectionParams {
    fn default() -> Self {
        let engine = Engine::default();
        Self {
            engine,
            host: "127.0.0.1".into(),
            port: engine.default_port(),
            user: engine.default_user().into(),
            password: String::new(),
            keyring_id: None,
            database: String::new(),
            tls: TlsMode::default(),
            tls_ca: String::new(),
        }
    }
}

pub type DbResult<T> = Result<T, String>;

/// Signal to cancel a SQL editor run: shared by whoever starts it and
/// whoever cancels it (`DbBackend::cancel_in_session`).
pub type CancelFlag = Arc<std::sync::atomic::AtomicBool>;

/// Error a cancelled editor run ends with.
/// A marker, not a text: the interface shows its own translated message.
pub const QUERY_CANCELLED: &str = "Query cancelled.";

/// Statement with parameters (see `ParamList`). The values travel apart
/// from the SQL: they need no escaping and arrive intact (`None` = NULL).
#[derive(Debug, Clone, PartialEq)]
pub struct Statement {
    pub sql: String,
    pub params: Vec<Option<String>>,
    /// The statement changes one specific row (UPDATE or DELETE by primary
    /// key): if it does not find exactly one, the transaction is rolled back.
    pub single_row: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct QueryResult {
    pub columns: Vec<String>,
    /// Each cell is `None` if the value is NULL in the database.
    pub rows: Vec<Vec<Option<String>>>,
    /// There were more rows than were read (SQL editor limit).
    pub truncated: bool,
}

#[derive(Debug, Clone)]
pub struct DatabaseInfo {
    pub name: String,
    pub charset: String,
    pub collation: String,
    /// `None` if the engine cannot count them without entering the database.
    pub table_count: Option<usize>,
    pub size_bytes: u64,
}

/// What the tree shows as a table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TableKind {
    #[default]
    Table,
    View,
    /// View with its data stored (PostgreSQL).
    MaterializedView,
}

impl TableKind {
    /// A view has no data of its own: it is not emptied or cloned, and its
    /// rows are not edited.
    pub fn is_view(self) -> bool {
        self != TableKind::Table
    }

    /// Word SQL names it with in `DROP …`.
    pub fn keyword(self) -> &'static str {
        match self {
            TableKind::Table => "TABLE",
            TableKind::View => "VIEW",
            TableKind::MaterializedView => "MATERIALIZED VIEW",
        }
    }
}

/// A table (or view) of a database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableEntry {
    pub name: String,
    pub kind: TableKind,
}

/// In tests, a table (not a view) with that name.
#[cfg(test)]
impl From<&str> for TableEntry {
    fn from(name: &str) -> Self {
        Self {
            name: name.into(),
            kind: TableKind::Table,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TableInfo {
    pub name: String,
    pub engine: String,
    pub row_count: u64,
    pub size_bytes: u64,
    pub collation: String,
}

#[derive(Debug, Clone)]
pub struct ColumnInfo {
    pub name: String,
    pub col_type: String,
    pub is_nullable: bool,
    pub default_val: Option<String>,
    /// Kind of key it is part of: "PRI", "UNI", "MUL" or nothing.
    pub column_key: String,
    pub extra: String,
    /// The server gives it a value on insert (AUTO_INCREMENT, serial,
    /// identity): it is not written in a new row.
    pub is_auto: bool,
}

impl ColumnInfo {
    pub fn is_primary(&self) -> bool {
        self.column_key == "PRI"
    }
}

/// Checks that a MySQL charset name only contains safe characters.
/// MySQL charsets are always alphanumeric with underscores (e.g. utf8mb4, latin1).
pub fn validate_charset(s: &str) -> DbResult<()> {
    if s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !s.is_empty() {
        Ok(())
    } else {
        Err(t!("charset-invalid", charset = s))
    }
}

/// Backend of a connection. It is `Sync` and every method takes `&self`: it
/// is shared (`Arc`) between the UI and the worker threads, which ask its
/// pool for connections when they need them.
///
/// Methods with `db: Option<&str>` run SQL in that database. MySQL sees
/// every database from any connection and does not need it (tables are
/// qualified with their database); in PostgreSQL it decides which one to
/// connect to. Without it, in the database the connection was opened with.
pub trait DbBackend: Send + Sync {
    fn engine(&self) -> Engine;

    /// Whether the connection is encrypted, according to the server on connect.
    fn security(&self) -> Security;

    // ── Reads (do not change the backend's state) ──────────────────────────────

    fn list_databases(&self) -> DbResult<Vec<String>>;
    /// Tables and views of a database.
    fn list_tables(&self, database: &str) -> DbResult<Vec<TableEntry>>;
    /// (database, table) pairs of the whole server.
    fn list_all_tables(&self) -> DbResult<Vec<(String, TableEntry)>>;
    /// Statement that recreates view `view` with its current definition, to
    /// see or change it.
    fn view_definition(&self, db: &str, view: &str) -> DbResult<String>;

    /// Runs arbitrary read SQL and returns the result.
    fn execute_query(&self, db: Option<&str>, sql: &str) -> DbResult<QueryResult>;

    /// SQL from the editor, on its own connection that is kept between runs:
    /// open transactions, variables (`SET @x`), `USE` and temporary tables are
    /// still alive in the next one. With `db`, `USE` first.
    /// Reads at most `max_rows` rows (the rest are discarded, `truncated`).
    /// With `cancel` set (before starting or while running, see
    /// `cancel_in_session`) it ends with the `QUERY_CANCELLED` error.
    fn execute_in_session(
        &self,
        db: Option<&str>,
        sql: &str,
        max_rows: usize,
        cancel: &CancelFlag,
    ) -> DbResult<QueryResult>;

    /// Cancels the `execute_in_session` run that received `cancel`: if it is
    /// already on the server, interrupts it there; if it is waiting for its
    /// turn, it never starts. The editor's connection and its state are kept.
    fn cancel_in_session(&self, cancel: &CancelFlag) -> DbResult<()>;

    fn get_databases_info(&self) -> DbResult<Vec<DatabaseInfo>>;
    fn get_tables_info(&self, db: &str) -> DbResult<Vec<TableInfo>>;
    /// Columns in order; those of the primary key have `column_key` "PRI".
    fn get_columns_info(&self, db: &str, table: &str) -> DbResult<Vec<ColumnInfo>>;

    // ── Writes (change data on the server) ─────────────────────────────────────

    /// Runs write SQL (INSERT/UPDATE/DELETE/DDL). Returns the affected rows.
    fn execute_write(&self, db: Option<&str>, sql: &str) -> DbResult<u64>;

    fn create_database(&self, name: &str, charset: &str) -> DbResult<()>;
    fn drop_database(&self, name: &str) -> DbResult<()>;
    /// Drops a table or a view (`kind` says which: they are dropped with
    /// different statements).
    fn drop_table(&self, db: &str, table: &str, kind: TableKind) -> DbResult<()>;
    fn truncate_table(&self, db: &str, table: &str) -> DbResult<()>;

    /// Runs a list of statements inside a transaction.
    /// If one fails, or a `single_row` one does not find its row, it rolls
    /// back and returns the error.
    fn execute_transaction(&self, db: Option<&str>, stmts: &[Statement]) -> DbResult<()>;
}

pub enum ConnectEvent {
    Status(String),
    Done(DbResult<Arc<dyn DbBackend>>),
}

/// Connects to the server of `params`, with the backend for its engine.
pub fn connect(
    params: &ConnectionParams,
    on_status: impl Fn(&str),
) -> DbResult<Arc<dyn DbBackend>> {
    Ok(match params.engine {
        Engine::Mysql => Arc::new(mysql::MysqlBackend::connect_with_progress(
            params, on_status,
        )?),
        Engine::Postgres => Arc::new(postgres::PostgresBackend::connect_with_progress(
            params, on_status,
        )?),
    })
}

/// Like `connect`, reporting progress and the result through `tx`.
pub fn connect_with_progress(
    params: &ConnectionParams,
    tx: &std::sync::mpsc::Sender<ConnectEvent>,
) {
    let send_status = |msg: &str| {
        let _ = tx.send(ConnectEvent::Status(msg.to_string()));
    };
    let result = connect(params, send_status);
    let _ = tx.send(ConnectEvent::Done(result));
}

/// The databases of the server of `params`, to pick one in the connection
/// form: connects, lists them all and disconnects.
pub fn list_server_databases(params: &ConnectionParams) -> DbResult<Vec<String>> {
    match params.engine {
        // Without limiting itself to the database in the form
        Engine::Mysql => {
            let all = ConnectionParams {
                database: String::new(),
                ..params.clone()
            };
            mysql::MysqlBackend::connect_with_progress(&all, |_| {})?.list_databases()
        }
        // Going in through the form's database, if any: it may be the only one
        // the server lets this user into, and the others are visible from it
        Engine::Postgres => {
            postgres::PostgresBackend::connect_with_progress(params, |_| {})?.all_databases()
        }
    }
}

/// Real servers for the tests marked `#[ignore]`:
/// `RUSTQL_TEST_MYSQL=user:password@host:port` and
/// `RUSTQL_TEST_POSTGRES=…`, with `cargo test -- --ignored`. The TLS ones
/// also need `RUSTQL_TEST_CA` (see `with_tls`).
/// They create and drop their own `rustql_test_*` databases.
#[cfg(test)]
pub mod test_server {
    use super::{ConnectionParams, Engine};

    fn from_env(var: &str, engine: Engine) -> ConnectionParams {
        let url = std::env::var(var).unwrap_or_else(|_| panic!("{var}=user:password@host:port"));
        let (auth, addr) = url.rsplit_once('@').expect("missing @");
        let (user, password) = auth.split_once(':').unwrap_or((auth, ""));
        let (host, port) = addr.rsplit_once(':').expect("missing :port");
        ConnectionParams {
            engine,
            host: host.into(),
            port: port.parse().expect("puerto"),
            user: user.into(),
            password: password.into(),
            keyring_id: None,
            database: String::new(),
            tls: Default::default(),
            tls_ca: String::new(),
        }
    }

    pub fn params() -> ConnectionParams {
        from_env("RUSTQL_TEST_MYSQL", Engine::Mysql)
    }

    pub fn backend() -> super::mysql::MysqlBackend {
        super::mysql::MysqlBackend::connect_with_progress(&params(), |_| {}).unwrap()
    }

    /// Parameters of `base` with security level `tls`. To verify,
    /// `RUSTQL_TEST_CA` is the file of the authority that signs the test
    /// servers' certificate (valid for 127.0.0.1).
    pub fn with_tls(base: ConnectionParams, tls: super::TlsMode, ca: bool) -> ConnectionParams {
        ConnectionParams {
            tls,
            tls_ca: if ca {
                std::env::var("RUSTQL_TEST_CA").expect("RUSTQL_TEST_CA=/ruta/ca.crt")
            } else {
                String::new()
            },
            ..base
        }
    }

    /// Names of some tables, to compare them without their kind.
    pub fn names(tables: Vec<super::TableEntry>) -> Vec<String> {
        tables.into_iter().map(|table| table.name).collect()
    }

    pub fn pg_params() -> ConnectionParams {
        from_env("RUSTQL_TEST_POSTGRES", Engine::Postgres)
    }

    pub fn pg_backend() -> super::postgres::PostgresBackend {
        super::postgres::PostgresBackend::connect_with_progress(&pg_params(), |_| {}).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use ::mysql::prelude::Queryable;

    use super::*;

    /// With NO_BACKSLASH_ESCAPES, escaping the quote as `\'`, this value
    /// turned `v = '…'` into `v = '\\\' OR 1=1 -- '`.
    const ATTACK: &str = "\\' OR 1=1 -- ";

    #[test]
    #[ignore = "necesita RUSTQL_TEST_MYSQL"]
    fn un_literal_no_permite_inyectar_en_ningun_sql_mode() {
        let backend = test_server::backend();
        // A single connection: sql_mode is per session
        let mut conn = backend.conn().unwrap();
        conn.query_drop("DROP DATABASE IF EXISTS rustql_test_escape")
            .unwrap();
        conn.query_drop("CREATE DATABASE rustql_test_escape")
            .unwrap();
        conn.query_drop("CREATE TABLE rustql_test_escape.t (v TEXT)")
            .unwrap();
        conn.query_drop("INSERT INTO rustql_test_escape.t VALUES ('a'), ('b')")
            .unwrap();

        let tricky = ["it's", "a\\b", "x\\'y", "line\nbreak", ATTACK];
        for mode in ["", "NO_BACKSLASH_ESCAPES"] {
            conn.query_drop(format!("SET SESSION sql_mode = '{mode}'"))
                .unwrap();
            let matches: Option<u64> = conn
                .query_first(format!(
                    "SELECT COUNT(*) FROM rustql_test_escape.t WHERE v = {}",
                    Engine::Mysql.literal(ATTACK)
                ))
                .unwrap();
            assert_eq!(matches, Some(0), "inyección con sql_mode={mode:?}");
            if mode.is_empty() {
                for value in tricky {
                    let back: Option<String> = conn
                        .query_first(format!("SELECT {}", Engine::Mysql.literal(value)))
                        .unwrap();
                    assert_eq!(back.as_deref(), Some(value));
                }
            }
        }
        conn.query_drop("DROP DATABASE rustql_test_escape").unwrap();
    }
}

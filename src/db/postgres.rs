//! PostgreSQL backend.
//!
//! In PostgreSQL a connection belongs to a single database. So that the rest
//! of the program sees it like MySQL (a server with its databases), the
//! backend opens connections to each database as they are requested and
//! routes each call to its own. Schemas are flattened into the table name
//! (see `dialect::pg_table_name`).
//!
//! Everything is read in text format, which is how the server writes any
//! type (arrays, JSON, dates…) without having to know it: reads go through
//! the simple protocol, and write parameters as text that the server
//! converts to its column's type.

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use futures_util::{StreamExt, pin_mut, stream};
use tokio::runtime::Runtime;
use tokio_postgres::config::SslMode;
use tokio_postgres::error::SqlState;
use tokio_postgres::types::private::BytesMut;
use tokio_postgres::types::{Format, IsNull, ToSql, Type, to_sql_checked};
use tokio_postgres::{CancelToken, Client, Config, SimpleQueryMessage};
use tokio_postgres_rustls::MakeRustlsConnect;

use super::dialect::{pg_split_table, pg_table_name};
use super::{
    CancelFlag, ColumnInfo, ConnectionParams, DatabaseInfo, DbBackend, DbResult, Engine,
    QUERY_CANCELLED, QueryResult, Security, Statement, TableEntry, TableInfo, TableKind, TlsMode,
    tls,
};
use crate::i18n::t;

const SQL: Engine = Engine::Postgres;

/// Databases tried in order when the connection is not limited to one:
/// the others are listed from it. After these, the one named after the
/// user.
const HOME_DATABASES: [&str; 2] = ["postgres", "template1"];

/// Connections kept open per database for the next query. Any extra ones
/// needed at the same time are opened and closed when they finish.
const MAX_IDLE: usize = 2;

/// A connection unused for this long is closed: with many databases open,
/// each one's connections would pile up on the server.
const IDLE_TIMEOUT: Duration = Duration::from_secs(120);

/// Databases queried at the same time when listing the tables of all of them.
const ALL_TABLES_CONCURRENCY: usize = 4;

/// Relations the tree shows as tables: tables (also partitioned and
/// foreign), views and materialized views of the user's schemas.
/// Partitions are not shown: their table is.
const USER_RELATIONS: &str = "c.relkind IN ('r', 'p', 'v', 'm', 'f') \
     AND NOT c.relispartition \
     AND n.nspname NOT IN ('pg_catalog', 'information_schema') \
     AND n.nspname !~ '^pg_(toast|temp)' \
     AND has_schema_privilege(n.oid, 'USAGE')";

/// Databases the user can enter.
const USER_DATABASES: &str =
    "d.datallowconn AND NOT d.datistemplate AND has_database_privilege(d.datname, 'CONNECT')";

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The error as the server tells it, with its detail and hint. If it does
/// not come from the server (network, TLS…), with its cause: the crate only
/// says what kind it is ("error connecting to server") and leaves the why
/// in the chain of causes.
fn describe(e: &tokio_postgres::Error) -> String {
    if let Some(db) = e.as_db_error() {
        let mut text = db.message().to_string();
        for extra in [db.detail(), db.hint()].into_iter().flatten() {
            text.push('\n');
            text.push_str(extra);
        }
        return text;
    }
    let mut text = e.to_string();
    let mut cause = std::error::Error::source(e);
    while let Some(inner) = cause {
        text.push_str(": ");
        text.push_str(&inner.to_string());
        cause = inner.source();
    }
    text
}

/// The server closed the connection (it restarted, an administrator ended
/// it, a firewall cut it): the failure is not the query's.
fn connection_lost(e: &tokio_postgres::Error) -> bool {
    e.is_closed()
        || matches!(
            e.code(),
            Some(code) if *code == SqlState::ADMIN_SHUTDOWN || *code == SqlState::CRASH_SHUTDOWN
        )
}

/// The failure is in the TLS handshake, not the server or the network.
fn is_tls_failure(e: &tokio_postgres::Error) -> bool {
    e.as_db_error().is_none() && e.to_string().contains("TLS handshake")
}

fn is_cancelled(e: &tokio_postgres::Error) -> bool {
    e.code() == Some(&SqlState::QUERY_CANCELED)
}

/// Parameter sent as text: the server converts it to the type the
/// statement expects in that position, as it would with a literal.
#[derive(Debug)]
struct TextParam<'a>(Option<&'a str>);

impl ToSql for TextParam<'_> {
    fn to_sql(
        &self,
        _: &Type,
        out: &mut BytesMut,
    ) -> Result<IsNull, Box<dyn std::error::Error + Sync + Send>> {
        match self.0 {
            Some(text) => {
                out.extend_from_slice(text.as_bytes());
                Ok(IsNull::No)
            }
            None => Ok(IsNull::Yes),
        }
    }

    fn accepts(_: &Type) -> bool {
        true
    }

    fn encode_format(&self, _: &Type) -> Format {
        Format::Text
    }

    to_sql_checked!();
}

/// Reads the first result set of a run (that of the first statement that
/// returns rows) and at most `max_rows` rows. Also returns the rows
/// affected by the last statement.
async fn read_results(
    client: &Client,
    sql: &str,
    max_rows: usize,
) -> Result<(QueryResult, u64), tokio_postgres::Error> {
    let stream = client.simple_query_raw(sql).await?;
    pin_mut!(stream);
    let mut result: Option<QueryResult> = None;
    // The rows arriving belong to the set being kept
    let mut reading = false;
    let mut affected = 0;
    while let Some(message) = stream.next().await {
        match message? {
            // It arrives even if the query returns no rows
            SimpleQueryMessage::RowDescription(columns) => {
                reading = result.is_none();
                if reading {
                    result = Some(QueryResult {
                        columns: columns.iter().map(|c| c.name().to_string()).collect(),
                        ..Default::default()
                    });
                }
            }
            SimpleQueryMessage::Row(row) => {
                if let Some(result) = result.as_mut().filter(|_| reading) {
                    if result.rows.len() == max_rows {
                        // When the stream is dropped, the rest is read and discarded
                        result.truncated = true;
                        break;
                    }
                    result.rows.push(
                        (0..row.len())
                            .map(|i| row.get(i).map(str::to_string))
                            .collect(),
                    );
                }
            }
            SimpleQueryMessage::CommandComplete(rows) => {
                reading = false;
                affected = rows;
            }
            _ => {}
        }
    }
    Ok((result.unwrap_or_default(), affected))
}

/// Open connections to a database, waiting for the next query, with the
/// moment they were left.
#[derive(Default)]
struct Pool {
    idle: Mutex<Vec<(Instant, Client)>>,
}

type Pools = Arc<Mutex<HashMap<String, Arc<Pool>>>>;

/// Every so often closes the connections unused for `IDLE_TIMEOUT`.
/// Ends when the backend goes away.
async fn close_idle_connections(pools: std::sync::Weak<Mutex<HashMap<String, Arc<Pool>>>>) {
    loop {
        tokio::time::sleep(IDLE_TIMEOUT / 4).await;
        let Some(pools) = pools.upgrade() else {
            return;
        };
        for pool in lock(&pools).values() {
            lock(&pool.idle).retain(|(since, _)| since.elapsed() < IDLE_TIMEOUT);
        }
    }
}

/// Connection to a database, taken from its `Pool`: it goes back when dropped.
struct Pooled {
    client: Option<Client>,
    pool: Arc<Pool>,
}

impl Pooled {
    /// It does not go back to the pool: it was left in an unknown state (a
    /// transaction that could not be rolled back).
    fn discard(mut self) {
        self.client = None;
    }
}

impl std::ops::Deref for Pooled {
    type Target = Client;
    fn deref(&self) -> &Client {
        self.client.as_ref().expect("only missing once dropped")
    }
}

impl Drop for Pooled {
    fn drop(&mut self) {
        if let Some(client) = self.client.take().filter(|c| !c.is_closed()) {
            let mut idle = lock(&self.pool.idle);
            if idle.len() < MAX_IDLE {
                idle.push((Instant::now(), client));
            }
        }
    }
}

/// What each batch of a result read in parts receives: the column names
/// and the rows, as text.
pub(super) type TextBatch<'a> = dyn FnMut(&[String], Vec<Vec<Option<String>>>) -> DbResult<()> + 'a;

/// Own connection to a database, outside the pool: whatever is set on it
/// (`search_path`…) does not reach the others. Used by the export.
pub(super) struct Dedicated<'a> {
    backend: &'a PostgresBackend,
    client: Client,
}

impl Dedicated<'_> {
    pub(super) fn query(&self, sql: &str) -> DbResult<QueryResult> {
        self.backend
            .rt
            .block_on(read_results(&self.client, sql, usize::MAX))
            .map(|(result, _)| result)
            .map_err(|e| describe(&e))
    }

    pub(super) fn execute(&self, sql: &str) -> DbResult<()> {
        self.query(sql).map(drop)
    }

    /// Reads the result of `sql` without loading it all into memory: hands
    /// `each` the columns and the rows in batches of up to `batch`. If `each`
    /// fails, the query is cancelled on the server.
    pub(super) fn read_batches(
        &self,
        sql: &str,
        batch: usize,
        each: &mut TextBatch<'_>,
    ) -> DbResult<()> {
        let rt = &self.backend.rt;
        let stream = rt
            .block_on(self.client.simple_query_raw(sql))
            .map_err(|e| describe(&e))?;
        let mut stream = Box::pin(stream.fuse());
        let mut columns: Vec<String> = Vec::new();
        loop {
            // `each` is called outside the runtime: it may use it too (the
            // destination of a copy is another connection of this backend)
            let rows = rt.block_on(async {
                let mut rows = Vec::with_capacity(batch);
                while rows.len() < batch {
                    match stream.next().await {
                        Some(Ok(SimpleQueryMessage::RowDescription(description))) => {
                            columns = description.iter().map(|c| c.name().to_string()).collect();
                        }
                        Some(Ok(SimpleQueryMessage::Row(row))) => rows.push(
                            (0..row.len())
                                .map(|i| row.get(i).map(str::to_string))
                                .collect(),
                        ),
                        Some(Ok(_)) => {}
                        Some(Err(e)) => return Err(describe(&e)),
                        None => break,
                    }
                }
                Ok(rows)
            })?;
            if rows.is_empty() {
                return Ok(());
            }
            if let Err(error) = each(&columns, rows) {
                // So the server stops sending a table that is no longer wanted;
                // it is read to the end so that this query, and not the next one,
                // receives the cancellation
                let cancel = self.client.cancel_token();
                let _ = rt.block_on(cancel.cancel_query(self.backend.tls.clone()));
                rt.block_on(async { while stream.next().await.is_some() {} });
                return Err(error);
            }
        }
    }
}

/// SQL editor connection to a database: kept between runs so that
/// transactions, session settings and temporary tables stay alive.
type EditorSession = Arc<Mutex<Option<Client>>>;

pub struct PostgresBackend {
    /// Drives the connections; calls wait on it from their threads.
    rt: Runtime,
    /// Server and credentials, without a database.
    config: Config,
    tls: MakeRustlsConnect,
    /// The connection is limited to this database: it is the only one listed.
    only_database: Option<String>,
    /// Database used to ask about the server (the list of databases, creating
    /// and dropping them) and where anything that names none runs.
    home: String,
    /// Whether the connection is encrypted, according to the server on connect.
    security: Security,
    pools: Pools,
    sessions: Mutex<HashMap<String, EditorSession>>,
    /// Editor runs currently on the server, with what is needed to cancel
    /// them.
    running: Mutex<Vec<(CancelFlag, CancelToken)>>,
}

impl PostgresBackend {
    pub fn connect_with_progress(
        params: &ConnectionParams,
        on_status: impl Fn(&str),
    ) -> DbResult<Self> {
        if params.keyring_id.is_some() && params.password.is_empty() {
            on_status(&t!("connect-reading-keyring"));
        }
        let password = params.resolve_password()?;
        let mut config = Config::new();
        config
            .host(params.host.trim())
            .port(params.port)
            .user(params.user.trim())
            .password(password)
            .application_name("RustQL")
            .connect_timeout(Duration::from_secs(15))
            // With "encrypt if possible", no TLS if the server does not offer it
            .ssl_mode(match params.tls {
                TlsMode::Prefer => SslMode::Prefer,
                TlsMode::Require | TlsMode::Verify => SslMode::Require,
            });

        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("rustql-postgres")
            .enable_all()
            .build()
            .map_err(|e| t!("connect-setup-failed", error = e.to_string()))?;
        let mut backend = Self {
            rt,
            config,
            tls: MakeRustlsConnect::new(tls::client_config(params.tls, params.ca_file())?),
            only_database: params.only_database().map(str::to_string),
            home: String::new(),
            security: Security::Plain,
            pools: Pools::default(),
            sessions: Mutex::default(),
            running: Mutex::default(),
        };
        backend
            .rt
            .spawn(close_idle_connections(Arc::downgrade(&backend.pools)));

        on_status(&t!("connect-connecting"));
        let candidates: Vec<&str> = match params.only_database() {
            Some(only) => vec![only],
            None => HOME_DATABASES
                .into_iter()
                .chain([params.user.trim()])
                .collect(),
        };
        let mut first_error = None;
        let mut candidates = candidates.into_iter();
        let mut next = candidates.next();
        while let Some(db) = next {
            match backend.rt.block_on(backend.open(db)) {
                Ok(client) => {
                    let encrypted = backend
                        .rt
                        .block_on(read_results(
                            &client,
                            "SELECT ssl FROM pg_stat_ssl WHERE pid = pg_backend_pid()",
                            1,
                        ))
                        .is_ok_and(|(result, _)| result.rows == [[Some("t".to_string())]]);
                    backend.security = Security::of(encrypted, &params.host);
                    backend.home = db.to_string();
                    lock(&backend.pool(db).idle).push((Instant::now(), client));
                    on_status(&t!("connect-established"));
                    return Ok(backend);
                }
                // "Encrypt if possible", and it is not: the server offers TLS but no
                // agreement is reached (only old versions).
                // Like libpq, it connects unencrypted
                Err(e)
                    if params.tls == TlsMode::Prefer
                        && backend.config.get_ssl_mode() != SslMode::Disable
                        && is_tls_failure(&e) =>
                {
                    backend.config.ssl_mode(SslMode::Disable);
                    continue;
                }
                Err(e) => {
                    // Trying another database is only worth it if the failure is this
                    // one's: it does not exist or entering it is not allowed
                    let about_database = matches!(
                        e.code(),
                        Some(code) if *code == SqlState::INVALID_CATALOG_NAME
                            || *code == SqlState::INVALID_AUTHORIZATION_SPECIFICATION
                            || *code == SqlState::INSUFFICIENT_PRIVILEGE
                    );
                    first_error.get_or_insert(e);
                    if !about_database {
                        break;
                    }
                    next = candidates.next();
                }
            }
        }
        let error = first_error.map(|e| describe(&e)).unwrap_or_default();
        let mut hint = tls::hint(params.tls, &error);
        if hint.is_empty() && params.only_database().is_none() {
            hint = format!("\n\n{}", t!("connect-hint-database"));
        }
        Err(t!("connect-failed", error = &error, hint = hint))
    }

    /// Opens a new connection to `db`.
    async fn open(&self, db: &str) -> Result<Client, tokio_postgres::Error> {
        let mut config = self.config.clone();
        config.dbname(db);
        let (client, connection) = config.connect(self.tls.clone()).await?;
        // The connection lives as long as its client
        tokio::spawn(async move {
            if let Err(e) = connection.await {
                ::log::debug!("PostgreSQL connection closed: {e}");
            }
        });
        Ok(client)
    }

    fn pool(&self, db: &str) -> Arc<Pool> {
        Arc::clone(lock(&self.pools).entry(db.to_string()).or_default())
    }

    /// A connection to `db` (or to the starting database): one of those
    /// waiting in its pool or, with `fresh` or if there are none, a new one.
    async fn client_from(&self, db: Option<&str>, fresh: bool) -> DbResult<Pooled> {
        let db = db.unwrap_or(&self.home);
        let pool = self.pool(db);
        // Those waiting in the pool; if the server closed one meanwhile, it
        // is discarded
        let idle = std::iter::from_fn(|| lock(&pool.idle).pop());
        let alive = idle
            .map(|(_, client)| client)
            .find(|client| !fresh && !client.is_closed());
        if let Some(client) = alive {
            return Ok(Pooled {
                client: Some(client),
                pool,
            });
        }
        let client = self
            .open(db)
            .await
            .map_err(|e| t!("db-no-connection-to", db = db, error = describe(&e)))?;
        Ok(Pooled {
            client: Some(client),
            pool,
        })
    }

    async fn client(&self, db: Option<&str>) -> DbResult<Pooled> {
        self.client_from(db, false).await
    }

    /// Full read in `db`.
    async fn query(&self, db: Option<&str>, sql: &str) -> DbResult<QueryResult> {
        let mut result = read_results(&*self.client(db).await?, sql, usize::MAX).await;
        // The connection waiting in the pool turned out to be dead: a read
        // can simply be repeated on a new one
        if result.as_ref().is_err_and(connection_lost) {
            let client = self.client_from(db, true).await?;
            result = read_results(&client, sql, usize::MAX).await;
        }
        result
            .map(|(result, _)| result)
            .map_err(|e| t!("db-query-error", error = describe(&e)))
    }

    /// Tables of `db` with the name they have in the tree: those of the
    /// default schema first and without a prefix.
    async fn tables(&self, db: &str) -> DbResult<Vec<TableEntry>> {
        let sql = format!(
            "SELECT n.nspname, c.relname, c.relkind \
             FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE {USER_RELATIONS} \
             ORDER BY n.nspname <> 'public', n.nspname, c.relname"
        );
        let result = self.query(Some(db), &sql).await?;
        Ok(result
            .rows
            .iter()
            .map(|row| TableEntry {
                name: pg_table_name(&col(row, 0), &col(row, 1)),
                kind: match col(row, 2).as_str() {
                    "v" => TableKind::View,
                    "m" => TableKind::MaterializedView,
                    _ => TableKind::Table,
                },
            })
            .collect())
    }

    /// Opens an own connection to `db`.
    pub(super) fn dedicated(&self, db: &str) -> DbResult<Dedicated<'_>> {
        let client = self
            .rt
            .block_on(self.open(db))
            .map_err(|e| t!("db-no-connection-to", db = db, error = describe(&e)))?;
        Ok(Dedicated {
            backend: self,
            client,
        })
    }

    /// Creates database `name` if it does not exist (there is no `CREATE
    /// DATABASE IF NOT EXISTS`).
    pub(super) fn ensure_database(&self, name: &str) -> DbResult<()> {
        let sql = format!(
            "SELECT 1 FROM pg_database WHERE datname = {}",
            SQL.literal(name)
        );
        if self.execute_query(None, &sql)?.rows.is_empty() {
            self.create_database(name, "")?;
        }
        Ok(())
    }

    /// Every database on the server the user can enter, even if the connection
    /// is limited to one (they are visible from any of them).
    pub fn all_databases(&self) -> DbResult<Vec<String>> {
        let sql = format!("SELECT d.datname FROM pg_database d WHERE {USER_DATABASES} ORDER BY 1");
        let result = self.rt.block_on(self.query(None, &sql))?;
        Ok(result.rows.iter().map(|row| col(row, 0)).collect())
    }

    /// Closes the own connections to `db` (those not in use).
    fn close_database(&self, db: &str) {
        lock(&self.pools).remove(db);
        lock(&self.sessions).remove(db);
    }
}

fn col(row: &[Option<String>], i: usize) -> String {
    row.get(i).and_then(|v| v.clone()).unwrap_or_default()
}

fn number<T: std::str::FromStr + Default>(row: &[Option<String>], i: usize) -> T {
    col(row, i).parse().unwrap_or_default()
}

/// Boolean column, which arrives in text format as "t" or "f".
fn flag(row: &[Option<String>], i: usize) -> bool {
    col(row, i) == "t"
}

impl DbBackend for PostgresBackend {
    fn engine(&self) -> Engine {
        SQL
    }

    fn security(&self) -> Security {
        self.security
    }

    fn list_databases(&self) -> DbResult<Vec<String>> {
        match &self.only_database {
            Some(only) => Ok(vec![only.clone()]),
            None => self.all_databases(),
        }
    }

    fn list_tables(&self, database: &str) -> DbResult<Vec<TableEntry>> {
        self.rt.block_on(self.tables(database))
    }

    fn view_definition(&self, db: &str, view: &str) -> DbResult<String> {
        let (schema, name) = pg_split_table(view);
        let sql = format!(
            "SELECT c.relkind, pg_get_viewdef(c.oid, true) \
             FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname = {} AND c.relname = {} AND c.relkind IN ('v', 'm')",
            SQL.literal(&schema),
            SQL.literal(name)
        );
        let result = self.rt.block_on(self.query(Some(db), &sql))?;
        let row = result
            .rows
            .first()
            .ok_or_else(|| t!("view-definition-missing", view = view))?;
        let definition = col(row, 1);
        let definition = definition.trim().trim_end_matches(';');
        let reference = SQL.qualified(db, view);
        Ok(if col(row, 0) == "m" {
            format!(
                "{}\nCREATE MATERIALIZED VIEW {reference} AS\n{definition}",
                t!("view-materialized-comment")
            )
        } else {
            format!("CREATE OR REPLACE VIEW {reference} AS\n{definition}")
        })
    }

    fn list_all_tables(&self) -> DbResult<Vec<(String, TableEntry)>> {
        // No query can see the tables of another database: each one is
        // asked. Those that do not let the user in are left without tables.
        let databases = self.list_databases()?;
        let per_database = self.rt.block_on(
            stream::iter(databases)
                .map(|db| async move {
                    let tables = self.tables(&db).await.unwrap_or_default();
                    (db, tables)
                })
                .buffered(ALL_TABLES_CONCURRENCY)
                .collect::<Vec<_>>(),
        );
        Ok(per_database
            .into_iter()
            .flat_map(|(db, tables)| tables.into_iter().map(move |t| (db.clone(), t)))
            .collect())
    }

    fn execute_query(&self, db: Option<&str>, sql: &str) -> DbResult<QueryResult> {
        self.rt.block_on(self.query(db, sql))
    }

    fn execute_in_session(
        &self,
        db: Option<&str>,
        sql: &str,
        max_rows: usize,
        cancel: &CancelFlag,
    ) -> DbResult<QueryResult> {
        let db = db.unwrap_or(&self.home);
        let session = Arc::clone(lock(&self.sessions).entry(db.to_string()).or_default());
        // The turn is awaited here: in each database, editor runs go one
        // at a time
        let mut session = lock(&session);
        if session.as_ref().is_none_or(Client::is_closed) {
            // The first time, or if the connection was lost (and the session
            // state with it)
            let client = self
                .rt
                .block_on(self.open(db))
                .map_err(|e| t!("editor-session-failed", error = describe(&e)))?;
            *session = Some(client);
        }
        let client = session.as_ref().expect("just opened");
        // Announce first and then check the signal: whoever cancels does the
        // opposite, so at least one of the two sees the other
        lock(&self.running).push((Arc::clone(cancel), client.cancel_token()));
        let result = if cancel.load(Ordering::SeqCst) {
            None
        } else {
            Some(self.rt.block_on(read_results(client, sql, max_rows)))
        };
        lock(&self.running).retain(|(flag, _)| !Arc::ptr_eq(flag, cancel));
        match result {
            None => Err(QUERY_CANCELLED.into()),
            Some(Ok((data, _))) => Ok(data),
            Some(Err(e)) if is_cancelled(&e) => Err(QUERY_CANCELLED.into()),
            Some(Err(e)) => Err(t!("db-query-error", error = describe(&e))),
        }
    }

    fn cancel_in_session(&self, cancel: &CancelFlag) -> DbResult<()> {
        cancel.store(true, Ordering::SeqCst);
        // Only if it is the one on the server. One waiting for its turn will
        // see the signal on arrival
        let token = lock(&self.running)
            .iter()
            .find(|(flag, _)| Arc::ptr_eq(flag, cancel))
            .map(|(_, token)| token.clone());
        let Some(token) = token else {
            return Ok(());
        };
        // It goes through another connection: the editor's is busy with the
        // query. If it arrives after the query finished, the server does nothing
        self.rt
            .block_on(token.cancel_query(self.tls.clone()))
            .map_err(|e| t!("cancel-query-failed", error = describe(&e)))
    }

    fn get_databases_info(&self) -> DbResult<Vec<DatabaseInfo>> {
        let only = match &self.only_database {
            Some(only) => format!("AND d.datname = {}", SQL.literal(only)),
            None => String::new(),
        };
        // Tables can only be counted in the database one is in
        let sql = format!(
            "SELECT d.datname, pg_encoding_to_char(d.encoding), d.datcollate, \
                    pg_database_size(d.datname), \
                    CASE WHEN d.datname = current_database() THEN ( \
                        SELECT count(*) FROM pg_class c \
                        JOIN pg_namespace n ON n.oid = c.relnamespace \
                        WHERE {USER_RELATIONS}) END \
             FROM pg_database d WHERE {USER_DATABASES} {only} ORDER BY 1"
        );
        let result = self.rt.block_on(self.query(None, &sql))?;
        Ok(result
            .rows
            .iter()
            .map(|row| DatabaseInfo {
                name: col(row, 0),
                charset: col(row, 1),
                collation: col(row, 2),
                size_bytes: number(row, 3),
                table_count: row.get(4).cloned().flatten().and_then(|n| n.parse().ok()),
            })
            .collect())
    }

    fn get_tables_info(&self, db: &str) -> DbResult<Vec<TableInfo>> {
        // reltuples is an estimate (-1 if the table was never analyzed)
        let sql = format!(
            "SELECT n.nspname, c.relname, COALESCE(a.amname, ''), \
                    GREATEST(c.reltuples, 0)::bigint, pg_total_relation_size(c.oid) \
             FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
             LEFT JOIN pg_am a ON a.oid = c.relam \
             WHERE {USER_RELATIONS} \
             ORDER BY n.nspname <> 'public', n.nspname, c.relname"
        );
        let result = self.rt.block_on(self.query(Some(db), &sql))?;
        Ok(result
            .rows
            .iter()
            .map(|row| TableInfo {
                name: pg_table_name(&col(row, 0), &col(row, 1)),
                engine: col(row, 2),
                row_count: number(row, 3),
                size_bytes: number(row, 4),
                collation: String::new(),
            })
            .collect())
    }

    fn get_columns_info(&self, db: &str, table: &str) -> DbResult<Vec<ColumnInfo>> {
        let (schema, table) = pg_split_table(table);
        let sql = format!(
            "SELECT a.attname, format_type(a.atttypid, a.atttypmod), NOT a.attnotnull, \
                    pg_get_expr(d.adbin, d.adrelid), \
                    (SELECT CASE WHEN bool_or(i.indisprimary) THEN 'PRI' \
                                 WHEN bool_or(i.indisunique) THEN 'UNI' \
                                 WHEN count(*) > 0 THEN 'MUL' END \
                     FROM pg_index i \
                     WHERE i.indrelid = a.attrelid AND a.attnum = ANY (i.indkey)), \
                    a.attidentity, a.attgenerated \
             FROM pg_attribute a \
             JOIN pg_class c ON c.oid = a.attrelid \
             JOIN pg_namespace n ON n.oid = c.relnamespace \
             LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum \
             WHERE n.nspname = {} AND c.relname = {} AND a.attnum > 0 AND NOT a.attisdropped \
             ORDER BY a.attnum",
            SQL.literal(&schema),
            SQL.literal(table)
        );
        let result = self.rt.block_on(self.query(Some(db), &sql))?;
        Ok(result
            .rows
            .iter()
            .map(|row| {
                let default_val = row.get(3).cloned().flatten();
                let (identity, generated) = (col(row, 5), col(row, 6));
                let extra = match (identity.as_str(), generated.as_str()) {
                    ("a", _) => "identity (always)",
                    ("d", _) => "identity",
                    (_, "s") => "generated (stored)",
                    (_, "v") => "generated (virtual)",
                    _ => "",
                };
                // A sequence (serial), an identity or an expression: the server
                // sets the value
                let from_sequence = default_val
                    .as_deref()
                    .is_some_and(|d| d.starts_with("nextval("));
                ColumnInfo {
                    name: col(row, 0),
                    col_type: col(row, 1),
                    is_nullable: flag(row, 2),
                    default_val,
                    column_key: col(row, 4),
                    is_auto: from_sequence || !extra.is_empty(),
                    extra: extra.to_string(),
                }
            })
            .collect())
    }

    fn execute_write(&self, db: Option<&str>, sql: &str) -> DbResult<u64> {
        self.rt.block_on(async {
            let client = self.client(db).await?;
            read_results(&client, sql, usize::MAX)
                .await
                .map(|(_, affected)| affected)
                .map_err(|e| t!("db-write-error", error = describe(&e)))
        })
    }

    /// PostgreSQL has no per-database charset like MySQL: it is created with
    /// the server encoding and `charset` is not used.
    fn create_database(&self, name: &str, _charset: &str) -> DbResult<()> {
        let sql = format!("CREATE DATABASE {}", SQL.quote_ident(name));
        self.execute_write(None, &sql).map(drop)
    }

    fn drop_database(&self, name: &str) -> DbResult<()> {
        // The server does not drop a database with open connections: our
        // own are closed first. They take a moment to close completely (or
        // are finishing a query), so it insists a little
        self.close_database(name);
        let sql = format!("DROP DATABASE {}", SQL.quote_ident(name));
        self.rt.block_on(async {
            let client = self.client(None).await?;
            let mut attempts = 0;
            loop {
                match client.batch_execute(&sql).await {
                    Ok(()) => return Ok(()),
                    Err(e) if e.code() == Some(&SqlState::OBJECT_IN_USE) && attempts < 10 => {
                        attempts += 1;
                        tokio::time::sleep(Duration::from_millis(200)).await;
                    }
                    Err(e) => return Err(t!("db-write-error", error = describe(&e))),
                }
            }
        })
    }

    fn drop_table(&self, db: &str, table: &str, kind: TableKind) -> DbResult<()> {
        let sql = SQL.drop_table(db, table, kind);
        self.execute_write(Some(db), &sql).map(drop)
    }

    fn truncate_table(&self, db: &str, table: &str) -> DbResult<()> {
        let sql = format!("TRUNCATE TABLE {}", SQL.qualified(db, table));
        self.execute_write(Some(db), &sql).map(drop)
    }

    fn execute_transaction(&self, db: Option<&str>, stmts: &[Statement]) -> DbResult<()> {
        self.rt.block_on(async {
            let client = self.client(db).await?;
            client
                .batch_execute("BEGIN")
                .await
                .map_err(|e| t!("db-begin-error", error = describe(&e)))?;
            let mut failure = None;
            for stmt in stmts {
                let params = stmt.params.iter().map(|p| TextParam(p.as_deref()));
                match client.execute_raw(stmt.sql.as_str(), params).await {
                    // An UPDATE counts the rows it finds, whether they change or not
                    Ok(found) if stmt.single_row && found != 1 => {
                        failure = Some(format!(
                            "{}\n\n{}",
                            t!("save-row-not-found", found = found),
                            stmt.sql
                        ));
                    }
                    Ok(_) => continue,
                    Err(e) => {
                        failure = Some(t!("db-transaction-error", error = describe(&e)));
                    }
                }
                break;
            }
            let Some(failure) = failure else {
                return client
                    .batch_execute("COMMIT")
                    .await
                    .map_err(|e| t!("db-commit-error", error = describe(&e)));
            };
            if client.batch_execute("ROLLBACK").await.is_err() {
                client.discard();
            }
            Err(failure)
        })
    }
}

#[cfg(test)]
mod tests;

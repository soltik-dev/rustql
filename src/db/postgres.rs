//! Backend de PostgreSQL.
//!
//! En PostgreSQL una conexión pertenece a una sola base de datos. Para que
//! el resto del programa lo vea como MySQL (un servidor con sus BBDD), el
//! backend abre conexiones a cada BBDD según se le piden y dirige cada
//! llamada a la suya. Los esquemas se aplanan en el nombre de la tabla (ver
//! `dialect::pg_table_name`).
//!
//! Todo se lee en formato texto, que es como el servidor escribe cualquier
//! tipo (arrays, JSON, fechas…) sin que haya que conocerlo: las lecturas van
//! por el protocolo simple y los parámetros de las escrituras, como texto
//! que el servidor convierte al tipo de su columna.

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use futures_util::{StreamExt, pin_mut, stream};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use tokio::runtime::Runtime;
use tokio_postgres::error::SqlState;
use tokio_postgres::types::private::BytesMut;
use tokio_postgres::types::{Format, IsNull, ToSql, Type, to_sql_checked};
use tokio_postgres::{CancelToken, Client, Config, SimpleQueryMessage};
use tokio_postgres_rustls::MakeRustlsConnect;

use super::dialect::{pg_split_table, pg_table_name};
use super::{
    CancelFlag, ColumnInfo, ConnectionParams, DatabaseInfo, DbBackend, DbResult, Engine,
    QUERY_CANCELLED, QueryResult, Statement, TableInfo,
};

const SQL: Engine = Engine::Postgres;

/// BBDD a las que se intenta entrar, por orden, cuando la conexión no se
/// limita a ninguna: desde ella se listan las demás. Tras estas, la que se
/// llama como el usuario.
const HOME_DATABASES: [&str; 2] = ["postgres", "template1"];

/// Conexiones que se guardan abiertas por BBDD para la siguiente consulta.
/// Las que hagan falta de más a la vez se abren y se cierran al terminar.
const MAX_IDLE: usize = 2;

/// Una conexión que lleva este tiempo sin usarse se cierra: con muchas BBDD
/// abiertas, las de cada una se acumularían en el servidor.
const IDLE_TIMEOUT: Duration = Duration::from_secs(120);

/// BBDD a las que se pregunta a la vez al listar las tablas de todas.
const ALL_TABLES_CONCURRENCY: usize = 4;

/// Relaciones que el árbol enseña como tablas: tablas (también particionadas
/// y foráneas), vistas y vistas materializadas de los esquemas del usuario.
/// Las particiones no salen: se ve su tabla.
const USER_RELATIONS: &str = "c.relkind IN ('r', 'p', 'v', 'm', 'f') \
     AND NOT c.relispartition \
     AND n.nspname NOT IN ('pg_catalog', 'information_schema') \
     AND n.nspname !~ '^pg_(toast|temp)' \
     AND has_schema_privilege(n.oid, 'USAGE')";

/// BBDD a las que el usuario puede entrar.
const USER_DATABASES: &str =
    "d.datallowconn AND NOT d.datistemplate AND has_database_privilege(d.datname, 'CONNECT')";

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// El error tal como lo cuenta el servidor, con su detalle y su pista.
fn describe(e: &tokio_postgres::Error) -> String {
    let Some(db) = e.as_db_error() else {
        return e.to_string();
    };
    let mut text = db.message().to_string();
    for extra in [db.detail(), db.hint()].into_iter().flatten() {
        text.push('\n');
        text.push_str(extra);
    }
    text
}

fn is_cancelled(e: &tokio_postgres::Error) -> bool {
    e.code() == Some(&SqlState::QUERY_CANCELED)
}

/// Parámetro enviado como texto: el servidor lo convierte al tipo que la
/// sentencia espera en esa posición, igual que haría con un literal.
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

/// Acepta cualquier certificado del servidor: la conexión va cifrada pero no
/// se comprueba con quién se habla. Es lo que hace libpq por defecto
/// (`sslmode=prefer`), y lo que permite entrar en los servidores con
/// certificado autofirmado, que son la mayoría.
#[derive(Debug)]
struct AnyCertificate(Arc<CryptoProvider>);

impl ServerCertVerifier for AnyCertificate {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

fn tls_connector() -> DbResult<MakeRustlsConnect> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(Arc::clone(&provider))
        .with_safe_default_protocol_versions()
        .map_err(|e| format!("No se pudo preparar TLS: {e}"))?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AnyCertificate(provider)))
        .with_no_client_auth();
    Ok(MakeRustlsConnect::new(config))
}

/// Lee el primer conjunto de resultados de una ejecución (el de la primera
/// sentencia que devuelve filas) y, como mucho, `max_rows` filas. Devuelve
/// también las filas afectadas por la última sentencia.
async fn read_results(
    client: &Client,
    sql: &str,
    max_rows: usize,
) -> Result<(QueryResult, u64), tokio_postgres::Error> {
    let stream = client.simple_query_raw(sql).await?;
    pin_mut!(stream);
    let mut result: Option<QueryResult> = None;
    // Las filas que llegan son del conjunto que se guarda
    let mut reading = false;
    let mut affected = 0;
    while let Some(message) = stream.next().await {
        match message? {
            // Llega aunque la consulta no devuelva ninguna fila
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
                        // Al soltar el stream, el resto se lee y se descarta
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

/// Conexiones abiertas a una BBDD, a la espera de la siguiente consulta, con
/// el momento en que se dejaron.
#[derive(Default)]
struct Pool {
    idle: Mutex<Vec<(Instant, Client)>>,
}

type Pools = Arc<Mutex<HashMap<String, Arc<Pool>>>>;

/// Cierra cada cierto tiempo las conexiones que llevan `IDLE_TIMEOUT` sin
/// usarse. Termina cuando el backend desaparece.
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

/// Conexión a una BBDD, sacada de su `Pool`: vuelve a él al soltarla.
struct Pooled {
    client: Option<Client>,
    pool: Arc<Pool>,
}

impl Pooled {
    /// No vuelve al pool: quedó en un estado que no se conoce (una
    /// transacción que no se pudo deshacer).
    fn discard(mut self) {
        self.client = None;
    }
}

impl std::ops::Deref for Pooled {
    type Target = Client;
    fn deref(&self) -> &Client {
        self.client.as_ref().expect("solo falta al soltarla")
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

/// Conexión del editor SQL a una BBDD: se mantiene entre ejecuciones para
/// que transacciones, ajustes de sesión y tablas temporales sigan vivos.
type EditorSession = Arc<Mutex<Option<Client>>>;

pub struct PostgresBackend {
    /// Mueve las conexiones; las llamadas esperan en él desde sus hilos.
    rt: Runtime,
    /// Servidor y credenciales, sin BBDD.
    config: Config,
    tls: MakeRustlsConnect,
    /// La conexión se limita a esta BBDD: es la única que se lista.
    only_database: Option<String>,
    /// BBDD desde la que se pregunta por el servidor (la lista de BBDD,
    /// crearlas, borrarlas) y en la que se ejecuta lo que no nombra ninguna.
    home: String,
    pools: Pools,
    sessions: Mutex<HashMap<String, EditorSession>>,
    /// Ejecuciones del editor que están ahora mismo en el servidor, con lo
    /// necesario para cancelarlas.
    running: Mutex<Vec<(CancelFlag, CancelToken)>>,
}

impl PostgresBackend {
    pub fn connect_with_progress(
        params: &ConnectionParams,
        on_status: impl Fn(&str),
    ) -> DbResult<Self> {
        if params.keyring_id.is_some() && params.password.is_empty() {
            on_status("Leyendo la contraseña del llavero...");
        }
        let password = params.resolve_password()?;
        let mut config = Config::new();
        config
            .host(params.host.trim())
            .port(params.port)
            .user(params.user.trim())
            .password(password)
            .application_name("RustQL")
            .connect_timeout(Duration::from_secs(15));

        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("rustql-postgres")
            .enable_all()
            .build()
            .map_err(|e| format!("No se pudo preparar la conexión: {e}"))?;
        let mut backend = Self {
            rt,
            config,
            tls: tls_connector()?,
            only_database: params.only_database().map(str::to_string),
            home: String::new(),
            pools: Pools::default(),
            sessions: Mutex::default(),
            running: Mutex::default(),
        };
        backend
            .rt
            .spawn(close_idle_connections(Arc::downgrade(&backend.pools)));

        on_status("Conectando con el servidor...");
        let candidates: Vec<&str> = match params.only_database() {
            Some(only) => vec![only],
            None => HOME_DATABASES
                .into_iter()
                .chain([params.user.trim()])
                .collect(),
        };
        let mut first_error = None;
        for db in candidates {
            match backend.rt.block_on(backend.open(db)) {
                Ok(client) => {
                    backend.home = db.to_string();
                    lock(&backend.pool(db).idle).push((Instant::now(), client));
                    on_status("Conexión establecida.");
                    return Ok(backend);
                }
                Err(e) => {
                    // Solo merece la pena probar otra BBDD si el fallo es de
                    // esta: no existe o no se permite entrar en ella
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
                }
            }
        }
        let error = first_error.map(|e| describe(&e)).unwrap_or_default();
        Err(if params.only_database().is_some() {
            format!("No se pudo conectar: {error}")
        } else {
            format!(
                "No se pudo conectar: {error}\n\nSi el servidor solo te deja entrar en tu base \
                 de datos, indícala en el campo «Base de datos»."
            )
        })
    }

    /// Abre una conexión nueva a `db`.
    async fn open(&self, db: &str) -> Result<Client, tokio_postgres::Error> {
        let mut config = self.config.clone();
        config.dbname(db);
        let (client, connection) = config.connect(self.tls.clone()).await?;
        // La conexión vive mientras viva su cliente
        tokio::spawn(async move {
            if let Err(e) = connection.await {
                ::log::debug!("conexión de PostgreSQL cerrada: {e}");
            }
        });
        Ok(client)
    }

    fn pool(&self, db: &str) -> Arc<Pool> {
        Arc::clone(lock(&self.pools).entry(db.to_string()).or_default())
    }

    /// Una conexión a `db` (o a la BBDD de partida): una de las que esperan
    /// en su pool o, con `fresh` o si no hay, una nueva.
    async fn client_from(&self, db: Option<&str>, fresh: bool) -> DbResult<Pooled> {
        let db = db.unwrap_or(&self.home);
        let pool = self.pool(db);
        // Las que esperan en el pool; si el servidor cerró alguna mientras
        // tanto, se descarta
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
            .map_err(|e| format!("Sin conexión con «{db}»: {}", describe(&e)))?;
        Ok(Pooled {
            client: Some(client),
            pool,
        })
    }

    async fn client(&self, db: Option<&str>) -> DbResult<Pooled> {
        self.client_from(db, false).await
    }

    /// Lectura completa en `db`.
    async fn query(&self, db: Option<&str>, sql: &str) -> DbResult<QueryResult> {
        let mut result = read_results(&*self.client(db).await?, sql, usize::MAX).await;
        // La conexión que esperaba en el pool resultó estar caída (el
        // servidor se reinició, un cortafuegos la cortó): una lectura se
        // puede repetir sin más en una nueva
        if result.as_ref().is_err_and(tokio_postgres::Error::is_closed) {
            let client = self.client_from(db, true).await?;
            result = read_results(&client, sql, usize::MAX).await;
        }
        result
            .map(|(result, _)| result)
            .map_err(|e| format!("Error en query: {}", describe(&e)))
    }

    /// Tablas de `db` con el nombre que llevan en el árbol: las del esquema
    /// por defecto primero y sin prefijo.
    async fn tables(&self, db: &str) -> DbResult<Vec<String>> {
        let sql = format!(
            "SELECT n.nspname, c.relname \
             FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE {USER_RELATIONS} \
             ORDER BY n.nspname <> 'public', n.nspname, c.relname"
        );
        let result = self.query(Some(db), &sql).await?;
        Ok(result
            .rows
            .iter()
            .map(|row| pg_table_name(&col(row, 0), &col(row, 1)))
            .collect())
    }

    /// Todas las BBDD del servidor en las que el usuario puede entrar, aunque
    /// la conexión se limite a una (se ven desde cualquiera).
    pub fn all_databases(&self) -> DbResult<Vec<String>> {
        let sql = format!("SELECT d.datname FROM pg_database d WHERE {USER_DATABASES} ORDER BY 1");
        let result = self.rt.block_on(self.query(None, &sql))?;
        Ok(result.rows.iter().map(|row| col(row, 0)).collect())
    }

    /// Cierra las conexiones propias a `db` (las que no estén en uso).
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

/// Columna booleana, que en formato texto llega como «t» o «f».
fn flag(row: &[Option<String>], i: usize) -> bool {
    col(row, i) == "t"
}

impl DbBackend for PostgresBackend {
    fn engine(&self) -> Engine {
        SQL
    }

    fn list_databases(&self) -> DbResult<Vec<String>> {
        match &self.only_database {
            Some(only) => Ok(vec![only.clone()]),
            None => self.all_databases(),
        }
    }

    fn list_tables(&self, database: &str) -> DbResult<Vec<String>> {
        self.rt.block_on(self.tables(database))
    }

    fn list_all_tables(&self) -> DbResult<Vec<(String, String)>> {
        // No hay consulta que vea las tablas de otra BBDD: se pregunta en
        // cada una. Las que no dejen entrar se quedan sin tablas.
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
        // El turno se espera aquí: en cada BBDD, las ejecuciones del editor
        // van de una en una
        let mut session = lock(&session);
        if session.as_ref().is_none_or(Client::is_closed) {
            // La primera vez, o si la conexión se perdió (y con ella el
            // estado de la sesión)
            let client = self
                .rt
                .block_on(self.open(db))
                .map_err(|e| format!("No se pudo abrir la sesión del editor: {}", describe(&e)))?;
            *session = Some(client);
        }
        let client = session.as_ref().expect("recién abierta");
        // Primero anunciarse y luego mirar la señal: quien cancela hace lo
        // contrario, así que al menos uno de los dos ve al otro
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
            Some(Err(e)) => Err(format!("Error en query: {}", describe(&e))),
        }
    }

    fn cancel_in_session(&self, cancel: &CancelFlag) -> DbResult<()> {
        cancel.store(true, Ordering::SeqCst);
        // Solo si es la que está en el servidor. La que espera su turno verá
        // la señal al llegar
        let token = lock(&self.running)
            .iter()
            .find(|(flag, _)| Arc::ptr_eq(flag, cancel))
            .map(|(_, token)| token.clone());
        let Some(token) = token else {
            return Ok(());
        };
        // Va por otra conexión: la del editor está ocupada con la consulta.
        // Si llega con la consulta ya terminada, el servidor no hace nada
        self.rt
            .block_on(token.cancel_query(self.tls.clone()))
            .map_err(|e| format!("No se pudo cancelar la consulta: {}", describe(&e)))
    }

    fn get_databases_info(&self) -> DbResult<Vec<DatabaseInfo>> {
        let only = match &self.only_database {
            Some(only) => format!("AND d.datname = {}", SQL.literal(only)),
            None => String::new(),
        };
        // Las tablas solo se pueden contar en la BBDD en la que se está
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
        // reltuples es una estimación (-1 si la tabla nunca se ha analizado)
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
            SQL.literal(schema),
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
                // Una secuencia (serial), una identidad o una expresión: el
                // valor lo pone el servidor
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
                .map_err(|e| format!("Error en escritura: {}", describe(&e)))
        })
    }

    /// PostgreSQL no tiene charset por BBDD como MySQL: se crea con la
    /// codificación del servidor y `charset` no se usa.
    fn create_database(&self, name: &str, _charset: &str) -> DbResult<()> {
        let sql = format!("CREATE DATABASE {}", SQL.quote_ident(name));
        self.execute_write(None, &sql).map(drop)
    }

    fn drop_database(&self, name: &str) -> DbResult<()> {
        // El servidor no borra una BBDD con conexiones abiertas: las propias
        // se cierran antes. Tardan un instante en cerrarse del todo (o están
        // terminando una consulta), así que se insiste un poco
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
                    Err(e) => return Err(format!("Error en escritura: {}", describe(&e))),
                }
            }
        })
    }

    fn drop_table(&self, db: &str, table: &str) -> DbResult<()> {
        let sql = format!("DROP TABLE {}", SQL.qualified(db, table));
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
                .map_err(|e| format!("Error iniciando transacción: {}", describe(&e)))?;
            let mut failure = None;
            for stmt in stmts {
                let params = stmt.params.iter().map(|p| TextParam(p.as_deref()));
                match client.execute_raw(stmt.sql.as_str(), params).await {
                    // Un UPDATE cuenta las filas que encuentra, cambien o no
                    Ok(found) if stmt.single_row && found != 1 => {
                        failure = Some(format!(
                            "No se ha guardado nada: una sentencia debía cambiar una fila y \
                             encontró {found}. La fila puede haber cambiado o desaparecido en \
                             el servidor; recarga la tabla (F5) y repite los cambios.\n\n{}",
                            stmt.sql
                        ));
                    }
                    Ok(_) => continue,
                    Err(e) => {
                        failure =
                            Some(format!("Error en transacción (ROLLBACK): {}", describe(&e)));
                    }
                }
                break;
            }
            let Some(failure) = failure else {
                return client
                    .batch_execute("COMMIT")
                    .await
                    .map_err(|e| format!("Error en COMMIT: {}", describe(&e)));
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

pub mod dialect;
pub mod export;
pub mod mysql;
pub mod postgres;
pub mod tls;

use std::sync::Arc;

pub use dialect::{Engine, ParamList};
pub use export::{ExportDestination, ExportEvent, ExportOptions};
pub use tls::TlsMode;

#[derive(Debug, Clone)]
pub struct ConnectionParams {
    pub engine: Engine,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    /// Perfil con la contraseña en el llavero del sistema: si `password` está
    /// vacía, se lee de ahí al conectar.
    pub keyring_id: Option<String>,
    /// Base de datos a la que se limita la conexión: el árbol solo enseña
    /// esa. Vacía = todas las del servidor.
    pub database: String,
    /// Si la conexión va cifrada y si se comprueba el certificado.
    pub tls: TlsMode,
    /// Fichero PEM con la autoridad que firma el certificado del servidor,
    /// para `TlsMode::Verify`. Vacío = solo las autoridades públicas.
    pub tls_ca: String,
}

impl ConnectionParams {
    /// Identidad estable de la conexión («user@host:port») para guardar
    /// estado por servidor, independiente del nombre del perfil.
    pub fn key(&self) -> String {
        format!("{}@{}:{}", self.user, self.host, self.port)
    }

    /// La contraseña escrita o, si está vacía, la del llavero. Puede bloquear
    /// (el llavero puede pedir desbloqueo): solo desde hilos de trabajo.
    /// El fichero de CA con el que verificar al servidor, si se indicó.
    pub fn ca_file(&self) -> Option<&str> {
        let path = self.tls_ca.trim();
        (!path.is_empty()).then_some(path)
    }

    /// La base de datos a la que se limita la conexión, si se indicó alguna.
    pub fn only_database(&self) -> Option<&str> {
        let database = self.database.trim();
        (!database.is_empty()).then_some(database)
    }

    pub fn resolve_password(&self) -> DbResult<String> {
        match &self.keyring_id {
            Some(id) if self.password.is_empty() => crate::secrets::get(id)
                .map_err(|e| format!("No se pudo leer la contraseña del llavero: {e}"))?
                .ok_or_else(|| {
                    "La contraseña de este perfil ya no está en el llavero del sistema: \
                     escríbela y vuelve a guardar el perfil."
                        .into()
                }),
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

/// Señal para cancelar una ejecución del editor SQL: la comparten quien la
/// lanza y quien la cancela (`DbBackend::cancel_in_session`).
pub type CancelFlag = Arc<std::sync::atomic::AtomicBool>;

/// Error con el que termina una ejecución del editor cancelada.
pub const QUERY_CANCELLED: &str = "Consulta cancelada.";

/// Sentencia con parámetros (ver `ParamList`). Los valores viajan aparte
/// del SQL: no hace falta escaparlos y llegan intactos (`None` = NULL).
#[derive(Debug, Clone, PartialEq)]
pub struct Statement {
    pub sql: String,
    pub params: Vec<Option<String>>,
    /// La sentencia cambia una fila concreta (UPDATE o DELETE por clave
    /// primaria): si no encuentra exactamente una, la transacción se deshace.
    pub single_row: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct QueryResult {
    pub columns: Vec<String>,
    /// Cada celda es `None` si el valor es NULL en la base de datos.
    pub rows: Vec<Vec<Option<String>>>,
    /// Había más filas de las que se leyeron (tope del editor SQL).
    pub truncated: bool,
}

#[derive(Debug, Clone)]
pub struct DatabaseInfo {
    pub name: String,
    pub charset: String,
    pub collation: String,
    /// `None` si el motor no puede contarlas sin entrar en la BBDD.
    pub table_count: Option<usize>,
    pub size_bytes: u64,
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
    /// Tipo de clave de la que forma parte: «PRI», «UNI», «MUL» o nada.
    pub column_key: String,
    pub extra: String,
    /// El servidor le da valor al insertar (AUTO_INCREMENT, serial,
    /// identidad): en una fila nueva no se escribe.
    pub is_auto: bool,
}

impl ColumnInfo {
    pub fn is_primary(&self) -> bool {
        self.column_key == "PRI"
    }
}

/// Valida que un nombre de charset MySQL solo contenga caracteres seguros.
/// Los charsets MySQL son siempre alfanuméricos con guiones bajos (ej. utf8mb4, latin1).
pub fn validate_charset(s: &str) -> DbResult<()> {
    if s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !s.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "Charset inválido: '{s}'. Solo se permiten caracteres alfanuméricos y '_'."
        ))
    }
}

/// Backend de una conexión. Es `Sync` y todos sus métodos toman `&self`: se
/// comparte (`Arc`) entre la UI y los hilos de trabajo, que piden conexiones
/// a su pool cuando las necesitan.
///
/// Los métodos con `db: Option<&str>` ejecutan SQL en esa BBDD. MySQL ve
/// todas desde cualquier conexión y no la necesita (las tablas van
/// cualificadas con su BBDD); en PostgreSQL decide a cuál se conecta. Sin
/// ella, en la BBDD con la que se abrió la conexión.
pub trait DbBackend: Send + Sync {
    fn engine(&self) -> Engine;

    // ── Lectura (no modifican el estado del backend) ───────────────────────────

    fn list_databases(&self) -> DbResult<Vec<String>>;
    fn list_tables(&self, database: &str) -> DbResult<Vec<String>>;
    /// Pares (BBDD, tabla) de todo el servidor, en una sola consulta.
    fn list_all_tables(&self) -> DbResult<Vec<(String, String)>>;

    /// Ejecuta SQL arbitrario de lectura y devuelve el resultado.
    fn execute_query(&self, db: Option<&str>, sql: &str) -> DbResult<QueryResult>;

    /// SQL del editor, en una conexión propia que se mantiene entre
    /// ejecuciones: transacciones abiertas, variables (`SET @x`), `USE` y
    /// tablas temporales siguen vivas en la siguiente. Con `db`, antes `USE`.
    /// Lee como mucho `max_rows` filas (el resto se descarta, `truncated`).
    /// Con `cancel` activada (antes de empezar o mientras se ejecuta, ver
    /// `cancel_in_session`) termina con el error `QUERY_CANCELLED`.
    fn execute_in_session(
        &self,
        db: Option<&str>,
        sql: &str,
        max_rows: usize,
        cancel: &CancelFlag,
    ) -> DbResult<QueryResult>;

    /// Cancela la ejecución de `execute_in_session` que recibió `cancel`: si
    /// ya está en el servidor, la interrumpe allí; si espera su turno, no
    /// llega a empezar. La conexión del editor y su estado se conservan.
    fn cancel_in_session(&self, cancel: &CancelFlag) -> DbResult<()>;

    fn get_databases_info(&self) -> DbResult<Vec<DatabaseInfo>>;
    fn get_tables_info(&self, db: &str) -> DbResult<Vec<TableInfo>>;
    /// Columnas en orden; las de la clave primaria tienen `column_key` «PRI».
    fn get_columns_info(&self, db: &str, table: &str) -> DbResult<Vec<ColumnInfo>>;

    // ── Escritura (modifican datos en el servidor) ─────────────────────────────

    /// Ejecuta SQL de escritura (INSERT/UPDATE/DELETE/DDL). Devuelve filas afectadas.
    fn execute_write(&self, db: Option<&str>, sql: &str) -> DbResult<u64>;

    fn create_database(&self, name: &str, charset: &str) -> DbResult<()>;
    fn drop_database(&self, name: &str) -> DbResult<()>;
    fn drop_table(&self, db: &str, table: &str) -> DbResult<()>;
    fn truncate_table(&self, db: &str, table: &str) -> DbResult<()>;

    /// Ejecuta una lista de sentencias dentro de una transacción.
    /// Si alguna falla, o una `single_row` no encuentra su fila, se hace
    /// ROLLBACK y se devuelve el error.
    fn execute_transaction(&self, db: Option<&str>, stmts: &[Statement]) -> DbResult<()>;
}

pub enum ConnectEvent {
    Status(String),
    Done(DbResult<Arc<dyn DbBackend>>),
}

/// Conecta con el servidor de `params`, con el backend de su motor.
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

/// Como `connect`, contando el progreso y el resultado por `tx`.
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

/// Las BBDD del servidor de `params`, para elegir una en el formulario de
/// conexión: se conecta, las lista todas y desconecta.
pub fn list_server_databases(params: &ConnectionParams) -> DbResult<Vec<String>> {
    match params.engine {
        // Sin limitarse a la BBDD que haya en el formulario
        Engine::Mysql => {
            let all = ConnectionParams {
                database: String::new(),
                ..params.clone()
            };
            mysql::MysqlBackend::connect_with_progress(&all, |_| {})?.list_databases()
        }
        // Entrando por la del formulario, si hay: puede ser la única en la
        // que el servidor deja entrar, y desde ella se ven las demás
        Engine::Postgres => {
            postgres::PostgresBackend::connect_with_progress(params, |_| {})?.all_databases()
        }
    }
}

/// Servidores reales para los tests marcados `#[ignore]`:
/// `RUSTQL_TEST_MYSQL=usuario:contraseña@host:puerto` y
/// `RUSTQL_TEST_POSTGRES=…`, con `cargo test -- --ignored`. Los de TLS
/// necesitan además `RUSTQL_TEST_CA` (ver `with_tls`).
/// Crean y borran sus propias BBDD `rustql_test_*`.
#[cfg(test)]
pub mod test_server {
    use super::{ConnectionParams, Engine};

    fn from_env(var: &str, engine: Engine) -> ConnectionParams {
        let url =
            std::env::var(var).unwrap_or_else(|_| panic!("{var}=usuario:contraseña@host:puerto"));
        let (auth, addr) = url.rsplit_once('@').expect("falta @");
        let (user, password) = auth.split_once(':').unwrap_or((auth, ""));
        let (host, port) = addr.rsplit_once(':').expect("falta :puerto");
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

    /// Parámetros de `base` con el nivel de seguridad `tls`. Para verificar,
    /// `RUSTQL_TEST_CA` es el fichero de la autoridad que firma el
    /// certificado de los servidores de prueba (válido para 127.0.0.1).
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

    /// Con NO_BACKSLASH_ESCAPES, escapando la comilla como `\'`, este valor
    /// convertía `v = '…'` en `v = '\\\' OR 1=1 -- '`.
    const ATTACK: &str = "\\' OR 1=1 -- ";

    #[test]
    #[ignore = "necesita RUSTQL_TEST_MYSQL"]
    fn un_literal_no_permite_inyectar_en_ningun_sql_mode() {
        let backend = test_server::backend();
        // Una sola conexión: sql_mode es de sesión
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

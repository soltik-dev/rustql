pub mod export;
pub mod mysql;

use std::sync::Arc;

pub use export::{ExportDestination, ExportEvent, ExportOptions};

#[derive(Debug, Clone)]
pub struct ConnectionParams {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    /// Perfil con la contraseña en el llavero del sistema: si `password` está
    /// vacía, se lee de ahí al conectar.
    pub keyring_id: Option<String>,
}

impl ConnectionParams {
    /// Identidad estable de la conexión («user@host:port») para guardar
    /// estado por servidor, independiente del nombre del perfil.
    pub fn key(&self) -> String {
        format!("{}@{}:{}", self.user, self.host, self.port)
    }

    /// La contraseña escrita o, si está vacía, la del llavero. Puede bloquear
    /// (el llavero puede pedir desbloqueo): solo desde hilos de trabajo.
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
        Self {
            host: "127.0.0.1".into(),
            port: 3306,
            user: "root".into(),
            password: String::new(),
            keyring_id: None,
        }
    }
}

pub type DbResult<T> = Result<T, String>;

/// Sentencia con parámetros posicionales (`?`). Los valores viajan aparte
/// del SQL: no hace falta escaparlos y llegan intactos (`None` = NULL).
#[derive(Debug, Clone, PartialEq)]
pub struct Statement {
    pub sql: String,
    pub params: Vec<Option<String>>,
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
    pub table_count: usize,
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
    pub column_key: String,
    pub extra: String,
}

/// Escapa un valor para uso dentro de comillas simples en SQL.
/// Cubre todas las secuencias de escape requeridas por MySQL. La comilla se
/// dobla (`''`) en vez de escaparse con `\'`: si el servidor tiene
/// NO_BACKSLASH_ESCAPES en el sql_mode, `\'` cerraría la cadena y el resto
/// del valor se ejecutaría como SQL. Con `''` lo peor que pasa en ese modo es
/// que las barras se lean literalmente.
pub fn escape_sql(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\0', "\\0")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\x1a', "\\Z")
        .replace('\'', "''")
}

/// Escapa un identificador para uso dentro de backticks en SQL.
pub fn escape_ident(s: &str) -> String {
    s.replace('`', "``")
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
pub trait DbBackend: Send + Sync {
    // ── Lectura (no modifican el estado del backend) ───────────────────────────

    fn list_databases(&self) -> DbResult<Vec<String>>;
    fn list_tables(&self, database: &str) -> DbResult<Vec<String>>;
    /// Pares (BBDD, tabla) de todo el servidor, en una sola consulta.
    fn list_all_tables(&self) -> DbResult<Vec<(String, String)>>;

    /// Ejecuta SQL arbitrario de lectura y devuelve el resultado.
    fn execute_query(&self, sql: &str) -> DbResult<QueryResult>;

    /// SQL del editor, en una conexión propia que se mantiene entre
    /// ejecuciones: transacciones abiertas, variables (`SET @x`), `USE` y
    /// tablas temporales siguen vivas en la siguiente. Con `db`, antes `USE`.
    /// Lee como mucho `max_rows` filas (el resto se descarta, `truncated`).
    fn execute_in_session(
        &self,
        db: Option<&str>,
        sql: &str,
        max_rows: usize,
    ) -> DbResult<QueryResult>;

    fn get_databases_info(&self) -> DbResult<Vec<DatabaseInfo>>;
    fn get_tables_info(&self, db: &str) -> DbResult<Vec<TableInfo>>;
    /// Columnas en orden; las de la clave primaria tienen `column_key` «PRI».
    fn get_columns_info(&self, db: &str, table: &str) -> DbResult<Vec<ColumnInfo>>;

    // ── Escritura (modifican datos en el servidor) ─────────────────────────────

    /// Ejecuta SQL de escritura (INSERT/UPDATE/DELETE/DDL). Devuelve filas afectadas.
    fn execute_write(&self, sql: &str) -> DbResult<u64>;

    fn create_database(&self, name: &str, charset: &str) -> DbResult<()>;
    fn drop_database(&self, name: &str) -> DbResult<()>;
    fn drop_table(&self, db: &str, table: &str) -> DbResult<()>;
    fn truncate_table(&self, db: &str, table: &str) -> DbResult<()>;

    /// Ejecuta una lista de sentencias dentro de una transacción.
    /// Si alguna falla se hace ROLLBACK y se devuelve el error.
    fn execute_transaction(&self, stmts: &[Statement]) -> DbResult<()>;

    // ── Helpers con implementación por defecto ─────────────────────────────────

    /// SELECT * con LIMIT opcional WHERE.
    #[allow(dead_code)]
    fn query_table(&self, db: &str, table: &str, limit: usize) -> DbResult<QueryResult> {
        self.execute_query(&format!(
            "SELECT * FROM `{}`.`{}` LIMIT {limit}",
            escape_ident(db),
            escape_ident(table)
        ))
    }

    #[allow(dead_code)]
    fn query_filtered(
        &self,
        db: &str,
        table: &str,
        filter: &str,
        limit: usize,
    ) -> DbResult<QueryResult> {
        self.execute_query(&format!(
            "SELECT * FROM `{}`.`{}` WHERE {filter} LIMIT {limit}",
            escape_ident(db),
            escape_ident(table)
        ))
    }
}

pub enum ConnectEvent {
    Status(String),
    Done(DbResult<Arc<dyn DbBackend>>),
}

pub fn connect_mysql_progress(
    params: &ConnectionParams,
    tx: &std::sync::mpsc::Sender<ConnectEvent>,
) {
    let send_status = |msg: &str| {
        let _ = tx.send(ConnectEvent::Status(msg.to_string()));
    };
    let result = mysql::MysqlBackend::connect_with_progress(params, send_status)
        .map(|b| -> Arc<dyn DbBackend> { Arc::new(b) });
    let _ = tx.send(ConnectEvent::Done(result));
}

/// Servidor real para los tests marcados `#[ignore]`:
/// `RUSTQL_TEST_MYSQL=usuario:contraseña@host:puerto cargo test -- --ignored`.
/// Crean y borran sus propias BBDD `rustql_test_*`.
#[cfg(test)]
pub mod test_server {
    use super::ConnectionParams;

    pub fn params() -> ConnectionParams {
        let url = std::env::var("RUSTQL_TEST_MYSQL")
            .expect("RUSTQL_TEST_MYSQL=usuario:contraseña@host:puerto");
        let (auth, addr) = url.rsplit_once('@').expect("falta @");
        let (user, password) = auth.split_once(':').unwrap_or((auth, ""));
        let (host, port) = addr.rsplit_once(':').expect("falta :puerto");
        ConnectionParams {
            host: host.into(),
            port: port.parse().expect("puerto"),
            user: user.into(),
            password: password.into(),
            keyring_id: None,
        }
    }

    pub fn backend() -> super::mysql::MysqlBackend {
        super::mysql::MysqlBackend::connect_with_progress(&params(), |_| {}).unwrap()
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
    fn escape_sql_dobla_las_comillas() {
        assert_eq!(escape_sql("it's"), "it''s");
        assert_eq!(escape_sql(ATTACK), "\\\\'' OR 1=1 -- ");
        assert_eq!(escape_sql("a\nb\\c"), "a\\nb\\\\c");
    }

    #[test]
    #[ignore = "necesita RUSTQL_TEST_MYSQL"]
    fn escape_sql_no_permite_inyectar_en_ningun_sql_mode() {
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
                    "SELECT COUNT(*) FROM rustql_test_escape.t WHERE v = '{}'",
                    escape_sql(ATTACK)
                ))
                .unwrap();
            assert_eq!(matches, Some(0), "inyección con sql_mode={mode:?}");
            if mode.is_empty() {
                for value in tricky {
                    let back: Option<String> = conn
                        .query_first(format!("SELECT '{}'", escape_sql(value)))
                        .unwrap();
                    assert_eq!(back.as_deref(), Some(value));
                }
            }
        }
        conn.query_drop("DROP DATABASE rustql_test_escape").unwrap();
    }
}

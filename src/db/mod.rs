pub mod mysql;

#[derive(Debug, Clone)]
pub struct ConnectionParams {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
}

impl Default for ConnectionParams {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            port: 3306,
            user: "root".into(),
            password: String::new(),
        }
    }
}

pub type DbResult<T> = Result<T, String>;

#[derive(Debug, Clone)]
pub struct QueryResult {
    pub columns: Vec<String>,
    /// Cada celda es `None` si el valor es NULL en la base de datos.
    pub rows: Vec<Vec<Option<String>>>,
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
/// Cubre todas las secuencias de escape requeridas por MySQL.
pub fn escape_sql(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\0', "\\0")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\x1a', "\\Z")
        .replace('\'', "\\'")
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
        Err(format!("Charset inválido: '{s}'. Solo se permiten caracteres alfanuméricos y '_'."))
    }
}

pub trait DbBackend: Send {
    // ── Lectura (no modifican el estado del backend) ───────────────────────────

    fn list_databases(&self) -> DbResult<Vec<String>>;
    fn list_tables(&self, database: &str) -> DbResult<Vec<String>>;

    /// Ejecuta SQL arbitrario de lectura y devuelve el resultado.
    fn execute_query(&self, sql: &str) -> DbResult<QueryResult>;

    fn get_databases_info(&self) -> DbResult<Vec<DatabaseInfo>>;
    fn get_tables_info(&self, db: &str) -> DbResult<Vec<TableInfo>>;
    fn get_columns_info(&self, db: &str, table: &str) -> DbResult<Vec<ColumnInfo>>;
    fn get_primary_keys(&self, db: &str, table: &str) -> DbResult<Vec<String>>;

    // ── Escritura (modifican datos en el servidor) ─────────────────────────────

    /// Ejecuta SQL de escritura (INSERT/UPDATE/DELETE/DDL). Devuelve filas afectadas.
    fn execute_write(&mut self, sql: &str) -> DbResult<u64>;

    fn create_database(&mut self, name: &str, charset: &str) -> DbResult<()>;
    fn drop_database(&mut self, name: &str) -> DbResult<()>;
    fn drop_table(&mut self, db: &str, table: &str) -> DbResult<()>;
    fn truncate_table(&mut self, db: &str, table: &str) -> DbResult<()>;

    /// Ejecuta una lista de sentencias dentro de una transacción.
    /// Si alguna falla se hace ROLLBACK y se devuelve el error.
    fn execute_transaction(&mut self, stmts: &[String]) -> DbResult<()>;

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
    Done(DbResult<Box<dyn DbBackend>>),
}

pub enum QueryEvent {
    Done {
        result: DbResult<QueryResult>,
        backend: Box<dyn DbBackend>,
        sql: String,
        elapsed_ms: u64,
    },
}

pub fn connect_mysql_progress(
    params: &ConnectionParams,
    tx: &std::sync::mpsc::Sender<ConnectEvent>,
) {
    let send_status = |msg: &str| {
        let _ = tx.send(ConnectEvent::Status(msg.to_string()));
    };
    let result = mysql::MysqlBackend::connect_with_progress(params, send_status)
        .map(|b| -> Box<dyn DbBackend> { Box::new(b) });
    let _ = tx.send(ConnectEvent::Done(result));
}

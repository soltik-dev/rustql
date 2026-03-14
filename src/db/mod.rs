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
    pub rows: Vec<Vec<String>>,
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
pub fn escape_sql(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
}

/// Escapa un identificador para uso dentro de backticks en SQL.
#[allow(dead_code)]
pub fn escape_ident(s: &str) -> String {
    s.replace('`', "``")
}

#[allow(dead_code)]
pub trait DbBackend: Send {
    fn list_databases(&mut self) -> DbResult<Vec<String>>;
    fn list_tables(&mut self, database: &str) -> DbResult<Vec<String>>;

    /// Ejecuta SQL arbitrario de lectura y devuelve el resultado.
    fn execute_query(&mut self, sql: &str) -> DbResult<QueryResult>;

    /// Ejecuta SQL de escritura (INSERT/UPDATE/DELETE/DDL). Devuelve filas afectadas.
    fn execute_write(&mut self, sql: &str) -> DbResult<u64>;

    fn get_databases_info(&mut self) -> DbResult<Vec<DatabaseInfo>>;
    fn get_tables_info(&mut self, db: &str) -> DbResult<Vec<TableInfo>>;
    fn get_columns_info(&mut self, db: &str, table: &str) -> DbResult<Vec<ColumnInfo>>;
    fn get_primary_keys(&mut self, db: &str, table: &str) -> DbResult<Vec<String>>;

    fn create_database(&mut self, name: &str, charset: &str) -> DbResult<()>;
    fn drop_database(&mut self, name: &str) -> DbResult<()>;
    fn drop_table(&mut self, db: &str, table: &str) -> DbResult<()>;
    fn truncate_table(&mut self, db: &str, table: &str) -> DbResult<()>;

    /// SELECT * con LIMIT opcional WHERE.
    fn query_table(&mut self, db: &str, table: &str, limit: usize) -> DbResult<QueryResult> {
        self.execute_query(&format!("SELECT * FROM `{db}`.`{table}` LIMIT {limit}"))
    }

    fn query_filtered(
        &mut self,
        db: &str,
        table: &str,
        filter: &str,
        limit: usize,
    ) -> DbResult<QueryResult> {
        self.execute_query(&format!(
            "SELECT * FROM `{db}`.`{table}` WHERE {filter} LIMIT {limit}"
        ))
    }
}

pub enum ConnectEvent {
    Status(String),
    Done(DbResult<Box<dyn DbBackend>>),
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

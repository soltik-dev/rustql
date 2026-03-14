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

pub trait DbBackend: Send {
    fn list_databases(&mut self) -> DbResult<Vec<String>>;
    fn list_tables(&mut self, database: &str) -> DbResult<Vec<String>>;
    fn query_table(&mut self, database: &str, table: &str, limit: usize) -> DbResult<QueryResult>;
}

pub fn connect_mysql(params: &ConnectionParams) -> DbResult<Box<dyn DbBackend>> {
    Ok(Box::new(mysql::MysqlBackend::connect(params)?))
}

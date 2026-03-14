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

    /// Método núcleo: ejecuta SQL arbitrario y devuelve el resultado.
    fn execute_query(&mut self, sql: &str) -> DbResult<QueryResult>;

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

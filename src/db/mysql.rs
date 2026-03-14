use mysql::{Opts, OptsBuilder, Pool, PooledConn, Value, prelude::Queryable};

use super::{ConnectionParams, DbBackend, DbResult, QueryResult};

fn value_to_string(v: Value) -> String {
    match v {
        Value::NULL => String::new(),
        Value::Bytes(b) => String::from_utf8_lossy(&b).into_owned(),
        Value::Int(i) => i.to_string(),
        Value::UInt(u) => u.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Double(d) => d.to_string(),
        Value::Date(y, m, d, h, min, s, _) => {
            format!("{y}-{m:02}-{d:02} {h:02}:{min:02}:{s:02}")
        }
        Value::Time(neg, days, h, m, s, _) => {
            let sign = if neg { "-" } else { "" };
            format!("{sign}{days}d {h:02}:{m:02}:{s:02}")
        }
    }
}

fn rows_to_result(raw_rows: Vec<mysql::Row>) -> QueryResult {
    let columns = raw_rows
        .first()
        .map(|r| {
            r.columns_ref()
                .iter()
                .map(|c| c.name_str().into_owned())
                .collect()
        })
        .unwrap_or_default();

    let rows = raw_rows
        .into_iter()
        .map(|mut row| {
            let n = row.len();
            (0..n)
                .map(|i| {
                    let v: Value = row.take(i).unwrap_or(Value::NULL);
                    value_to_string(v)
                })
                .collect()
        })
        .collect();

    QueryResult { columns, rows }
}

/// Almacena el Pool en lugar de una PooledConn individual.
/// El pool detecta conexiones muertas y reconecta de forma transparente
/// al solicitar una nueva conexión mediante get_conn().
pub struct MysqlBackend {
    pool: Pool,
}

impl MysqlBackend {
    pub fn connect_with_progress(
        params: &ConnectionParams,
        on_status: impl Fn(&str),
    ) -> DbResult<Self> {
        on_status("Estableciendo conexión TCP...");
        let opts: Opts = OptsBuilder::new()
            .ip_or_hostname(Some(params.host.as_str()))
            .tcp_port(params.port)
            .user(Some(params.user.as_str()))
            .pass(Some(params.password.as_str()))
            .into();

        let pool = Pool::new(opts).map_err(|e| format!("Error de conexión: {e}"))?;

        on_status("Autenticando...");
        pool.get_conn()
            .map_err(|e| format!("No se pudo conectar: {e}"))?;

        on_status("Conexión establecida.");
        Ok(Self { pool })
    }

    /// Obtiene una conexión del pool. Si la conexión previa había muerto,
    /// el pool crea una nueva automáticamente.
    fn conn(&self) -> DbResult<PooledConn> {
        self.pool
            .get_conn()
            .map_err(|e| format!("Sin conexión con el servidor: {e}"))
    }
}

impl DbBackend for MysqlBackend {
    fn list_databases(&mut self) -> DbResult<Vec<String>> {
        self.conn()?
            .query("SHOW DATABASES")
            .map_err(|e| format!("Error en query: {e}"))
    }

    fn list_tables(&mut self, database: &str) -> DbResult<Vec<String>> {
        let query = format!("SHOW TABLES FROM `{database}`");
        self.conn()?
            .query(query)
            .map_err(|e| format!("Error en query: {e}"))
    }

    fn execute_query(&mut self, sql: &str) -> DbResult<QueryResult> {
        let raw_rows: Vec<mysql::Row> = self
            .conn()?
            .query(sql)
            .map_err(|e| format!("Error en query: {e}"))?;

        Ok(rows_to_result(raw_rows))
    }
}

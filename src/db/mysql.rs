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

pub struct MysqlBackend {
    conn: PooledConn,
}

impl MysqlBackend {
    pub fn connect(params: &ConnectionParams) -> DbResult<Self> {
        let opts: Opts = OptsBuilder::new()
            .ip_or_hostname(Some(params.host.as_str()))
            .tcp_port(params.port)
            .user(Some(params.user.as_str()))
            .pass(Some(params.password.as_str()))
            .into();

        let pool = Pool::new(opts).map_err(|e| format!("Error de conexión: {e}"))?;
        let conn = pool.get_conn().map_err(|e| format!("Error al obtener conexión: {e}"))?;

        Ok(Self { conn })
    }
}

impl DbBackend for MysqlBackend {
    fn list_databases(&mut self) -> DbResult<Vec<String>> {
        self.conn
            .query("SHOW DATABASES")
            .map_err(|e| format!("Error en query: {e}"))
    }

    fn list_tables(&mut self, database: &str) -> DbResult<Vec<String>> {
        let query = format!("SHOW TABLES FROM `{database}`");
        self.conn
            .query(query)
            .map_err(|e| format!("Error en query: {e}"))
    }

    fn query_table(&mut self, database: &str, table: &str, limit: usize) -> DbResult<QueryResult> {
        let query = format!("SELECT * FROM `{database}`.`{table}` LIMIT {limit}");
        let raw_rows: Vec<mysql::Row> = self.conn
            .query(query)
            .map_err(|e| format!("Error en query: {e}"))?;

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

        Ok(QueryResult { columns, rows })
    }
}

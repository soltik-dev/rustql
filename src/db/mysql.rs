use mysql::{Opts, OptsBuilder, Pool, PoolConstraints, PoolOpts, PooledConn, Value, prelude::Queryable};

use super::{
    ColumnInfo, ConnectionParams, DatabaseInfo, DbBackend, DbResult, QueryResult, TableInfo,
    escape_sql,
};

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

fn col(row: &[String], i: usize) -> String {
    row.get(i).cloned().unwrap_or_default()
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
        // min:0 evita crear 10 conexiones en paralelo al arrancar (default min:10)
        // para un cliente GUI con un solo usuario basta con min:0, max:1
        let pool_constraints = PoolConstraints::new(0, 1).unwrap();
        let pool_opts = PoolOpts::default().with_constraints(pool_constraints);
        let opts: Opts = OptsBuilder::new()
            .ip_or_hostname(Some(params.host.as_str()))
            .tcp_port(params.port)
            .user(Some(params.user.as_str()))
            .pass(Some(params.password.as_str()))
            .pool_opts(pool_opts)
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
        let query = format!("SHOW TABLES FROM `{}`", escape_sql(database));
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

    fn execute_write(&mut self, sql: &str) -> DbResult<u64> {
        let mut conn = self.conn()?;
        conn.query_drop(sql)
            .map_err(|e| format!("Error en escritura: {e}"))?;
        Ok(conn.affected_rows())
    }

    fn get_databases_info(&mut self) -> DbResult<Vec<DatabaseInfo>> {
        let sql = "SELECT s.SCHEMA_NAME, \
                          s.DEFAULT_CHARACTER_SET_NAME, \
                          s.DEFAULT_COLLATION_NAME, \
                          COALESCE(COUNT(t.TABLE_NAME), 0), \
                          COALESCE(SUM(t.DATA_LENGTH + COALESCE(t.INDEX_LENGTH,0)), 0) \
                   FROM information_schema.SCHEMATA s \
                   LEFT JOIN information_schema.TABLES t ON t.TABLE_SCHEMA = s.SCHEMA_NAME \
                   GROUP BY s.SCHEMA_NAME, s.DEFAULT_CHARACTER_SET_NAME, s.DEFAULT_COLLATION_NAME \
                   ORDER BY s.SCHEMA_NAME";
        let r = self.execute_query(sql)?;
        Ok(r.rows
            .iter()
            .map(|row| DatabaseInfo {
                name: col(row, 0),
                charset: col(row, 1),
                collation: col(row, 2),
                table_count: col(row, 3).parse().unwrap_or(0),
                size_bytes: col(row, 4).parse().unwrap_or(0),
            })
            .collect())
    }

    fn get_tables_info(&mut self, db: &str) -> DbResult<Vec<TableInfo>> {
        let sql = format!(
            "SELECT TABLE_NAME, \
                    COALESCE(ENGINE,''), \
                    COALESCE(TABLE_ROWS,0), \
                    COALESCE(DATA_LENGTH,0)+COALESCE(INDEX_LENGTH,0), \
                    COALESCE(TABLE_COLLATION,'') \
             FROM information_schema.TABLES \
             WHERE TABLE_SCHEMA='{}' AND TABLE_TYPE='BASE TABLE' \
             ORDER BY TABLE_NAME",
            escape_sql(db)
        );
        let r = self.execute_query(&sql)?;
        Ok(r.rows
            .iter()
            .map(|row| TableInfo {
                name: col(row, 0),
                engine: col(row, 1),
                row_count: col(row, 2).parse().unwrap_or(0),
                size_bytes: col(row, 3).parse().unwrap_or(0),
                collation: col(row, 4),
            })
            .collect())
    }

    fn get_columns_info(&mut self, db: &str, table: &str) -> DbResult<Vec<ColumnInfo>> {
        let sql = format!(
            "SELECT COLUMN_NAME, COLUMN_TYPE, IS_NULLABLE, COLUMN_DEFAULT, COLUMN_KEY, EXTRA \
             FROM information_schema.COLUMNS \
             WHERE TABLE_SCHEMA='{}' AND TABLE_NAME='{}' \
             ORDER BY ORDINAL_POSITION",
            escape_sql(db),
            escape_sql(table)
        );
        let r = self.execute_query(&sql)?;
        Ok(r.rows
            .iter()
            .map(|row| {
                let default_raw = col(row, 3);
                ColumnInfo {
                    name: col(row, 0),
                    col_type: col(row, 1),
                    is_nullable: col(row, 2).eq_ignore_ascii_case("YES"),
                    default_val: if default_raw.is_empty() {
                        None
                    } else {
                        Some(default_raw)
                    },
                    column_key: col(row, 4),
                    extra: col(row, 5),
                }
            })
            .collect())
    }

    fn get_primary_keys(&mut self, db: &str, table: &str) -> DbResult<Vec<String>> {
        let sql = format!(
            "SELECT COLUMN_NAME \
             FROM information_schema.COLUMNS \
             WHERE TABLE_SCHEMA='{}' AND TABLE_NAME='{}' AND COLUMN_KEY='PRI' \
             ORDER BY ORDINAL_POSITION",
            escape_sql(db),
            escape_sql(table)
        );
        let r = self.execute_query(&sql)?;
        Ok(r.rows.iter().map(|row| col(row, 0)).collect())
    }

    fn create_database(&mut self, name: &str, charset: &str) -> DbResult<()> {
        let sql = format!(
            "CREATE DATABASE `{}` CHARACTER SET {}",
            name.replace('`', "``"),
            charset
        );
        self.execute_write(&sql).map(|_| ())
    }

    fn drop_database(&mut self, name: &str) -> DbResult<()> {
        let sql = format!("DROP DATABASE `{}`", name.replace('`', "``"));
        self.execute_write(&sql).map(|_| ())
    }

    fn drop_table(&mut self, db: &str, table: &str) -> DbResult<()> {
        let sql = format!(
            "DROP TABLE `{}`.`{}`",
            db.replace('`', "``"),
            table.replace('`', "``")
        );
        self.execute_write(&sql).map(|_| ())
    }

    fn truncate_table(&mut self, db: &str, table: &str) -> DbResult<()> {
        let sql = format!(
            "TRUNCATE TABLE `{}`.`{}`",
            db.replace('`', "``"),
            table.replace('`', "``")
        );
        self.execute_write(&sql).map(|_| ())
    }
}

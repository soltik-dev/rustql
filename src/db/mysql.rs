use std::sync::{Arc, Mutex, PoisonError};

use mysql::{Conn, Opts, OptsBuilder, Pool, PoolConstraints, PoolOpts, PooledConn, Value, prelude::Queryable};

use super::{
    ColumnInfo, ConnectionParams, DatabaseInfo, DbBackend, DbResult, QueryResult, Statement,
    TableInfo, escape_ident, validate_charset,
};

fn value_to_string(v: Value) -> Option<String> {
    match v {
        Value::NULL => None,
        Value::Bytes(b) => Some(String::from_utf8_lossy(&b).into_owned()),
        Value::Int(i) => Some(i.to_string()),
        Value::UInt(u) => Some(u.to_string()),
        Value::Float(f) => Some(f.to_string()),
        Value::Double(d) => Some(d.to_string()),
        Value::Date(y, m, d, h, min, s, _) => {
            Some(format!("{y}-{m:02}-{d:02} {h:02}:{min:02}:{s:02}"))
        }
        Value::Time(neg, days, h, m, s, _) => {
            let sign = if neg { "-" } else { "" };
            Some(format!("{sign}{days}d {h:02}:{m:02}:{s:02}"))
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

fn col(row: &[Option<String>], i: usize) -> String {
    row.get(i).and_then(|v| v.as_deref()).unwrap_or("").to_string()
}

/// Almacena el Pool en lugar de una PooledConn individual.
/// El pool detecta conexiones muertas y reconecta de forma transparente
/// al solicitar una nueva conexión mediante get_conn().
pub struct MysqlBackend {
    pool: Pool,
    opts: Opts,
    /// Conexión propia del editor SQL, fuera del pool: el pool resetea cada
    /// conexión al devolverla (COM_RESET_CONNECTION), lo que deshace
    /// transacciones, variables y tablas temporales entre ejecuciones.
    /// Se abre al primer uso.
    session: Arc<Mutex<Option<Conn>>>,
}

impl MysqlBackend {
    pub fn connect_with_progress(
        params: &ConnectionParams,
        on_status: impl Fn(&str),
    ) -> DbResult<Self> {
        if params.keyring_id.is_some() && params.password.is_empty() {
            on_status("Leyendo la contraseña del llavero...");
        }
        let password = params.resolve_password()?;
        on_status("Estableciendo conexión TCP...");
        // min:0 evita crear 10 conexiones en paralelo al arrancar (default min:10).
        // max:4 deja que los trabajos en segundo plano (datos, metadatos,
        // escrituras) no esperen unos a otros; el editor SQL va aparte.
        let pool_constraints = PoolConstraints::new(0, 4).unwrap();
        let pool_opts = PoolOpts::default().with_constraints(pool_constraints);
        let opts: Opts = OptsBuilder::new()
            .ip_or_hostname(Some(params.host.as_str()))
            .tcp_port(params.port)
            .user(Some(params.user.as_str()))
            .pass(Some(password.as_str()))
            .pool_opts(pool_opts)
            .into();

        let pool = Pool::new(opts.clone()).map_err(|e| format!("Error de conexión: {e}"))?;

        on_status("Autenticando...");
        pool.get_conn()
            .map_err(|e| format!("No se pudo conectar: {e}"))?;

        on_status("Conexión establecida.");
        Ok(Self {
            pool,
            opts,
            session: Arc::default(),
        })
    }

    /// Obtiene una conexión del pool. Si la conexión previa había muerto,
    /// el pool crea una nueva automáticamente.
    /// `pub(super)` para que export.rs pueda mantener una sesión estable
    /// (variables como FOREIGN_KEY_CHECKS no sobreviven al pool).
    pub(super) fn conn(&self) -> DbResult<PooledConn> {
        self.pool
            .get_conn()
            .map_err(|e| format!("Sin conexión con el servidor: {e}"))
    }

    /// Lectura con parámetros posicionales (`?`).
    fn query_with(&self, sql: &str, params: Vec<Option<String>>) -> DbResult<QueryResult> {
        let raw_rows: Vec<mysql::Row> = self
            .conn()?
            .exec(sql, params)
            .map_err(|e| format!("Error en query: {e}"))?;
        Ok(rows_to_result(raw_rows))
    }
}

impl DbBackend for MysqlBackend {
    fn list_databases(&self) -> DbResult<Vec<String>> {
        self.conn()?
            .query("SHOW DATABASES")
            .map_err(|e| format!("Error en query: {e}"))
    }

    fn list_tables(&self, database: &str) -> DbResult<Vec<String>> {
        let query = format!("SHOW TABLES FROM `{}`", escape_ident(database));
        self.conn()?
            .query(query)
            .map_err(|e| format!("Error en query: {e}"))
    }

    fn execute_query(&self, sql: &str) -> DbResult<QueryResult> {
        let raw_rows: Vec<mysql::Row> = self
            .conn()?
            .query(sql)
            .map_err(|e| format!("Error en query: {e}"))?;

        Ok(rows_to_result(raw_rows))
    }

    fn execute_in_session(&self, db: Option<&str>, sql: &str) -> DbResult<QueryResult> {
        let mut session = self.session.lock().unwrap_or_else(PoisonError::into_inner);
        let conn = match &mut *session {
            Some(conn) => conn,
            None => session.insert(
                Conn::new(self.opts.clone())
                    .map_err(|e| format!("No se pudo abrir la sesión del editor: {e}"))?,
            ),
        };
        let result = (|| {
            if let Some(db) = db {
                conn.query_drop(format!("USE `{}`", escape_ident(db)))?;
            }
            conn.query::<mysql::Row, _>(sql)
        })();
        match result {
            Ok(raw_rows) => Ok(rows_to_result(raw_rows)),
            Err(e) => {
                // Conexión perdida (servidor reiniciado, timeout…): la próxima
                // ejecución abre otra; el estado de la sesión ya se ha perdido.
                if conn.ping().is_err() {
                    *session = None;
                }
                Err(format!("Error en query: {e}"))
            }
        }
    }

    fn execute_write(&self, sql: &str) -> DbResult<u64> {
        let mut conn = self.conn()?;
        conn.query_drop(sql)
            .map_err(|e| format!("Error en escritura: {e}"))?;
        Ok(conn.affected_rows())
    }

    fn get_databases_info(&self) -> DbResult<Vec<DatabaseInfo>> {
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

    fn get_tables_info(&self, db: &str) -> DbResult<Vec<TableInfo>> {
        let sql = "SELECT TABLE_NAME, \
                          COALESCE(ENGINE,''), \
                          COALESCE(TABLE_ROWS,0), \
                          COALESCE(DATA_LENGTH,0)+COALESCE(INDEX_LENGTH,0), \
                          COALESCE(TABLE_COLLATION,'') \
                   FROM information_schema.TABLES \
                   WHERE TABLE_SCHEMA = ? AND TABLE_TYPE='BASE TABLE' \
                   ORDER BY TABLE_NAME";
        let r = self.query_with(sql, vec![Some(db.into())])?;
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

    fn get_columns_info(&self, db: &str, table: &str) -> DbResult<Vec<ColumnInfo>> {
        let sql = "SELECT COLUMN_NAME, COLUMN_TYPE, IS_NULLABLE, COLUMN_DEFAULT, COLUMN_KEY, EXTRA \
                   FROM information_schema.COLUMNS \
                   WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? \
                   ORDER BY ORDINAL_POSITION";
        let r = self.query_with(sql, vec![Some(db.into()), Some(table.into())])?;
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

    fn get_primary_keys(&self, db: &str, table: &str) -> DbResult<Vec<String>> {
        let sql = "SELECT COLUMN_NAME \
                   FROM information_schema.COLUMNS \
                   WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? AND COLUMN_KEY='PRI' \
                   ORDER BY ORDINAL_POSITION";
        let r = self.query_with(sql, vec![Some(db.into()), Some(table.into())])?;
        Ok(r.rows.iter().map(|row| col(row, 0)).collect())
    }

    fn create_database(&self, name: &str, charset: &str) -> DbResult<()> {
        validate_charset(charset)?;
        let sql = format!(
            "CREATE DATABASE `{}` CHARACTER SET {}",
            escape_ident(name),
            charset
        );
        self.execute_write(&sql).map(|_| ())
    }

    fn drop_database(&self, name: &str) -> DbResult<()> {
        let sql = format!("DROP DATABASE `{}`", escape_ident(name));
        self.execute_write(&sql).map(|_| ())
    }

    fn drop_table(&self, db: &str, table: &str) -> DbResult<()> {
        let sql = format!(
            "DROP TABLE `{}`.`{}`",
            escape_ident(db),
            escape_ident(table)
        );
        self.execute_write(&sql).map(|_| ())
    }

    fn truncate_table(&self, db: &str, table: &str) -> DbResult<()> {
        let sql = format!(
            "TRUNCATE TABLE `{}`.`{}`",
            escape_ident(db),
            escape_ident(table)
        );
        self.execute_write(&sql).map(|_| ())
    }

    fn execute_transaction(&self, stmts: &[Statement]) -> DbResult<()> {
        let mut conn = self.conn()?;
        conn.query_drop("START TRANSACTION")
            .map_err(|e| format!("Error iniciando transacción: {e}"))?;
        for stmt in stmts {
            if let Err(e) = conn.exec_drop(stmt.sql.as_str(), stmt.params.clone()) {
                let _ = conn.query_drop("ROLLBACK");
                return Err(format!("Error en transacción (ROLLBACK): {e}"));
            }
        }
        conn.query_drop("COMMIT")
            .map_err(|e| format!("Error en COMMIT: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_server;

    #[test]
    #[ignore = "necesita RUSTQL_TEST_MYSQL"]
    fn metadatos_con_parametros() {
        let b = test_server::backend();
        // Nombres con comilla y barra: con el escape a mano eran frágiles
        let db = "rustql_test_meta'\\x";
        let qdb = format!("`{}`", escape_ident(db));
        b.execute_write(&format!("DROP DATABASE IF EXISTS {qdb}")).unwrap();
        b.execute_write(&format!("CREATE DATABASE {qdb}")).unwrap();
        b.execute_write(&format!(
            "CREATE TABLE {qdb}.t (id INT AUTO_INCREMENT PRIMARY KEY, \
             v VARCHAR(10) NOT NULL DEFAULT 'x', n INT NULL)"
        ))
        .unwrap();
        b.execute_write(&format!("INSERT INTO {qdb}.t (v) VALUES ('a'), ('b')"))
            .unwrap();
        b.execute_write(&format!("ANALYZE TABLE {qdb}.t")).unwrap();

        let tables = b.get_tables_info(db);
        let columns = b.get_columns_info(db, "t");
        let pks = b.get_primary_keys(db, "t");
        b.execute_write(&format!("DROP DATABASE {qdb}")).unwrap();

        let tables = tables.unwrap();
        assert_eq!(tables.len(), 1);
        assert_eq!(tables[0].name, "t");
        assert_eq!(tables[0].row_count, 2);
        let columns = columns.unwrap();
        let names: Vec<_> = columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["id", "v", "n"]);
        assert!(columns[0].extra.contains("auto_increment"));
        assert!(!columns[1].is_nullable && columns[2].is_nullable);
        assert_eq!(pks.unwrap(), ["id"]);
    }

    #[test]
    #[ignore = "necesita RUSTQL_TEST_MYSQL"]
    fn la_sesion_del_editor_sobrevive_entre_ejecuciones() {
        let b = test_server::backend();
        b.execute_write("DROP DATABASE IF EXISTS rustql_test_session").unwrap();
        b.execute_write("CREATE DATABASE rustql_test_session").unwrap();
        b.execute_write("CREATE TABLE rustql_test_session.t (id INT)").unwrap();
        let db = Some("rustql_test_session");
        let run = |sql: &str| b.execute_in_session(db, sql).unwrap().rows;
        let count = || {
            b.execute_query("SELECT COUNT(*) FROM rustql_test_session.t")
                .unwrap()
                .rows[0][0]
                .clone()
        };

        // Cada línea es una ejecución (F9) distinta
        run("SET @x = 42");
        run("START TRANSACTION");
        run("INSERT INTO t VALUES (1)");
        assert_eq!(run("SELECT @x, COUNT(*) FROM t")[0], [Some("42".into()), Some("1".into())]);
        assert_eq!(count(), Some("0".into()), "sin COMMIT, el resto no la ve");
        run("ROLLBACK");
        assert_eq!(run("SELECT COUNT(*) FROM t")[0], [Some("0".into())]);

        run("START TRANSACTION");
        run("INSERT INTO t VALUES (2)");
        run("COMMIT");
        assert_eq!(count(), Some("1".into()));

        b.execute_write("DROP DATABASE rustql_test_session").unwrap();
    }
}

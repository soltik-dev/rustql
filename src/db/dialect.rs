//! Database engine of a connection and its SQL dialect: how identifiers,
//! literals and the statements that differ between engines are written.
//! The rest of the program builds its SQL with this and assumes no
//! particular engine.

use std::borrow::Cow;

use serde::{Deserialize, Serialize};

use super::TableKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    /// MySQL and MariaDB.
    #[default]
    Mysql,
    Postgres,
}

/// PostgreSQL schema whose tables appear in the tree without a prefix;
/// those of other schemas appear as `schema.table`.
pub const PG_DEFAULT_SCHEMA: &str = "public";

/// Name under which table `table` of PostgreSQL schema `schema` is shown:
/// `table` or `schema.table`. The schema ends at the first dot, so two
/// cases are written differently so the name can be read back:
/// - a table of the default schema with a dot in its name keeps the
///   prefix (`public.a.b`);
/// - a schema with a dot in its name is quoted (`"a.b".table`).
pub fn pg_table_name(schema: &str, table: &str) -> String {
    if schema == PG_DEFAULT_SCHEMA && !table.contains('.') {
        table.to_string()
    } else if schema.contains('.') || schema.starts_with('"') {
        format!("\"{}\".{table}", schema.replace('"', "\"\""))
    } else {
        format!("{schema}.{table}")
    }
}

/// Schema and table of a name produced by `pg_table_name`.
pub fn pg_split_table(name: &str) -> (Cow<'_, str>, &str) {
    if let Some(split) = split_quoted_schema(name) {
        return split;
    }
    let (schema, table) = name.split_once('.').unwrap_or((PG_DEFAULT_SCHEMA, name));
    (Cow::Borrowed(schema), table)
}

/// `"schema".table`, with the schema's quotes doubled. `None` if the name
/// does not have that shape (and then the quotes are part of the name).
fn split_quoted_schema(name: &str) -> Option<(Cow<'_, str>, &str)> {
    let rest = name.strip_prefix('"')?;
    let mut search_from = 0;
    loop {
        let quote = search_from + rest[search_from..].find('"')?;
        match rest[quote + 1..].chars().next()? {
            // Doubled quote: still part of the schema
            '"' => search_from = quote + 2,
            '.' => {
                let schema = rest[..quote].replace("\"\"", "\"");
                return Some((Cow::Owned(schema), &rest[quote + 2..]));
            }
            _ => return None,
        }
    }
}

impl Engine {
    pub const ALL: [Engine; 2] = [Engine::Mysql, Engine::Postgres];

    pub fn label(self) -> &'static str {
        match self {
            Engine::Mysql => "MySQL / MariaDB",
            Engine::Postgres => "PostgreSQL",
        }
    }

    pub fn default_port(self) -> u16 {
        match self {
            Engine::Mysql => 3306,
            Engine::Postgres => 5432,
        }
    }

    /// Usual administrator user: the one the form suggests.
    pub fn default_user(self) -> &'static str {
        match self {
            Engine::Mysql => "root",
            Engine::Postgres => "postgres",
        }
    }

    /// Charsets offered by the create-database dialog. Empty if the engine
    /// has no per-database charset (it is created with the server encoding).
    pub fn database_charsets(self) -> &'static [&'static str] {
        match self {
            Engine::Mysql => &["utf8mb4", "utf8", "latin1", "ascii", "binary"],
            Engine::Postgres => &[],
        }
    }

    /// Column types offered by the create-table dialog, and whether they
    /// take a length (`VARCHAR(50)`).
    pub fn column_types(self) -> &'static [(&'static str, bool)] {
        match self {
            Engine::Mysql => &[
                ("INT", true),
                ("BIGINT", true),
                ("SMALLINT", true),
                ("TINYINT", true),
                ("VARCHAR", true),
                ("TEXT", false),
                ("MEDIUMTEXT", false),
                ("LONGTEXT", false),
                ("CHAR", true),
                ("DECIMAL", true),
                ("FLOAT", true),
                ("DOUBLE", true),
                ("DATE", false),
                ("DATETIME", false),
                ("TIMESTAMP", false),
                ("TIME", false),
                ("BOOLEAN", false),
                ("JSON", false),
                ("BLOB", false),
            ],
            Engine::Postgres => &[
                ("INTEGER", false),
                ("BIGINT", false),
                ("SMALLINT", false),
                ("SERIAL", false),
                ("BIGSERIAL", false),
                ("VARCHAR", true),
                ("TEXT", false),
                ("CHAR", true),
                ("NUMERIC", true),
                ("REAL", false),
                ("DOUBLE PRECISION", false),
                ("DATE", false),
                ("TIMESTAMP", false),
                ("TIMESTAMPTZ", false),
                ("TIME", false),
                ("BOOLEAN", false),
                ("JSONB", false),
                ("UUID", false),
                ("BYTEA", false),
            ],
        }
    }

    /// Identifier inside the engine's quotes: `` `name` `` or `"name"`.
    pub fn quote_ident(self, s: &str) -> String {
        match self {
            Engine::Mysql => format!("`{}`", s.replace('`', "``")),
            Engine::Postgres => format!("\"{}\"", s.replace('"', "\"\"")),
        }
    }

    /// Tree table, without its database. In PostgreSQL it carries its schema.
    pub fn table_ref(self, table: &str) -> String {
        match self {
            Engine::Mysql => self.quote_ident(table),
            Engine::Postgres => {
                let (schema, table) = pg_split_table(table);
                format!("{}.{}", self.quote_ident(&schema), self.quote_ident(table))
            }
        }
    }

    /// Tree table as it is named in a statement. In MySQL it goes with its
    /// database (`` `db`.`table` ``); in PostgreSQL the database is the
    /// connection's and cannot be named: it goes with its schema.
    pub fn qualified(self, db: &str, table: &str) -> String {
        match self {
            Engine::Mysql => format!("{}.{}", self.quote_ident(db), self.table_ref(table)),
            Engine::Postgres => self.table_ref(table),
        }
    }

    /// String literal, with its quotes.
    pub fn literal(self, s: &str) -> String {
        match self {
            // The quote is doubled (`''`) instead of escaped as `\'`: if the
            // server has NO_BACKSLASH_ESCAPES in its sql_mode, `\'` would
            // close the string and the rest of the value would run as SQL.
            // With `''` the worst that can happen in that mode is that
            // backslashes are read literally.
            Engine::Mysql => format!(
                "'{}'",
                s.replace('\\', "\\\\")
                    .replace('\0', "\\0")
                    .replace('\n', "\\n")
                    .replace('\r', "\\r")
                    .replace('\x1a', "\\Z")
                    .replace('\'', "''")
            ),
            // Backslashes are literals or escapes depending on
            // standard_conforming_strings: with a backslash the E'…' syntax
            // is used, which always treats them as escapes. That way a value
            // with `\'` cannot close the string in either mode.
            Engine::Postgres => {
                let quoted = s.replace('\'', "''");
                if quoted.contains('\\') {
                    format!("E'{}'", quoted.replace('\\', "\\\\"))
                } else {
                    format!("'{quoted}'")
                }
            }
        }
    }

    /// Placeholder for parameter number `n` (from 1) of a statement.
    pub fn placeholder(self, n: usize) -> String {
        match self {
            Engine::Mysql => "?".to_string(),
            Engine::Postgres => format!("${n}"),
        }
    }

    /// Condition "`column` looks like `pattern`" (with LIKE wildcards),
    /// case-insensitive and for columns of any type.
    pub fn like(self, column: &str, pattern: &str) -> String {
        let (column, pattern) = (self.quote_ident(column), self.literal(pattern));
        match self {
            // LIKE converts only numbers and dates to text, and with the usual
            // collations it is case-insensitive
            Engine::Mysql => format!("{column} LIKE {pattern}"),
            // LIKE is case-sensitive and only accepts text
            Engine::Postgres => format!("{column}::text ILIKE {pattern}"),
        }
    }

    /// INSERT into `target` of the columns `columns` (already quoted) with
    /// one parameter for each. With no columns, a row with every default
    /// value.
    pub fn insert(self, target: &str, columns: &[String]) -> String {
        if columns.is_empty() {
            return match self {
                Engine::Mysql => format!("INSERT INTO {target} () VALUES ()"),
                Engine::Postgres => format!("INSERT INTO {target} DEFAULT VALUES"),
            };
        }
        let placeholders: Vec<String> = (1..=columns.len()).map(|n| self.placeholder(n)).collect();
        format!(
            "INSERT INTO {target} ({}) VALUES ({})",
            columns.join(", "),
            placeholders.join(", ")
        )
    }

    /// Drops table or view `table` of `db`.
    pub fn drop_table(self, db: &str, table: &str, kind: TableKind) -> String {
        format!("DROP {} {}", kind.keyword(), self.qualified(db, table))
    }

    /// Creates table `target` with the structure of `source` (columns, keys,
    /// indexes and default values), without its data.
    pub fn create_table_like(self, target: &str, source: &str) -> String {
        match self {
            Engine::Mysql => format!("CREATE TABLE {target} LIKE {source}"),
            Engine::Postgres => format!("CREATE TABLE {target} (LIKE {source} INCLUDING ALL)"),
        }
    }
}

/// Parameters of a statement, in the order they appear in it.
pub struct ParamList {
    engine: Engine,
    values: Vec<Option<String>>,
}

impl ParamList {
    pub fn new(engine: Engine) -> Self {
        Self {
            engine,
            values: Vec::new(),
        }
    }

    /// Adds a value (`None` = NULL) and returns its placeholder.
    pub fn push(&mut self, value: Option<String>) -> String {
        self.values.push(value);
        self.engine.placeholder(self.values.len())
    }

    pub fn into_values(self) -> Vec<Option<String>> {
        self.values
    }
}

#[cfg(test)]
mod tests {
    use super::Engine::{Mysql, Postgres};
    use super::*;

    /// With NO_BACKSLASH_ESCAPES, escaping the quote as `\'`, this value
    /// turned `v = '…'` into `v = '\\\' OR 1=1 -- '`.
    const ATTACK: &str = "\\' OR 1=1 -- ";

    #[test]
    fn identificadores_con_las_comillas_de_cada_motor() {
        assert_eq!(Mysql.quote_ident("a`b"), "`a``b`");
        assert_eq!(Mysql.qualified("bd", "ta`bla"), "`bd`.`ta``bla`");
        assert_eq!(Postgres.quote_ident("a\"b"), "\"a\"\"b\"");
        // In MySQL a dot is part of the table name
        assert_eq!(Mysql.qualified("bd", "a.b"), "`bd`.`a.b`");
    }

    #[test]
    fn las_tablas_de_postgres_llevan_su_esquema_y_no_su_bbdd() {
        assert_eq!(
            Postgres.qualified("tienda", "pedidos"),
            "\"public\".\"pedidos\""
        );
        assert_eq!(
            Postgres.qualified("tienda", "ventas.pedidos"),
            "\"ventas\".\"pedidos\""
        );
        assert_eq!(
            Postgres.table_ref("ventas.pe\"didos"),
            "\"ventas\".\"pe\"\"didos\""
        );
    }

    #[test]
    fn el_nombre_de_una_tabla_de_postgres_es_reversible() {
        for (schema, table, shown) in [
            ("public", "pedidos", "pedidos"),
            ("ventas", "pedidos", "ventas.pedidos"),
            // Without the prefix it would read as table "b" of schema "a"
            ("public", "a.b", "public.a.b"),
            ("ventas", "a.b", "ventas.a.b"),
            // A schema with a dot is quoted
            ("a.b", "t", "\"a.b\".t"),
            ("a.b", "t.u", "\"a.b\".t.u"),
            ("a\".b", "t", "\"a\"\".b\".t"),
            ("\"raro", "t", "\"\"\"raro\".t"),
            // Quotes that do not enclose a schema are part of the name
            ("public", "\"x\"y", "\"x\"y"),
            ("public", "\"abierta", "\"abierta"),
            ("public", "\"x\".y", "public.\"x\".y"),
            ("ven\"tas", "t", "ven\"tas.t"),
        ] {
            assert_eq!(pg_table_name(schema, table), shown);
            let (back_schema, back_table) = pg_split_table(shown);
            assert_eq!(
                (back_schema.as_ref(), back_table),
                (schema, table),
                "{shown}"
            );
        }
        assert_eq!(Postgres.table_ref("\"a.b\".t"), "\"a.b\".\"t\"");
    }

    #[test]
    fn literal_de_mysql_dobla_las_comillas() {
        assert_eq!(Mysql.literal("it's"), "'it''s'");
        assert_eq!(Mysql.literal(ATTACK), "'\\\\'' OR 1=1 -- '");
        assert_eq!(Mysql.literal("a\nb\\c"), "'a\\nb\\\\c'");
    }

    #[test]
    fn literal_de_postgres_no_depende_de_como_se_lean_las_barras() {
        assert_eq!(Postgres.literal("it's"), "'it''s'");
        // Without backslashes, the standard form; line breaks go as they are
        assert_eq!(Postgres.literal("a\nb"), "'a\nb'");
        assert_eq!(Postgres.literal("a\\b"), "E'a\\\\b'");
        assert_eq!(Postgres.literal(ATTACK), "E'\\\\'' OR 1=1 -- '");
    }

    #[test]
    fn parametros_en_el_orden_de_la_sentencia() {
        let mut params = ParamList::new(Mysql);
        assert_eq!(params.push(Some("a".into())), "?");
        assert_eq!(params.push(None), "?");
        assert_eq!(params.into_values(), [Some("a".to_string()), None]);

        let mut params = ParamList::new(Postgres);
        assert_eq!(params.push(Some("a".into())), "$1");
        assert_eq!(params.push(None), "$2");
    }

    #[test]
    fn insert_con_y_sin_columnas() {
        let cols = ["`a`".to_string(), "`b`".to_string()];
        assert_eq!(
            Mysql.insert("`t`", &cols),
            "INSERT INTO `t` (`a`, `b`) VALUES (?, ?)"
        );
        assert_eq!(Mysql.insert("`t`", &[]), "INSERT INTO `t` () VALUES ()");
        let cols = ["\"a\"".to_string(), "\"b\"".to_string()];
        assert_eq!(
            Postgres.insert("\"t\"", &cols),
            "INSERT INTO \"t\" (\"a\", \"b\") VALUES ($1, $2)"
        );
        assert_eq!(
            Postgres.insert("\"t\"", &[]),
            "INSERT INTO \"t\" DEFAULT VALUES"
        );
    }

    #[test]
    fn like_sirve_para_cualquier_columna() {
        assert_eq!(Mysql.like("id", "%1%"), "`id` LIKE '%1%'");
        assert_eq!(Postgres.like("id", "%1%"), "\"id\"::text ILIKE '%1%'");
    }
}

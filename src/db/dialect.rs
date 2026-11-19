//! Motor de base de datos de una conexión y su dialecto SQL: cómo se
//! escriben los identificadores, los literales y las sentencias que no son
//! iguales en todos los motores. El resto del programa compone su SQL con
//! esto y no asume ninguno.

use std::borrow::Cow;

use serde::{Deserialize, Serialize};

use super::TableKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    /// MySQL y MariaDB.
    #[default]
    Mysql,
    Postgres,
}

/// Esquema de PostgreSQL cuyas tablas salen en el árbol sin prefijo; las de
/// los demás, como `esquema.tabla`.
pub const PG_DEFAULT_SCHEMA: &str = "public";

/// Nombre con el que se enseña la tabla `table` del esquema `schema` de
/// PostgreSQL: `tabla` o `esquema.tabla`. El esquema acaba en el primer
/// punto, así que hay dos casos que se escriben de otra forma para que el
/// nombre se pueda volver a leer:
/// - una tabla del esquema por defecto con un punto en el nombre conserva
///   el prefijo (`public.a.b`);
/// - un esquema con un punto en el nombre va entre comillas (`"a.b".tabla`).
pub fn pg_table_name(schema: &str, table: &str) -> String {
    if schema == PG_DEFAULT_SCHEMA && !table.contains('.') {
        table.to_string()
    } else if schema.contains('.') || schema.starts_with('"') {
        format!("\"{}\".{table}", schema.replace('"', "\"\""))
    } else {
        format!("{schema}.{table}")
    }
}

/// Esquema y tabla de un nombre dado por `pg_table_name`.
pub fn pg_split_table(name: &str) -> (Cow<'_, str>, &str) {
    if let Some(split) = split_quoted_schema(name) {
        return split;
    }
    let (schema, table) = name.split_once('.').unwrap_or((PG_DEFAULT_SCHEMA, name));
    (Cow::Borrowed(schema), table)
}

/// `"esquema".tabla`, con las comillas del esquema dobladas. `None` si el
/// nombre no tiene esa forma (y entonces las comillas son parte del nombre).
fn split_quoted_schema(name: &str) -> Option<(Cow<'_, str>, &str)> {
    let rest = name.strip_prefix('"')?;
    let mut search_from = 0;
    loop {
        let quote = search_from + rest[search_from..].find('"')?;
        match rest[quote + 1..].chars().next()? {
            // Comilla doblada: sigue siendo el esquema
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

    /// Usuario administrador habitual: el que propone el formulario.
    pub fn default_user(self) -> &'static str {
        match self {
            Engine::Mysql => "root",
            Engine::Postgres => "postgres",
        }
    }

    /// Charsets que ofrece el diálogo de crear una BBDD. Vacío si el motor
    /// no los tiene por BBDD (se crea con la codificación del servidor).
    pub fn database_charsets(self) -> &'static [&'static str] {
        match self {
            Engine::Mysql => &["utf8mb4", "utf8", "latin1", "ascii", "binary"],
            Engine::Postgres => &[],
        }
    }

    /// Tipos de columna que ofrece el diálogo de crear una tabla, y si
    /// admiten longitud (`VARCHAR(50)`).
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

    /// Identificador entre las comillas del motor: `` `nombre` `` o `"nombre"`.
    pub fn quote_ident(self, s: &str) -> String {
        match self {
            Engine::Mysql => format!("`{}`", s.replace('`', "``")),
            Engine::Postgres => format!("\"{}\"", s.replace('"', "\"\"")),
        }
    }

    /// Tabla del árbol, sin su BBDD. En PostgreSQL lleva su esquema.
    pub fn table_ref(self, table: &str) -> String {
        match self {
            Engine::Mysql => self.quote_ident(table),
            Engine::Postgres => {
                let (schema, table) = pg_split_table(table);
                format!("{}.{}", self.quote_ident(&schema), self.quote_ident(table))
            }
        }
    }

    /// Tabla del árbol tal como se nombra en una sentencia. En MySQL va con
    /// su BBDD (`` `bd`.`tabla` ``); en PostgreSQL la BBDD es la de la
    /// conexión y no se puede nombrar: va con su esquema.
    pub fn qualified(self, db: &str, table: &str) -> String {
        match self {
            Engine::Mysql => format!("{}.{}", self.quote_ident(db), self.table_ref(table)),
            Engine::Postgres => self.table_ref(table),
        }
    }

    /// Literal de cadena, con sus comillas.
    pub fn literal(self, s: &str) -> String {
        match self {
            // La comilla se dobla (`''`) en vez de escaparse con `\'`: si el
            // servidor tiene NO_BACKSLASH_ESCAPES en el sql_mode, `\'`
            // cerraría la cadena y el resto del valor se ejecutaría como
            // SQL. Con `''` lo peor que pasa en ese modo es que las barras se
            // lean literalmente.
            Engine::Mysql => format!(
                "'{}'",
                s.replace('\\', "\\\\")
                    .replace('\0', "\\0")
                    .replace('\n', "\\n")
                    .replace('\r', "\\r")
                    .replace('\x1a', "\\Z")
                    .replace('\'', "''")
            ),
            // Las barras son literales o escapes según
            // standard_conforming_strings: con una barra se usa la sintaxis
            // E'…', que las trata siempre como escapes. Un valor con `\'`
            // no puede así cerrar la cadena en ninguno de los dos modos.
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

    /// Marca del parámetro número `n` (desde 1) de una sentencia.
    pub fn placeholder(self, n: usize) -> String {
        match self {
            Engine::Mysql => "?".to_string(),
            Engine::Postgres => format!("${n}"),
        }
    }

    /// Condición «`column` se parece a `pattern`» (con los comodines de
    /// LIKE), sin distinguir mayúsculas y para columnas de cualquier tipo.
    pub fn like(self, column: &str, pattern: &str) -> String {
        let (column, pattern) = (self.quote_ident(column), self.literal(pattern));
        match self {
            // LIKE convierte solo los números y las fechas a texto, y con
            // las collations habituales no distingue mayúsculas
            Engine::Mysql => format!("{column} LIKE {pattern}"),
            // LIKE distingue mayúsculas y solo admite texto
            Engine::Postgres => format!("{column}::text ILIKE {pattern}"),
        }
    }

    /// INSERT en `target` de las columnas `columns` (ya entre comillas) con
    /// un parámetro por cada una. Sin columnas, una fila con todos los
    /// valores por defecto.
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

    /// Elimina la tabla o vista `table` de `db`.
    pub fn drop_table(self, db: &str, table: &str, kind: TableKind) -> String {
        format!("DROP {} {}", kind.keyword(), self.qualified(db, table))
    }

    /// Crea la tabla `target` con la estructura de `source` (columnas,
    /// claves, índices y valores por defecto), sin sus datos.
    pub fn create_table_like(self, target: &str, source: &str) -> String {
        match self {
            Engine::Mysql => format!("CREATE TABLE {target} LIKE {source}"),
            Engine::Postgres => format!("CREATE TABLE {target} (LIKE {source} INCLUDING ALL)"),
        }
    }
}

/// Parámetros de una sentencia, en el orden en que aparecen en ella.
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

    /// Añade un valor (`None` = NULL) y devuelve su marca en la sentencia.
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

    /// Con NO_BACKSLASH_ESCAPES, escapando la comilla como `\'`, este valor
    /// convertía `v = '…'` en `v = '\\\' OR 1=1 -- '`.
    const ATTACK: &str = "\\' OR 1=1 -- ";

    #[test]
    fn identificadores_con_las_comillas_de_cada_motor() {
        assert_eq!(Mysql.quote_ident("a`b"), "`a``b`");
        assert_eq!(Mysql.qualified("bd", "ta`bla"), "`bd`.`ta``bla`");
        assert_eq!(Postgres.quote_ident("a\"b"), "\"a\"\"b\"");
        // En MySQL un punto es parte del nombre de la tabla
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
            // Sin el prefijo se leería como la tabla «b» del esquema «a»
            ("public", "a.b", "public.a.b"),
            ("ventas", "a.b", "ventas.a.b"),
            // Un esquema con un punto va entre comillas
            ("a.b", "t", "\"a.b\".t"),
            ("a.b", "t.u", "\"a.b\".t.u"),
            ("a\".b", "t", "\"a\"\".b\".t"),
            ("\"raro", "t", "\"\"\"raro\".t"),
            // Unas comillas que no encierran un esquema son parte del nombre
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
        // Sin barras, la forma estándar; los saltos de línea van tal cual
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

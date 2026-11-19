//! Dependency-free SQL syntax highlighting: a light lexer + a LayoutJob
//! cached per frame (only recomputed if the text, theme or size changes).

use eframe::egui;

use crate::db::Engine;
use crate::ui::theme::{self, Palette};

#[derive(Debug, Clone, Copy, PartialEq)]
enum TokenKind {
    Keyword,
    StringLit,
    Number,
    Comment,
    Operator,
    Other,
}

impl TokenKind {
    fn color(self, p: &Palette) -> egui::Color32 {
        match self {
            TokenKind::Keyword => p.sql_keyword,
            TokenKind::StringLit => p.sql_string,
            TokenKind::Number => p.sql_number,
            TokenKind::Comment => p.sql_comment,
            TokenKind::Operator => p.sql_operator,
            TokenKind::Other => p.text_primary,
        }
    }
}

struct Token {
    start: usize,
    end: usize,
    kind: TokenKind,
}

/// Reserved words and types of MySQL and PostgreSQL. MUST be kept sorted
/// (it is searched with binary_search; a test checks it).
static KEYWORDS: &[&str] = &[
    "ACTION",
    "ADD",
    "ALL",
    "ALTER",
    "ALWAYS",
    "ANALYZE",
    "AND",
    "ANY",
    "ARRAY",
    "AS",
    "ASC",
    "AUTO_INCREMENT",
    "AVG",
    "BEGIN",
    "BETWEEN",
    "BIGINT",
    "BIGSERIAL",
    "BINARY",
    "BIT",
    "BLOB",
    "BOOL",
    "BOOLEAN",
    "BY",
    "BYTEA",
    "CASCADE",
    "CASE",
    "CAST",
    "CHANGE",
    "CHAR",
    "CHARACTER",
    "CHARSET",
    "CHECK",
    "COLLATE",
    "COLUMN",
    "COMMIT",
    "CONCURRENTLY",
    "CONFLICT",
    "CONSTRAINT",
    "COUNT",
    "CREATE",
    "CROSS",
    "CURRENT_DATE",
    "CURRENT_TIME",
    "CURRENT_TIMESTAMP",
    "CURRENT_USER",
    "DATABASE",
    "DATE",
    "DATETIME",
    "DECIMAL",
    "DEFAULT",
    "DELAYED",
    "DELETE",
    "DESC",
    "DESCRIBE",
    "DISTINCT",
    "DIV",
    "DO",
    "DOUBLE",
    "DROP",
    "DUPLICATE",
    "ELSE",
    "END",
    "ENGINE",
    "ENUM",
    "EVENT",
    "EXCEPT",
    "EXISTS",
    "EXPLAIN",
    "EXTENSION",
    "FALSE",
    "FETCH",
    "FLOAT",
    "FOREIGN",
    "FROM",
    "FULL",
    "FULLTEXT",
    "FUNCTION",
    "GENERATED",
    "GEOMETRY",
    "GRANT",
    "GROUP",
    "HAVING",
    "HIGH_PRIORITY",
    "IDENTIFIED",
    "IDENTITY",
    "IF",
    "IGNORE",
    "ILIKE",
    "IN",
    "INDEX",
    "INNER",
    "INSERT",
    "INT",
    "INTEGER",
    "INTERSECT",
    "INTERVAL",
    "INTO",
    "IS",
    "JOIN",
    "JSON",
    "JSONB",
    "KEY",
    "LATERAL",
    "LEFT",
    "LIKE",
    "LIMIT",
    "LINESTRING",
    "LOCK",
    "LONGBLOB",
    "LONGTEXT",
    "LOW_PRIORITY",
    "MATERIALIZED",
    "MAX",
    "MEDIUMBLOB",
    "MEDIUMINT",
    "MEDIUMTEXT",
    "MIN",
    "MOD",
    "MODIFY",
    "NO",
    "NOT",
    "NOTHING",
    "NOW",
    "NULL",
    "NULLS",
    "NUMERIC",
    "OFFSET",
    "ON",
    "ONLY",
    "OR",
    "ORDER",
    "OUTER",
    "OVER",
    "PARTITION",
    "POINT",
    "POLYGON",
    "PRIMARY",
    "PRIVILEGES",
    "PROCEDURE",
    "QUICK",
    "REAL",
    "RECURSIVE",
    "REFERENCES",
    "REGEXP",
    "RENAME",
    "REPLACE",
    "RESTRICT",
    "RETURNING",
    "REVOKE",
    "RIGHT",
    "RLIKE",
    "ROLLBACK",
    "SAVEPOINT",
    "SCHEMA",
    "SELECT",
    "SEQUENCE",
    "SERIAL",
    "SET",
    "SHOW",
    "SIGNED",
    "SIMILAR",
    "SMALLINT",
    "SMALLSERIAL",
    "SPATIAL",
    "START",
    "STRAIGHT_JOIN",
    "SUM",
    "TABLE",
    "TABLES",
    "TEMP",
    "TEMPORARY",
    "TEXT",
    "THEN",
    "TIME",
    "TIMESTAMP",
    "TIMESTAMPTZ",
    "TINYBLOB",
    "TINYINT",
    "TINYTEXT",
    "TO",
    "TRANSACTION",
    "TRIGGER",
    "TRUE",
    "TRUNCATE",
    "UNION",
    "UNIQUE",
    "UNKNOWN",
    "UNLOCK",
    "UNSIGNED",
    "UPDATE",
    "USE",
    "USING",
    "UUID",
    "VACUUM",
    "VALUES",
    "VARBINARY",
    "VARCHAR",
    "VIEW",
    "WHEN",
    "WHERE",
    "WINDOW",
    "WITH",
    "XOR",
    "YEAR",
    "ZEROFILL",
];

fn is_word_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '$'
}

/// If a PostgreSQL dollar-quoted string starts at `chars[i]` (`$$…$$` or
/// `$tag$…$tag$`), where its opening marker ends.
fn dollar_quote_open(chars: &[(usize, char)], i: usize) -> Option<usize> {
    let mut j = i + 1;
    while j < chars.len() && (chars[j].1.is_ascii_alphanumeric() || chars[j].1 == '_') {
        j += 1;
    }
    // `$1` is a parameter, not a marker
    let tagged_by_number = chars.get(i + 1).is_some_and(|c| c.1.is_ascii_digit());
    (j < chars.len() && chars[j].1 == '$' && !tagged_by_number).then_some(j + 1)
}

fn tokenize(engine: Engine, sql: &str) -> Vec<Token> {
    let mysql = engine == Engine::Mysql;
    let chars: Vec<(usize, char)> = sql.char_indices().collect();
    let n = chars.len();
    let byte_at = |i: usize| if i < n { chars[i].0 } else { sql.len() };

    let mut tokens: Vec<Token> = Vec::new();
    let push = |start: usize, end: usize, kind: TokenKind, tokens: &mut Vec<Token>| {
        // Merge contiguous runs of "Other" (spaces, identifiers…)
        if kind == TokenKind::Other
            && let Some(last) = tokens.last_mut()
            && last.kind == TokenKind::Other
            && last.end == start
        {
            last.end = end;
            return;
        }
        tokens.push(Token { start, end, kind });
    };

    let mut i = 0;
    while i < n {
        let (start, c) = chars[i];
        let kind = match c {
            // Comments: -- to EOL, # to EOL, /* … */
            '-' if i + 1 < n && chars[i + 1].1 == '-' => {
                while i < n && chars[i].1 != '\n' {
                    i += 1;
                }
                TokenKind::Comment
            }
            // In PostgreSQL `#` is an operator (`data #> '{a}'`)
            '#' if mysql => {
                while i < n && chars[i].1 != '\n' {
                    i += 1;
                }
                TokenKind::Comment
            }
            '/' if i + 1 < n && chars[i + 1].1 == '*' => {
                i += 2;
                while i < n && !(chars[i].1 == '*' && i + 1 < n && chars[i + 1].1 == '/') {
                    i += 1;
                }
                i = (i + 2).min(n); // tolerates an unclosed comment
                TokenKind::Comment
            }
            // Dollar-quoted string: until its marker repeats
            '$' if !mysql && dollar_quote_open(&chars, i).is_some() => {
                let body = dollar_quote_open(&chars, i).unwrap_or(n);
                let tag: Vec<char> = chars[i..body].iter().map(|c| c.1).collect();
                i = body;
                while i < n && !chars[i..].iter().map(|c| &c.1).take(tag.len()).eq(&tag) {
                    i += 1;
                }
                i = (i + tag.len()).min(n); // tolerates an unclosed string
                TokenKind::StringLit
            }
            // Strings with a doubled quote ('' / "") and, in MySQL, \x escapes.
            // In PostgreSQL double quotes enclose an identifier:
            // normal colour
            '\'' | '"' => {
                let quote = c;
                i += 1;
                while i < n {
                    match chars[i].1 {
                        '\\' if mysql => i += 2,
                        ch if ch == quote => {
                            if i + 1 < n && chars[i + 1].1 == quote {
                                i += 2; // doubled quote: the string goes on
                            } else {
                                i += 1;
                                break;
                            }
                        }
                        _ => i += 1,
                    }
                }
                i = i.min(n);
                if quote == '"' && !mysql {
                    TokenKind::Other
                } else {
                    TokenKind::StringLit
                }
            }
            // Identifier in backticks: normal colour
            '`' if mysql => {
                i += 1;
                while i < n && chars[i].1 != '`' {
                    i += 1;
                }
                i = (i + 1).min(n);
                TokenKind::Other
            }
            // Numbers: integers, decimals, exponent, hex 0x…
            '0'..='9' => {
                i += 1;
                if c == '0' && i < n && (chars[i].1 == 'x' || chars[i].1 == 'X') {
                    i += 1;
                    while i < n && chars[i].1.is_ascii_hexdigit() {
                        i += 1;
                    }
                } else {
                    while i < n && (chars[i].1.is_ascii_digit() || chars[i].1 == '.') {
                        i += 1;
                    }
                    if i < n && (chars[i].1 == 'e' || chars[i].1 == 'E') {
                        i += 1;
                        if i < n && (chars[i].1 == '+' || chars[i].1 == '-') {
                            i += 1;
                        }
                        while i < n && chars[i].1.is_ascii_digit() {
                            i += 1;
                        }
                    }
                }
                TokenKind::Number
            }
            '.' if i + 1 < n && chars[i + 1].1.is_ascii_digit() => {
                i += 1;
                while i < n && chars[i].1.is_ascii_digit() {
                    i += 1;
                }
                TokenKind::Number
            }
            // Words: keyword or identifier
            c if is_word_start(c) => {
                i += 1;
                while i < n && is_word_char(chars[i].1) {
                    i += 1;
                }
                let word = &sql[start..byte_at(i)];
                let upper = word.to_ascii_uppercase();
                if KEYWORDS.binary_search(&upper.as_str()).is_ok() {
                    TokenKind::Keyword
                } else {
                    TokenKind::Other
                }
            }
            // Operators and punctuation
            '=' | '<' | '>' | '!' | '+' | '-' | '*' | '/' | '%' | ',' | ';' | '(' | ')' | '.'
            | '|' | '&' | '^' | '~' | '?' | ':' | '@' => {
                i += 1;
                TokenKind::Operator
            }
            // Spaces and any other character
            _ => {
                i += 1;
                TokenKind::Other
            }
        };
        push(start, byte_at(i), kind, &mut tokens);
    }

    tokens
}

fn layout_job(engine: Engine, dark: bool, font_size: f32, sql: &str) -> egui::text::LayoutJob {
    let p = theme::of(dark);
    let font_id = egui::FontId::monospace(font_size);
    let mut job = egui::text::LayoutJob::default();
    for tok in tokenize(engine, sql) {
        job.append(
            &sql[tok.start..tok.end],
            0.0,
            egui::TextFormat {
                font_id: font_id.clone(),
                color: tok.kind.color(p),
                ..Default::default()
            },
        );
    }
    job
}

#[derive(Default)]
struct Highlighter;

type Key<'a> = (Engine, bool, u32, &'a str);

impl egui::cache::ComputerMut<Key<'_>, egui::text::LayoutJob> for Highlighter {
    fn compute(&mut self, (engine, dark, font_size_bits, sql): Key<'_>) -> egui::text::LayoutJob {
        layout_job(engine, dark, f32::from_bits(font_size_bits), sql)
    }
}

type HighlightCache = egui::cache::FrameCache<egui::text::LayoutJob, Highlighter>;

/// Highlighted LayoutJob for `sql` in the dialect of `engine`, cached
/// (key: engine + theme + size + text).
pub fn highlight(
    ctx: &egui::Context,
    engine: Engine,
    dark: bool,
    font_size: f32,
    sql: &str,
) -> egui::text::LayoutJob {
    ctx.memory_mut(|m| {
        m.caches
            .cache::<HighlightCache>()
            .get((engine, dark, font_size.to_bits(), sql))
            .clone()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keywords_ordenadas() {
        assert!(
            KEYWORDS.is_sorted(),
            "KEYWORDS debe estar ordenada para binary_search"
        );
    }

    fn kinds(sql: &str) -> Vec<(String, TokenKind)> {
        kinds_in(Engine::Mysql, sql)
    }

    fn kinds_in(engine: Engine, sql: &str) -> Vec<(String, TokenKind)> {
        tokenize(engine, sql)
            .into_iter()
            .map(|t| (sql[t.start..t.end].to_owned(), t.kind))
            .collect()
    }

    #[test]
    fn tokeniza_query_basica() {
        let toks = kinds("SELECT id FROM users WHERE name = 'ana' -- top\nLIMIT 1.5");
        assert!(toks.contains(&("SELECT".into(), TokenKind::Keyword)));
        assert!(toks.contains(&("FROM".into(), TokenKind::Keyword)));
        assert!(toks.contains(&("'ana'".into(), TokenKind::StringLit)));
        assert!(toks.contains(&("-- top".into(), TokenKind::Comment)));
        assert!(toks.contains(&("1.5".into(), TokenKind::Number)));
        assert!(toks.contains(&("=".into(), TokenKind::Operator)));
    }

    #[test]
    fn cubre_todo_el_texto() {
        let sql = "sel /* sin cerrar";
        let toks = tokenize(Engine::Mysql, sql);
        let total: usize = toks.iter().map(|t| t.end - t.start).sum();
        assert_eq!(total, sql.len());
    }

    #[test]
    fn string_sin_cerrar_y_comilla_doblada() {
        let toks = kinds("'it''s' 'abierta");
        assert_eq!(toks[0], ("'it''s'".into(), TokenKind::StringLit));
        assert!(
            toks.iter()
                .any(|(s, k)| s == "'abierta" && *k == TokenKind::StringLit)
        );
    }

    #[test]
    fn cada_motor_con_sus_comillas_y_comentarios() {
        let sql = "SELECT \"col\", `x` FROM t # resto";
        let mysql = kinds_in(Engine::Mysql, sql);
        assert!(mysql.contains(&("\"col\"".into(), TokenKind::StringLit)));
        assert!(mysql.contains(&("# resto".into(), TokenKind::Comment)));
        // In PostgreSQL double quotes are an identifier and `#` an
        // operator
        let postgres = kinds_in(Engine::Postgres, sql);
        assert!(!postgres.iter().any(|(_, k)| *k == TokenKind::StringLit));
        assert!(!postgres.iter().any(|(_, k)| *k == TokenKind::Comment));

        // The backslash only escapes in MySQL: there the string goes on after `\\'`
        let sql = "'a\\' AND 'b'";
        assert_eq!(kinds_in(Engine::Postgres, sql)[0].0, "'a\\'");
        assert_eq!(kinds_in(Engine::Mysql, sql)[0].0, "'a\\' AND '");
    }

    #[test]
    fn cadenas_entre_dolares_de_postgres() {
        let sql = "DO $cuerpo$ SELECT 'x'; $$ $cuerpo$ RETURNING $1, $$a$$, $sin cerrar";
        let toks = kinds_in(Engine::Postgres, sql);
        assert_eq!(toks[0], ("DO".into(), TokenKind::Keyword));
        assert!(toks.contains(&(
            "$cuerpo$ SELECT 'x'; $$ $cuerpo$".into(),
            TokenKind::StringLit
        )));
        assert!(toks.contains(&("RETURNING".into(), TokenKind::Keyword)));
        assert!(toks.contains(&("$$a$$".into(), TokenKind::StringLit)));
        // Neither the parameter nor a lone dollar opens a string
        assert!(
            !toks
                .iter()
                .any(|(s, k)| s.contains("$1") && *k == TokenKind::StringLit)
        );
        let total: usize = toks.iter().map(|(s, _)| s.len()).sum();
        assert_eq!(total, sql.len());
    }
}

//! Resaltado de sintaxis SQL sin dependencias: lexer ligero + LayoutJob
//! cacheado por frame (solo se recalcula si cambia texto, tema o tamaño).

use eframe::egui;

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

/// Palabras reservadas y tipos de MySQL. DEBE mantenerse ordenada
/// (se busca con binary_search; hay un test que lo comprueba).
static KEYWORDS: &[&str] = &[
    "ACTION",
    "ADD",
    "ALL",
    "ALTER",
    "ANALYZE",
    "AND",
    "AS",
    "ASC",
    "AUTO_INCREMENT",
    "AVG",
    "BEGIN",
    "BETWEEN",
    "BIGINT",
    "BINARY",
    "BIT",
    "BLOB",
    "BOOL",
    "BOOLEAN",
    "BY",
    "CASCADE",
    "CASE",
    "CHANGE",
    "CHAR",
    "CHARACTER",
    "CHARSET",
    "CHECK",
    "COLLATE",
    "COLUMN",
    "COMMIT",
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
    "DOUBLE",
    "DROP",
    "DUPLICATE",
    "ELSE",
    "END",
    "ENGINE",
    "ENUM",
    "EVENT",
    "EXISTS",
    "EXPLAIN",
    "FALSE",
    "FLOAT",
    "FOREIGN",
    "FROM",
    "FULLTEXT",
    "FUNCTION",
    "GEOMETRY",
    "GRANT",
    "GROUP",
    "HAVING",
    "HIGH_PRIORITY",
    "IDENTIFIED",
    "IF",
    "IGNORE",
    "IN",
    "INDEX",
    "INNER",
    "INSERT",
    "INT",
    "INTEGER",
    "INTERVAL",
    "INTO",
    "IS",
    "JOIN",
    "JSON",
    "KEY",
    "LEFT",
    "LIKE",
    "LIMIT",
    "LINESTRING",
    "LOCK",
    "LONGBLOB",
    "LONGTEXT",
    "LOW_PRIORITY",
    "MAX",
    "MEDIUMBLOB",
    "MEDIUMINT",
    "MEDIUMTEXT",
    "MIN",
    "MOD",
    "MODIFY",
    "NO",
    "NOT",
    "NOW",
    "NULL",
    "NUMERIC",
    "OFFSET",
    "ON",
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
    "REVOKE",
    "RIGHT",
    "RLIKE",
    "ROLLBACK",
    "SAVEPOINT",
    "SCHEMA",
    "SELECT",
    "SERIAL",
    "SET",
    "SHOW",
    "SIGNED",
    "SMALLINT",
    "SPATIAL",
    "START",
    "STRAIGHT_JOIN",
    "SUM",
    "TABLE",
    "TABLES",
    "TEMPORARY",
    "TEXT",
    "THEN",
    "TIME",
    "TIMESTAMP",
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

fn tokenize(sql: &str) -> Vec<Token> {
    let chars: Vec<(usize, char)> = sql.char_indices().collect();
    let n = chars.len();
    let byte_at = |i: usize| if i < n { chars[i].0 } else { sql.len() };

    let mut tokens: Vec<Token> = Vec::new();
    let push = |start: usize, end: usize, kind: TokenKind, tokens: &mut Vec<Token>| {
        // Fusionar runs contiguos de "Other" (espacios, identificadores…)
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
            // Comentarios: -- hasta EOL, # hasta EOL, /* … */
            '-' if i + 1 < n && chars[i + 1].1 == '-' => {
                while i < n && chars[i].1 != '\n' {
                    i += 1;
                }
                TokenKind::Comment
            }
            '#' => {
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
                i = (i + 2).min(n); // tolera comentario sin cerrar
                TokenKind::Comment
            }
            // Strings con escapes \x y comilla doblada ('' / "")
            '\'' | '"' => {
                let quote = c;
                i += 1;
                while i < n {
                    match chars[i].1 {
                        '\\' => i += 2,
                        ch if ch == quote => {
                            if i + 1 < n && chars[i + 1].1 == quote {
                                i += 2; // comilla doblada: sigue el string
                            } else {
                                i += 1;
                                break;
                            }
                        }
                        _ => i += 1,
                    }
                }
                i = i.min(n);
                TokenKind::StringLit
            }
            // Identificador entre backticks: color normal
            '`' => {
                i += 1;
                while i < n && chars[i].1 != '`' {
                    i += 1;
                }
                i = (i + 1).min(n);
                TokenKind::Other
            }
            // Números: enteros, decimales, exponente, hex 0x…
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
            // Palabras: keyword o identificador
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
            // Operadores y puntuación
            '=' | '<' | '>' | '!' | '+' | '-' | '*' | '/' | '%' | ',' | ';' | '(' | ')' | '.'
            | '|' | '&' | '^' | '~' | '?' | ':' | '@' => {
                i += 1;
                TokenKind::Operator
            }
            // Espacios y cualquier otro carácter
            _ => {
                i += 1;
                TokenKind::Other
            }
        };
        push(start, byte_at(i), kind, &mut tokens);
    }

    tokens
}

fn layout_job(dark: bool, font_size: f32, sql: &str) -> egui::text::LayoutJob {
    let p = theme::of(dark);
    let font_id = egui::FontId::monospace(font_size);
    let mut job = egui::text::LayoutJob::default();
    for tok in tokenize(sql) {
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

impl egui::cache::ComputerMut<(bool, u32, &str), egui::text::LayoutJob> for Highlighter {
    fn compute(&mut self, (dark, font_size_bits, sql): (bool, u32, &str)) -> egui::text::LayoutJob {
        layout_job(dark, f32::from_bits(font_size_bits), sql)
    }
}

type HighlightCache = egui::cache::FrameCache<egui::text::LayoutJob, Highlighter>;

/// LayoutJob resaltado para `sql`, cacheado (clave: tema + tamaño + texto).
pub fn highlight(
    ctx: &egui::Context,
    dark: bool,
    font_size: f32,
    sql: &str,
) -> egui::text::LayoutJob {
    ctx.memory_mut(|m| {
        m.caches
            .cache::<HighlightCache>()
            .get((dark, font_size.to_bits(), sql))
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
        tokenize(sql)
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
        let toks = tokenize(sql);
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
}

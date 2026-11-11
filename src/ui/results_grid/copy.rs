use crate::db::quote_ident;

// ── Filas en el formato de copia ─────────────────────────────────────────────

pub fn format_rows_for_copy(
    columns: &[String],
    rows: &[&Vec<Option<String>>],
    fmt: &crate::settings::CopyFormat,
    table_name: Option<&str>,
) -> String {
    match fmt {
        crate::settings::CopyFormat::Csv => {
            let header = columns
                .iter()
                .map(|c| format!("\"{}\"", c.replace('"', "\"\"")))
                .collect::<Vec<_>>()
                .join(",");
            let data_rows: Vec<String> = rows
                .iter()
                .map(|row| {
                    row.iter()
                        .map(|cell| match cell {
                            None => "NULL".to_string(),
                            Some(v) => format!("\"{}\"", v.replace('"', "\"\"")),
                        })
                        .collect::<Vec<_>>()
                        .join(",")
                })
                .collect();
            std::iter::once(header)
                .chain(data_rows)
                .collect::<Vec<_>>()
                .join("\n")
        }
        crate::settings::CopyFormat::Tsv => rows
            .iter()
            .map(|row| {
                row.iter()
                    .map(|cell| cell.as_deref().unwrap_or(""))
                    .collect::<Vec<_>>()
                    .join("\t")
            })
            .collect::<Vec<_>>()
            .join("\n"),
        crate::settings::CopyFormat::SqlInserts => {
            let col_list = columns
                .iter()
                .map(|c| quote_ident(c))
                .collect::<Vec<_>>()
                .join(", ");
            let tbl = quote_ident(table_name.unwrap_or("tabla"));
            rows.iter()
                .map(|row| {
                    let vals: Vec<String> = row
                        .iter()
                        .map(|cell| match cell {
                            None => "NULL".to_string(),
                            Some(v) => format!("'{}'", crate::db::escape_sql(v)),
                        })
                        .collect();
                    format!(
                        "INSERT INTO {tbl} ({col_list}) VALUES ({});",
                        vals.join(", ")
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
    }
}

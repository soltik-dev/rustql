use crate::db::Engine;

// ── Rows in the copy format ──────────────────────────────────────────────────

pub fn format_rows_for_copy(
    columns: &[String],
    rows: &[&Vec<Option<String>>],
    fmt: &crate::settings::CopyFormat,
    table_name: Option<&str>,
    engine: Engine,
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
                .map(|c| engine.quote_ident(c))
                .collect::<Vec<_>>()
                .join(", ");
            let tbl = engine.table_ref(table_name.unwrap_or("tabla"));
            rows.iter()
                .map(|row| {
                    let vals: Vec<String> = row
                        .iter()
                        .map(|cell| match cell {
                            None => "NULL".to_string(),
                            Some(v) => engine.literal(v),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::CopyFormat;

    #[test]
    fn los_insert_copiados_van_en_el_dialecto_de_la_conexion() {
        let columns = ["id".to_string(), "v".to_string()];
        let row = vec![Some("1".to_string()), Some("a\\b'c".to_string())];
        let null = vec![Some("2".to_string()), None];
        let copy = |engine, table| {
            format_rows_for_copy(
                &columns,
                &[&row, &null],
                &CopyFormat::SqlInserts,
                table,
                engine,
            )
        };
        assert_eq!(
            copy(Engine::Mysql, Some("t")),
            "INSERT INTO `t` (`id`, `v`) VALUES ('1', 'a\\\\b''c');\n\
             INSERT INTO `t` (`id`, `v`) VALUES ('2', NULL);"
        );
        assert_eq!(
            copy(Engine::Postgres, Some("ventas.t")),
            "INSERT INTO \"ventas\".\"t\" (\"id\", \"v\") VALUES ('1', E'a\\\\b''c');\n\
             INSERT INTO \"ventas\".\"t\" (\"id\", \"v\") VALUES ('2', NULL);"
        );
    }
}

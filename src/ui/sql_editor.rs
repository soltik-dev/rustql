use eframe::egui;

use crate::db::QueryResult;

#[derive(Default, Clone)]
pub struct SqlTab {
    pub db_context: String,
    pub sql_text: String,
    pub result: Option<Result<QueryResult, String>>,
}

pub enum SqlEditorAction {
    None,
    Execute(String),
}

/// Panel "SQL": editor de consultas libre.
pub fn show(
    ui: &mut egui::Ui,
    sql_text: &mut String,
    last_result: &Option<Result<QueryResult, String>>,
    f9_pressed: bool,
    db_context: &mut String,
    databases: &[String],
) -> SqlEditorAction {
    let mut action = SqlEditorAction::None;

    // Barra de controles
    ui.horizontal(|ui| {
        ui.label("BD:");
        egui::ComboBox::from_id_salt("sql_db_context")
            .selected_text(if db_context.is_empty() { "(ninguna)" } else { db_context.as_str() })
            .width(140.0)
            .show_ui(ui, |ui| {
                if ui.selectable_label(db_context.is_empty(), "(ninguna)").clicked() {
                    db_context.clear();
                }
                for db in databases {
                    if ui.selectable_label(db_context == db, db).clicked() {
                        *db_context = db.clone();
                    }
                }
            });

        ui.separator();
        ui.label("SQL libre:");
        if ui.button("▶ Ejecutar  F9").clicked() || f9_pressed {
            let sql = sql_text.trim().to_string();
            if !sql.is_empty() {
                let full_sql = if db_context.is_empty() {
                    sql
                } else {
                    format!("USE `{}`;\n{}", db_context, sql)
                };
                action = SqlEditorAction::Execute(full_sql);
            }
        }
        if ui.button("Limpiar").clicked() {
            sql_text.clear();
        }
    });
    ui.separator();

    // Editor
    let available = ui.available_size();
    let editor_height = (available.y * 0.35).max(80.0).min(300.0);

    ui.add_sized(
        [available.x, editor_height],
        egui::TextEdit::multiline(sql_text)
            .font(egui::TextStyle::Monospace)
            .hint_text("SELECT * FROM tabla LIMIT 100"),
    );

    ui.separator();

    // Resultado
    match last_result {
        None => {
            ui.centered_and_justified(|ui| {
                ui.label("Escribe una consulta y pulsa F9 o el botón Ejecutar");
            });
        }
        Some(Err(e)) => {
            ui.colored_label(egui::Color32::RED, format!("Error: {e}"));
        }
        Some(Ok(data)) => {
            ui.horizontal(|ui| {
                ui.label(format!("{} filas, {} columnas", data.rows.len(), data.columns.len()));
            });
            ui.separator();

            if !data.columns.is_empty() {
                use egui_extras::{Column, TableBuilder};

                let col_count = data.columns.len();
                TableBuilder::new(ui)
                    .striped(true)
                    .resizable(true)
                    .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                    .columns(Column::auto().clip(true).at_least(60.0), col_count)
                    .header(24.0, |mut header| {
                        for col_name in &data.columns {
                            header.col(|ui| {
                                ui.strong(col_name);
                            });
                        }
                    })
                    .body(|body| {
                        body.rows(18.0, data.rows.len(), |mut row| {
                            let row_data = &data.rows[row.index()];
                            for cell in row_data {
                                row.col(|ui| {
                                    ui.add(egui::Label::new(cell).truncate());
                                });
                            }
                        });
                    });
            }
        }
    }

    action
}

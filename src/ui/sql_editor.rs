use std::sync::Arc;

use eframe::egui;

use crate::db::QueryResult;
use crate::ui::{sql_highlight, theme};

#[derive(Default, Clone)]
pub struct SqlTab {
    pub db_context: String,
    pub sql_text: String,
    pub result: Option<Result<QueryResult, String>>,
}

pub enum SqlEditorAction {
    None,
    Execute { sql: String, db_context: String },
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
            .selected_text(if db_context.is_empty() {
                "(ninguna)"
            } else {
                db_context.as_str()
            })
            .width(140.0)
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(db_context.is_empty(), "(ninguna)")
                    .clicked()
                {
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
        if theme::primary_button(ui, format!("{} Ejecutar  F9", crate::ui::icons::CODE)).clicked()
            || f9_pressed
        {
            let sql = sql_text.trim().to_string();
            if !sql.is_empty() {
                action = SqlEditorAction::Execute {
                    sql,
                    db_context: db_context.clone(),
                };
            }
        }
        if ui.button("Limpiar").clicked() {
            sql_text.clear();
        }
    });
    ui.separator();

    // Editor con resaltado de sintaxis en superficie enmarcada
    let p = theme::get(ui);
    let available = ui.available_size();
    let editor_height = (available.y * 0.35).clamp(80.0, 300.0);

    let dark = ui.visuals().dark_mode;
    let font_size = egui::TextStyle::Monospace.resolve(ui.style()).size;
    let mut layouter = move |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap_width: f32| {
        let mut job = sql_highlight::highlight(ui.ctx(), dark, font_size, buf.as_str());
        job.wrap.max_width = wrap_width;
        ui.fonts_mut(|f| f.layout_job(job))
    };
    let layouter: &mut dyn FnMut(&egui::Ui, &dyn egui::TextBuffer, f32) -> Arc<egui::Galley> =
        &mut layouter;

    egui::Frame::NONE
        .fill(p.bg_sunken)
        .stroke(egui::Stroke::new(1.0, p.border_subtle))
        .corner_radius(egui::CornerRadius::same(theme::RADIUS_MD))
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            ui.add_sized(
                [ui.available_width(), editor_height],
                egui::TextEdit::multiline(sql_text)
                    .code_editor()
                    .layouter(layouter)
                    .frame(egui::Frame::NONE)
                    .hint_text("SELECT * FROM tabla LIMIT 100"),
            );
        });

    ui.separator();

    // Resultado
    match last_result {
        None => {
            ui.centered_and_justified(|ui| {
                ui.label(
                    egui::RichText::new("Escribe una consulta y pulsa F9 o el botón Ejecutar")
                        .color(p.text_muted),
                );
            });
        }
        Some(Err(e)) => {
            theme::error_card(ui, &format!("Error: {e}"));
        }
        Some(Ok(data)) => {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!(
                        "{} filas, {} columnas",
                        data.rows.len(),
                        data.columns.len()
                    ))
                    .color(p.text_muted),
                );
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
                                    ui.add(
                                        egui::Label::new(cell.as_deref().unwrap_or("")).truncate(),
                                    );
                                });
                            }
                        });
                    });
            }
        }
    }

    action
}

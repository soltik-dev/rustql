use std::sync::Arc;

use eframe::egui;

use crate::db::QueryResult;
use crate::settings::CopyFormat;
use crate::ui::results_grid::{self, GridAction, GridSelection};
use crate::ui::{CMD, sql_highlight, theme};

/// Una pestaña del editor SQL: la consulta, su contexto y el último
/// resultado.
#[derive(Default)]
pub struct SqlTab {
    pub db_context: String,
    pub sql_text: String,
    pub result: Option<Result<QueryResult, String>>,
    /// Ejecución en curso (su número), si la hay.
    pub running: Option<u64>,
    /// Ejecución que dio `result`: distingue un resultado del siguiente.
    pub result_run: u64,
    /// Lo que tardó la consulta de `result`.
    pub elapsed_ms: Option<u64>,
    /// Cursor y filas seleccionadas en el grid del resultado.
    pub selection: GridSelection,
}

#[derive(Default)]
pub enum SqlEditorAction {
    #[default]
    None,
    Execute {
        sql: String,
        db_context: String,
    },
}

/// Panel "SQL": editor de consultas libre. Mientras la consulta anterior no
/// termina no se puede lanzar otra. `run_pressed`: el atajo de ejecutar
/// (Ctrl+Enter, o F9 como en HeidiSQL). `copy_format`: formato en el que
/// Ctrl+C copia las filas seleccionadas del resultado.
pub fn show(
    ui: &mut egui::Ui,
    tab: &mut SqlTab,
    run_pressed: bool,
    databases: &[String],
    copy_format: &CopyFormat,
) -> SqlEditorAction {
    let mut action = SqlEditorAction::None;
    let running = tab.running.is_some();
    let SqlTab {
        db_context,
        sql_text,
        result,
        result_run,
        elapsed_ms,
        selection,
        ..
    } = tab;

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
        let execute = theme::primary_button_enabled(
            ui,
            !running,
            format!("{} Ejecutar  {CMD}Enter", crate::ui::icons::CODE),
        );
        if !running && (execute.clicked() || run_pressed) {
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
        if running {
            ui.spinner();
            ui.label("Ejecutando…");
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
    match result {
        None => {
            ui.centered_and_justified(|ui| {
                ui.label(
                    egui::RichText::new(format!(
                        "Escribe una consulta y pulsa {CMD}Enter o el botón Ejecutar"
                    ))
                    .color(p.text_muted),
                );
            });
        }
        Some(Err(e)) => {
            theme::error_card(ui, &format!("Error: {e}"));
        }
        Some(Ok(data)) => {
            let took = elapsed_ms.map_or_else(String::new, |ms| format!(" · {ms} ms"));
            if data.columns.is_empty() {
                // INSERT, UPDATE, DDL…: no hay nada que mostrar
                ui.label(
                    egui::RichText::new(format!("Ejecutada, sin resultado que mostrar{took}"))
                        .color(p.text_muted),
                );
                return action;
            }
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!(
                        "{} filas, {} columnas{took}",
                        data.rows.len(),
                        data.columns.len()
                    ))
                    .color(p.text_muted),
                );
                if data.truncated {
                    ui.label(
                        egui::RichText::new(format!(
                            "· solo las primeras {} (hay más; el tope se cambia en Ajustes)",
                            data.rows.len()
                        ))
                        .color(p.warning),
                    );
                }
            });
            ui.separator();

            // El mismo grid que el de las tablas, en solo lectura
            let grid_action = results_grid::show_read_only(ui, *result_run, data, selection);
            if let GridAction::CopyRows(fmt) = grid_action {
                let fmt = fmt.as_ref().unwrap_or(copy_format);
                results_grid::copy_rows(ui.ctx(), data, &selection.rows, fmt, None);
            }
        }
    }

    action
}

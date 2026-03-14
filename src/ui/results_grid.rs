use eframe::egui;

use crate::db::QueryResult;

/// Muestra el grid de datos de una tabla en el panel central.
pub fn show(
    ui: &mut egui::Ui,
    selected_table: &Option<(String, String)>,
    result: &Option<QueryResult>,
    error: &Option<String>,
) {
    match (selected_table, result, error) {
        (None, _, _) => {
            ui.centered_and_justified(|ui| {
                ui.label("Selecciona una tabla en el panel izquierdo");
            });
        }
        (Some((db, table)), _, Some(err)) => {
            ui.label(format!("{db}.{table}"));
            ui.separator();
            ui.colored_label(egui::Color32::RED, format!("Error: {err}"));
        }
        (Some((db, table)), Some(data), None) => {
            ui.horizontal(|ui| {
                ui.strong(format!("{db}.{table}"));
                ui.label(format!("— {} filas", data.rows.len()));
            });
            ui.separator();

            if data.columns.is_empty() {
                ui.label("(tabla vacía)");
                return;
            }

            egui::ScrollArea::both()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    egui::Grid::new("results_grid")
                        .striped(true)
                        .min_col_width(80.0)
                        .spacing([12.0, 4.0])
                        .show(ui, |ui| {
                            // Cabecera
                            for col in &data.columns {
                                ui.strong(col);
                            }
                            ui.end_row();

                            // Filas de datos
                            for row in &data.rows {
                                for cell in row {
                                    ui.label(cell);
                                }
                                ui.end_row();
                            }
                        });
                });
        }
        (Some(_), None, None) => {
            ui.centered_and_justified(|ui| {
                ui.label("Cargando...");
            });
        }
    }
}

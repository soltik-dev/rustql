use std::collections::HashMap;

use eframe::egui;

use crate::db::DbBackend;

/// Muestra el panel izquierdo con el árbol de databases y tablas.
/// Retorna Some((database, tabla)) si el usuario hace click en una tabla.
pub fn show(
    ui: &mut egui::Ui,
    backend: &mut Box<dyn DbBackend>,
    databases: &mut Vec<String>,
    tables_cache: &mut HashMap<String, Vec<String>>,
    selected_table: &Option<(String, String)>,
) -> Option<(String, String)> {
    let mut clicked = None;

    ui.heading("Esquema");
    ui.separator();

    // Cargar la lista de databases la primera vez
    if databases.is_empty() {
        match backend.list_databases() {
            Ok(dbs) => *databases = dbs,
            Err(e) => {
                ui.colored_label(egui::Color32::RED, format!("Error: {e}"));
                return None;
            }
        }
    }

    egui::ScrollArea::vertical().show(ui, |ui| {
        let dbs = databases.clone();

        for db in &dbs {
            egui::CollapsingHeader::new(db)
                .id_salt(db)
                .default_open(false)
                .show(ui, |ui| {
                    // Cargar tablas on-demand al expandir
                    if !tables_cache.contains_key(db) {
                        match backend.list_tables(db) {
                            Ok(tables) => {
                                tables_cache.insert(db.clone(), tables);
                            }
                            Err(e) => {
                                ui.colored_label(egui::Color32::RED, format!("Error: {e}"));
                                return;
                            }
                        }
                    }

                    if let Some(tables) = tables_cache.get(db) {
                        if tables.is_empty() {
                            ui.label("(sin tablas)");
                        } else {
                            for table in tables.clone() {
                                let is_selected = selected_table
                                    .as_ref()
                                    .is_some_and(|(d, t)| d == db && t == &table);

                                if ui
                                    .selectable_label(is_selected, format!("  {table}"))
                                    .clicked()
                                {
                                    clicked = Some((db.clone(), table.clone()));
                                }
                            }
                        }
                    }
                });
        }
    });

    clicked
}

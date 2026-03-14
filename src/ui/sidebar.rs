use std::collections::HashMap;

use eframe::egui;

use crate::db::DbBackend;

fn clip_text(s: &str, max_chars: usize) -> String {
    let mut chars = s.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{truncated}…")
    } else {
        truncated
    }
}

/// Muestra el panel izquierdo con buscador y árbol de databases/tablas.
/// Retorna Some((database, tabla)) si el usuario hace click en una tabla.
pub fn show(
    ui: &mut egui::Ui,
    backend: &mut Box<dyn DbBackend>,
    databases: &mut Vec<String>,
    tables_cache: &mut HashMap<String, Vec<String>>,
    selected_table: &Option<(String, String)>,
    table_search: &mut String,
) -> Option<(String, String)> {
    let mut clicked = None;

    ui.heading("Esquema");

    // Buscador en tiempo real
    ui.add(
        egui::TextEdit::singleline(table_search)
            .hint_text("Buscar tabla...")
            .desired_width(f32::INFINITY),
    );
    ui.separator();

    if databases.is_empty() {
        match backend.list_databases() {
            Ok(dbs) => *databases = dbs,
            Err(e) => {
                ui.colored_label(egui::Color32::RED, format!("Error: {e}"));
                return None;
            }
        }
    }

    let filter = table_search.to_lowercase();
    let filtering = !filter.is_empty();

    egui::ScrollArea::vertical().show(ui, |ui| {
        let dbs = databases.clone();

        for db in &dbs {
            // Cargar tablas si aún no están en caché (necesario para filtrar)
            if filtering && !tables_cache.contains_key(db) {
                if let Ok(tables) = backend.list_tables(db) {
                    tables_cache.insert(db.clone(), tables);
                }
            }

            // Calcular tablas visibles según filtro
            let matching: Vec<String> = if filtering {
                tables_cache
                    .get(db)
                    .map(|ts| {
                        ts.iter()
                            .filter(|t| t.to_lowercase().contains(&filter))
                            .cloned()
                            .collect()
                    })
                    .unwrap_or_default()
            } else {
                vec![] // se cargan lazy al expandir
            };

            // Ocultar bases de datos sin resultados cuando hay filtro activo
            if filtering && matching.is_empty() {
                continue;
            }

            // Con filtro activo forzamos apertura; sin filtro, el usuario controla
            let open = if filtering { Some(true) } else { None };

            let header = egui::CollapsingHeader::new(clip_text(db, 28))
                .id_salt(db)
                .default_open(false);

            let header = header.open(open);

            header.show(ui, |ui| {
                // Sin filtro: carga lazy al expandir
                if !filtering && !tables_cache.contains_key(db) {
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

                let tables: Vec<String> = if filtering {
                    matching.clone()
                } else {
                    tables_cache.get(db).cloned().unwrap_or_default()
                };

                if tables.is_empty() {
                    ui.label(if filtering {
                        "(sin coincidencias)"
                    } else {
                        "(sin tablas)"
                    });
                    return;
                }

                for table in &tables {
                    let is_selected = selected_table
                        .as_ref()
                        .is_some_and(|(d, t)| d == db && t == table);

                    let display = format!("  {}", clip_text(table, 24));
                    let resp = ui.selectable_label(is_selected, display);

                    let resp = if table.len() > 24 {
                        resp.on_hover_text(table)
                    } else {
                        resp
                    };

                    if resp.clicked() {
                        clicked = Some((db.clone(), table.clone()));
                    }
                }
            });
        }
    });

    clicked
}

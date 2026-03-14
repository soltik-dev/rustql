use std::cell::RefCell;
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

#[derive(Debug, Clone)]
pub enum SidebarAction {
    None,
    SelectServer,
    SelectDatabase(String),
    SelectTable { db: String, table: String },
    // Acciones de menú contextual
    CreateDatabase,
    CreateTable { db: String },
    DropDatabase { db: String },
    TruncateAllTables { db: String },
    DropTable { db: String, table: String },
    TruncateTable { db: String, table: String },
    CopyTableName(String),
    Refresh,
    Disconnect,
}

/// Muestra el panel izquierdo con buscador y árbol servidor/databases/tablas.
/// Retorna la acción que el usuario tomó.
pub fn show(
    ui: &mut egui::Ui,
    backend: &mut Box<dyn DbBackend>,
    databases: &mut Vec<String>,
    tables_cache: &mut HashMap<String, Vec<String>>,
    selected_db: Option<&str>,
    selected_table: Option<&str>,
    table_search: &mut String,
    host: &str,
) -> SidebarAction {
    let action: RefCell<SidebarAction> = RefCell::new(SidebarAction::None);

    ui.heading("Esquema");

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
                return SidebarAction::None;
            }
        }
    }

    let filter = table_search.to_lowercase();
    let filtering = !filter.is_empty();

    let server_label = format!("🔌 {}", clip_text(host, 28));

    egui::ScrollArea::vertical().show(ui, |ui| {
        let dbs = databases.clone();

        let server_ch = egui::CollapsingHeader::new(&server_label)
            .id_salt("server_root")
            .default_open(true)
            .show(ui, |ui| {
                for db in &dbs {
                    // Cargar tablas si necesario para filtrar
                    if filtering && !tables_cache.contains_key(db) {
                        if let Ok(tables) = backend.list_tables(db) {
                            tables_cache.insert(db.clone(), tables);
                        }
                    }

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
                        vec![]
                    };

                    if filtering && matching.is_empty() {
                        continue;
                    }

                    let open = if filtering { Some(true) } else { None };

                    let is_db_selected = selected_db == Some(db.as_str());
                    let db_label = clip_text(db, 24);

                    let db_ch = egui::CollapsingHeader::new(
                        egui::RichText::new(&db_label).strong(),
                    )
                    .id_salt(db)
                    .default_open(false)
                    .open(open)
                    .show(ui, |ui: &mut egui::Ui| {
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
                            let is_selected = is_db_selected
                                && selected_table == Some(table.as_str());

                            let display = format!("  {}", clip_text(table, 22));
                            let resp = ui.selectable_label(is_selected, display);

                            let resp = if table.len() > 22 {
                                resp.on_hover_text(table)
                            } else {
                                resp
                            };

                            if resp.clicked() {
                                *action.borrow_mut() = SidebarAction::SelectTable {
                                    db: db.clone(),
                                    table: table.clone(),
                                };
                            }

                            let db_clone = db.clone();
                            let table_clone = table.clone();
                            resp.context_menu(|ui: &mut egui::Ui| {
                                if ui.button("📋 Copiar nombre").clicked() {
                                    *action.borrow_mut() =
                                        SidebarAction::CopyTableName(table_clone.clone());
                                    ui.close_menu();
                                }
                                ui.separator();
                                if ui.button("🗑 Truncar tabla").clicked() {
                                    *action.borrow_mut() = SidebarAction::TruncateTable {
                                        db: db_clone.clone(),
                                        table: table_clone.clone(),
                                    };
                                    ui.close_menu();
                                }
                                if ui
                                    .add(egui::Button::new("✖ Eliminar tabla").fill(
                                        egui::Color32::from_rgb(180, 40, 40),
                                    ))
                                    .clicked()
                                {
                                    *action.borrow_mut() = SidebarAction::DropTable {
                                        db: db_clone.clone(),
                                        table: table_clone.clone(),
                                    };
                                    ui.close_menu();
                                }
                            });
                        }
                    });

                    // Click en la cabecera de BBDD → seleccionar
                    if db_ch.header_response.clicked() {
                        *action.borrow_mut() = SidebarAction::SelectDatabase(db.clone());
                    }

                    let db_clone2 = db.clone();
                    db_ch.header_response.context_menu(|ui: &mut egui::Ui| {
                        if ui.button("➕ Crear tabla").clicked() {
                            *action.borrow_mut() =
                                SidebarAction::CreateTable { db: db_clone2.clone() };
                            ui.close_menu();
                        }
                        ui.separator();
                        if ui.button("🔄 Vaciar todas las tablas").clicked() {
                            *action.borrow_mut() =
                                SidebarAction::TruncateAllTables { db: db_clone2.clone() };
                            ui.close_menu();
                        }
                        if ui
                            .add(egui::Button::new("✖ Eliminar base de datos").fill(
                                egui::Color32::from_rgb(180, 40, 40),
                            ))
                            .clicked()
                        {
                            *action.borrow_mut() =
                                SidebarAction::DropDatabase { db: db_clone2.clone() };
                            ui.close_menu();
                        }
                    });
                }
            });

        // Click en la raíz Servidor → seleccionar
        if server_ch.header_response.clicked() {
            *action.borrow_mut() = SidebarAction::SelectServer;
        }

        server_ch.header_response.context_menu(|ui| {
            if ui.button("➕ Crear base de datos").clicked() {
                *action.borrow_mut() = SidebarAction::CreateDatabase;
                ui.close_menu();
            }
            ui.separator();
            if ui.button("🔄 Refrescar").clicked() {
                *action.borrow_mut() = SidebarAction::Refresh;
                ui.close_menu();
            }
            if ui.button("⏏ Desconectar").clicked() {
                *action.borrow_mut() = SidebarAction::Disconnect;
                ui.close_menu();
            }
        });
    });

    action.into_inner()
}

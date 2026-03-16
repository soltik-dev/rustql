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
    ToggleFav { db: String, table: String },
    PinTable { db: String, table: String },
    OpenTableStructure { db: String, table: String },
    OpenInSql { db: String, table: String },
}

/// Muestra el panel izquierdo con buscador y árbol servidor/databases/tablas.
pub fn show(
    ui: &mut egui::Ui,
    backend: &mut Box<dyn DbBackend>,
    databases: &mut Vec<String>,
    tables_cache: &mut HashMap<String, Vec<String>>,
    selected_db: Option<&str>,
    selected_table: Option<&str>,
    table_search: &mut String,
    fav_tables: &std::collections::HashSet<String>,
    show_only_fav: &mut bool,
    host: &str,
    server_name: &str,
) -> SidebarAction {
    let action: RefCell<SidebarAction> = RefCell::new(SidebarAction::None);

    ui.heading("Esquema");

    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(table_search)
                .hint_text("Buscar tabla...")
                .desired_width(ui.available_width() - 50.0),
        );
        let fav_btn = ui.add(
            egui::Button::new("⭐")
                .selected(*show_only_fav)
                .small(),
        );
        if fav_btn.clicked() {
            *show_only_fav = !*show_only_fav;
        }
        fav_btn.on_hover_text(if *show_only_fav {
            "Mostrando solo favoritos"
        } else {
            "Filtrar favoritos"
        });
        let refresh_btn = ui.add(egui::Button::new("🔄").small());
        if refresh_btn.clicked() {
            *action.borrow_mut() = SidebarAction::Refresh;
        }
        refresh_btn.on_hover_text("Recargar esquema");
    });
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

    let _ = host;
    let server_label = format!("🔌 {}", clip_text(server_name, 28));

    ui.spacing_mut().indent = 10.0;

    egui::ScrollArea::vertical().auto_shrink([false, true]).show(ui, |ui| {
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
        let dbs = databases.clone();

        let server_ch = egui::CollapsingHeader::new(&server_label)
            .id_salt("server_root")
            .default_open(true)
            .show(ui, |ui| {
                for db in &dbs {
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

                    if *show_only_fav {
                        let has_fav = tables_cache.get(db).map_or(false, |ts| {
                            ts.iter().any(|t| fav_tables.contains(&format!("{}/{}", db, t)))
                        });
                        if !has_fav {
                            continue;
                        }
                    }
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

                        let tables: Vec<String> = {
                            let base = if filtering {
                                matching.clone()
                            } else {
                                tables_cache.get(db).cloned().unwrap_or_default()
                            };
                            if *show_only_fav {
                                base.into_iter()
                                    .filter(|t| fav_tables.contains(&format!("{}/{}", db, t)))
                                    .collect()
                            } else {
                                base
                            }
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
                            let is_selected =
                                is_db_selected && selected_table == Some(table.as_str());

                            let display = clip_text(table, 26);
                            let db_clone = db.clone();
                            let table_clone = table.clone();

                            let fav_key = format!("{}/{}", db, table);
                            let is_fav = fav_tables.contains(&fav_key);

                            let fav_color = if ui.visuals().dark_mode {
                                egui::Color32::from_rgb(210, 170, 50)
                            } else {
                                egui::Color32::from_rgb(150, 85, 10)
                            };
                            let label_text = egui::RichText::new(&display).color(
                                if is_fav { fav_color } else { ui.visuals().text_color() },
                            );

                            let resp = ui.selectable_label(is_selected, label_text);

                            if resp.clicked() || resp.double_clicked() {
                                *action.borrow_mut() = SidebarAction::SelectTable {
                                    db: db_clone.clone(),
                                    table: table_clone.clone(),
                                };
                            }

                            let resp = if table.len() > 22 {
                                resp.on_hover_text(table)
                            } else {
                                resp
                            };

                            resp.context_menu(|ui: &mut egui::Ui| {
                                if ui.button("⭐ Marcar favorito").clicked() {
                                    *action.borrow_mut() = SidebarAction::ToggleFav {
                                        db: db_clone.clone(),
                                        table: table_clone.clone(),
                                    };
                                    ui.close();
                                }
                                if ui.button("📌 Fijar tabla").clicked() {
                                    *action.borrow_mut() = SidebarAction::PinTable {
                                        db: db_clone.clone(),
                                        table: table_clone.clone(),
                                    };
                                    ui.close();
                                }
                                if ui.button("✏ Ver estructura").clicked() {
                                    *action.borrow_mut() = SidebarAction::OpenTableStructure {
                                        db: db_clone.clone(),
                                        table: table_clone.clone(),
                                    };
                                    ui.close();
                                }
                                if ui.button("▶ Abrir en SQL").clicked() {
                                    *action.borrow_mut() = SidebarAction::OpenInSql {
                                        db: db_clone.clone(),
                                        table: table_clone.clone(),
                                    };
                                    ui.close();
                                }
                                if ui.button("📋 Copiar nombre").clicked() {
                                    *action.borrow_mut() =
                                        SidebarAction::CopyTableName(table_clone.clone());
                                    ui.close();
                                }
                                ui.separator();
                                if ui.button("🗑 Truncar tabla").clicked() {
                                    *action.borrow_mut() = SidebarAction::TruncateTable {
                                        db: db_clone.clone(),
                                        table: table_clone.clone(),
                                    };
                                    ui.close();
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
                                    ui.close();
                                }
                            });
                        }
                    });

                    if db_ch.header_response.clicked() {
                        *action.borrow_mut() = SidebarAction::SelectDatabase(db.clone());
                    }

                    let db_clone2 = db.clone();
                    db_ch.header_response.context_menu(|ui: &mut egui::Ui| {
                        if ui.button("➕ Crear tabla").clicked() {
                            *action.borrow_mut() =
                                SidebarAction::CreateTable { db: db_clone2.clone() };
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("🔄 Vaciar todas las tablas").clicked() {
                            *action.borrow_mut() =
                                SidebarAction::TruncateAllTables { db: db_clone2.clone() };
                            ui.close();
                        }
                        if ui
                            .add(egui::Button::new("✖ Eliminar base de datos").fill(
                                egui::Color32::from_rgb(180, 40, 40),
                            ))
                            .clicked()
                        {
                            *action.borrow_mut() =
                                SidebarAction::DropDatabase { db: db_clone2.clone() };
                            ui.close();
                        }
                    });
                }
            });

        if server_ch.header_response.clicked() {
            *action.borrow_mut() = SidebarAction::SelectServer;
        }

        server_ch.header_response.context_menu(|ui| {
            if ui.button("➕ Crear base de datos").clicked() {
                *action.borrow_mut() = SidebarAction::CreateDatabase;
                ui.close();
            }
            ui.separator();
            if ui.button("🔄 Refrescar").clicked() {
                *action.borrow_mut() = SidebarAction::Refresh;
                ui.close();
            }
            if ui.button("⏏ Desconectar").clicked() {
                *action.borrow_mut() = SidebarAction::Disconnect;
                ui.close();
            }
        });
    });

    action.into_inner()
}

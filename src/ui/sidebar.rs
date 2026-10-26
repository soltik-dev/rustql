use std::cell::RefCell;
use std::collections::HashMap;

use eframe::egui;

use crate::db::DbBackend;
use crate::ui::icons::*;
use crate::ui::theme;

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
    OpenTableStructure { db: String, table: String },
    OpenInSql { db: String, table: String },
    CloneTable { db: String, table: String },
    ExportDatabase(String),
    ExportTable { db: String, table: String },
}

/// Muestra el panel izquierdo con buscador y árbol servidor/databases/tablas.
#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut egui::Ui,
    backend: Option<&dyn DbBackend>,
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
    let p = theme::get(ui);

    ui.add_space(4.0);
    theme::section_label(ui, "Esquema");
    ui.add_space(2.0);

    let ctrl_f = ui.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::F));

    ui.horizontal(|ui| {
        let fav_btn = ui.add(
            egui::Button::new(STAR_FILLED)
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
        let refresh_btn = ui.add(egui::Button::new(REFRESH).small());
        if refresh_btn.clicked() {
            *action.borrow_mut() = SidebarAction::Refresh;
        }
        refresh_btn.on_hover_text("Recargar esquema");

        // Buscador tipo pill
        egui::Frame::NONE
            .fill(p.bg_surface)
            .stroke(egui::Stroke::new(1.0, p.border_subtle))
            .corner_radius(egui::CornerRadius::same(theme::RADIUS_LG))
            .inner_margin(egui::Margin::symmetric(8, 3))
            .show(ui, |ui| {
                let search_resp = ui.add(
                    egui::TextEdit::singleline(table_search)
                        .id_salt("sidebar_table_search")
                        .hint_text("Buscar…  Ctrl+F")
                        .frame(egui::Frame::NONE)
                        .desired_width(f32::INFINITY),
                );
                if ctrl_f {
                    search_resp.request_focus();
                }
            });
    });
    ui.separator();

    if databases.is_empty()
        && let Some(b) = backend
    {
        match b.list_databases() {
            Ok(dbs) => *databases = dbs,
            Err(e) => {
                ui.colored_label(p.danger, format!("Error: {e}"));
                return SidebarAction::None;
            }
        }
    }

    let filter = table_search.to_lowercase();
    let filtering = !filter.is_empty();

    let _ = host;
    let server_label = format!("{} {}", PLUG, clip_text(server_name, 28));

    ui.spacing_mut().indent = 10.0;

    egui::ScrollArea::vertical()
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
            let dbs = databases.clone();

            let server_ch =
                egui::CollapsingHeader::new(&server_label)
                    .id_salt("server_root")
                    .default_open(true)
                    .show(ui, |ui| {
                        for db in &dbs {
                            if filtering
                                && !tables_cache.contains_key(db)
                                && let Some(b) = backend
                                && let Ok(tables) = b.list_tables(db)
                            {
                                tables_cache.insert(db.clone(), tables);
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
                                let has_fav = tables_cache.get(db).is_some_and(|ts| {
                                    ts.iter()
                                        .any(|t| fav_tables.contains(&format!("{}/{}", db, t)))
                                });
                                if !has_fav {
                                    continue;
                                }
                            }
                            if filtering && matching.is_empty() {
                                continue;
                            }

                            let open: Option<bool> = None;
                            let is_db_selected = selected_db == Some(db.as_str());
                            let db_label = clip_text(db, 24);

                            let db_ch = egui::CollapsingHeader::new(
                                egui::RichText::new(&db_label).strong(),
                            )
                            .id_salt(db)
                            .default_open(false)
                            .open(open)
                            .show(ui, |ui: &mut egui::Ui| {
                                if !filtering
                                    && !tables_cache.contains_key(db)
                                    && let Some(b) = backend
                                {
                                    match b.list_tables(db) {
                                        Ok(tables) => {
                                            tables_cache.insert(db.clone(), tables);
                                        }
                                        Err(e) => {
                                            ui.colored_label(p.danger, format!("Error: {e}"));
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
                                            .filter(|t| {
                                                fav_tables.contains(&format!("{}/{}", db, t))
                                            })
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

                                    let fav_color = p.fav_star;

                                    // Fila completa: iconos a la izquierda, texto a continuación
                                    let available_w = ui.available_width();
                                    let row_height = ui.spacing().interact_size.y;
                                    let icon_w = 18.0;
                                    let (row_rect, _) = ui.allocate_exact_size(
                                        egui::vec2(available_w, row_height),
                                        egui::Sense::hover(),
                                    );

                                    let fav_rect = egui::Rect::from_min_size(
                                        row_rect.min,
                                        egui::vec2(icon_w, row_height),
                                    );
                                    let label_rect = egui::Rect::from_min_max(
                                        egui::pos2(row_rect.min.x + icon_w, row_rect.min.y),
                                        row_rect.max,
                                    );

                                    let fav_resp = ui.interact(
                                        fav_rect,
                                        ui.scope_id().with(("fav", db.as_str(), table.as_str())),
                                        egui::Sense::click(),
                                    );
                                    let resp = ui.interact(
                                        label_rect,
                                        ui.scope_id().with(("row", db.as_str(), table.as_str())),
                                        egui::Sense::click(),
                                    );

                                    // Fondo hover/selected
                                    let is_hovered = resp.hovered() || fav_resp.hovered();
                                    let bg = if is_selected {
                                        p.grid_sel_row_bg
                                    } else if is_hovered {
                                        p.bg_surface
                                    } else {
                                        egui::Color32::TRANSPARENT
                                    };
                                    ui.painter().rect_filled(
                                        row_rect,
                                        f32::from(theme::RADIUS_SM),
                                        bg,
                                    );

                                    // Icono fav: STAR_FILLED o STAR
                                    let star = if is_fav { STAR_FILLED } else { STAR };
                                    let star_color = if is_fav {
                                        fav_color
                                    } else if fav_resp.hovered() {
                                        ui.visuals().strong_text_color()
                                    } else {
                                        ui.visuals().weak_text_color()
                                    };
                                    ui.painter().text(
                                        fav_rect.center(),
                                        egui::Align2::CENTER_CENTER,
                                        star,
                                        egui::TextStyle::Body.resolve(ui.style()),
                                        star_color,
                                    );

                                    // Texto de la tabla
                                    let text_color = if is_selected {
                                        p.text_primary
                                    } else if is_fav {
                                        fav_color
                                    } else {
                                        ui.visuals().text_color()
                                    };
                                    ui.painter().text(
                                        label_rect.left_center() + egui::vec2(2.0, 0.0),
                                        egui::Align2::LEFT_CENTER,
                                        &display,
                                        egui::TextStyle::Body.resolve(ui.style()),
                                        text_color,
                                    );

                                    if fav_resp.clicked() {
                                        *action.borrow_mut() = SidebarAction::ToggleFav {
                                            db: db_clone.clone(),
                                            table: table_clone.clone(),
                                        };
                                    }
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
                                        if ui
                                            .button(format!("{STAR_FILLED} Marcar favorito"))
                                            .clicked()
                                        {
                                            *action.borrow_mut() = SidebarAction::ToggleFav {
                                                db: db_clone.clone(),
                                                table: table_clone.clone(),
                                            };
                                            ui.close();
                                        }
                                        if ui.button(format!("{EDIT} Ver estructura")).clicked() {
                                            *action.borrow_mut() =
                                                SidebarAction::OpenTableStructure {
                                                    db: db_clone.clone(),
                                                    table: table_clone.clone(),
                                                };
                                            ui.close();
                                        }
                                        if ui.button(format!("{CODE} Abrir en SQL")).clicked() {
                                            *action.borrow_mut() = SidebarAction::OpenInSql {
                                                db: db_clone.clone(),
                                                table: table_clone.clone(),
                                            };
                                            ui.close();
                                        }
                                        if ui.button(format!("{COPY} Copiar nombre")).clicked() {
                                            *action.borrow_mut() =
                                                SidebarAction::CopyTableName(table_clone.clone());
                                            ui.close();
                                        }
                                        if ui.button(format!("{CLONE} Clonar tabla")).clicked() {
                                            *action.borrow_mut() = SidebarAction::CloneTable {
                                                db: db_clone.clone(),
                                                table: table_clone.clone(),
                                            };
                                            ui.close();
                                        }
                                        if ui.button(format!("{EXPORT} Exportar tabla")).clicked() {
                                            *action.borrow_mut() = SidebarAction::ExportTable {
                                                db: db_clone.clone(),
                                                table: table_clone.clone(),
                                            };
                                            ui.close();
                                        }
                                        ui.separator();
                                        if ui.button(format!("{TRASH} Truncar tabla")).clicked() {
                                            *action.borrow_mut() = SidebarAction::TruncateTable {
                                                db: db_clone.clone(),
                                                table: table_clone.clone(),
                                            };
                                            ui.close();
                                        }
                                        if theme::danger_button(
                                            ui,
                                            format!("{TRASH} Eliminar tabla"),
                                        )
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
                                if ui.button(format!("{PLUS} Crear tabla")).clicked() {
                                    *action.borrow_mut() = SidebarAction::CreateTable {
                                        db: db_clone2.clone(),
                                    };
                                    ui.close();
                                }
                                if ui
                                    .button(format!("{EXPORT} Exportar base de datos"))
                                    .clicked()
                                {
                                    *action.borrow_mut() =
                                        SidebarAction::ExportDatabase(db_clone2.clone());
                                    ui.close();
                                }
                                ui.separator();
                                if ui
                                    .button(format!("{TRASH} Vaciar todas las tablas"))
                                    .clicked()
                                {
                                    *action.borrow_mut() = SidebarAction::TruncateAllTables {
                                        db: db_clone2.clone(),
                                    };
                                    ui.close();
                                }
                                if theme::danger_button(
                                    ui,
                                    format!("{TRASH} Eliminar base de datos"),
                                )
                                .clicked()
                                {
                                    *action.borrow_mut() = SidebarAction::DropDatabase {
                                        db: db_clone2.clone(),
                                    };
                                    ui.close();
                                }
                            });
                        }
                    });

            if server_ch.header_response.clicked() {
                *action.borrow_mut() = SidebarAction::SelectServer;
            }

            server_ch.header_response.context_menu(|ui| {
                if ui.button(format!("{PLUS} Crear base de datos")).clicked() {
                    *action.borrow_mut() = SidebarAction::CreateDatabase;
                    ui.close();
                }
                ui.separator();
                if ui.button(format!("{REFRESH} Refrescar")).clicked() {
                    *action.borrow_mut() = SidebarAction::Refresh;
                    ui.close();
                }
                if ui.button(format!("{PLUG} Desconectar")).clicked() {
                    *action.borrow_mut() = SidebarAction::Disconnect;
                    ui.close();
                }
            });
        });

    action.into_inner()
}

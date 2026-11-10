use std::cell::RefCell;
use std::collections::HashMap;

use eframe::egui;

use crate::loadable::Loadable;
use crate::table_state::table_key;
use crate::ui::icons::*;
use crate::ui::{CMD, ellipsize, theme};

/// Si `db` tiene alguna tabla favorita. Los favoritos son claves «bd/tabla»:
/// no hace falta haber cargado las tablas de la BBDD para saberlo.
fn has_favorite(fav_tables: &std::collections::HashSet<String>, db: &str) -> bool {
    fav_tables.iter().any(|key| {
        key.strip_prefix(db)
            .is_some_and(|rest| rest.starts_with('/'))
    })
}

#[derive(Debug, Clone, Default)]
pub enum SidebarAction {
    #[default]
    None,
    SelectServer,
    SelectDatabase(String),
    SelectTable {
        db: String,
        table: String,
    },
    CreateDatabase,
    CreateTable {
        db: String,
    },
    DropDatabase {
        db: String,
    },
    TruncateAllTables {
        db: String,
    },
    DropTable {
        db: String,
        table: String,
    },
    TruncateTable {
        db: String,
        table: String,
    },
    CopyTableName(String),
    Refresh,
    Disconnect,
    ToggleFav {
        db: String,
        table: String,
    },
    OpenTableStructure {
        db: String,
        table: String,
    },
    OpenInSql {
        db: String,
        table: String,
    },
    CloneTable {
        db: String,
        table: String,
    },
    ExportDatabase(String),
    ExportTable {
        db: String,
        table: String,
    },
}

/// Lo que el sidebar pide a la sesión: la acción del usuario y los datos
/// que le faltan para dibujar el árbol (los carga la sesión en segundo plano).
#[derive(Default)]
pub struct SidebarOutput {
    pub action: SidebarAction,
    /// BBDD desplegadas cuyas tablas aún no se han pedido.
    pub load_tables: Vec<String>,
    /// El buscador necesita las tablas de todas las BBDD.
    pub load_all_tables: bool,
}

fn loading_label(ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.spinner();
        ui.label("Cargando…");
    });
}

/// Muestra el panel izquierdo con buscador y árbol servidor/databases/tablas.
/// `conn_color` (color de la conexión) tiñe la raíz del árbol y la selección.
#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut egui::Ui,
    databases: &Loadable<Vec<String>>,
    tables_cache: &HashMap<String, Loadable<Vec<String>>>,
    selected_db: Option<&str>,
    selected_table: Option<&str>,
    table_search: &mut String,
    fav_tables: &std::collections::HashSet<String>,
    show_only_fav: &mut bool,
    server_name: &str,
    conn_color: Option<egui::Color32>,
) -> SidebarOutput {
    let action: RefCell<SidebarAction> = RefCell::new(SidebarAction::None);
    let mut load_tables = Vec::new();
    let mut load_all_tables = false;
    let p = theme::get(ui);

    ui.add_space(4.0);
    theme::section_label(ui, "Esquema");
    ui.add_space(2.0);

    let ctrl_f = ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::F));

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
                        .hint_text(format!("Buscar…  {CMD}F"))
                        .frame(egui::Frame::NONE)
                        .desired_width(f32::INFINITY),
                );
                if ctrl_f {
                    search_resp.request_focus();
                }
            });
    });
    ui.separator();

    let dbs = match databases {
        Loadable::Ready(dbs) => dbs,
        other => {
            if let Loadable::Failed(e) = other {
                ui.colored_label(p.danger, format!("Error: {e}"));
            } else {
                loading_label(ui);
            }
            return SidebarOutput {
                action: action.into_inner(),
                load_tables,
                load_all_tables,
            };
        }
    };

    let filter = table_search.to_lowercase();
    let filtering = !filter.is_empty();

    let server_label = format!("{} {}", PLUG, ellipsize(server_name, 28));

    ui.spacing_mut().indent = 10.0;

    egui::ScrollArea::vertical()
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);

            // Hueco para el fondo de la raíz: se rellena al conocer su rect.
            let server_bg = ui.painter().add(egui::Shape::Noop);
            let server_ch =
                egui::CollapsingHeader::new(&server_label)
                    .id_salt("server_root")
                    .default_open(true)
                    .show(ui, |ui| {
                        for db in dbs {
                            let db_tables = tables_cache.get(db);
                            if filtering && db_tables.is_none_or(Loadable::needs_load) {
                                load_all_tables = true;
                            }

                            let matching: Vec<String> = if filtering {
                                db_tables
                                    .and_then(Loadable::ready)
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

                            if *show_only_fav && !has_favorite(fav_tables, db) {
                                continue;
                            }
                            if filtering && matching.is_empty() {
                                continue;
                            }

                            let is_db_selected = selected_db == Some(db.as_str());
                            let db_label = ellipsize(db, 24);

                            let db_ch = egui::CollapsingHeader::new(
                                egui::RichText::new(&db_label).strong(),
                            )
                            // Al buscar, las BBDD con coincidencias salen
                            // desplegadas: lo que se busca son tablas. Con
                            // otro id, para que al borrar la búsqueda el
                            // árbol vuelva a estar como estaba.
                            .id_salt((db, filtering))
                            .default_open(filtering)
                            .show(ui, |ui: &mut egui::Ui| {
                                let tables: Vec<String> = {
                                    let base = if filtering {
                                        matching.clone()
                                    } else {
                                        match db_tables {
                                            Some(Loadable::Ready(ts)) => ts.clone(),
                                            Some(Loadable::Failed(e)) => {
                                                ui.colored_label(p.danger, format!("Error: {e}"));
                                                return;
                                            }
                                            Some(Loadable::Loading(_)) => {
                                                loading_label(ui);
                                                return;
                                            }
                                            None | Some(Loadable::NotLoaded) => {
                                                load_tables.push(db.clone());
                                                loading_label(ui);
                                                return;
                                            }
                                        }
                                    };
                                    if *show_only_fav {
                                        base.into_iter()
                                            .filter(|t| fav_tables.contains(&table_key(db, t)))
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

                                    let db_clone = db.clone();
                                    let table_clone = table.clone();

                                    let fav_key = table_key(db, table);
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
                                        conn_color
                                            .map_or(p.grid_sel_row_bg, |c| c.gamma_multiply(0.3))
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
                                    // Recortado al ancho del panel, no a un
                                    // número fijo de caracteres
                                    let mut job = egui::text::LayoutJob::simple_singleline(
                                        table.clone(),
                                        egui::TextStyle::Body.resolve(ui.style()),
                                        text_color,
                                    );
                                    job.wrap = egui::text::TextWrapping::truncate_at_width(
                                        label_rect.width() - 4.0,
                                    );
                                    let galley = ui.painter().layout_job(job);
                                    let name_cut = galley.elided;
                                    ui.painter().galley(
                                        egui::pos2(
                                            label_rect.left() + 2.0,
                                            label_rect.center().y - galley.size().y / 2.0,
                                        ),
                                        galley,
                                        text_color,
                                    );

                                    if fav_resp.clicked() {
                                        *action.borrow_mut() = SidebarAction::ToggleFav {
                                            db: db_clone.clone(),
                                            table: table_clone.clone(),
                                        };
                                    }
                                    if resp.clicked() {
                                        *action.borrow_mut() = SidebarAction::SelectTable {
                                            db: db_clone.clone(),
                                            table: table_clone.clone(),
                                        };
                                    }

                                    // Nombre completo si no cabe
                                    let resp = if name_cut {
                                        resp.on_hover_text(table)
                                    } else {
                                        resp
                                    };

                                    resp.context_menu(|ui: &mut egui::Ui| {
                                        let fav_label = if is_fav {
                                            format!("{STAR} Quitar de favoritos")
                                        } else {
                                            format!("{STAR_FILLED} Marcar favorito")
                                        };
                                        if ui.button(fav_label).clicked() {
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
                                        if ui.button(format!("{TRASH} Vaciar tabla")).clicked() {
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

            if let Some(c) = conn_color {
                let rect = egui::Rect::from_x_y_ranges(
                    ui.max_rect().x_range(),
                    server_ch.header_response.rect.y_range(),
                );
                ui.painter().set(
                    server_bg,
                    egui::epaint::RectShape::filled(rect, theme::RADIUS_SM, c.gamma_multiply(0.22)),
                );
            }

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

    SidebarOutput {
        action: action.into_inner(),
        load_tables,
        load_all_tables,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn favoritos_sin_cargar_las_tablas() {
        let favs = ["tienda/clientes".to_string()].into_iter().collect();
        assert!(has_favorite(&favs, "tienda"));
        assert!(!has_favorite(&favs, "tien"), "no basta con el prefijo");
        assert!(!has_favorite(&favs, "otra"));
    }
}

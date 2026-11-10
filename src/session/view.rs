//! Dibujo de la sesión según su estado. Aquí solo se recoge lo que hace el
//! usuario (`FrameSignals`); `Session::show` lo aplica después.

use eframe::egui;

use super::schema::show_loading;
use super::{ConnState, FrameKeys, SelectedNode, Session, Shared};
use crate::loadable::Loadable;
use crate::ui::connection_panel::SaveProfile;
use crate::ui::db_info::DbInfoAction;
use crate::ui::filter_bar::FilterAction;
use crate::ui::results_grid::{ApplyBarAction, GridAction};
use crate::ui::sidebar::SidebarOutput;
use crate::ui::sql_editor::SqlEditorAction;
use crate::ui::tab_bar::{ActiveTab, TabBarResult};
use crate::ui::{
    connection_panel, db_info, filter_bar, results_grid, server_info, sidebar, sql_editor, tab_bar,
    table_structure, theme,
};

/// Lo que ha hecho el usuario en este frame.
#[derive(Default)]
pub(super) struct FrameSignals {
    // Formulario de conexión
    pub connect: bool,
    pub cancel_connect: bool,
    pub save_profile: Option<SaveProfile>,
    pub delete_profile: bool,
    pub profile_selected: Option<usize>,
    // Conectado
    pub sidebar: SidebarOutput,
    pub tab_bar: TabBarResult,
    pub filter: FilterAction,
    pub grid: GridAction,
    pub apply_bar: ApplyBarAction,
    pub db_info: DbInfoAction,
    pub sql: SqlEditorAction,
}

impl FrameSignals {
    /// Solo lo que pide datos para dibujar (las tablas del árbol): lo demás
    /// es una acción del usuario.
    pub fn data_requests_only(self) -> Self {
        Self {
            sidebar: SidebarOutput {
                load_tables: self.sidebar.load_tables,
                load_all_tables: self.sidebar.load_all_tables,
                ..Default::default()
            },
            ..Default::default()
        }
    }
}

impl Session {
    pub(super) fn draw(
        &mut self,
        ui: &mut egui::Ui,
        sh: &Shared,
        keys: FrameKeys,
        show_sidebar: bool,
    ) -> FrameSignals {
        let mut signals = FrameSignals::default();
        match &self.state {
            ConnState::Disconnected => self.draw_form(ui, sh, &mut signals),
            ConnState::Connecting { .. } => self.draw_connecting(ui, sh, &mut signals),
            ConnState::Connected(_) => {
                self.draw_connected(ui, sh, keys, show_sidebar, &mut signals)
            }
        }
        signals
    }

    /// Formulario de conexión con los favoritos a la izquierda.
    fn draw_form(&mut self, ui: &mut egui::Ui, sh: &Shared, signals: &mut FrameSignals) {
        // El contenido va bajo push_id(sid) para que el estado de los widgets
        // (árbol expandido, scroll, cursores) sea propio de cada sesión.
        let sid = self.id;
        egui::Panel::left("conn_favorites")
            .exact_size(200.0)
            .show(ui, |ui| {
                ui.push_id(sid, |ui| {
                    let (sel, connect_now) = connection_panel::show_favorites(
                        ui,
                        &sh.profiles,
                        &self.selected_profile_idx,
                    );
                    signals.profile_selected = sel;
                    if connect_now {
                        signals.connect = true;
                    }
                });
            });

        egui::CentralPanel::default().show(ui, |ui| {
            ui.push_id(sid, |ui| {
                let result = connection_panel::show_form(
                    ui,
                    &mut self.params,
                    &mut self.port_str,
                    &self.error_message,
                    &mut self.profile_name,
                    &mut self.save_password,
                    &mut self.color,
                    &self.selected_profile_idx,
                );
                signals.connect |= result.connect;
                signals.save_profile = result.save_profile;
                signals.delete_profile = result.delete_profile;
            });
        });
    }

    fn draw_connecting(&self, ui: &mut egui::Ui, sh: &Shared, signals: &mut FrameSignals) {
        let ConnState::Connecting {
            host,
            status,
            start,
            ..
        } = &self.state
        else {
            return;
        };
        let sid = self.id;
        let elapsed = start.elapsed().as_secs();
        egui::Panel::left("conn_favorites")
            .exact_size(200.0)
            .show(ui, |ui| {
                ui.push_id(sid, |ui| {
                    let _ = connection_panel::show_favorites(
                        ui,
                        &sh.profiles,
                        &self.selected_profile_idx,
                    );
                });
            });

        egui::CentralPanel::default().show(ui, |ui| {
            ui.add_space(ui.available_height() / 3.0);
            ui.vertical_centered(|ui| {
                ui.spinner();
                ui.add_space(10.0);
                ui.strong(format!("Conectando a {}...", host));
                ui.add_space(6.0);
                ui.label(status.as_str());
                ui.add_space(4.0);
                ui.label(format!("{}s", elapsed));
                ui.add_space(16.0);
                if ui.button("Cancelar").clicked() {
                    signals.cancel_connect = true;
                }
            });
        });
    }

    fn draw_connected(
        &mut self,
        ui: &mut egui::Ui,
        sh: &Shared,
        keys: FrameKeys,
        show_sidebar: bool,
        signals: &mut FrameSignals,
    ) {
        let sid = self.id;
        let conn_color = self.color.map(|c| theme::get(ui).conn(c));
        let FrameKeys { f9, ctrl_enter, .. } = keys;
        // Mientras carga la tabla, el grid se muestra sin aplicar sus
        // acciones: irían por posición sobre los datos anteriores.
        let running = self.table_view.query_loading || self.table_view.saving;

        // ── Sidebar ───────────────────────────────────────────────
        if show_sidebar {
            let (sel_db, sel_table) = match &self.schema.selected_node {
                Some(SelectedNode::Table { db, table }) => {
                    (Some(db.as_str()), Some(table.as_str()))
                }
                Some(SelectedNode::Database(db)) => (Some(db.as_str()), None),
                _ => (None, None),
            };

            let sidebar_panel = egui::Panel::left("sidebar_v2")
                .default_size(200.0)
                .min_size(160.0)
                .show(ui, |ui| {
                    ui.push_id(sid, |ui| {
                        sidebar::show(
                            ui,
                            &self.schema.databases,
                            &self.schema.tables_cache,
                            sel_db,
                            sel_table,
                            &mut self.schema.table_search,
                            &sh.settings.fav_tables,
                            &mut self.show_only_fav,
                            &self.title,
                            conn_color,
                        )
                    })
                    .inner
                });
            signals.sidebar = sidebar_panel.inner;

            // Borde del árbol con el color de la conexión. Sube sobre la
            // línea de las pestañas para no duplicar su grosor.
            if let Some(c) = conn_color {
                let r = sidebar_panel.response.rect;
                ui.painter().rect_stroke(
                    r.with_min_y(r.top() - theme::CONN_FRAME_WIDTH),
                    0.0,
                    egui::Stroke::new(theme::CONN_FRAME_WIDTH, c),
                    egui::StrokeKind::Inside,
                );
            }
        }

        // ── Tab bar ───────────────────────────────────────────────
        let has_db = matches!(
            &self.schema.selected_node,
            Some(SelectedNode::Database(_) | SelectedNode::Table { .. })
        );
        let has_table = matches!(&self.schema.selected_node, Some(SelectedNode::Table { .. }));

        {
            let db_name_tab = match &self.schema.selected_node {
                Some(SelectedNode::Database(d)) => Some(d.clone()),
                Some(SelectedNode::Table { db, .. }) => Some(db.clone()),
                _ => None,
            };
            let table_name_tab = match &self.schema.selected_node {
                Some(SelectedNode::Table { table, .. }) => Some(table.clone()),
                _ => None,
            };

            egui::Panel::top("tab_bar")
                .min_size(30.0)
                .resizable(false)
                .show(ui, |ui| {
                    ui.push_id(sid, |ui| {
                        ui.add_space(2.0);
                        signals.tab_bar = tab_bar::show(
                            ui,
                            self.active_tab,
                            has_db,
                            has_table,
                            &self.title,
                            db_name_tab.as_deref(),
                            table_name_tab.as_deref(),
                            self.sql_tabs.len(),
                        );
                    });
                });
        }

        // ── Apply bar (solo en tab Datos con cambios pendientes) ──
        let has_pending = !self.table_view.pending_edits.is_empty()
            || !self.table_view.pending_deletes.is_empty()
            || !self.table_view.pending_inserts.is_empty();
        if !running && self.active_tab == ActiveTab::Datos && has_pending {
            // pk_missing solo bloquea si hay edits pendientes (DELETE e INSERT no requieren PK en la UI)
            let pk_missing = self.schema.open_table_columns().is_some()
                && self.table_view.primary_keys.is_empty()
                && !self.table_view.pending_edits.is_empty();
            let apply_frame =
                egui::Frame::side_top_panel(ui.style()).fill(theme::get(ui).bg_raised);
            egui::Panel::bottom("apply_bar")
                .exact_size(36.0)
                .frame(apply_frame)
                .show(ui, |ui| {
                    ui.push_id(sid, |ui| {
                        signals.apply_bar = results_grid::show_apply_bar(
                            ui,
                            &self.table_view.pending_edits,
                            self.table_view.pending_deletes.len(),
                            self.table_view.pending_inserts.len(),
                            ctrl_enter,
                            pk_missing,
                        );
                    });
                });
        }

        // ── Panel central ─────────────────────────────────────────
        egui::CentralPanel::default().show(ui, |ui| {
            ui.push_id(sid, |ui| match self.active_tab {
                ActiveTab::Datos => {
                    let selected_table = match &self.schema.selected_node {
                        Some(SelectedNode::Table { db, table }) => {
                            Some((db.as_str(), table.as_str()))
                        }
                        _ => None,
                    };
                    if selected_table.is_some() {
                        let action = filter_bar::show(
                            ui,
                            &mut self.table_view.filter_text,
                            f9 && !running,
                            &self.table_view.filter_history,
                        );
                        if !running {
                            signals.filter = action;
                        }
                        ui.separator();
                    }
                    let col_info_slice = self
                        .schema
                        .open_table_columns()
                        .map_or(&[][..], Vec::as_slice);
                    let action = results_grid::show(
                        ui,
                        selected_table,
                        &mut self.table_view,
                        col_info_slice,
                        // Con query en curso: spinner en el grid
                        running,
                        sh.settings.query_limit,
                    );
                    if !running {
                        signals.grid = action;
                    }
                }

                ActiveTab::Servidor => match &self.schema.databases_info {
                    Loadable::Ready(dbs) => server_info::show(ui, &self.params.host, dbs),
                    other => show_loading(ui, other),
                },

                ActiveTab::Bbdd => {
                    let db_name = match &self.schema.selected_node {
                        Some(SelectedNode::Database(d)) => Some(d.clone()),
                        Some(SelectedNode::Table { db, .. }) => Some(db.clone()),
                        _ => None,
                    };
                    if let Some(db) = &db_name {
                        match self.schema.tables_info.get(db) {
                            Some(Loadable::Ready(tables)) => {
                                signals.db_info = db_info::show(ui, db, tables);
                            }
                            other => show_loading(ui, other.unwrap_or(&Loadable::NotLoaded)),
                        }
                    } else {
                        ui.centered_and_justified(|ui| {
                            ui.label("Selecciona una base de datos en el panel izquierdo");
                        });
                    }
                }

                ActiveTab::Tabla => {
                    if let Some(SelectedNode::Table { db, table }) = &self.schema.selected_node {
                        let key = (db.clone(), table.clone());
                        match self.schema.columns_info.get(&key) {
                            Some(Loadable::Ready(cols)) => {
                                table_structure::show(ui, db, table, cols);
                            }
                            other => show_loading(ui, other.unwrap_or(&Loadable::NotLoaded)),
                        }
                    } else {
                        ui.centered_and_justified(|ui| {
                            ui.label("Selecciona una tabla en el panel izquierdo");
                        });
                    }
                }

                ActiveTab::Sql(tab_idx) => {
                    if let Some(tab) = self.sql_tabs.get_mut(tab_idx) {
                        let databases =
                            self.schema.databases.ready().map_or(&[][..], Vec::as_slice);
                        signals.sql = sql_editor::show(
                            ui,
                            &mut tab.sql_text,
                            &tab.result,
                            tab.running.is_some(),
                            ctrl_enter || f9,
                            &mut tab.db_context,
                            databases,
                        );
                    }
                }
            });
        });
    }
}

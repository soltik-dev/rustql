//! Drawing of the session according to its state. This only collects
//! what the user does (`FrameSignals`); `Session::show` applies it later.

use eframe::egui;

use super::schema::show_loading;
use super::{ConnState, FrameKeys, SelectedNode, Session, Shared};
use crate::i18n::t;
use crate::loadable::Loadable;
use crate::ui::connection_panel::{FormView, SaveProfile};
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

/// What the user did in this frame.
#[derive(Default)]
pub(super) struct FrameSignals {
    // Connection form
    pub connect: bool,
    pub cancel_connect: bool,
    pub save_profile: Option<SaveProfile>,
    pub delete_profile: bool,
    pub profile_selected: Option<usize>,
    pub load_databases: bool,
    // Connected
    pub sidebar: SidebarOutput,
    pub tab_bar: TabBarResult,
    pub filter: FilterAction,
    pub grid: GridAction,
    pub apply_bar: ApplyBarAction,
    pub db_info: DbInfoAction,
    pub sql: SqlEditorAction,
}

impl FrameSignals {
    /// Only what requests data to draw (the tree's tables): everything else
    /// is a user action.
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

    /// Connection form with the favourites on the left.
    fn draw_form(&mut self, ui: &mut egui::Ui, sh: &Shared, signals: &mut FrameSignals) {
        // The content goes under push_id(sid) so that widget state (expanded
        // tree, scroll, cursors) belongs to each session.
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
                    FormView {
                        params: &mut self.params,
                        port_str: &mut self.port_str,
                        error_message: &self.error_message,
                        profile_name: &mut self.profile_name,
                        save_password: &mut self.save_password,
                        color: &mut self.color,
                        selected_idx: self.selected_profile_idx,
                        databases: &self.db_picker.list,
                    },
                );
                signals.load_databases = result.load_databases;
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
                ui.strong(t!("session-connecting", host = host));
                ui.add_space(6.0);
                ui.label(status.as_str());
                ui.add_space(4.0);
                ui.label(format!("{}s", elapsed));
                ui.add_space(16.0);
                if ui.button(t!("common-cancel")).clicked() {
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
        // While the table loads, the grid is shown without applying its
        // actions: they would go by position over the previous data.
        let running = self.table_view.query_loading || self.table_view.saving;

        // ── Sidebar ───────────────────────────────────────────────
        if show_sidebar {
            // Over the field, not the whole schema: the sidebar search
            // borrows it at the same time
            let selected = self.schema.selected_node.as_ref();
            let sel_db = selected.and_then(SelectedNode::db);
            let sel_table = selected.and_then(SelectedNode::table).map(|(_, t)| t);

            let sidebar_panel = egui::Panel::left("sidebar_v2")
                .default_size(200.0)
                .min_size(160.0)
                .show(ui, |ui| {
                    ui.push_id(sid, |ui| {
                        let view = sidebar::SidebarView {
                            databases: &self.schema.databases,
                            tables: &self.schema.tables_cache,
                            kinds: &self.schema.kinds,
                            selected_db: sel_db,
                            selected_table: sel_table,
                            favorites: &sh.settings.fav_tables,
                            server_name: &self.title,
                            conn_color,
                            single_database: self.params.only_database().is_some(),
                        };
                        sidebar::show(
                            ui,
                            &view,
                            &mut self.schema.table_search,
                            &mut self.show_only_fav,
                        )
                    })
                    .inner
                });
            signals.sidebar = sidebar_panel.inner;

            // Tree border in the connection's colour. It rises over the tab
            // line so its thickness is not doubled.
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
        {
            let sql_titles: Vec<String> = self
                .sql_tabs
                .iter()
                .enumerate()
                .map(|(i, tab)| tab.title(i))
                .collect();
            let view = tab_bar::TabBarView {
                active: self.active_tab,
                server_name: &self.title,
                db_name: self.schema.selected_db(),
                table_name: self.schema.selected_table().map(|(_, table)| table),
                sql_titles: &sql_titles,
            };

            egui::Panel::top("tab_bar")
                .min_size(30.0)
                .resizable(false)
                .show(ui, |ui| {
                    ui.push_id(sid, |ui| {
                        ui.add_space(2.0);
                        signals.tab_bar = tab_bar::show(ui, &view, &mut self.sql_tab_rename);
                    });
                });
        }

        // ── Apply bar (only in the Data tab with pending changes) ───
        let has_pending = !self.table_view.pending_edits.is_empty()
            || !self.table_view.pending_deletes.is_empty()
            || !self.table_view.pending_inserts.is_empty();
        if !running && self.active_tab == ActiveTab::Datos && has_pending {
            // pk_missing only blocks if there are pending edits (DELETE and INSERT need no PK in the UI)
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

        // ── Central panel ─────────────────────────────────────────
        egui::CentralPanel::default().show(ui, |ui| {
            ui.push_id(sid, |ui| match self.active_tab {
                ActiveTab::Datos => {
                    let selected_table = self.schema.selected_table();
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
                        self.params.engine,
                        selected_table
                            .map(|(db, table)| self.schema.kind(db, table))
                            .unwrap_or_default(),
                        // With a query running: spinner in the grid
                        running,
                    );
                    if !running {
                        signals.grid = action;
                    }
                }

                ActiveTab::Servidor => match &self.schema.databases_info {
                    Loadable::Ready(dbs) => {
                        let tables = &self.schema.tables_cache;
                        server_info::show(ui, &self.params.host, dbs, |db| {
                            tables.get(db).and_then(Loadable::ready).map(Vec::len)
                        });
                    }
                    other => show_loading(ui, other),
                },

                ActiveTab::Bbdd => {
                    if let Some(db) = self.schema.selected_db() {
                        match self.schema.tables_info.get(db) {
                            Some(Loadable::Ready(tables)) => {
                                signals.db_info = db_info::show(ui, db, tables);
                            }
                            other => show_loading(ui, other.unwrap_or(&Loadable::NotLoaded)),
                        }
                    } else {
                        ui.centered_and_justified(|ui| {
                            ui.label(t!("session-pick-database"));
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
                            ui.label(t!("session-pick-table"));
                        });
                    }
                }

                ActiveTab::Sql(tab_idx) => {
                    if let Some(tab) = self.sql_tabs.get_mut(tab_idx) {
                        let databases =
                            self.schema.databases.ready().map_or(&[][..], Vec::as_slice);
                        signals.sql = sql_editor::show(
                            ui,
                            tab,
                            ctrl_enter || f9,
                            databases,
                            self.params.engine,
                        );
                    }
                }
            });
        });
    }
}

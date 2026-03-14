use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use eframe::egui;

use crate::db::{
    ColumnInfo, ConnectionParams, ConnectEvent, DatabaseInfo, DbBackend, QueryResult, TableInfo,
    connect_mysql_progress, escape_sql,
};
use crate::log::LogEntry;
use crate::profiles::{self, ConnectionProfile};
use crate::settings::{self, AppSettings};
use crate::ui::db_info::DbInfoAction;
use crate::ui::modals::{self, DropKind, ModalAction, ModalState, NewColumnDef, TruncateKind};
use crate::ui::results_grid::{
    EditingCell, GridAction, PendingEdit, SortSpec, compute_sort_indices,
};
use crate::ui::sidebar::SidebarAction;
use crate::ui::sql_editor::SqlEditorAction;
use crate::ui::tab_bar::ActiveTab;
use crate::ui::{
    connection_panel, db_info, filter_bar, log_panel, modals as modals_ui, results_grid,
    server_info, settings_panel, sidebar, sql_editor, statusbar, tab_bar, table_structure,
};

// ── Estados de la aplicación ─────────────────────────────────────────────────

pub enum AppState {
    Disconnected,
    Connecting {
        receiver: Receiver<ConnectEvent>,
        host: String,
        status: String,
        start: Instant,
    },
    Connected(Box<dyn DbBackend>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum SelectedNode {
    Server,
    Database(String),
    Table { db: String, table: String },
}

// ── Estado principal ─────────────────────────────────────────────────────────

pub struct RustqlApp {
    pub state: AppState,
    pub params: ConnectionParams,
    pub port_str: String,
    pub error_message: Option<String>,

    // Árbol de esquema
    pub databases: Vec<String>,
    pub tables_cache: HashMap<String, Vec<String>>,
    pub table_search: String,
    pub selected_node: Option<SelectedNode>,

    // Datos de la tabla activa (pestaña Datos)
    pub table_data: Option<QueryResult>,
    pub table_error: Option<String>,
    pub sort_specs: Vec<SortSpec>,
    pub sort_indices: Vec<usize>,
    pub filter_text: String,

    // Edición de celdas
    pub pending_edits: Vec<PendingEdit>,
    pub editing_cell: Option<EditingCell>,
    pub primary_keys: Vec<String>,

    // Pestañas
    pub active_tab: ActiveTab,

    // Cachés para paneles de info
    pub databases_info: Option<Vec<DatabaseInfo>>,
    pub tables_info: HashMap<String, Vec<TableInfo>>,
    pub columns_info: HashMap<(String, String), Vec<ColumnInfo>>,

    // Editor SQL libre
    pub sql_editor_text: String,
    pub sql_editor_result: Option<Result<QueryResult, String>>,

    // Modal activo
    pub modal: ModalState,

    // Log y sesión
    pub query_log: Vec<LogEntry>,
    pub profiles: Vec<ConnectionProfile>,
    pub selected_profile_idx: Option<usize>,
    pub profile_name: String,
    pub save_password: bool,

    // Visibilidad de paneles
    pub show_sidebar: bool,
    pub show_log: bool,
    pub show_settings: bool,
    pub settings: AppSettings,
    pub settings_dirty: bool,
}

impl Default for RustqlApp {
    fn default() -> Self {
        let params = ConnectionParams::default();
        let port_str = params.port.to_string();
        Self {
            state: AppState::Disconnected,
            port_str,
            params,
            error_message: None,
            databases: Vec::new(),
            tables_cache: HashMap::new(),
            table_search: String::new(),
            selected_node: None,
            table_data: None,
            table_error: None,
            sort_specs: Vec::new(),
            sort_indices: Vec::new(),
            filter_text: String::new(),
            pending_edits: Vec::new(),
            editing_cell: None,
            primary_keys: Vec::new(),
            active_tab: ActiveTab::default(),
            databases_info: None,
            tables_info: HashMap::new(),
            columns_info: HashMap::new(),
            sql_editor_text: String::new(),
            sql_editor_result: None,
            modal: ModalState::None,
            query_log: Vec::new(),
            profiles: profiles::load(),
            selected_profile_idx: None,
            profile_name: String::new(),
            save_password: false,
            show_sidebar: true,
            show_log: true,
            show_settings: false,
            settings: settings::load(),
            settings_dirty: true,
        }
    }
}

// ── Métodos auxiliares ────────────────────────────────────────────────────────

impl RustqlApp {
    fn run_query(&mut self, sql: &str) {
        let start = Instant::now();
        let result = match &mut self.state {
            AppState::Connected(b) => b.execute_query(sql),
            _ => return,
        };
        let duration_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(data) => {
                let rows = data.rows.len();
                self.push_log(LogEntry::ok(sql, duration_ms, rows));
                self.sort_specs.clear();
                self.sort_indices = (0..rows).collect();
                self.table_data = Some(data);
                self.table_error = None;
            }
            Err(e) => {
                self.push_log(LogEntry::err(sql, duration_ms, &e));
                self.table_data = None;
                self.table_error = Some(e);
                self.sort_specs.clear();
                self.sort_indices.clear();
            }
        }
    }

    fn run_write(&mut self, sql: &str) -> Result<u64, String> {
        let start = Instant::now();
        let result = match &mut self.state {
            AppState::Connected(b) => b.execute_write(sql),
            _ => return Err("No conectado".into()),
        };
        let duration_ms = start.elapsed().as_millis() as u64;
        match &result {
            Ok(n) => self.push_log(LogEntry::ok(sql, duration_ms, *n as usize)),
            Err(e) => self.push_log(LogEntry::err(sql, duration_ms, e.as_str())),
        }
        result
    }

    fn push_log(&mut self, entry: LogEntry) {
        if self.query_log.len() >= 500 {
            self.query_log.remove(0);
        }
        self.query_log.push(entry);
    }

    fn build_sql(&self, db: &str, table: &str) -> String {
        let filter = self.filter_text.trim();
        if filter.is_empty() {
            format!("SELECT * FROM `{}`.`{}` LIMIT 500", escape_sql(db), escape_sql(table))
        } else {
            format!(
                "SELECT * FROM `{}`.`{}` WHERE {filter} LIMIT 500",
                escape_sql(db),
                escape_sql(table)
            )
        }
    }

    fn recompute_sort(&mut self) {
        if let Some(data) = &self.table_data {
            self.sort_indices = compute_sort_indices(&data.rows, &self.sort_specs);
        }
    }

    fn load_primary_keys(&mut self, db: &str, table: &str) {
        let result = match &mut self.state {
            AppState::Connected(b) => b.get_primary_keys(db, table),
            _ => return,
        };
        self.primary_keys = result.unwrap_or_default();
    }

    fn load_databases_info(&mut self) {
        let result = match &mut self.state {
            AppState::Connected(b) => b.get_databases_info(),
            _ => return,
        };
        self.databases_info = result.ok();
    }

    fn load_tables_info(&mut self, db: &str) {
        let result = match &mut self.state {
            AppState::Connected(b) => b.get_tables_info(db),
            _ => return,
        };
        if let Ok(info) = result {
            self.tables_info.insert(db.to_string(), info);
        }
    }

    fn load_columns_info(&mut self, db: &str, table: &str) {
        let result = match &mut self.state {
            AppState::Connected(b) => b.get_columns_info(db, table),
            _ => return,
        };
        if let Ok(cols) = result {
            self.columns_info.insert((db.to_string(), table.to_string()), cols);
        }
    }

    fn maybe_load_tab_data(&mut self) {
        match self.active_tab {
            ActiveTab::Servidor => {
                if self.databases_info.is_none() {
                    self.load_databases_info();
                }
            }
            ActiveTab::Bbdd => {
                let db = match &self.selected_node {
                    Some(SelectedNode::Database(d)) => Some(d.clone()),
                    Some(SelectedNode::Table { db, .. }) => Some(db.clone()),
                    _ => None,
                };
                if let Some(db) = db {
                    if !self.tables_info.contains_key(&db) {
                        self.load_tables_info(&db);
                    }
                }
            }
            ActiveTab::Tabla => {
                let key = match &self.selected_node {
                    Some(SelectedNode::Table { db, table }) => {
                        Some((db.clone(), table.clone()))
                    }
                    _ => None,
                };
                if let Some((db, table)) = key {
                    let k = (db.clone(), table.clone());
                    if !self.columns_info.contains_key(&k) {
                        self.load_columns_info(&db, &table);
                    }
                }
            }
            _ => {}
        }
    }

    fn invalidate_after_write(&mut self, db: Option<&str>, table: Option<&str>) {
        // Invalidar bases de datos info
        self.databases_info = None;

        if let Some(db) = db {
            // Invalidar caché de tablas de esa BD
            self.tables_cache.remove(db);
            self.tables_info.remove(db);
            // Recargar lista de bases de datos del sidebar
            self.databases.clear();

            if let Some(table) = table {
                let key = (db.to_string(), table.to_string());
                self.columns_info.remove(&key);

                // Si la tabla eliminada era la seleccionada, limpiar
                if matches!(
                    &self.selected_node,
                    Some(SelectedNode::Table { db: d, table: t }) if d == db && t == table
                ) {
                    self.selected_node = Some(SelectedNode::Database(db.to_string()));
                    self.table_data = None;
                    self.table_error = None;
                    self.pending_edits.clear();
                    self.editing_cell = None;
                    self.primary_keys.clear();
                }
            } else {
                // BD entera eliminada
                if matches!(
                    &self.selected_node,
                    Some(SelectedNode::Database(d) | SelectedNode::Table { db: d, .. }) if d == db
                ) {
                    self.selected_node = Some(SelectedNode::Server);
                    self.active_tab = ActiveTab::Servidor;
                    self.table_data = None;
                    self.table_error = None;
                    self.pending_edits.clear();
                    self.editing_cell = None;
                    self.primary_keys.clear();
                }
            }
        }
    }

    fn apply_pending_edits(&mut self) {
        let (db, table) = match &self.selected_node {
            Some(SelectedNode::Table { db, table }) => (db.clone(), table.clone()),
            _ => return,
        };

        if self.primary_keys.is_empty() {
            self.table_error =
                Some("No se puede editar: la tabla no tiene clave primaria".into());
            return;
        }

        let data = match &self.table_data {
            Some(d) => d.clone(),
            None => return,
        };

        let edits = self.pending_edits.clone();
        let pks = self.primary_keys.clone();
        let mut all_ok = true;

        for edit in &edits {
            let col_name = match data.columns.get(edit.col_idx) {
                Some(c) => c.clone(),
                None => continue,
            };

            let where_parts: Vec<String> = pks
                .iter()
                .filter_map(|pk| {
                    let pk_idx = data.columns.iter().position(|c| c == pk)?;
                    let pk_val = data.rows.get(edit.row_idx)?.get(pk_idx)?;
                    Some(format!(
                        "`{}` = '{}'",
                        pk.replace('`', "``"),
                        escape_sql(pk_val)
                    ))
                })
                .collect();

            if where_parts.is_empty() {
                continue;
            }

            let sql = format!(
                "UPDATE `{}`.`{}` SET `{}` = '{}' WHERE {}",
                db.replace('`', "``"),
                table.replace('`', "``"),
                col_name.replace('`', "``"),
                escape_sql(&edit.new_value),
                where_parts.join(" AND ")
            );

            if self.run_write(&sql).is_err() {
                all_ok = false;
            }
        }

        if all_ok {
            self.pending_edits.clear();
            self.editing_cell = None;
            let sql = self.build_sql(&db, &table);
            self.run_query(&sql);
        }
    }

    fn on_connected(&mut self, backend: Box<dyn DbBackend>) {
        self.state = AppState::Connected(backend);
        self.error_message = None;
        self.databases.clear();
        self.tables_cache.clear();
        self.table_search.clear();
        self.selected_node = None;
        self.active_tab = ActiveTab::default();
        self.table_data = None;
        self.table_error = None;
        self.sort_specs.clear();
        self.sort_indices.clear();
        self.filter_text.clear();
        self.pending_edits.clear();
        self.editing_cell = None;
        self.primary_keys.clear();
        self.modal = ModalState::None;
        self.databases_info = None;
        self.tables_info.clear();
        self.columns_info.clear();
        self.sql_editor_text.clear();
        self.sql_editor_result = None;
        self.query_log.clear();
    }
}

// ── Loop principal ────────────────────────────────────────────────────────────

impl eframe::App for RustqlApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Aplicar ajustes si cambiaron
        if self.settings_dirty {
            settings::apply(ctx, &self.settings);
            settings::save(&self.settings);
            self.settings_dirty = false;
        }

        let f9 = ctx.input(|i| i.key_pressed(egui::Key::F9));
        let ctrl_enter =
            ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::Enter));

        // ── Proactive data loading for current tab ────────────────────────
        if matches!(&self.state, AppState::Connected(_)) {
            self.maybe_load_tab_data();
        }

        // ── Polling del hilo de conexión ─────────────────────────────────
        let mut connect_result: Option<Result<Box<dyn DbBackend>, String>> = None;
        if let AppState::Connecting { receiver, status, .. } = &mut self.state {
            loop {
                match receiver.try_recv() {
                    Ok(ConnectEvent::Status(s)) => {
                        *status = s;
                        ctx.request_repaint_after(Duration::from_millis(50));
                    }
                    Ok(ConnectEvent::Done(result)) => {
                        connect_result = Some(result);
                        break;
                    }
                    Err(mpsc::TryRecvError::Empty) => {
                        ctx.request_repaint_after(Duration::from_millis(50));
                        break;
                    }
                    Err(mpsc::TryRecvError::Disconnected) => {
                        connect_result =
                            Some(Err("El hilo de conexión terminó inesperadamente".into()));
                        break;
                    }
                }
            }
        }
        if let Some(result) = connect_result {
            match result {
                Ok(backend) => self.on_connected(backend),
                Err(e) => {
                    self.state = AppState::Disconnected;
                    self.error_message = Some(e);
                }
            }
        }

        // ── Señales del frame ────────────────────────────────────────────
        let mut connect_clicked = false;
        let mut cancel_connect = false;
        let mut save_profile = false;
        let mut delete_profile = false;
        let mut profile_selected: Option<usize> = None;
        let mut sidebar_action = SidebarAction::None;
        let mut filter_action = filter_bar::FilterAction::None;
        let mut settings_changed = false;
        let mut grid_action = GridAction::None;
        let mut apply_bar_action = GridAction::None;
        let mut tab_clicked: Option<ActiveTab> = None;
        let mut db_info_action = DbInfoAction::None;
        let mut sql_action = SqlEditorAction::None;
        let mut should_disconnect = false;

        // ── Paneles inferiores (primero siempre) ─────────────────────────
        if matches!(self.state, AppState::Connected(_)) {
            egui::TopBottomPanel::bottom("statusbar")
                .exact_height(26.0)
                .show(ctx, |ui| {
                    statusbar::show(
                        ui,
                        &mut self.show_sidebar,
                        &mut self.show_log,
                        &mut self.show_settings,
                    );
                });

            if self.show_log {
                egui::TopBottomPanel::bottom("log_panel")
                    .resizable(true)
                    .min_height(80.0)
                    .max_height(300.0)
                    .show(ctx, |ui| {
                        log_panel::show(ui, &self.query_log);
                    });
            }
        }

        // ── Panel de ajustes (derecha) ───────────────────────────────────
        if self.show_settings {
            egui::SidePanel::right("settings_panel")
                .exact_width(240.0)
                .show(ctx, |ui| {
                    settings_changed = settings_panel::show(ui, &mut self.settings);
                });
        }

        // ── Contenido principal según estado ─────────────────────────────
        match &mut self.state {
            AppState::Disconnected => {
                egui::SidePanel::left("conn_favorites")
                    .exact_width(200.0)
                    .show(ctx, |ui| {
                        profile_selected = connection_panel::show_favorites(
                            ui,
                            &self.profiles,
                            &self.selected_profile_idx,
                        );
                    });

                egui::CentralPanel::default().show(ctx, |ui| {
                    let result = connection_panel::show_form(
                        ui,
                        &mut self.params,
                        &mut self.port_str,
                        &self.error_message,
                        &mut self.profile_name,
                        &mut self.save_password,
                        &self.selected_profile_idx,
                    );
                    connect_clicked = result.connect;
                    save_profile = result.save_profile;
                    delete_profile = result.delete_profile;
                });
            }

            AppState::Connecting { host, status, start, .. } => {
                let host = host.clone();
                let status = status.clone();
                let elapsed = start.elapsed().as_secs();
                egui::SidePanel::left("conn_favorites")
                    .exact_width(200.0)
                    .show(ctx, |ui| {
                        connection_panel::show_favorites(
                            ui,
                            &self.profiles,
                            &self.selected_profile_idx,
                        );
                    });

                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.add_space(ui.available_height() / 3.0);
                    ui.vertical_centered(|ui| {
                        ui.spinner();
                        ui.add_space(10.0);
                        ui.strong(format!("Conectando a {}...", host));
                        ui.add_space(6.0);
                        ui.label(&status);
                        ui.add_space(4.0);
                        ui.label(format!("{}s", elapsed));
                        ui.add_space(16.0);
                        if ui.button("Cancelar").clicked() {
                            cancel_connect = true;
                        }
                    });
                });
            }

            AppState::Connected(backend) => {
                // ── Sidebar ───────────────────────────────────────────────
                if self.show_sidebar {
                    let (sel_db, sel_table) = match &self.selected_node {
                        Some(SelectedNode::Table { db, table }) => {
                            (Some(db.as_str()), Some(table.as_str()))
                        }
                        Some(SelectedNode::Database(db)) => (Some(db.as_str()), None),
                        _ => (None, None),
                    };

                    egui::SidePanel::left("sidebar")
                        .min_width(180.0)
                        .show(ctx, |ui| {
                            sidebar_action = sidebar::show(
                                ui,
                                backend,
                                &mut self.databases,
                                &mut self.tables_cache,
                                sel_db,
                                sel_table,
                                &mut self.table_search,
                                &self.params.host,
                            );
                        });
                }

                // ── Tab bar ───────────────────────────────────────────────
                let has_db = matches!(
                    &self.selected_node,
                    Some(SelectedNode::Database(_) | SelectedNode::Table { .. })
                );
                let has_table =
                    matches!(&self.selected_node, Some(SelectedNode::Table { .. }));

                egui::TopBottomPanel::top("tab_bar")
                    .exact_height(34.0)
                    .show(ctx, |ui| {
                        ui.add_space(4.0);
                        tab_clicked =
                            tab_bar::show(ui, self.active_tab, has_db, has_table);
                    });

                // ── Apply bar (solo en tab Datos con cambios pendientes) ──
                if self.active_tab == ActiveTab::Datos && !self.pending_edits.is_empty() {
                    let pk_missing = self.table_data.is_some() && self.primary_keys.is_empty();
                    egui::TopBottomPanel::bottom("apply_bar")
                        .exact_height(36.0)
                        .show(ctx, |ui| {
                            apply_bar_action = results_grid::show_apply_bar(
                                ui,
                                &self.pending_edits,
                                ctrl_enter,
                                pk_missing,
                            );
                        });
                }

                // ── Panel central ─────────────────────────────────────────
                egui::CentralPanel::default().show(ctx, |ui| {
                    match self.active_tab {
                        ActiveTab::Servidor => {
                            let dbs = self.databases_info.as_deref().unwrap_or(&[]);
                            server_info::show(ui, &self.params.host, dbs);
                        }

                        ActiveTab::Bbdd => {
                            let db_name = match &self.selected_node {
                                Some(SelectedNode::Database(d)) => Some(d.clone()),
                                Some(SelectedNode::Table { db, .. }) => Some(db.clone()),
                                _ => None,
                            };
                            if let Some(db) = &db_name {
                                let empty = Vec::new();
                                let tables =
                                    self.tables_info.get(db).unwrap_or(&empty);
                                db_info_action = db_info::show(ui, db, tables);
                            } else {
                                ui.centered_and_justified(|ui| {
                                    ui.label(
                                        "Selecciona una base de datos en el panel izquierdo",
                                    );
                                });
                            }
                        }

                        ActiveTab::Tabla => {
                            if let Some(SelectedNode::Table { db, table }) =
                                &self.selected_node.clone()
                            {
                                let key = (db.clone(), table.clone());
                                let empty = Vec::new();
                                let cols = self.columns_info.get(&key).unwrap_or(&empty);
                                table_structure::show(ui, db, table, cols);
                            } else {
                                ui.centered_and_justified(|ui| {
                                    ui.label("Selecciona una tabla en el panel izquierdo");
                                });
                            }
                        }

                        ActiveTab::Datos => {
                            let selected_table = match &self.selected_node {
                                Some(SelectedNode::Table { db, table }) => {
                                    Some((db.clone(), table.clone()))
                                }
                                _ => None,
                            };
                            if selected_table.is_some() {
                                let apply_f9 = f9 && self.active_tab == ActiveTab::Datos;
                                filter_action =
                                    filter_bar::show(ui, &mut self.filter_text, apply_f9);
                                ui.separator();
                            }
                            grid_action = results_grid::show(
                                ui,
                                &selected_table,
                                &self.table_data,
                                &self.table_error,
                                &mut self.sort_specs,
                                &self.sort_indices,
                                &self.pending_edits,
                                &mut self.editing_cell,
                            );
                        }

                        ActiveTab::Sql => {
                            let apply_f9 = f9 && self.active_tab == ActiveTab::Sql;
                            sql_action = sql_editor::show(
                                ui,
                                &mut self.sql_editor_text,
                                &self.sql_editor_result,
                                apply_f9,
                            );
                        }
                    }
                });
            }
        }

        // ── Modal (sobre todo) ────────────────────────────────────────────
        let modal_action = modals_ui::show(ctx, &mut self.modal);

        // ── Aplicar señales ───────────────────────────────────────────────

        if cancel_connect {
            self.state = AppState::Disconnected;
        }

        if settings_changed {
            self.settings_dirty = true;
        }

        if let Some(t) = tab_clicked {
            self.active_tab = t;
        }

        // Grid action
        match grid_action {
            GridAction::SortChanged => self.recompute_sort(),
            GridAction::CellEditStarted { row_idx, col_idx, current_value } => {
                self.editing_cell = Some(EditingCell {
                    row_idx,
                    col_idx,
                    buffer: current_value,
                });
            }
            GridAction::CellEditCommitted { row_idx, col_idx, new_value } => {
                self.editing_cell = None;
                let original = self
                    .table_data
                    .as_ref()
                    .and_then(|d| d.rows.get(row_idx))
                    .and_then(|r| r.get(col_idx))
                    .cloned()
                    .unwrap_or_default();

                if new_value == original {
                    // Sin cambio real — asegurarse de no añadir edit
                    self.pending_edits
                        .retain(|e| !(e.row_idx == row_idx && e.col_idx == col_idx));
                } else if let Some(existing) = self
                    .pending_edits
                    .iter_mut()
                    .find(|e| e.row_idx == row_idx && e.col_idx == col_idx)
                {
                    if new_value == existing.original {
                        // Revertido al original
                        let idx = self
                            .pending_edits
                            .iter()
                            .position(|e| e.row_idx == row_idx && e.col_idx == col_idx)
                            .unwrap();
                        self.pending_edits.remove(idx);
                    } else {
                        existing.new_value = new_value;
                    }
                } else {
                    self.pending_edits.push(PendingEdit {
                        row_idx,
                        col_idx,
                        original,
                        new_value,
                    });
                }
            }
            GridAction::CellEditCancelled => {
                self.editing_cell = None;
            }
            GridAction::ApplyEdits => {
                self.apply_pending_edits();
            }
            GridAction::DiscardEdits => {
                self.pending_edits.clear();
                self.editing_cell = None;
            }
            GridAction::None => {}
        }

        // Apply bar action (solo visible cuando hay pending_edits)
        match apply_bar_action {
            GridAction::ApplyEdits => self.apply_pending_edits(),
            GridAction::DiscardEdits => {
                self.pending_edits.clear();
                self.editing_cell = None;
            }
            _ => {}
        }

        // Sidebar action
        match sidebar_action {
            SidebarAction::None => {}
            SidebarAction::SelectServer => {
                self.selected_node = Some(SelectedNode::Server);
                self.active_tab = ActiveTab::Servidor;
            }
            SidebarAction::SelectDatabase(db) => {
                self.selected_node = Some(SelectedNode::Database(db.clone()));
                self.active_tab = ActiveTab::Bbdd;
                if !self.tables_info.contains_key(&db) {
                    self.load_tables_info(&db);
                }
            }
            SidebarAction::SelectTable { db, table } => {
                let already_selected = matches!(
                    &self.selected_node,
                    Some(SelectedNode::Table { db: d, table: t }) if *d == db && *t == table
                );
                if !already_selected {
                    self.selected_node = Some(SelectedNode::Table {
                        db: db.clone(),
                        table: table.clone(),
                    });
                    self.pending_edits.clear();
                    self.editing_cell = None;
                    let sql = self.build_sql(&db, &table);
                    self.run_query(&sql);
                    self.load_primary_keys(&db, &table);
                }
                self.active_tab = ActiveTab::Datos;
            }
            SidebarAction::CreateDatabase => {
                self.modal = ModalState::CreateDatabase {
                    name: String::new(),
                    charset: "utf8mb4".into(),
                    error: None,
                };
            }
            SidebarAction::CreateTable { db } => {
                self.modal = ModalState::CreateTable {
                    db,
                    name: String::new(),
                    columns: vec![NewColumnDef::new()],
                    error: None,
                };
            }
            SidebarAction::DropDatabase { db } => {
                let target = db.clone();
                self.modal = ModalState::ConfirmDrop {
                    kind: DropKind::Database,
                    target,
                    db: Some(db),
                    table: None,
                };
            }
            SidebarAction::TruncateAllTables { db } => {
                let target = format!("todas las tablas de '{db}'");
                self.modal = ModalState::ConfirmTruncate {
                    kind: TruncateKind::AllInDatabase,
                    target,
                    db: Some(db),
                    table: None,
                };
            }
            SidebarAction::DropTable { db, table } => {
                let target = format!("{db}.{table}");
                self.modal = ModalState::ConfirmDrop {
                    kind: DropKind::Table,
                    target,
                    db: Some(db),
                    table: Some(table),
                };
            }
            SidebarAction::TruncateTable { db, table } => {
                let target = format!("{db}.{table}");
                self.modal = ModalState::ConfirmTruncate {
                    kind: TruncateKind::Table,
                    target,
                    db: Some(db),
                    table: Some(table),
                };
            }
            SidebarAction::CopyTableName(name) => {
                ctx.copy_text(name);
            }
            SidebarAction::Refresh => {
                self.databases.clear();
                self.tables_cache.clear();
                self.databases_info = None;
                self.tables_info.clear();
            }
            SidebarAction::Disconnect => {
                should_disconnect = true;
            }
        }

        if should_disconnect {
            self.state = AppState::Disconnected;
        }

        // Filter action
        match filter_action {
            filter_bar::FilterAction::Apply | filter_bar::FilterAction::Clear => {
                if let Some(SelectedNode::Table { db, table }) =
                    self.selected_node.clone().as_ref()
                {
                    let sql = self.build_sql(db, table);
                    self.run_query(&sql);
                }
            }
            filter_bar::FilterAction::None => {}
        }

        // DB info: double-click on table opens it
        if let DbInfoAction::OpenTable(table) = db_info_action {
            let db = match &self.selected_node {
                Some(SelectedNode::Database(d)) => Some(d.clone()),
                Some(SelectedNode::Table { db, .. }) => Some(db.clone()),
                _ => None,
            };
            if let Some(db) = db {
                self.selected_node = Some(SelectedNode::Table {
                    db: db.clone(),
                    table: table.clone(),
                });
                self.active_tab = ActiveTab::Datos;
                self.pending_edits.clear();
                self.editing_cell = None;
                let sql = self.build_sql(&db, &table);
                self.run_query(&sql);
                self.load_primary_keys(&db, &table);
            }
        }

        // SQL editor
        if let SqlEditorAction::Execute(sql) = sql_action {
            let start = Instant::now();
            let result = match &mut self.state {
                AppState::Connected(b) => b.execute_query(&sql),
                _ => return,
            };
            let duration_ms = start.elapsed().as_millis() as u64;
            match result {
                Ok(data) => {
                    let rows = data.rows.len();
                    self.push_log(LogEntry::ok(&sql, duration_ms, rows));
                    self.sql_editor_result = Some(Ok(data));
                }
                Err(e) => {
                    self.push_log(LogEntry::err(&sql, duration_ms, &e));
                    self.sql_editor_result = Some(Err(e));
                }
            }
        }

        // Modal action
        match modal_action {
            ModalAction::None => {}
            ModalAction::Cancel => {
                self.modal = ModalState::None;
            }
            ModalAction::CreateDatabase { name, charset } => {
                let sql = format!(
                    "CREATE DATABASE `{}` CHARACTER SET {}",
                    name.replace('`', "``"),
                    charset
                );
                let result = self.run_write(&sql);
                match result {
                    Ok(_) => {
                        self.modal = ModalState::None;
                        self.databases.clear();
                        self.databases_info = None;
                    }
                    Err(e) => {
                        if let ModalState::CreateDatabase { error, .. } = &mut self.modal {
                            *error = Some(e);
                        }
                    }
                }
            }
            ModalAction::CreateTable { db, name, columns } => {
                let sql = modals::build_create_table_sql(&db, &name, &columns);
                let result = self.run_write(&sql);
                match result {
                    Ok(_) => {
                        self.modal = ModalState::None;
                        self.invalidate_after_write(Some(&db), None);
                    }
                    Err(e) => {
                        if let ModalState::CreateTable { error, .. } = &mut self.modal {
                            *error = Some(e);
                        }
                    }
                }
            }
            ModalAction::DropDatabase(db) => {
                let result = self.run_write(&format!("DROP DATABASE `{}`", db.replace('`', "``")));
                self.modal = ModalState::None;
                if result.is_ok() {
                    self.invalidate_after_write(Some(&db), None);
                }
            }
            ModalAction::DropTable { db, table } => {
                let result = self.run_write(&format!(
                    "DROP TABLE `{}`.`{}`",
                    db.replace('`', "``"),
                    table.replace('`', "``")
                ));
                self.modal = ModalState::None;
                if result.is_ok() {
                    self.invalidate_after_write(Some(&db), Some(&table));
                }
            }
            ModalAction::TruncateTable { db, table } => {
                let result = self.run_write(&format!(
                    "TRUNCATE TABLE `{}`.`{}`",
                    db.replace('`', "``"),
                    table.replace('`', "``")
                ));
                self.modal = ModalState::None;
                if result.is_ok() {
                    // Recargar datos si es la tabla activa
                    if matches!(
                        &self.selected_node,
                        Some(SelectedNode::Table { db: d, table: t }) if *d == db && *t == table
                    ) {
                        let sql = self.build_sql(&db, &table);
                        self.run_query(&sql);
                    }
                    self.invalidate_after_write(Some(&db), Some(&table));
                }
            }
            ModalAction::TruncateAllTables(db) => {
                self.modal = ModalState::None;
                let tables = self.tables_cache.get(&db).cloned().unwrap_or_default();
                for table in &tables {
                    let _ = self.run_write(&format!(
                        "TRUNCATE TABLE `{}`.`{}`",
                        db.replace('`', "``"),
                        table.replace('`', "``")
                    ));
                }
                self.invalidate_after_write(Some(&db), None);
            }
        }

        // Profile selection & management
        if let Some(idx) = profile_selected {
            if matches!(self.state, AppState::Disconnected) {
                let profile = &self.profiles[idx];
                self.params = profile.to_params();
                self.port_str = self.params.port.to_string();
                self.profile_name = profile.name.clone();
                self.save_password = profile.password.is_some();
                self.selected_profile_idx = Some(idx);
                self.error_message = None;
            }
        }

        if save_profile && !self.profile_name.trim().is_empty() {
            if let Ok(p) = self.port_str.parse::<u16>() {
                self.params.port = p;
            }
            let password = if self.save_password {
                Some(self.params.password.clone())
            } else {
                None
            };
            let profile = ConnectionProfile {
                name: self.profile_name.trim().to_string(),
                host: self.params.host.clone(),
                port: self.params.port,
                user: self.params.user.clone(),
                password,
            };
            if let Some(idx) = self.selected_profile_idx {
                self.profiles[idx] = profile;
            } else if let Some(pos) = self
                .profiles
                .iter()
                .position(|p| p.name == self.profile_name.trim())
            {
                self.profiles[pos] = profile;
                self.selected_profile_idx = Some(pos);
            } else {
                self.profiles.push(profile);
                self.selected_profile_idx = Some(self.profiles.len() - 1);
            }
            profiles::save(&self.profiles);
        }

        if delete_profile {
            if let Some(idx) = self.selected_profile_idx {
                self.profiles.remove(idx);
                self.selected_profile_idx = None;
                self.profile_name.clear();
                profiles::save(&self.profiles);
            }
        }

        // Iniciar conexión async
        if connect_clicked {
            if let Ok(p) = self.port_str.parse::<u16>() {
                self.params.port = p;
            }
            let params = self.params.clone();
            let host = self.params.host.clone();
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                connect_mysql_progress(&params, &tx);
            });
            self.state = AppState::Connecting {
                receiver: rx,
                host,
                status: "Iniciando...".to_string(),
                start: Instant::now(),
            };
            self.error_message = None;
        }
    }
}

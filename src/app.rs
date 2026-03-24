use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use eframe::egui;

use crate::db::{
    ColumnInfo, ConnectionParams, ConnectEvent, DatabaseInfo, DbBackend, QueryResult, TableInfo,
    connect_mysql_progress, escape_ident, escape_sql,
};
use crate::log::LogEntry;
use crate::profiles::{self, ConnectionProfile};
use crate::settings::{self, AppSettings, SavedSqlTab};
use crate::table_state::{self, TableStates};
use crate::ui::db_info::DbInfoAction;
use crate::ui::modals::{self, DropKind, ModalAction, ModalState, NewColumnDef, TruncateKind};
use crate::ui::results_grid::{EditingCell, GridAction, PendingEdit, PendingInsert, RowRef, SortDir, SortSpec};
use crate::ui::sidebar::SidebarAction;
use crate::ui::sql_editor::{SqlEditorAction, SqlTab};
use crate::ui::tab_bar::{ActiveTab, TabBarResult};
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
    /// Backend cedido temporalmente a un hilo de query; la UI sigue mostrando
    /// la pantalla connected (con spinner en el grid).
    QueryRunning,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SelectedNode {
    Server,
    Database(String),
    Table { db: String, table: String },
}

// ── Sub-structs de estado ─────────────────────────────────────────────────────

/// Árbol de esquema visible en el sidebar + cachés de info de servidor.
#[derive(Default)]
pub struct SchemaCache {
    pub databases: Vec<String>,
    pub tables_cache: HashMap<String, Vec<String>>,
    pub table_search: String,
    pub selected_node: Option<SelectedNode>,
    pub databases_info: Option<Vec<DatabaseInfo>>,
    pub tables_info: HashMap<String, Vec<TableInfo>>,
    pub columns_info: HashMap<(String, String), Vec<ColumnInfo>>,
}

/// Datos de la tabla activa: resultado de query, edición inline, selección de filas.
#[derive(Default)]
pub struct TableViewState {
    pub table_data: Option<QueryResult>,
    pub table_error: Option<String>,
    pub sort_specs: Vec<SortSpec>,
    pub sort_indices: Vec<usize>,
    pub filter_text: String,
    pub filter_history: Vec<String>,
    pub pending_edits: Vec<PendingEdit>,
    pub editing_cell: Option<EditingCell>,
    pub primary_keys: Vec<String>,
    pub pending_deletes: std::collections::BTreeSet<usize>,
    pub pending_inserts: Vec<PendingInsert>,
    pub selected_cell: Option<(usize, usize)>,
    pub selected_rows: std::collections::BTreeSet<usize>,
    pub last_selected_row: Option<usize>,
    pub drag_select_from: Option<usize>,
    pub query_loading: bool,
    pub query_receiver: Option<std::sync::mpsc::Receiver<crate::db::QueryEvent>>,
}

/// Estado de los paneles UI: visibilidad, modal, settings, log.
pub struct UiPanelState {
    pub show_sidebar: bool,
    pub show_log: bool,
    pub show_settings: bool,
    pub show_only_fav: bool,
    pub settings: AppSettings,
    pub settings_dirty: bool,
    pub modal: ModalState,
    pub query_log: std::collections::VecDeque<LogEntry>,
}

// ── Estado principal ─────────────────────────────────────────────────────────

pub struct RustqlApp {
    pub state: AppState,
    pub params: ConnectionParams,
    pub port_str: String,
    pub error_message: Option<String>,

    pub schema: SchemaCache,
    pub table_view: TableViewState,
    pub ui: UiPanelState,

    // Pestañas
    pub active_tab: ActiveTab,
    pub sql_tabs: Vec<SqlTab>,
    pub pinned_tables: Vec<(String, String)>,

    // Sesión / perfiles
    pub profiles: Vec<ConnectionProfile>,
    pub selected_profile_idx: Option<usize>,
    pub profile_name: String,
    pub save_password: bool,

    // Estado persistido por tabla (sort, filtro, historial)
    pub table_states: TableStates,

    /// Cambios pendientes guardados por tabla fijada (clave: "db/tabla").
    /// Persisten aunque el usuario cambie de tabla.
    pub pinned_pending_edits: HashMap<String, Vec<PendingEdit>>,
}

impl Default for RustqlApp {
    fn default() -> Self {
        let params = ConnectionParams::default();
        let port_str = params.port.to_string();
        let settings = settings::load();
        let show_sidebar = settings.show_sidebar;
        let show_log = settings.show_log;
        Self {
            state: AppState::Disconnected,
            port_str,
            params,
            error_message: None,
            schema: SchemaCache::default(),
            table_view: TableViewState::default(),
            ui: UiPanelState {
                show_sidebar,
                show_log,
                show_settings: false,
                show_only_fav: false,
                settings,
                settings_dirty: false,
                modal: ModalState::None,
                query_log: std::collections::VecDeque::new(),
            },
            active_tab: ActiveTab::default(),
            sql_tabs: vec![SqlTab::default()],
            pinned_tables: Vec::new(),
            profiles: profiles::load(),
            selected_profile_idx: None,
            profile_name: String::new(),
            save_password: false,
            table_states: table_state::load(),
            pinned_pending_edits: HashMap::new(),
        }
    }
}

// ── Métodos auxiliares ────────────────────────────────────────────────────────

impl RustqlApp {
    fn run_query_async(&mut self, sql: String) {
        let backend = match std::mem::replace(&mut self.state, AppState::QueryRunning) {
            AppState::Connected(b) => b,
            other => { self.state = other; return; }
        };
        let (tx, rx) = std::sync::mpsc::channel();
        self.table_view.query_receiver = Some(rx);
        self.table_view.query_loading = true;
        std::thread::spawn(move || {
            let start = std::time::Instant::now();
            let result = backend.execute_query(&sql);
            let elapsed_ms = start.elapsed().as_millis() as u64;
            let _ = tx.send(crate::db::QueryEvent::Done { result, backend, sql, elapsed_ms });
        });
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
        if self.ui.query_log.len() >= 500 {
            self.ui.query_log.pop_front();
        }
        self.ui.query_log.push_back(entry);
    }

    fn build_sql(&self, db: &str, table: &str) -> String {
        let filter = self.table_view.filter_text.trim();

        let order_by = if self.table_view.sort_specs.is_empty() {
            String::new()
        } else {
            let parts: Vec<String> = self.table_view.sort_specs.iter().map(|s| {
                let dir = if s.dir == SortDir::Desc { "DESC" } else { "ASC" };
                format!("`{}` {}", s.col_name.replace('`', "``"), dir)
            }).collect();
            format!(" ORDER BY {}", parts.join(", "))
        };

        let limit = self.ui.settings.query_limit;
        if filter.is_empty() {
            format!("SELECT * FROM `{}`.`{}`{} LIMIT {limit}", escape_ident(db), escape_ident(table), order_by)
        } else {
            format!(
                "SELECT * FROM `{}`.`{}` WHERE {filter}{} LIMIT {limit}",
                escape_ident(db),
                escape_ident(table),
                order_by
            )
        }
    }

    fn load_primary_keys(&mut self, db: &str, table: &str) {
        let result = match &mut self.state {
            AppState::Connected(b) => b.get_primary_keys(db, table),
            _ => return,
        };
        self.table_view.primary_keys = result.unwrap_or_default();
    }

    fn load_databases_info(&mut self) {
        let result = match &mut self.state {
            AppState::Connected(b) => b.get_databases_info(),
            _ => return,
        };
        self.schema.databases_info = result.ok();
    }

    fn load_tables_info(&mut self, db: &str) {
        let result = match &mut self.state {
            AppState::Connected(b) => b.get_tables_info(db),
            _ => return,
        };
        if let Ok(info) = result {
            self.schema.tables_info.insert(db.to_string(), info);
        }
    }

    fn load_columns_info(&mut self, db: &str, table: &str) {
        let result = match &mut self.state {
            AppState::Connected(b) => b.get_columns_info(db, table),
            _ => return,
        };
        if let Ok(cols) = result {
            self.schema.columns_info.insert((db.to_string(), table.to_string()), cols);
        }
    }

    fn maybe_load_tab_data(&mut self) {
        match self.active_tab {
            ActiveTab::Servidor => {
                if self.schema.databases_info.is_none() {
                    self.load_databases_info();
                }
            }
            ActiveTab::Bbdd => {
                let db = match &self.schema.selected_node {
                    Some(SelectedNode::Database(d)) => Some(d.clone()),
                    Some(SelectedNode::Table { db, .. }) => Some(db.clone()),
                    _ => None,
                };
                if let Some(db) = db
                    && !self.schema.tables_info.contains_key(&db) {
                        self.load_tables_info(&db);
                    }
            }
            ActiveTab::Tabla => {
                let key = match &self.schema.selected_node {
                    Some(SelectedNode::Table { db, table }) => {
                        Some((db.clone(), table.clone()))
                    }
                    _ => None,
                };
                if let Some((db, table)) = key {
                    let k = (db.clone(), table.clone());
                    if !self.schema.columns_info.contains_key(&k) {
                        self.load_columns_info(&db, &table);
                    }
                }
            }
            ActiveTab::Sql(_) | ActiveTab::Pinned(_) | ActiveTab::Datos => {}
        }
    }

    fn invalidate_after_write(&mut self, db: Option<&str>, table: Option<&str>) {
        // Invalidar bases de datos info
        self.schema.databases_info = None;

        if let Some(db) = db {
            // Invalidar caché de tablas de esa BD
            self.schema.tables_cache.remove(db);
            self.schema.tables_info.remove(db);
            // Recargar lista de bases de datos del sidebar
            self.schema.databases.clear();

            if let Some(table) = table {
                let key = (db.to_string(), table.to_string());
                self.schema.columns_info.remove(&key);

                // Si la tabla eliminada era la seleccionada, limpiar
                if matches!(
                    &self.schema.selected_node,
                    Some(SelectedNode::Table { db: d, table: t }) if d == db && t == table
                ) {
                    self.schema.selected_node = Some(SelectedNode::Database(db.to_string()));
                    self.table_view.table_data = None;
                    self.table_view.table_error = None;
                    self.table_view.pending_edits.clear();
                    self.table_view.pending_deletes.clear();
                    self.table_view.pending_inserts.clear();
                    self.table_view.editing_cell = None;
                    self.table_view.primary_keys.clear();
                }
            } else {
                // BD entera eliminada
                if matches!(
                    &self.schema.selected_node,
                    Some(SelectedNode::Database(d) | SelectedNode::Table { db: d, .. }) if d == db
                ) {
                    self.schema.selected_node = Some(SelectedNode::Server);
                    self.active_tab = ActiveTab::Servidor;
                    self.table_view.table_data = None;
                    self.table_view.table_error = None;
                    self.table_view.pending_edits.clear();
                    self.table_view.pending_deletes.clear();
                    self.table_view.pending_inserts.clear();
                    self.table_view.editing_cell = None;
                    self.table_view.primary_keys.clear();
                }
            }
        }
    }

    fn apply_pending_edits(&mut self) {
        self.apply_pending_changes();
    }

    fn apply_pending_changes(&mut self) {
        let (db, table) = match &self.schema.selected_node {
            Some(SelectedNode::Table { db, table }) => (db.clone(), table.clone()),
            _ => return,
        };

        // 1. Apply deletes (en transacción para garantizar atomicidad)
        if !self.table_view.pending_deletes.is_empty() {
            if self.table_view.primary_keys.is_empty() {
                self.ui.modal = modals::ModalState::Error {
                    title: "Error al eliminar".into(),
                    message: "No se puede eliminar: tabla sin clave primaria".into(),
                };
                return;
            }
            let data = match &self.table_view.table_data { Some(d) => d.clone(), None => return };
            let pks = self.table_view.primary_keys.clone();
            let deletes: Vec<usize> = self.table_view.pending_deletes.iter().cloned().collect();
            let mut stmts: Vec<String> = Vec::new();
            for row_idx in &deletes {
                let where_parts: Vec<String> = pks.iter().filter_map(|pk| {
                    let pk_idx = data.columns.iter().position(|c| c == pk)?;
                    let pk_val = data.rows.get(*row_idx)?.get(pk_idx)?;
                    Some(format!("`{}` = '{}'", pk.replace('`', "``"), escape_sql(pk_val.as_deref().unwrap_or(""))))
                }).collect();
                if where_parts.is_empty() { continue; }
                stmts.push(format!("DELETE FROM `{}`.`{}` WHERE {}",
                    db.replace('`', "``"), table.replace('`', "``"), where_parts.join(" AND ")));
            }
            let result = match &mut self.state {
                AppState::Connected(b) => b.execute_transaction(&stmts),
                _ => return,
            };
            match result {
                Ok(()) => {
                    let n = stmts.len();
                    self.push_log(LogEntry::ok(format!("DELETE x{n} (transacción)"), 0, n));
                    self.table_view.pending_deletes.clear();
                }
                Err(e) => {
                    self.push_log(LogEntry::err("DELETE (transacción)", 0, &e));
                    self.ui.modal = modals::ModalState::Error {
                        title: "Error al eliminar".into(),
                        message: e,
                    };
                }
            }
        }

        // 2. Apply edits (en transacción para garantizar atomicidad)
        if !self.table_view.pending_edits.is_empty() {
            if self.table_view.primary_keys.is_empty() {
                self.ui.modal = modals::ModalState::Error {
                    title: "Error al editar".into(),
                    message: "No se puede editar: la tabla no tiene clave primaria".into(),
                };
                return;
            }
            let data = match &self.table_view.table_data {
                Some(d) => d.clone(),
                None => return,
            };
            let edits = self.table_view.pending_edits.clone();
            let pks = self.table_view.primary_keys.clone();
            let mut stmts: Vec<String> = Vec::new();

            for edit in &edits {
                let col_name = match data.columns.get(edit.col_idx) {
                    Some(c) => c.clone(),
                    None => continue,
                };
                let where_parts: Vec<String> = pks.iter().filter_map(|pk| {
                    let pk_idx = data.columns.iter().position(|c| c == pk)?;
                    let pk_val = data.rows.get(edit.row_idx)?.get(pk_idx)?;
                    Some(format!("`{}` = '{}'", pk.replace('`', "``"), escape_sql(pk_val.as_deref().unwrap_or(""))))
                }).collect();
                if where_parts.is_empty() { continue; }
                let set_clause = if edit.is_null {
                    format!("`{}` = NULL", col_name.replace('`', "``"))
                } else {
                    format!("`{}` = '{}'", col_name.replace('`', "``"), escape_sql(&edit.new_value))
                };
                stmts.push(format!("UPDATE `{}`.`{}` SET {} WHERE {}",
                    db.replace('`', "``"), table.replace('`', "``"),
                    set_clause, where_parts.join(" AND ")));
            }
            let result = match &mut self.state {
                AppState::Connected(b) => b.execute_transaction(&stmts),
                _ => return,
            };
            match result {
                Ok(()) => {
                    let n = stmts.len();
                    self.push_log(LogEntry::ok(format!("UPDATE x{n} (transacción)"), 0, n));
                    self.table_view.pending_edits.clear();
                    self.table_view.editing_cell = None;
                    let key = Self::pinned_table_key(&db, &table);
                    self.pinned_pending_edits.remove(&key);
                }
                Err(e) => {
                    self.push_log(LogEntry::err("UPDATE (transacción)", 0, &e));
                    self.ui.modal = modals::ModalState::Error {
                        title: "Error al guardar".into(),
                        message: e,
                    };
                }
            }
        }

        // 3. Apply inserts (en transacción para garantizar atomicidad)
        if !self.table_view.pending_inserts.is_empty() {
            let col_names = self.table_view.table_data.as_ref().map(|d| d.columns.clone()).unwrap_or_default();
            let inserts = self.table_view.pending_inserts.clone();
            let mut stmts: Vec<String> = Vec::new();
            for ins in &inserts {
                let (cols, vals): (Vec<_>, Vec<_>) = col_names.iter().enumerate()
                    .filter(|(i, _)| !ins.auto_cols.get(*i).copied().unwrap_or(false))
                    .map(|(i, col)| {
                        let val = ins.values.get(i).and_then(|v| v.as_deref());
                        let col_sql = format!("`{}`", col.replace('`', "``"));
                        let val_sql = match val {
                            None => "NULL".to_string(),
                            Some(v) => format!("'{}'", escape_sql(v)),
                        };
                        (col_sql, val_sql)
                    }).unzip();
                if cols.is_empty() { continue; }
                stmts.push(format!("INSERT INTO `{}`.`{}` ({}) VALUES ({})",
                    db.replace('`', "``"), table.replace('`', "``"),
                    cols.join(", "), vals.join(", ")));
            }
            let result = match &mut self.state {
                AppState::Connected(b) => b.execute_transaction(&stmts),
                _ => return,
            };
            match result {
                Ok(()) => {
                    let n = stmts.len();
                    self.push_log(LogEntry::ok(format!("INSERT x{n} (transacción)"), 0, n));
                    self.table_view.pending_inserts.clear();
                }
                Err(e) => {
                    self.push_log(LogEntry::err("INSERT (transacción)", 0, &e));
                    self.ui.modal = modals::ModalState::Error {
                        title: "Error al insertar".into(),
                        message: e,
                    };
                }
            }
        }

        // 4. Reload if all clear
        if self.table_view.pending_deletes.is_empty() && self.table_view.pending_inserts.is_empty() && self.table_view.pending_edits.is_empty() {
            let sql = self.build_sql(&db, &table);
            self.run_query_async(sql);
        }
    }

    /// Persiste el sort+filtro+historial de la tabla actualmente seleccionada.
    fn save_table_state(&mut self) {
        if let Some(SelectedNode::Table { db, table }) = &self.schema.selected_node.clone() {
            let mut ts = table_state::get(&self.table_states, db, table);
            ts.sort_specs = self.table_view.sort_specs.clone();
            ts.filter_text = self.table_view.filter_text.clone();
            ts.filter_history = self.table_view.filter_history.clone();
            table_state::set(&mut self.table_states, db, table, ts);
            table_state::save(&self.table_states);
        }
    }

    pub fn pinned_table_key(db: &str, table: &str) -> String {
        format!("{}/{}", db, table)
    }

    fn is_table_pinned(&self, db: &str, table: &str) -> bool {
        self.pinned_tables.iter().any(|(d, t)| d == db && t == table)
    }

    /// Devuelve true si hay cambios pendientes en la tabla actual que NO está fijada,
    /// lo que requiere confirmación antes de cambiar de tabla.
    fn needs_confirm_before_switch(&self) -> bool {
        let has_pending = !self.table_view.pending_edits.is_empty()
            || !self.table_view.pending_deletes.is_empty()
            || !self.table_view.pending_inserts.is_empty();
        if !has_pending {
            return false;
        }
        match &self.schema.selected_node {
            Some(SelectedNode::Table { db, table }) => !self.is_table_pinned(db, table),
            _ => false,
        }
    }

    /// Si la tabla actual está fijada, guarda sus pending_edits en pinned_pending_edits.
    fn save_pinned_edits_if_needed(&mut self) {
        if let Some(SelectedNode::Table { db, table }) = &self.schema.selected_node.clone()
            && self.is_table_pinned(db, table) {
                let key = Self::pinned_table_key(db, table);
                if self.table_view.pending_edits.is_empty() {
                    self.pinned_pending_edits.remove(&key);
                } else {
                    self.pinned_pending_edits.insert(key, self.table_view.pending_edits.clone());
                }
            }
    }

    /// Si la nueva tabla está fijada, restaura sus pending_edits guardados.
    /// Si no está fijada, limpia los pending_edits.
    fn restore_pinned_edits_for(&mut self, db: &str, table: &str) {
        if self.is_table_pinned(db, table) {
            let key = Self::pinned_table_key(db, table);
            self.table_view.pending_edits = self.pinned_pending_edits
                .get(&key)
                .cloned()
                .unwrap_or_default();
        } else {
            self.table_view.pending_edits.clear();
        }
    }

    /// Cambia la tabla seleccionada: guarda el estado actual, restaura el
    /// de la nueva tabla, ejecuta la query con el filtro guardado y reaplica
    /// la ordenación guardada.
    fn on_table_selected(&mut self, db: String, table: String) {
        self.table_view.selected_cell = None;
        self.table_view.selected_rows.clear();
        self.table_view.last_selected_row = None;
        self.table_view.drag_select_from = None;
        self.table_view.pending_deletes.clear();
        self.table_view.pending_inserts.clear();

        // 1. Si la tabla actual está fijada, preservar sus cambios pendientes
        self.save_pinned_edits_if_needed();

        // 2. Guardar estado de la tabla anterior (sort, filtro, historial)
        self.save_table_state();

        // 3. Cargar estado de la nueva tabla
        let ts = table_state::get(&self.table_states, &db, &table);
        self.table_view.filter_text = ts.filter_text.clone();
        self.table_view.filter_history = ts.filter_history.clone();

        // 4. Actualizar selección
        self.schema.selected_node =
            Some(SelectedNode::Table { db: db.clone(), table: table.clone() });

        // 5. Restaurar pending_edits para la nueva tabla (solo si está fijada)
        self.restore_pinned_edits_for(&db, &table);
        self.table_view.editing_cell = None;

        // 6. Restaurar sort ANTES de build_sql para que el ORDER BY quede en la query
        self.table_view.sort_specs = ts.sort_specs.clone();

        // 7. Cargar claves primarias e info de columnas (antes de run_query_async, que cambia el estado a QueryRunning)
        self.load_primary_keys(&db, &table);
        let col_key = (db.clone(), table.clone());
        if !self.schema.columns_info.contains_key(&col_key) {
            self.load_columns_info(&db, &table);
        }

        // 8. Ejecutar query (server-side sort + filter restaurados)
        let sql = self.build_sql(&db, &table);
        self.run_query_async(sql);
        // sort_indices es identidad tras run_query (datos ya ordenados por servidor)
    }

    fn save_tabs_state(&mut self) {
        self.ui.settings.pinned_tables = self.pinned_tables.clone();
        self.ui.settings.sql_tabs = self.sql_tabs.iter().map(|t| SavedSqlTab {
            db_context: t.db_context.clone(),
            sql_text: t.sql_text.clone(),
        }).collect();
        settings::save(&self.ui.settings);
    }

    fn on_connected(&mut self, backend: Box<dyn DbBackend>) {
        self.state = AppState::Connected(backend);
        self.error_message = None;
        self.schema.databases.clear();
        self.schema.tables_cache.clear();
        self.schema.table_search.clear();
        self.schema.selected_node = None;
        self.active_tab = ActiveTab::default();
        self.table_view.table_data = None;
        self.table_view.pending_deletes.clear();
        self.table_view.pending_inserts.clear();
        self.table_view.selected_rows.clear();
        self.table_view.last_selected_row = None;
        self.table_view.drag_select_from = None;
        self.table_view.query_loading = false;
        self.table_view.query_receiver = None;
        self.table_view.table_error = None;
        self.table_view.sort_specs.clear();
        self.table_view.sort_indices.clear();
        self.table_view.filter_text.clear();
        self.table_view.pending_edits.clear();
        self.table_view.editing_cell = None;
        self.table_view.primary_keys.clear();
        self.ui.modal = ModalState::None;
        self.schema.databases_info = None;
        self.schema.tables_info.clear();
        self.schema.columns_info.clear();
        // Restaurar pestañas guardadas desde settings
        self.pinned_tables = self.ui.settings.pinned_tables.clone();
        self.sql_tabs = if self.ui.settings.sql_tabs.is_empty() {
            vec![SqlTab::default()]
        } else {
            self.ui.settings.sql_tabs.iter().map(|t| SqlTab {
                db_context: t.db_context.clone(),
                sql_text: t.sql_text.clone(),
                result: None,
            }).collect()
        };
        self.ui.query_log.clear();
        self.pinned_pending_edits.clear();
    }
}

// ── Loop principal ────────────────────────────────────────────────────────────

impl RustqlApp {
    /// Sondea el canal del hilo de conexión y actualiza el estado si terminó.
    fn poll_connection(&mut self, ctx: &egui::Context) {
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
    }

    /// Sondea el canal de query asíncrona y actualiza la vista si terminó.
    fn poll_query_receiver(&mut self, ctx: &egui::Context) {
        let mut query_done: Option<crate::db::QueryEvent> = None;
        if let Some(rx) = &self.table_view.query_receiver {
            match rx.try_recv() {
                Ok(evt) => query_done = Some(evt),
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(Duration::from_millis(50));
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.table_view.query_receiver = None;
                    self.table_view.query_loading = false;
                }
            }
        }
        if let Some(crate::db::QueryEvent::Done { result, backend, sql, elapsed_ms }) = query_done {
            self.table_view.query_receiver = None;
            self.table_view.query_loading = false;
            self.state = AppState::Connected(backend);
            match result {
                Ok(data) => {
                    let rows = data.rows.len();
                    self.push_log(LogEntry::ok(sql, elapsed_ms, rows));
                    self.table_view.sort_indices = (0..rows).collect();
                    self.table_view.table_data = Some(data);
                    self.table_view.table_error = None;
                }
                Err(e) => {
                    self.push_log(LogEntry::err(sql, elapsed_ms, &e));
                    self.table_view.table_data = None;
                    self.table_view.table_error = Some(e);
                    self.table_view.sort_indices.clear();
                }
            }
        }
    }

    /// Procesa la acción resultante del modal activo.
    fn handle_modal_action(&mut self, modal_action: ModalAction) {
        match modal_action {
            ModalAction::None => {}
            ModalAction::Cancel => {
                self.ui.modal = ModalState::None;
            }
            ModalAction::ApplyAndSwitch { new_db, new_table } => {
                self.ui.modal = ModalState::None;
                self.apply_pending_edits();
                if self.table_view.pending_edits.is_empty() {
                    self.on_table_selected(new_db, new_table);
                    self.active_tab = ActiveTab::Datos;
                }
            }
            ModalAction::DiscardAndSwitch { new_db, new_table } => {
                self.ui.modal = ModalState::None;
                self.table_view.pending_edits.clear();
                self.table_view.editing_cell = None;
                if let Some(SelectedNode::Table { db, table }) = &self.schema.selected_node.clone() {
                    let key = Self::pinned_table_key(db, table);
                    self.pinned_pending_edits.remove(&key);
                }
                self.on_table_selected(new_db, new_table);
                self.active_tab = ActiveTab::Datos;
            }
            ModalAction::CreateDatabase { name, charset } => {
                let result = match &mut self.state {
                    AppState::Connected(b) => b.create_database(&name, &charset),
                    _ => Err("No conectado".into()),
                };
                if let Ok(()) = &result {
                    self.push_log(LogEntry::ok(format!("CREATE DATABASE `{name}`"), 0, 0));
                } else if let Err(ref e) = result {
                    self.push_log(LogEntry::err(format!("CREATE DATABASE `{name}`"), 0, e));
                }
                match result {
                    Ok(()) => {
                        self.ui.modal = ModalState::None;
                        self.schema.databases.clear();
                        self.schema.databases_info = None;
                    }
                    Err(e) => {
                        if let ModalState::CreateDatabase { error, .. } = &mut self.ui.modal {
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
                        self.ui.modal = ModalState::None;
                        self.invalidate_after_write(Some(&db), None);
                    }
                    Err(e) => {
                        if let ModalState::CreateTable { error, .. } = &mut self.ui.modal {
                            *error = Some(e);
                        }
                    }
                }
            }
            ModalAction::DropDatabase(db) => {
                let result = match &mut self.state {
                    AppState::Connected(b) => b.drop_database(&db),
                    _ => Err("No conectado".into()),
                };
                if let Ok(()) = &result {
                    self.push_log(LogEntry::ok(format!("DROP DATABASE `{db}`"), 0, 0));
                } else if let Err(ref e) = result {
                    self.push_log(LogEntry::err(format!("DROP DATABASE `{db}`"), 0, e));
                }
                self.ui.modal = ModalState::None;
                if result.is_ok() {
                    self.invalidate_after_write(Some(&db), None);
                }
            }
            ModalAction::DropTable { db, table } => {
                let result = match &mut self.state {
                    AppState::Connected(b) => b.drop_table(&db, &table),
                    _ => Err("No conectado".into()),
                };
                if let Ok(()) = &result {
                    self.push_log(LogEntry::ok(format!("DROP TABLE `{db}`.`{table}`"), 0, 0));
                } else if let Err(ref e) = result {
                    self.push_log(LogEntry::err(format!("DROP TABLE `{db}`.`{table}`"), 0, e));
                }
                self.ui.modal = ModalState::None;
                if result.is_ok() {
                    self.invalidate_after_write(Some(&db), Some(&table));
                }
            }
            ModalAction::TruncateTable { db, table } => {
                let result = match &mut self.state {
                    AppState::Connected(b) => b.truncate_table(&db, &table),
                    _ => Err("No conectado".into()),
                };
                if let Ok(()) = &result {
                    self.push_log(LogEntry::ok(format!("TRUNCATE TABLE `{db}`.`{table}`"), 0, 0));
                } else if let Err(ref e) = result {
                    self.push_log(LogEntry::err(format!("TRUNCATE TABLE `{db}`.`{table}`"), 0, e));
                }
                let result = result.map(|_| 0u64);
                self.ui.modal = ModalState::None;
                if result.is_ok() {
                    if matches!(
                        &self.schema.selected_node,
                        Some(SelectedNode::Table { db: d, table: t }) if *d == db && *t == table
                    ) {
                        let sql = self.build_sql(&db, &table);
                        self.run_query_async(sql);
                    }
                    self.invalidate_after_write(Some(&db), Some(&table));
                }
            }
            ModalAction::TruncateAllTables(db) => {
                self.ui.modal = ModalState::None;
                let tables = self.schema.tables_cache.get(&db).cloned().unwrap_or_default();
                for table in &tables {
                    let _ = self.run_write(&format!(
                        "TRUNCATE TABLE `{}`.`{}`",
                        db.replace('`', "``"),
                        table.replace('`', "``")
                    ));
                }
                self.invalidate_after_write(Some(&db), None);
            }
            ModalAction::ApplyFilter { sql_fragment } => {
                self.ui.modal = ModalState::None;
                self.table_view.filter_text = sql_fragment;
                if let Some(SelectedNode::Table { db, table }) = &self.schema.selected_node.clone() {
                    let sql = self.build_sql(db, table);
                    self.run_query_async(sql);
                }
            }
            ModalAction::DeleteRow { db, table, row_idx } => {
                self.ui.modal = ModalState::None;
                let Some(data) = self.table_view.table_data.clone() else { return; };
                let pks = self.table_view.primary_keys.clone();
                let where_parts: Vec<String> = pks
                    .iter()
                    .filter_map(|pk| {
                        let pk_idx = data.columns.iter().position(|c| c == pk)?;
                        let pk_val = data.rows.get(row_idx)?.get(pk_idx)?;
                        Some(format!(
                            "`{}` = '{}'",
                            pk.replace('`', "``"),
                            escape_sql(pk_val.as_deref().unwrap_or(""))
                        ))
                    })
                    .collect();
                if !where_parts.is_empty() {
                    let sql = format!(
                        "DELETE FROM `{}`.`{}` WHERE {}",
                        db.replace('`', "``"),
                        table.replace('`', "``"),
                        where_parts.join(" AND ")
                    );
                    if self.run_write(&sql).is_ok() {
                        self.table_view.pending_edits.retain(|e| e.row_idx != row_idx);
                        let sql = self.build_sql(&db, &table);
                        self.run_query_async(sql);
                    }
                }
            }
            ModalAction::CloneTable { db, src_table, new_name, copy_data } => {
                let create_sql = format!(
                    "CREATE TABLE `{}`.`{}` LIKE `{}`.`{}`",
                    db.replace('`', "``"), new_name.replace('`', "``"),
                    db.replace('`', "``"), src_table.replace('`', "``")
                );
                match self.run_write(&create_sql) {
                    Ok(_) => {
                        if copy_data {
                            let insert_sql = format!(
                                "INSERT INTO `{}`.`{}` SELECT * FROM `{}`.`{}`",
                                db.replace('`', "``"), new_name.replace('`', "``"),
                                db.replace('`', "``"), src_table.replace('`', "``")
                            );
                            if let Err(e) = self.run_write(&insert_sql) {
                                if let ModalState::CloneTable { error, .. } = &mut self.ui.modal {
                                    *error = Some(e);
                                }
                                return;
                            }
                        }
                        self.ui.modal = ModalState::None;
                        self.invalidate_after_write(Some(&db), None);
                    }
                    Err(e) => {
                        if let ModalState::CloneTable { error, .. } = &mut self.ui.modal {
                            *error = Some(e);
                        }
                    }
                }
            }
            ModalAction::InsertRow { db, table, col_names, values, nulls } => {
                if self.table_view.table_data.is_none() { return; }
                let col_list: Vec<String> = col_names
                    .iter()
                    .map(|c| format!("`{}`", c.replace('`', "``")))
                    .collect();
                let val_list: Vec<String> = values
                    .iter()
                    .zip(nulls.iter())
                    .map(|(val, is_null)| {
                        if *is_null { "NULL".to_string() } else { format!("'{}'", escape_sql(val)) }
                    })
                    .collect();
                let sql = format!(
                    "INSERT INTO `{}`.`{}` ({}) VALUES ({})",
                    db.replace('`', "``"),
                    table.replace('`', "``"),
                    col_list.join(", "),
                    val_list.join(", ")
                );
                match self.run_write(&sql) {
                    Ok(_) => {
                        self.ui.modal = ModalState::None;
                        let sql = self.build_sql(&db, &table);
                        self.run_query_async(sql);
                    }
                    Err(e) => {
                        if let ModalState::InsertRow { error, .. } = &mut self.ui.modal {
                            *error = Some(e);
                        }
                    }
                }
            }
        }
    }
}

impl eframe::App for RustqlApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Aplicar ajustes si cambiaron
        if self.ui.settings_dirty {
            settings::setup_fonts(ctx, &self.ui.settings);
            settings::apply(ctx, &self.ui.settings);
            settings::save(&self.ui.settings);
            self.ui.settings_dirty = false;
        }

        let f9 = ctx.input(|i| i.key_pressed(egui::Key::F9));
        let f5 = ctx.input(|i| i.key_pressed(egui::Key::F5));
        let ctrl_enter =
            ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::Enter));

        // ── Proactive data loading for current tab ────────────────────────
        if matches!(&self.state, AppState::Connected(_)) {
            self.maybe_load_tab_data();
        }

        // ── Polling de hilos de fondo ─────────────────────────────────────
        self.poll_connection(ctx);
        self.poll_query_receiver(ctx);

        // Snapshot de visibilidad de paneles para detectar cambios
        let (prev_sidebar, prev_log) = (self.ui.show_sidebar, self.ui.show_log);

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
        let mut tab_bar_result = TabBarResult::default();
        let mut db_info_action = DbInfoAction::None;
        let mut sql_action = SqlEditorAction::None;
        let mut should_disconnect = false;

        let ctrl_t = ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::T));

        // ── Paneles inferiores (primero siempre) ─────────────────────────
        {
            let sb_host = match &self.state {
                AppState::Connected(_) => {
                    Some(format!("{}:{}", self.params.host, self.params.port))
                }
                AppState::Connecting { host, .. } => {
                    Some(format!("Conectando a {}…", host))
                }
                AppState::Disconnected => None,
                AppState::QueryRunning => Some(format!("{}:{}", self.params.host, self.params.port)),
            };
            let sb_db = match &self.schema.selected_node {
                Some(SelectedNode::Database(d)) => Some(d.clone()),
                Some(SelectedNode::Table { db, .. }) => Some(db.clone()),
                _ => None,
            };
            egui::TopBottomPanel::bottom("statusbar")
                .exact_height(26.0)
                .show(ctx, |ui| {
                    statusbar::show(
                        ui,
                        &mut self.ui.show_sidebar,
                        &mut self.ui.show_log,
                        &mut self.ui.show_settings,
                        sb_host.as_deref(),
                        sb_db.as_deref(),
                    );
                });
        }
        if matches!(self.state, AppState::Connected(_) | AppState::QueryRunning)

            && self.ui.show_log {
                egui::TopBottomPanel::bottom("log_panel")
                    .resizable(true)
                    .min_height(80.0)
                    .max_height(300.0)
                    .show(ctx, |ui| {
                        log_panel::show(ui, &self.ui.query_log);
                    });
            }

        // ── Panel de ajustes (derecha) ───────────────────────────────────
        if self.ui.show_settings {
            egui::SidePanel::right("settings_panel")
                .exact_width(240.0)
                .show(ctx, |ui| {
                    settings_changed = settings_panel::show(ui, &mut self.ui.settings);
                });
        }

        // ── Contenido principal según estado ─────────────────────────────
        match &mut self.state {
            AppState::Disconnected => {
                egui::SidePanel::left("conn_favorites")
                    .exact_width(200.0)
                    .show(ctx, |ui| {
                        let (sel, connect_now) = connection_panel::show_favorites(
                            ui,
                            &self.profiles,
                            &self.selected_profile_idx,
                        );
                        profile_selected = sel;
                        if connect_now {
                            connect_clicked = true;
                        }
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
                    connect_clicked |= result.connect;
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
                        let _ = connection_panel::show_favorites(
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
                if self.ui.show_sidebar {
                    let (sel_db, sel_table) = match &self.schema.selected_node {
                        Some(SelectedNode::Table { db, table }) => {
                            (Some(db.as_str()), Some(table.as_str()))
                        }
                        Some(SelectedNode::Database(db)) => (Some(db.as_str()), None),
                        _ => (None, None),
                    };

                    let server_name = self.selected_profile_idx
                        .and_then(|i| self.profiles.get(i))
                        .map(|p| p.name.clone())
                        .unwrap_or_else(|| self.params.host.clone());

                    egui::SidePanel::left("sidebar_v2")
                        .default_width(200.0)
                        .min_width(160.0)
                        .show(ctx, |ui| {
                            sidebar_action = sidebar::show(
                                ui,
                                Some(backend.as_ref()),
                                &mut self.schema.databases,
                                &mut self.schema.tables_cache,
                                sel_db,
                                sel_table,
                                &mut self.schema.table_search,
                                &self.ui.settings.fav_tables,
                                &mut self.ui.show_only_fav,
                                &self.params.host,
                                &server_name,
                            );
                        });
                }

                // ── Tab bar ───────────────────────────────────────────────
                let has_db = matches!(
                    &self.schema.selected_node,
                    Some(SelectedNode::Database(_) | SelectedNode::Table { .. })
                );
                let has_table =
                    matches!(&self.schema.selected_node, Some(SelectedNode::Table { .. }));

                {
                    let server_name_tab = self.selected_profile_idx
                        .and_then(|i| self.profiles.get(i))
                        .map(|p| p.name.clone())
                        .unwrap_or_else(|| self.params.host.clone());

                    let db_name_tab = match &self.schema.selected_node {
                        Some(SelectedNode::Database(d)) => Some(d.clone()),
                        Some(SelectedNode::Table { db, .. }) => Some(db.clone()),
                        _ => None,
                    };
                    let table_name_tab = match &self.schema.selected_node {
                        Some(SelectedNode::Table { table, .. }) => Some(table.clone()),
                        _ => None,
                    };

                    let sql_tab_count = self.sql_tabs.len();
                    let pinned_tables = self.pinned_tables.clone();

                    // Calcular cuántos cambios pendientes tiene cada tabla fijada
                    let pinned_pending_counts: Vec<usize> = pinned_tables.iter().map(|(db, table)| {
                        let key = Self::pinned_table_key(db, table);
                        // Si es la tabla actualmente abierta, usar los pending_edits en vivo
                        let is_current = matches!(
                            &self.schema.selected_node,
                            Some(SelectedNode::Table { db: d, table: t }) if d == db && t == table
                        );
                        if is_current {
                            self.table_view.pending_edits.len()
                        } else {
                            self.pinned_pending_edits.get(&key).map_or(0, |v| v.len())
                        }
                    }).collect();

                    egui::TopBottomPanel::top("tab_bar")
                        .min_height(30.0)
                        .resizable(false)
                        .show(ctx, |ui| {
                            ui.add_space(2.0);
                            tab_bar_result = tab_bar::show(
                                ui,
                                self.active_tab,
                                has_db,
                                has_table,
                                &server_name_tab,
                                db_name_tab.as_deref(),
                                table_name_tab.as_deref(),
                                sql_tab_count,
                                &pinned_tables,
                                &pinned_pending_counts,
                            );
                        });
                }

                // ── Apply bar (solo en tab Datos con cambios pendientes) ──
                let has_pending = !self.table_view.pending_edits.is_empty()
                    || !self.table_view.pending_deletes.is_empty()
                    || !self.table_view.pending_inserts.is_empty();
                if self.active_tab == ActiveTab::Datos && has_pending {
                    // pk_missing solo bloquea si hay edits pendientes (DELETE e INSERT no requieren PK en la UI)
                    let pk_missing = self.table_view.table_data.is_some()
                        && self.table_view.primary_keys.is_empty()
                        && !self.table_view.pending_edits.is_empty();
                    egui::TopBottomPanel::bottom("apply_bar")
                        .exact_height(36.0)
                        .show(ctx, |ui| {
                            apply_bar_action = results_grid::show_apply_bar(
                                ui,
                                &self.table_view.pending_edits,
                                self.table_view.pending_deletes.len(),
                                self.table_view.pending_inserts.len(),
                                ctrl_enter,
                                pk_missing,
                            );
                        });
                }

                // ── Panel central ─────────────────────────────────────────
                egui::CentralPanel::default().show(ctx, |ui| {
                    match self.active_tab {
                        ActiveTab::Servidor => {
                            let dbs = self.schema.databases_info.as_deref().unwrap_or(&[]);
                            server_info::show(ui, &self.params.host, dbs);
                        }

                        ActiveTab::Bbdd => {
                            let db_name = match &self.schema.selected_node {
                                Some(SelectedNode::Database(d)) => Some(d.clone()),
                                Some(SelectedNode::Table { db, .. }) => Some(db.clone()),
                                _ => None,
                            };
                            if let Some(db) = &db_name {
                                let empty = Vec::new();
                                let tables =
                                    self.schema.tables_info.get(db).unwrap_or(&empty);
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
                                &self.schema.selected_node.clone()
                            {
                                let key = (db.clone(), table.clone());
                                let empty = Vec::new();
                                let cols = self.schema.columns_info.get(&key).unwrap_or(&empty);
                                table_structure::show(ui, db, table, cols);
                            } else {
                                ui.centered_and_justified(|ui| {
                                    ui.label("Selecciona una tabla en el panel izquierdo");
                                });
                            }
                        }

                        ActiveTab::Datos => {
                            let selected_table = match &self.schema.selected_node {
                                Some(SelectedNode::Table { db, table }) => {
                                    Some((db.clone(), table.clone()))
                                }
                                _ => None,
                            };
                            if selected_table.is_some() {
                                let apply_f9 = f9 && self.active_tab == ActiveTab::Datos;
                                filter_action = filter_bar::show(
                                    ui,
                                    &mut self.table_view.filter_text,
                                    apply_f9,
                                    &self.table_view.filter_history.clone(),
                                );
                                ui.separator();
                            }
                            let col_info_key = match &self.schema.selected_node {
                                Some(SelectedNode::Table { db, table }) => Some((db.clone(), table.clone())),
                                _ => None,
                            };
                            let empty_cols: Vec<ColumnInfo> = Vec::new();
                            let col_info_slice = col_info_key.as_ref()
                                .and_then(|k| self.schema.columns_info.get(k))
                                .map(|v| v.as_slice())
                                .unwrap_or(empty_cols.as_slice());
                            grid_action = results_grid::show(
                                ui,
                                &selected_table,
                                &self.table_view.table_data,
                                &self.table_view.table_error,
                                &mut self.table_view.sort_specs,
                                &self.table_view.sort_indices,
                                &self.table_view.pending_edits,
                                &self.table_view.pending_deletes,
                                &self.table_view.pending_inserts,
                                &mut self.table_view.editing_cell,
                                &mut self.table_view.selected_cell,
                                &mut self.table_view.selected_rows,
                                &mut self.table_view.last_selected_row,
                                &mut self.table_view.drag_select_from,
                                &self.table_view.primary_keys,
                                col_info_slice,
                                self.table_view.query_loading,
                            );
                        }

                        ActiveTab::Sql(tab_idx) => {
                            if let Some(tab) = self.sql_tabs.get_mut(tab_idx) {
                                let apply_f9 = f9 && matches!(self.active_tab, ActiveTab::Sql(_));
                                let result_clone = tab.result.clone();
                                let databases_clone = self.schema.databases.clone();
                                sql_action = sql_editor::show(
                                    ui,
                                    &mut tab.sql_text,
                                    &result_clone,
                                    apply_f9,
                                    &mut tab.db_context,
                                    &databases_clone,
                                );
                            }
                        }

                        ActiveTab::Pinned(idx) => {
                            if let Some((pin_db, pin_table)) = self.pinned_tables.get(idx).cloned() {
                                ui.centered_and_justified(|ui| {
                                    ui.vertical_centered(|ui| {
                                        ui.strong(format!("Tabla fijada: {pin_db}.{pin_table}"));
                                        ui.add_space(8.0);
                                        if ui.button("Ir a datos").clicked() {
                                            // Will be handled after render
                                        }
                                        ui.label("(Haz doble-click en la tabla del sidebar para cargar datos)");
                                    });
                                });
                            }
                        }
                    }
                });
            }

            AppState::QueryRunning => {
                // Renderizamos el mismo layout que Connected pero sin backend disponible.
                // El sidebar funciona con la caché ya cargada (None = no hacer queries).
                if self.ui.show_sidebar {
                    let (sel_db, sel_table) = match &self.schema.selected_node {
                        Some(SelectedNode::Table { db, table }) => {
                            (Some(db.as_str()), Some(table.as_str()))
                        }
                        Some(SelectedNode::Database(db)) => (Some(db.as_str()), None),
                        _ => (None, None),
                    };
                    let server_name = self.selected_profile_idx
                        .and_then(|i| self.profiles.get(i))
                        .map(|p| p.name.clone())
                        .unwrap_or_else(|| self.params.host.clone());
                    egui::SidePanel::left("sidebar_v2")
                        .default_width(200.0)
                        .min_width(160.0)
                        .show(ctx, |ui| {
                            sidebar_action = sidebar::show(
                                ui,
                                None,
                                &mut self.schema.databases,
                                &mut self.schema.tables_cache,
                                sel_db,
                                sel_table,
                                &mut self.schema.table_search,
                                &self.ui.settings.fav_tables,
                                &mut self.ui.show_only_fav,
                                &self.params.host,
                                &server_name,
                            );
                        });
                }

                let has_db = matches!(
                    &self.schema.selected_node,
                    Some(SelectedNode::Database(_) | SelectedNode::Table { .. })
                );
                let has_table =
                    matches!(&self.schema.selected_node, Some(SelectedNode::Table { .. }));
                {
                    let server_name_tab = self.selected_profile_idx
                        .and_then(|i| self.profiles.get(i))
                        .map(|p| p.name.clone())
                        .unwrap_or_else(|| self.params.host.clone());
                    let db_name_tab = match &self.schema.selected_node {
                        Some(SelectedNode::Database(d)) => Some(d.clone()),
                        Some(SelectedNode::Table { db, .. }) => Some(db.clone()),
                        _ => None,
                    };
                    let table_name_tab = match &self.schema.selected_node {
                        Some(SelectedNode::Table { table, .. }) => Some(table.clone()),
                        _ => None,
                    };
                    let sql_tab_count = self.sql_tabs.len();
                    let pinned_tables = self.pinned_tables.clone();
                    let pinned_pending_counts: Vec<usize> = pinned_tables.iter().map(|(db, table)| {
                        let key = Self::pinned_table_key(db, table);
                        let is_current = matches!(
                            &self.schema.selected_node,
                            Some(SelectedNode::Table { db: d, table: t }) if d == db && t == table
                        );
                        if is_current { self.table_view.pending_edits.len() }
                        else { self.pinned_pending_edits.get(&key).map_or(0, |v| v.len()) }
                    }).collect();
                    egui::TopBottomPanel::top("tab_bar")
                        .min_height(30.0)
                        .resizable(false)
                        .show(ctx, |ui| {
                            ui.add_space(2.0);
                            tab_bar_result = tab_bar::show(
                                ui,
                                self.active_tab,
                                has_db,
                                has_table,
                                &server_name_tab,
                                db_name_tab.as_deref(),
                                table_name_tab.as_deref(),
                                sql_tab_count,
                                &pinned_tables,
                                &pinned_pending_counts,
                            );
                        });
                }

                egui::CentralPanel::default().show(ctx, |ui| {
                    match self.active_tab {
                        ActiveTab::Datos => {
                            let selected_table = match &self.schema.selected_node {
                                Some(SelectedNode::Table { db, table }) => Some((db.clone(), table.clone())),
                                _ => None,
                            };
                            if selected_table.is_some() {
                                filter_bar::show(ui, &mut self.table_view.filter_text, false, &self.table_view.filter_history.clone());
                                ui.separator();
                            }
                            let col_info_key = match &self.schema.selected_node {
                                Some(SelectedNode::Table { db, table }) => Some((db.clone(), table.clone())),
                                _ => None,
                            };
                            let empty_cols: Vec<ColumnInfo> = Vec::new();
                            let col_info_slice = col_info_key.as_ref()
                                .and_then(|k| self.schema.columns_info.get(k))
                                .map(|v| v.as_slice())
                                .unwrap_or(empty_cols.as_slice());
                            results_grid::show(
                                ui,
                                &selected_table,
                                &self.table_view.table_data,
                                &self.table_view.table_error,
                                &mut self.table_view.sort_specs,
                                &self.table_view.sort_indices,
                                &self.table_view.pending_edits,
                                &self.table_view.pending_deletes,
                                &self.table_view.pending_inserts,
                                &mut self.table_view.editing_cell,
                                &mut self.table_view.selected_cell,
                                &mut self.table_view.selected_rows,
                                &mut self.table_view.last_selected_row,
                                &mut self.table_view.drag_select_from,
                                &self.table_view.primary_keys,
                                col_info_slice,
                                true, // query_loading=true: muestra spinner en el grid
                            );
                        }
                        _ => {
                            ui.centered_and_justified(|ui| {
                                ui.spinner();
                                ui.label("Ejecutando consulta…");
                            });
                        }
                    }
                });
            }
        }

        // ── Modal (sobre todo) ────────────────────────────────────────────
        let modal_action = modals_ui::show(ctx, &mut self.ui.modal);

        // ── Aplicar señales ───────────────────────────────────────────────

        if cancel_connect {
            self.state = AppState::Disconnected;
        }

        if settings_changed {
            self.ui.settings_dirty = true;
        }

        // Persistir visibilidad de paneles cuando cambia
        if self.ui.show_sidebar != prev_sidebar || self.ui.show_log != prev_log {
            self.ui.settings.show_sidebar = self.ui.show_sidebar;
            self.ui.settings.show_log = self.ui.show_log;
            settings::save(&self.ui.settings);
        }

        // Handle tab bar result
        if let Some(t) = tab_bar_result.tab_clicked {
            match t {
                ActiveTab::Pinned(idx) => {
                    if let Some((pin_db, pin_table)) = self.pinned_tables.get(idx).cloned() {
                        let already_selected = matches!(
                            &self.schema.selected_node,
                            Some(SelectedNode::Table { db, table }) if *db == pin_db && *table == pin_table
                        );
                        if !already_selected {
                            if self.needs_confirm_before_switch() {
                                let current_table = match &self.schema.selected_node {
                                    Some(SelectedNode::Table { table: t, .. }) => t.clone(),
                                    _ => String::new(),
                                };
                                let count = self.table_view.pending_edits.len();
                                self.ui.modal = modals::ModalState::ConfirmSwitchTable {
                                    new_db: pin_db,
                                    new_table: pin_table,
                                    pending_count: count,
                                    current_table,
                                };
                            } else {
                                self.on_table_selected(pin_db, pin_table);
                                self.active_tab = ActiveTab::Datos;
                            }
                        } else {
                            self.active_tab = ActiveTab::Datos;
                        }
                    }
                }
                other => self.active_tab = other,
            }
        }

        if tab_bar_result.new_sql_tab || ctrl_t {
            let db_ctx = match &self.schema.selected_node {
                Some(SelectedNode::Database(d)) => d.clone(),
                Some(SelectedNode::Table { db, .. }) => db.clone(),
                _ => String::new(),
            };
            let new_idx = self.sql_tabs.len();
            self.sql_tabs.push(SqlTab {
                db_context: db_ctx,
                sql_text: String::new(),
                result: None,
            });
            self.active_tab = ActiveTab::Sql(new_idx);
            self.save_tabs_state();
        }

        if let Some(close_idx) = tab_bar_result.close_sql_tab
            && close_idx < self.sql_tabs.len() {
                self.sql_tabs.remove(close_idx);
                if self.sql_tabs.is_empty() {
                    self.sql_tabs.push(SqlTab::default());
                }
                self.active_tab = match self.active_tab {
                    ActiveTab::Sql(i) if i >= close_idx => {
                        let new_i = if i > 0 { i.saturating_sub(1) } else { 0 };
                        ActiveTab::Sql(new_i.min(self.sql_tabs.len().saturating_sub(1)))
                    }
                    other => other,
                };
                self.save_tabs_state();
            }

        if let Some(close_idx) = tab_bar_result.close_pinned
            && close_idx < self.pinned_tables.len() {
                self.pinned_tables.remove(close_idx);
                if let ActiveTab::Pinned(i) = self.active_tab
                    && i >= close_idx {
                        self.active_tab = if self.pinned_tables.is_empty() {
                            ActiveTab::Datos
                        } else {
                            ActiveTab::Pinned(i.saturating_sub(1).min(self.pinned_tables.len().saturating_sub(1)))
                        };
                    }
                self.save_tabs_state();
            }

        // Grid action
        match grid_action {
            GridAction::SortChanged => {
                // Re-ejecutar con ORDER BY para sort server-side (LIMIT correcto)
                if let Some(SelectedNode::Table { db, table }) = &self.schema.selected_node.clone() {
                    let sql = self.build_sql(db, table);
                    self.run_query_async(sql);
                }
                self.save_table_state();
            }
            GridAction::CellEditStarted { row, col_idx, current_value } => {
                self.table_view.editing_cell = Some(EditingCell {
                    row,
                    col_idx,
                    buffer: current_value,
                    needs_focus: true,
                });
            }
            GridAction::CellSetNull { row, col_idx } => {
                self.table_view.editing_cell = None;
                match row {
                    RowRef::Insert(insert_idx) => {
                        if let Some(ins) = self.table_view.pending_inserts.get_mut(insert_idx)
                            && col_idx < ins.values.len() {
                                ins.values[col_idx] = None;
                            }
                    }
                    RowRef::Existing(row_idx) => {
                        let original = self
                            .table_view.table_data
                            .as_ref()
                            .and_then(|d| d.rows.get(row_idx))
                            .and_then(|r| r.get(col_idx))
                            .cloned()
                            .unwrap_or_default();
                        self.table_view.pending_edits
                            .retain(|e| !(e.row_idx == row_idx && e.col_idx == col_idx));
                        self.table_view.pending_edits.push(PendingEdit {
                            row_idx,
                            col_idx,
                            original,
                            new_value: String::new(),
                            is_null: true,
                        });
                    }
                }
            }
            GridAction::CellEditCommitted { row, col_idx, new_value } => {
                self.table_view.editing_cell = None;
                match row {
                    RowRef::Insert(insert_idx) => {
                        if let Some(ins) = self.table_view.pending_inserts.get_mut(insert_idx)
                            && col_idx < ins.values.len() {
                                ins.values[col_idx] = Some(new_value);
                            }
                    }
                    RowRef::Existing(row_idx) => {
                        let original = self
                            .table_view.table_data
                            .as_ref()
                            .and_then(|d| d.rows.get(row_idx))
                            .and_then(|r| r.get(col_idx))
                            .cloned()
                            .unwrap_or_default();

                        if original.as_deref() == Some(new_value.as_str()) {
                            self.table_view.pending_edits
                                .retain(|e| !(e.row_idx == row_idx && e.col_idx == col_idx));
                        } else if let Some(existing) = self
                            .table_view.pending_edits
                            .iter_mut()
                            .find(|e| e.row_idx == row_idx && e.col_idx == col_idx)
                        {
                            if existing.original.as_deref() == Some(new_value.as_str()) && !existing.is_null {
                                let idx = self
                                    .table_view.pending_edits
                                    .iter()
                                    .position(|e| e.row_idx == row_idx && e.col_idx == col_idx)
                                    .unwrap();
                                self.table_view.pending_edits.remove(idx);
                            } else {
                                existing.new_value = new_value;
                                existing.is_null = false;
                            }
                        } else {
                            self.table_view.pending_edits.push(PendingEdit {
                                row_idx,
                                col_idx,
                                original,
                                new_value,
                                is_null: false,
                            });
                        }
                    }
                }
            }
            GridAction::CellEditCancelled => {
                self.table_view.editing_cell = None;
            }
            GridAction::CellEditTabNext { row, col_idx, new_value } => {
                // Commit current value, then move focus to next column
                match row {
                    RowRef::Insert(insert_idx) => {
                        if let Some(ins) = self.table_view.pending_inserts.get_mut(insert_idx)
                            && col_idx < ins.values.len() {
                                ins.values[col_idx] = Some(new_value);
                            }
                    }
                    RowRef::Existing(row_idx) => {
                        let original = self
                            .table_view.table_data
                            .as_ref()
                            .and_then(|d| d.rows.get(row_idx))
                            .and_then(|r| r.get(col_idx))
                            .cloned()
                            .unwrap_or_default();
                        if original.as_deref() != Some(new_value.as_str()) {
                            self.table_view.pending_edits.retain(|e| !(e.row_idx == row_idx && e.col_idx == col_idx));
                            self.table_view.pending_edits.push(PendingEdit {
                                row_idx, col_idx, original, new_value, is_null: false,
                            });
                        }
                    }
                }
                // Move to next column
                let col_count = self.table_view.table_data.as_ref().map(|d| d.columns.len()).unwrap_or(1);
                let next_col = (col_idx + 1) % col_count;
                let next_val = match row {
                    RowRef::Insert(insert_idx) => self
                        .table_view.pending_inserts
                        .get(insert_idx)
                        .and_then(|ins| ins.values.get(next_col))
                        .and_then(|v| v.as_deref())
                        .unwrap_or("")
                        .to_string(),
                    RowRef::Existing(row_idx) => self
                        .table_view.table_data
                        .as_ref()
                        .and_then(|d| d.rows.get(row_idx))
                        .and_then(|r| r.get(next_col))
                        .and_then(|v| v.as_deref())
                        .unwrap_or("")
                        .to_string(),
                };
                self.table_view.editing_cell = Some(EditingCell {
                    row,
                    col_idx: next_col,
                    buffer: next_val,
                    needs_focus: true,
                });
            }
            GridAction::ApplyEdits => {
                self.apply_pending_changes();
            }
            GridAction::DiscardEdits => {
                self.table_view.pending_edits.clear();
                self.table_view.pending_deletes.clear();
                self.table_view.pending_inserts.clear();
                self.table_view.editing_cell = None;
                if let Some(SelectedNode::Table { db, table }) = &self.schema.selected_node.clone() {
                    let key = Self::pinned_table_key(db, table);
                    self.pinned_pending_edits.remove(&key);
                }
            }
            GridAction::FilterBy { sql_fragment } => {
                self.table_view.filter_text = sql_fragment;
                if let Some(SelectedNode::Table { db, table }) = &self.schema.selected_node.clone() {
                    let sql = self.build_sql(db, table);
                    self.run_query_async(sql);
                }
            }
            GridAction::FilterByClipboard { col_name } => {
                match arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
                    Ok(text) => {
                        let esc_col = format!("`{}`", col_name.replace('`', "``"));
                        let esc_val = escape_sql(&text);
                        self.table_view.filter_text = format!("{esc_col} = '{esc_val}'");
                        if let Some(SelectedNode::Table { db, table }) = &self.schema.selected_node.clone() {
                            let sql = self.build_sql(db, table);
                            self.run_query_async(sql);
                        }
                    }
                    Err(e) => {
                        self.table_view.table_error = Some(format!("No se pudo leer el portapapeles: {e}"));
                    }
                }
            }
            GridAction::FilterUserInput { col_name } => {
                self.ui.modal = ModalState::FilterInput {
                    col_name,
                    value: String::new(),
                    operator: modals::FilterOperator::default(),
                };
            }
            GridAction::MarkDeleteRow { row_idx } => {
                if !self.table_view.pending_deletes.remove(&row_idx) {
                    self.table_view.pending_deletes.insert(row_idx);
                }
            }
            GridAction::InsertRow => {
                // Create blank insert row at the bottom
                let col_count = self.table_view.table_data.as_ref().map(|d| d.columns.len()).unwrap_or(0);
                if col_count == 0 { } else {
                    let col_info_key = match &self.schema.selected_node {
                        Some(SelectedNode::Table { db, table }) => Some((db.clone(), table.clone())),
                        _ => None,
                    };
                    let empty_cols: Vec<ColumnInfo> = Vec::new();
                    let ci = col_info_key.as_ref()
                        .and_then(|k| self.schema.columns_info.get(k))
                        .map(|v| v.as_slice())
                        .unwrap_or(empty_cols.as_slice());
                    let auto_cols: Vec<bool> = (0..col_count).map(|i| {
                        ci.get(i).map(|c| c.extra.contains("auto_increment")).unwrap_or(false)
                    }).collect();
                    let values: Vec<Option<String>> = (0..col_count).map(|i| {
                        if auto_cols.get(i).copied().unwrap_or(false) {
                            None
                        } else {
                            ci.get(i).and_then(|c| c.default_val.clone()).map(Some).unwrap_or(Some(String::new()))
                        }
                    }).collect();
                    self.table_view.pending_inserts.push(PendingInsert { values, auto_cols });
                }
            }
            GridAction::CloneRow { row_idx } => {
                let col_count = self.table_view.table_data.as_ref().map(|d| d.columns.len()).unwrap_or(0);
                if col_count == 0 { } else {
                    let col_info_key = match &self.schema.selected_node {
                        Some(SelectedNode::Table { db, table }) => Some((db.clone(), table.clone())),
                        _ => None,
                    };
                    let empty_cols: Vec<ColumnInfo> = Vec::new();
                    let ci = col_info_key.as_ref()
                        .and_then(|k| self.schema.columns_info.get(k))
                        .map(|v| v.as_slice())
                        .unwrap_or(empty_cols.as_slice());
                    let auto_cols: Vec<bool> = (0..col_count).map(|i| {
                        ci.get(i).map(|c| c.extra.contains("auto_increment")).unwrap_or(false)
                    }).collect();
                    let row_data = self.table_view.table_data.as_ref().and_then(|d| d.rows.get(row_idx));
                    let values: Vec<Option<String>> = (0..col_count).map(|i| {
                        if auto_cols.get(i).copied().unwrap_or(false) {
                            None // Skip auto_increment columns
                        } else {
                            row_data.and_then(|r| r.get(i)).cloned().unwrap_or(None)
                        }
                    }).collect();
                    self.table_view.pending_inserts.push(PendingInsert { values, auto_cols });
                }
            }
            GridAction::RemoveInsertRow { insert_idx } => {
                if insert_idx < self.table_view.pending_inserts.len() {
                    self.table_view.pending_inserts.remove(insert_idx);
                }
            }
            GridAction::RowSelected { row_idx, shift, ctrl } => {
                if shift {
                    let anchor = self.table_view.last_selected_row.unwrap_or(row_idx);
                    let (lo, hi) = (anchor.min(row_idx), anchor.max(row_idx));
                    self.table_view.selected_rows = (lo..=hi).collect();
                } else if ctrl {
                    if !self.table_view.selected_rows.remove(&row_idx) {
                        self.table_view.selected_rows.insert(row_idx);
                        self.table_view.last_selected_row = Some(row_idx);
                    }
                } else {
                    self.table_view.selected_rows.clear();
                    self.table_view.selected_rows.insert(row_idx);
                    self.table_view.last_selected_row = Some(row_idx);
                    self.table_view.drag_select_from = Some(row_idx);
                }
                self.table_view.selected_cell = None;
            }
            GridAction::CopySelectedRows => {
                if let Some(data) = &self.table_view.table_data {
                    let fmt = self.ui.settings.copy_format.clone();
                    let rows: Vec<&Vec<Option<String>>> = self.table_view.selected_rows.iter()
                        .filter_map(|&i| data.rows.get(i))
                        .collect();
                    if !rows.is_empty() {
                        let text = results_grid::format_rows_for_copy(&data.columns, &rows, &fmt);
                        ctx.copy_text(text);
                    }
                }
            }
            GridAction::CellSelected { row_idx, col_idx } => {
                self.table_view.selected_cell = Some((row_idx, col_idx));
                self.table_view.selected_rows.clear();
                self.table_view.last_selected_row = None;
            }
            GridAction::MarkDeleteRows { row_indices } => {
                for row_idx in row_indices {
                    if !self.table_view.pending_deletes.remove(&row_idx) {
                        self.table_view.pending_deletes.insert(row_idx);
                    }
                }
            }
            GridAction::None => {}
        }

        // Apply bar action (solo visible cuando hay cambios pendientes)
        match apply_bar_action {
            GridAction::ApplyEdits => self.apply_pending_changes(),
            GridAction::DiscardEdits => {
                self.table_view.pending_edits.clear();
                self.table_view.pending_deletes.clear();
                self.table_view.pending_inserts.clear();
                self.table_view.editing_cell = None;
                if let Some(SelectedNode::Table { db, table }) = &self.schema.selected_node.clone() {
                    let key = Self::pinned_table_key(db, table);
                    self.pinned_pending_edits.remove(&key);
                }
            }
            _ => {}
        }

        // Sidebar action
        match sidebar_action {
            SidebarAction::None => {}
            SidebarAction::SelectServer => {
                self.schema.selected_node = Some(SelectedNode::Server);
                self.active_tab = ActiveTab::Servidor;
            }
            SidebarAction::SelectDatabase(db) => {
                self.schema.selected_node = Some(SelectedNode::Database(db.clone()));
                self.active_tab = ActiveTab::Bbdd;
                if !self.schema.tables_info.contains_key(&db) {
                    self.load_tables_info(&db);
                }
            }
            SidebarAction::SelectTable { db, table } => {
                let already_selected = matches!(
                    &self.schema.selected_node,
                    Some(SelectedNode::Table { db: d, table: t }) if *d == db && *t == table
                );
                if !already_selected {
                    if self.needs_confirm_before_switch() {
                        let current_table = match &self.schema.selected_node {
                            Some(SelectedNode::Table { table: t, .. }) => t.clone(),
                            _ => String::new(),
                        };
                        let count = self.table_view.pending_edits.len();
                        self.ui.modal = modals::ModalState::ConfirmSwitchTable {
                            new_db: db,
                            new_table: table,
                            pending_count: count,
                            current_table,
                        };
                    } else {
                        self.on_table_selected(db, table);
                        self.active_tab = ActiveTab::Datos;
                    }
                } else {
                    self.active_tab = ActiveTab::Datos;
                }
            }
            SidebarAction::CreateDatabase => {
                self.ui.modal = ModalState::CreateDatabase {
                    name: String::new(),
                    charset: "utf8mb4".into(),
                    error: None,
                };
            }
            SidebarAction::CreateTable { db } => {
                self.ui.modal = ModalState::CreateTable {
                    db,
                    name: String::new(),
                    columns: vec![NewColumnDef::new()],
                    error: None,
                };
            }
            SidebarAction::DropDatabase { db } => {
                let target = db.clone();
                self.ui.modal = ModalState::ConfirmDrop {
                    kind: DropKind::Database,
                    target,
                    db: Some(db),
                    table: None,
                };
            }
            SidebarAction::TruncateAllTables { db } => {
                let target = format!("todas las tablas de '{db}'");
                self.ui.modal = ModalState::ConfirmTruncate {
                    kind: TruncateKind::AllInDatabase,
                    target,
                    db: Some(db),
                    table: None,
                };
            }
            SidebarAction::DropTable { db, table } => {
                let target = format!("{db}.{table}");
                self.ui.modal = ModalState::ConfirmDrop {
                    kind: DropKind::Table,
                    target,
                    db: Some(db),
                    table: Some(table),
                };
            }
            SidebarAction::TruncateTable { db, table } => {
                let target = format!("{db}.{table}");
                self.ui.modal = ModalState::ConfirmTruncate {
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
                self.schema.databases.clear();
                self.schema.tables_cache.clear();
                self.schema.databases_info = None;
                self.schema.tables_info.clear();
            }
            SidebarAction::Disconnect => {
                should_disconnect = true;
            }
            SidebarAction::ToggleFav { db, table } => {
                let key = format!("{}/{}", db, table);
                if self.ui.settings.fav_tables.contains(&key) {
                    self.ui.settings.fav_tables.remove(&key);
                } else {
                    self.ui.settings.fav_tables.insert(key);
                }
                settings::save(&self.ui.settings);
            }
            SidebarAction::PinTable { db, table } => {
                let already_pinned = self.pinned_tables.iter().any(|(d, t)| *d == db && *t == table);
                if !already_pinned {
                    self.pinned_tables.push((db, table));
                    self.save_tabs_state();
                }
            }
            SidebarAction::OpenTableStructure { db, table } => {
                let already_selected = matches!(
                    &self.schema.selected_node,
                    Some(SelectedNode::Table { db: d, table: t }) if *d == db && *t == table
                );
                if !already_selected {
                    if self.needs_confirm_before_switch() {
                        let current_table = match &self.schema.selected_node {
                            Some(SelectedNode::Table { table: t, .. }) => t.clone(),
                            _ => String::new(),
                        };
                        let count = self.table_view.pending_edits.len();
                        self.ui.modal = modals::ModalState::ConfirmSwitchTable {
                            new_db: db,
                            new_table: table,
                            pending_count: count,
                            current_table,
                        };
                    } else {
                        self.on_table_selected(db, table);
                        self.active_tab = ActiveTab::Tabla;
                    }
                } else {
                    self.active_tab = ActiveTab::Tabla;
                }
            }
            SidebarAction::OpenInSql { db, table } => {
                let new_idx = self.sql_tabs.len();
                let sql_text = format!("SELECT * FROM `{}`.`{}` WHERE ", db, table);
                self.sql_tabs.push(SqlTab {
                    db_context: db.clone(),
                    sql_text,
                    result: None,
                });
                self.active_tab = ActiveTab::Sql(new_idx);
                self.save_tabs_state();
            }
            SidebarAction::CloneTable { db, table } => {
                self.ui.modal = ModalState::CloneTable {
                    db,
                    table,
                    new_name: String::new(),
                    copy_data: true,
                    error: None,
                };
            }
        }

        if should_disconnect {
            self.state = AppState::Disconnected;
        }

        // F5: recargar grid (invalida también caché de estructura de la tabla)
        if f5
            && let Some(SelectedNode::Table { db, table }) = &self.schema.selected_node.clone() {
                // Invalidar caché de columnas para reflejar cambios estructurales
                self.schema.columns_info.remove(&(db.clone(), table.clone()));
                let sql = self.build_sql(db, table);
                self.run_query_async(sql);
            }

        // Filter action
        match filter_action {
            filter_bar::FilterAction::Apply => {
                if let Some(SelectedNode::Table { db, table }) =
                    self.schema.selected_node.clone().as_ref()
                {
                    // Añadir al historial de la tabla si hay texto
                    let filter = self.table_view.filter_text.trim().to_string();
                    if !filter.is_empty() {
                        self.table_view.filter_history.retain(|f| f != &filter);
                        self.table_view.filter_history.insert(0, filter);
                        self.table_view.filter_history.truncate(20);
                    }
                    let sql = self.build_sql(db, table);
                    self.run_query_async(sql);
                    self.save_table_state();
                }
            }
            filter_bar::FilterAction::Clear => {
                if let Some(SelectedNode::Table { db, table }) =
                    self.schema.selected_node.clone().as_ref()
                {
                    let sql = self.build_sql(db, table);
                    self.run_query_async(sql);
                }
            }
            filter_bar::FilterAction::None => {}
        }

        // DB info: double-click on table opens it
        if let DbInfoAction::OpenTable(table) = db_info_action {
            let db = match &self.schema.selected_node {
                Some(SelectedNode::Database(d)) => Some(d.clone()),
                Some(SelectedNode::Table { db, .. }) => Some(db.clone()),
                _ => None,
            };
            if let Some(db) = db {
                if self.needs_confirm_before_switch() {
                    let current_table = match &self.schema.selected_node {
                        Some(SelectedNode::Table { table: t, .. }) => t.clone(),
                        _ => String::new(),
                    };
                    let count = self.table_view.pending_edits.len();
                    self.ui.modal = modals::ModalState::ConfirmSwitchTable {
                        new_db: db,
                        new_table: table,
                        pending_count: count,
                        current_table,
                    };
                } else {
                    self.on_table_selected(db, table);
                    self.active_tab = ActiveTab::Datos;
                }
            }
        }

        // Persistir texto de pestañas SQL al ejecutar
        if matches!(sql_action, SqlEditorAction::Execute { .. }) {
            self.save_tabs_state();
        }

        // SQL editor
        if let SqlEditorAction::Execute { sql, db_context } = sql_action {
            // Ejecutar USE por separado para que el log muestre solo la query del usuario
            if !db_context.is_empty() {
                let use_sql = format!("USE `{}`", db_context.replace('`', "``"));
                let _ = match &mut self.state {
                    AppState::Connected(b) => b.execute_write(&use_sql),
                    _ => return,
                };
            }
            let start = Instant::now();
            let result = match &mut self.state {
                AppState::Connected(b) => b.execute_query(&sql),
                _ => return,
            };
            let duration_ms = start.elapsed().as_millis() as u64;
            let query_result = match result {
                Ok(data) => {
                    let rows = data.rows.len();
                    self.push_log(LogEntry::ok(&sql, duration_ms, rows));
                    Some(Ok(data))
                }
                Err(e) => {
                    self.push_log(LogEntry::err(&sql, duration_ms, &e));
                    Some(Err(e))
                }
            };
            // Store result in active SQL tab
            if let ActiveTab::Sql(tab_idx) = self.active_tab
                && let Some(tab) = self.sql_tabs.get_mut(tab_idx) {
                    tab.result = query_result.clone();
                }
        }

        // Modal action
        self.handle_modal_action(modal_action);

        // Profile selection & management
        if let Some(idx) = profile_selected
            && matches!(self.state, AppState::Disconnected) {
                let profile = &self.profiles[idx];
                self.params = profile.to_params();
                self.port_str = self.params.port.to_string();
                self.profile_name = profile.name.clone();
                self.save_password = profile.password.is_some();
                self.selected_profile_idx = Some(idx);
                self.error_message = None;
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

        if delete_profile
            && let Some(idx) = self.selected_profile_idx {
                self.profiles.remove(idx);
                self.selected_profile_idx = None;
                self.profile_name.clear();
                profiles::save(&self.profiles);
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


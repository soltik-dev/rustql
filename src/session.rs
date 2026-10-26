use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use eframe::egui;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::db::{
    ColumnInfo, ConnectEvent, ConnectionParams, DatabaseInfo, DbBackend, ExportDestination,
    ExportEvent, ExportOptions, QueryResult, TableInfo, connect_mysql_progress, escape_ident,
    escape_sql,
};
use crate::log::LogEntry;
use crate::profiles::{self, ConnColor, ConnectionProfile};
use crate::secrets;
use crate::settings::{self, AppSettings, SavedSqlTab};
use crate::table_state::{self, TableStates};
use crate::ui::db_info::DbInfoAction;
use crate::ui::modals::{
    self, DropKind, ExportPhase, ExportRunning, ModalAction, ModalState, NewColumnDef, Reload,
    TableExportEntry, TableExportState, TruncateKind,
};
use crate::ui::results_grid::{
    EditingCell, GridAction, PendingEdit, PendingInsert, RowRef, SortDir, SortSpec,
};
use crate::ui::sidebar::SidebarAction;
use crate::ui::sql_editor::{SqlEditorAction, SqlTab};
use crate::ui::tab_bar::{ActiveTab, TabBarResult};
use crate::ui::{
    connection_panel, db_info, filter_bar, modals as modals_ui, results_grid, server_info, sidebar,
    sql_editor, tab_bar, table_structure, theme,
};

// ── Estados de la conexión ───────────────────────────────────────────────────

pub enum ConnState {
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

// ── Estado compartido entre sesiones ─────────────────────────────────────────

/// Lo que comparten todas las pestañas de conexión: ajustes (favoritos,
/// pestañas SQL guardadas…), perfiles y estado persistido por tabla.
pub struct Shared {
    pub settings: AppSettings,
    pub profiles: Vec<ConnectionProfile>,
    /// Estado persistido por tabla (sort, filtro, historial)
    pub table_states: TableStates,
}

/// Teclas globales del frame: solo las recibe la sesión activa.
#[derive(Clone, Copy)]
pub struct FrameKeys {
    pub f5: bool,
    pub f9: bool,
    pub ctrl_enter: bool,
    pub ctrl_t: bool,
}

impl FrameKeys {
    pub fn read(ctx: &egui::Context) -> Self {
        ctx.input(|i| Self {
            f5: i.key_pressed(egui::Key::F5),
            f9: i.key_pressed(egui::Key::F9),
            ctrl_enter: i.modifiers.ctrl && i.key_pressed(egui::Key::Enter),
            ctrl_t: i.modifiers.ctrl && i.key_pressed(egui::Key::T),
        })
    }
}

/// Peticiones de la sesión que solo puede resolver la app.
#[derive(Default)]
pub struct SessionOutcome {
    /// El usuario confirmó cerrar esta pestaña pese a los cambios pendientes.
    pub close: bool,
    /// Se eliminó el perfil con este índice: las demás sesiones deben
    /// reajustar su selected_profile_idx.
    pub profile_removed: Option<usize>,
}

// ── Sesión ───────────────────────────────────────────────────────────────────

/// Una pestaña de conexión: el formulario o un servidor conectado con su
/// árbol, tabla abierta, pestañas SQL, log y modal propios.
pub struct Session {
    /// Identificador único: separa el estado de los widgets egui entre sesiones.
    pub id: u64,
    pub state: ConnState,
    pub params: ConnectionParams,
    pub port_str: String,
    pub error_message: Option<String>,
    /// Nombre de la pestaña y de la raíz del árbol (perfil u host), fijado al conectar.
    pub title: String,
    pub color: Option<ConnColor>,

    pub schema: SchemaCache,
    pub table_view: TableViewState,
    pub modal: ModalState,
    pub query_log: std::collections::VecDeque<LogEntry>,
    pub show_only_fav: bool,

    // Pestañas
    pub active_tab: ActiveTab,
    pub sql_tabs: Vec<SqlTab>,

    // Formulario de conexión / perfiles
    pub selected_profile_idx: Option<usize>,
    pub profile_name: String,
    pub save_password: bool,
    /// Contraseñas que un hilo terminó de guardar en el llavero: (id del perfil, resultado).
    keyring_tx: mpsc::Sender<(String, Result<(), String>)>,
    keyring_rx: Receiver<(String, Result<(), String>)>,
}

impl Session {
    pub fn new(id: u64) -> Self {
        let params = ConnectionParams::default();
        let port_str = params.port.to_string();
        let (keyring_tx, keyring_rx) = mpsc::channel();
        Self {
            id,
            state: ConnState::Disconnected,
            port_str,
            params,
            error_message: None,
            title: String::new(),
            color: None,
            schema: SchemaCache::default(),
            table_view: TableViewState::default(),
            modal: ModalState::None,
            query_log: std::collections::VecDeque::new(),
            show_only_fav: false,
            active_tab: ActiveTab::default(),
            sql_tabs: vec![SqlTab::default()],
            selected_profile_idx: None,
            profile_name: String::new(),
            save_password: false,
            keyring_tx,
            keyring_rx,
        }
    }

    /// Texto de la pestaña de conexión.
    pub fn tab_title(&self) -> &str {
        match self.state {
            ConnState::Disconnected => "Nueva conexión",
            _ => &self.title,
        }
    }

    /// Conectando o con una query en curso.
    pub fn is_busy(&self) -> bool {
        matches!(
            self.state,
            ConnState::Connecting { .. } | ConnState::QueryRunning
        )
    }

    pub fn is_connected(&self) -> bool {
        matches!(
            self.state,
            ConnState::Connected(_) | ConnState::QueryRunning
        )
    }

    /// Host y BBDD activa para la barra de estado.
    pub fn status(&self) -> (Option<String>, Option<String>) {
        let host = match &self.state {
            ConnState::Connected(_) | ConnState::QueryRunning => {
                Some(format!("{}:{}", self.params.host, self.params.port))
            }
            ConnState::Connecting { host, .. } => Some(format!("Conectando a {}…", host)),
            ConnState::Disconnected => None,
        };
        let db = match &self.schema.selected_node {
            Some(SelectedNode::Database(d)) => Some(d.clone()),
            Some(SelectedNode::Table { db, .. }) => Some(db.clone()),
            _ => None,
        };
        (host, db)
    }

    /// Antes de cerrar la pestaña: cancela la exportación en marcha, si la hay.
    pub fn shutdown(&self) {
        if let ModalState::ExportData {
            phase: ExportPhase::Running(running),
            ..
        } = &self.modal
        {
            running.cancel_flag.store(true, Ordering::Relaxed);
        }
    }

    /// Vuelve al formulario. La query en curso, si la hay, se descarta: sin
    /// esto su resultado reconectaba la sesión al llegar.
    fn disconnect(&mut self) {
        self.state = ConnState::Disconnected;
        self.table_view.query_receiver = None;
        self.table_view.query_loading = false;
    }
}

// ── Métodos auxiliares ────────────────────────────────────────────────────────

impl Session {
    fn run_query_async(&mut self, sql: String) {
        let backend = match std::mem::replace(&mut self.state, ConnState::QueryRunning) {
            ConnState::Connected(b) => b,
            other => {
                self.state = other;
                return;
            }
        };
        let (tx, rx) = std::sync::mpsc::channel();
        self.table_view.query_receiver = Some(rx);
        self.table_view.query_loading = true;
        std::thread::spawn(move || {
            let start = std::time::Instant::now();
            let result = backend.execute_query(&sql);
            let elapsed_ms = start.elapsed().as_millis() as u64;
            let _ = tx.send(crate::db::QueryEvent::Done {
                result,
                backend,
                sql,
                elapsed_ms,
            });
        });
    }

    fn run_write(&mut self, sql: &str) -> Result<u64, String> {
        let start = Instant::now();
        let result = match &mut self.state {
            ConnState::Connected(b) => b.execute_write(sql),
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
            self.query_log.pop_front();
        }
        self.query_log.push_back(entry);
    }

    fn build_sql(&self, sh: &Shared, db: &str, table: &str) -> String {
        let filter = self.table_view.filter_text.trim();

        let order_by = if self.table_view.sort_specs.is_empty() {
            String::new()
        } else {
            let parts: Vec<String> = self
                .table_view
                .sort_specs
                .iter()
                .map(|s| {
                    let dir = if s.dir == SortDir::Desc {
                        "DESC"
                    } else {
                        "ASC"
                    };
                    format!("`{}` {}", s.col_name.replace('`', "``"), dir)
                })
                .collect();
            format!(" ORDER BY {}", parts.join(", "))
        };

        let limit = sh.settings.query_limit;
        if filter.is_empty() {
            format!(
                "SELECT * FROM `{}`.`{}`{} LIMIT {limit}",
                escape_ident(db),
                escape_ident(table),
                order_by
            )
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
            ConnState::Connected(b) => b.get_primary_keys(db, table),
            _ => return,
        };
        self.table_view.primary_keys = result.unwrap_or_default();
    }

    fn load_databases_info(&mut self) {
        let result = match &mut self.state {
            ConnState::Connected(b) => b.get_databases_info(),
            _ => return,
        };
        self.schema.databases_info = result.ok();
    }

    fn load_tables_info(&mut self, db: &str) {
        let result = match &mut self.state {
            ConnState::Connected(b) => b.get_tables_info(db),
            _ => return,
        };
        if let Ok(info) = result {
            self.schema.tables_info.insert(db.to_string(), info);
        }
    }

    fn load_columns_info(&mut self, db: &str, table: &str) {
        let result = match &mut self.state {
            ConnState::Connected(b) => b.get_columns_info(db, table),
            _ => return,
        };
        if let Ok(cols) = result {
            self.schema
                .columns_info
                .insert((db.to_string(), table.to_string()), cols);
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
                    && !self.schema.tables_info.contains_key(&db)
                {
                    self.load_tables_info(&db);
                }
            }
            ActiveTab::Tabla => {
                let key = match &self.schema.selected_node {
                    Some(SelectedNode::Table { db, table }) => Some((db.clone(), table.clone())),
                    _ => None,
                };
                if let Some((db, table)) = key {
                    let k = (db.clone(), table.clone());
                    if !self.schema.columns_info.contains_key(&k) {
                        self.load_columns_info(&db, &table);
                    }
                }
            }
            ActiveTab::Sql(_) | ActiveTab::Datos => {}
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

    /// Aplica los cambios pendientes y recarga la tabla si se guardaron todos.
    fn apply_pending_changes(&mut self, sh: &Shared) {
        if self.commit_pending_changes()
            && let Some(SelectedNode::Table { db, table }) = &self.schema.selected_node.clone()
        {
            let sql = self.build_sql(sh, db, table);
            self.run_query_async(sql);
        }
    }

    /// Ejecuta deletes, edits e inserts pendientes (cada grupo en su transacción).
    /// Devuelve true si no queda ninguno. No recarga: el llamante decide, porque
    /// la recarga cede el backend a un hilo y bloquearía otras operaciones.
    fn commit_pending_changes(&mut self) -> bool {
        let (db, table) = match &self.schema.selected_node {
            Some(SelectedNode::Table { db, table }) => (db.clone(), table.clone()),
            _ => return false,
        };

        // 1. Apply deletes (en transacción para garantizar atomicidad)
        if !self.table_view.pending_deletes.is_empty() {
            if self.table_view.primary_keys.is_empty() {
                self.modal = modals::ModalState::Error {
                    title: "Error al eliminar".into(),
                    message: "No se puede eliminar: tabla sin clave primaria".into(),
                };
                return false;
            }
            let data = match &self.table_view.table_data {
                Some(d) => d.clone(),
                None => return false,
            };
            let pks = self.table_view.primary_keys.clone();
            let deletes: Vec<usize> = self.table_view.pending_deletes.iter().cloned().collect();
            let mut stmts: Vec<String> = Vec::new();
            for row_idx in &deletes {
                let where_parts: Vec<String> = pks
                    .iter()
                    .filter_map(|pk| {
                        let pk_idx = data.columns.iter().position(|c| c == pk)?;
                        let pk_val = data.rows.get(*row_idx)?.get(pk_idx)?;
                        Some(format!(
                            "`{}` = '{}'",
                            pk.replace('`', "``"),
                            escape_sql(pk_val.as_deref().unwrap_or(""))
                        ))
                    })
                    .collect();
                if where_parts.is_empty() {
                    continue;
                }
                stmts.push(format!(
                    "DELETE FROM `{}`.`{}` WHERE {}",
                    db.replace('`', "``"),
                    table.replace('`', "``"),
                    where_parts.join(" AND ")
                ));
            }
            let result = match &mut self.state {
                ConnState::Connected(b) => b.execute_transaction(&stmts),
                _ => return false,
            };
            match result {
                Ok(()) => {
                    let n = stmts.len();
                    self.push_log(LogEntry::ok(format!("DELETE x{n} (transacción)"), 0, n));
                    self.table_view.pending_deletes.clear();
                }
                Err(e) => {
                    self.push_log(LogEntry::err("DELETE (transacción)", 0, &e));
                    self.modal = modals::ModalState::Error {
                        title: "Error al eliminar".into(),
                        message: e,
                    };
                }
            }
        }

        // 2. Apply edits (en transacción para garantizar atomicidad)
        if !self.table_view.pending_edits.is_empty() {
            if self.table_view.primary_keys.is_empty() {
                self.modal = modals::ModalState::Error {
                    title: "Error al editar".into(),
                    message: "No se puede editar: la tabla no tiene clave primaria".into(),
                };
                return false;
            }
            let data = match &self.table_view.table_data {
                Some(d) => d.clone(),
                None => return false,
            };
            let edits = self.table_view.pending_edits.clone();
            let pks = self.table_view.primary_keys.clone();
            let mut stmts: Vec<String> = Vec::new();

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
                            escape_sql(pk_val.as_deref().unwrap_or(""))
                        ))
                    })
                    .collect();
                if where_parts.is_empty() {
                    continue;
                }
                let set_clause = if edit.is_null {
                    format!("`{}` = NULL", col_name.replace('`', "``"))
                } else {
                    format!(
                        "`{}` = '{}'",
                        col_name.replace('`', "``"),
                        escape_sql(&edit.new_value)
                    )
                };
                stmts.push(format!(
                    "UPDATE `{}`.`{}` SET {} WHERE {}",
                    db.replace('`', "``"),
                    table.replace('`', "``"),
                    set_clause,
                    where_parts.join(" AND ")
                ));
            }
            let result = match &mut self.state {
                ConnState::Connected(b) => b.execute_transaction(&stmts),
                _ => return false,
            };
            match result {
                Ok(()) => {
                    let n = stmts.len();
                    self.push_log(LogEntry::ok(format!("UPDATE x{n} (transacción)"), 0, n));
                    self.table_view.pending_edits.clear();
                    self.table_view.editing_cell = None;
                }
                Err(e) => {
                    self.push_log(LogEntry::err("UPDATE (transacción)", 0, &e));
                    self.modal = modals::ModalState::Error {
                        title: "Error al guardar".into(),
                        message: e,
                    };
                }
            }
        }

        // 3. Apply inserts (en transacción para garantizar atomicidad)
        if !self.table_view.pending_inserts.is_empty() {
            let col_names = self
                .table_view
                .table_data
                .as_ref()
                .map(|d| d.columns.clone())
                .unwrap_or_default();
            let inserts = self.table_view.pending_inserts.clone();
            let mut stmts: Vec<String> = Vec::new();
            for ins in &inserts {
                let (cols, vals): (Vec<_>, Vec<_>) = col_names
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| !ins.auto_cols.get(*i).copied().unwrap_or(false))
                    .map(|(i, col)| {
                        let val = ins.values.get(i).and_then(|v| v.as_deref());
                        let col_sql = format!("`{}`", col.replace('`', "``"));
                        let val_sql = match val {
                            None => "NULL".to_string(),
                            Some(v) => format!("'{}'", escape_sql(v)),
                        };
                        (col_sql, val_sql)
                    })
                    .unzip();
                if cols.is_empty() {
                    continue;
                }
                stmts.push(format!(
                    "INSERT INTO `{}`.`{}` ({}) VALUES ({})",
                    db.replace('`', "``"),
                    table.replace('`', "``"),
                    cols.join(", "),
                    vals.join(", ")
                ));
            }
            let result = match &mut self.state {
                ConnState::Connected(b) => b.execute_transaction(&stmts),
                _ => return false,
            };
            match result {
                Ok(()) => {
                    let n = stmts.len();
                    self.push_log(LogEntry::ok(format!("INSERT x{n} (transacción)"), 0, n));
                    self.table_view.pending_inserts.clear();
                }
                Err(e) => {
                    self.push_log(LogEntry::err("INSERT (transacción)", 0, &e));
                    self.modal = modals::ModalState::Error {
                        title: "Error al insertar".into(),
                        message: e,
                    };
                }
            }
        }

        self.pending_count() == 0
    }

    /// Persiste el sort+filtro+historial de la tabla actualmente seleccionada.
    fn save_table_state(&mut self, sh: &mut Shared) {
        if let Some(SelectedNode::Table { db, table }) = &self.schema.selected_node.clone() {
            let mut ts = table_state::get(&sh.table_states, db, table);
            ts.sort_specs = self.table_view.sort_specs.clone();
            ts.filter_text = self.table_view.filter_text.clone();
            ts.filter_history = self.table_view.filter_history.clone();
            table_state::set(&mut sh.table_states, db, table, ts);
            table_state::save(&sh.table_states);
        }
    }

    /// Total de cambios pendientes (edits + deletes + inserts) en la tabla actual.
    pub fn pending_count(&self) -> usize {
        self.table_view.pending_edits.len()
            + self.table_view.pending_deletes.len()
            + self.table_view.pending_inserts.len()
    }

    fn discard_pending(&mut self) {
        self.table_view.pending_edits.clear();
        self.table_view.pending_deletes.clear();
        self.table_view.pending_inserts.clear();
        self.table_view.editing_cell = None;
    }

    /// Recarga la tabla abierta. Si hay cambios pendientes pregunta antes qué
    /// hacer con ellos: van por posición de fila y con los datos nuevos
    /// apuntarían a otras filas.
    fn request_reload(&mut self, sh: &mut Shared, reload: Reload) {
        let Some(SelectedNode::Table { table, .. }) = &self.schema.selected_node else {
            return;
        };
        let pending_count = self.pending_count();
        if pending_count > 0 {
            self.modal = ModalState::ConfirmReload {
                reload,
                pending_count,
                current_table: table.clone(),
            };
        } else {
            self.reload_now(sh, reload);
        }
    }

    fn reload_now(&mut self, sh: &mut Shared, reload: Reload) {
        let Some(SelectedNode::Table { db, table }) = self.schema.selected_node.clone() else {
            return;
        };
        // La celda en edición también va por posición.
        self.table_view.editing_cell = None;
        match reload {
            Reload::Sort(specs) => {
                self.table_view.sort_specs = specs;
                self.clear_row_selection();
                self.save_table_state(sh);
            }
            Reload::Filter(filter) => {
                if !filter.is_empty() {
                    let history = &mut self.table_view.filter_history;
                    history.retain(|f| f != &filter);
                    history.insert(0, filter.clone());
                    history.truncate(20);
                }
                self.table_view.filter_text = filter;
                self.clear_row_selection();
                self.save_table_state(sh);
            }
            Reload::Refresh => {
                // Invalidar caché de columnas para reflejar cambios estructurales
                self.schema
                    .columns_info
                    .remove(&(db.clone(), table.clone()));
            }
        }
        let sql = self.build_sql(sh, &db, &table);
        self.run_query_async(sql);
    }

    fn clear_row_selection(&mut self) {
        self.table_view.selected_cell = None;
        self.table_view.selected_rows.clear();
        self.table_view.last_selected_row = None;
        self.table_view.drag_select_from = None;
    }

    /// Abre una tabla en la pestaña indicada. Si hay cambios pendientes en la
    /// tabla actual, pide confirmación antes de cambiar.
    fn switch_to_table(&mut self, sh: &mut Shared, db: String, table: String, tab: ActiveTab) {
        let already_selected = matches!(
            &self.schema.selected_node,
            Some(SelectedNode::Table { db: d, table: t }) if *d == db && *t == table
        );
        if already_selected {
            self.active_tab = tab;
        } else if self.pending_count() > 0 {
            let current_table = match &self.schema.selected_node {
                Some(SelectedNode::Table { table: t, .. }) => t.clone(),
                _ => String::new(),
            };
            self.modal = modals::ModalState::ConfirmSwitchTable {
                new_db: db,
                new_table: table,
                pending_count: self.pending_count(),
                current_table,
            };
        } else {
            self.on_table_selected(sh, db, table);
            self.active_tab = tab;
        }
    }

    /// Cambia la tabla seleccionada: guarda el estado actual, restaura el
    /// de la nueva tabla, ejecuta la query con el filtro guardado y reaplica
    /// la ordenación guardada.
    fn on_table_selected(&mut self, sh: &mut Shared, db: String, table: String) {
        self.table_view.selected_cell = None;
        self.table_view.selected_rows.clear();
        self.table_view.last_selected_row = None;
        self.table_view.drag_select_from = None;
        self.table_view.pending_edits.clear();
        self.table_view.pending_deletes.clear();
        self.table_view.pending_inserts.clear();
        self.table_view.editing_cell = None;

        // 1. Guardar estado de la tabla anterior (sort, filtro, historial)
        self.save_table_state(sh);

        // 2. Cargar estado de la nueva tabla
        let ts = table_state::get(&sh.table_states, &db, &table);
        self.table_view.filter_text = ts.filter_text.clone();
        self.table_view.filter_history = ts.filter_history.clone();

        // 3. Actualizar selección
        self.schema.selected_node = Some(SelectedNode::Table {
            db: db.clone(),
            table: table.clone(),
        });

        // 4. Restaurar sort ANTES de build_sql para que el ORDER BY quede en la query
        self.table_view.sort_specs = ts.sort_specs.clone();

        // 5. Cargar claves primarias e info de columnas (antes de run_query_async, que cambia el estado a QueryRunning)
        self.load_primary_keys(&db, &table);
        let col_key = (db.clone(), table.clone());
        if !self.schema.columns_info.contains_key(&col_key) {
            self.load_columns_info(&db, &table);
        }

        // 6. Ejecutar query (server-side sort + filter restaurados)
        let sql = self.build_sql(sh, &db, &table);
        self.run_query_async(sql);
        // sort_indices es identidad tras run_query (datos ya ordenados por servidor)
    }

    fn save_tabs_state(&mut self, sh: &mut Shared) {
        let tabs = self
            .sql_tabs
            .iter()
            .map(|t| SavedSqlTab {
                db_context: t.db_context.clone(),
                sql_text: t.sql_text.clone(),
            })
            .collect();
        sh.settings.sql_tabs_by_conn.insert(self.params.key(), tabs);
        settings::save(&sh.settings);
    }

    fn on_connected(&mut self, backend: Box<dyn DbBackend>, sh: &mut Shared) {
        self.state = ConnState::Connected(backend);
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
        self.modal = ModalState::None;
        self.schema.databases_info = None;
        self.schema.tables_info.clear();
        self.schema.columns_info.clear();
        // Restaurar las pestañas SQL de esta conexión. Las guardadas antes de
        // haber conexiones múltiples las hereda la primera que no tenga propias.
        let key = self.params.key();
        let (saved, migrated) = match sh.settings.sql_tabs_by_conn.get(&key) {
            Some(tabs) => (tabs.clone(), false),
            None => {
                let legacy = std::mem::take(&mut sh.settings.sql_tabs);
                let migrated = !legacy.is_empty();
                (legacy, migrated)
            }
        };
        self.sql_tabs = if saved.is_empty() {
            vec![SqlTab::default()]
        } else {
            saved
                .iter()
                .map(|t| SqlTab {
                    db_context: t.db_context.clone(),
                    sql_text: t.sql_text.clone(),
                    result: None,
                })
                .collect()
        };
        if migrated {
            self.save_tabs_state(sh);
        }
        self.query_log.clear();
    }
}

// ── Loop principal ────────────────────────────────────────────────────────────

impl Session {
    /// Sondea el canal del hilo de conexión y actualiza el estado si terminó.
    fn poll_connection(&mut self, ctx: &egui::Context, sh: &mut Shared) {
        let mut connect_result: Option<Result<Box<dyn DbBackend>, String>> = None;
        if let ConnState::Connecting {
            receiver, status, ..
        } = &mut self.state
        {
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
                Ok(backend) => self.on_connected(backend, sh),
                Err(e) => {
                    self.state = ConnState::Disconnected;
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
        if let Some(crate::db::QueryEvent::Done {
            result,
            backend,
            sql,
            elapsed_ms,
        }) = query_done
        {
            self.table_view.query_receiver = None;
            self.table_view.query_loading = false;
            self.state = ConnState::Connected(backend);
            match result {
                Ok(data) => {
                    // Toda recarga pasa por request_reload, que antes resuelve
                    // los cambios pendientes. Si alguna se lo salta, mejor
                    // perderlos que aplicarlos a las filas que ahora ocupan
                    // esas posiciones.
                    if self.pending_count() > 0 {
                        log::warn!("Datos recargados con cambios pendientes: se descartan");
                        self.discard_pending();
                    }
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

    /// Consume eventos del hilo de exportación y actualiza el estado del modal.
    /// Los errores también se registran en el log de consultas para que quede
    /// constancia después de cerrar el modal.
    fn poll_export_progress(&mut self, ctx: &egui::Context) {
        let mut new_phase: Option<ExportPhase> = None;
        let mut log_entries: Vec<LogEntry> = Vec::new();

        {
            let ModalState::ExportData { tables, phase, .. } = &mut self.modal else {
                return;
            };
            let ExportPhase::Running(running) = phase else {
                return;
            };

            loop {
                match running.receiver.try_recv() {
                    Ok(ExportEvent::TableStarted(name)) => {
                        if let Some(e) = tables.iter_mut().find(|e| e.name == name) {
                            e.state = TableExportState::InProgress;
                        }
                    }
                    Ok(ExportEvent::TableDone { table, rows }) => {
                        if let Some(e) = tables.iter_mut().find(|e| e.name == table) {
                            e.state = TableExportState::Done { rows };
                        }
                        running.done_count += 1;
                        running.rows_done += rows;
                    }
                    Ok(ExportEvent::TableError { table, error }) => {
                        log_entries.push(LogEntry::err(
                            format!("Export `{table}`"),
                            0,
                            error.as_str(),
                        ));
                        if let Some(e) = tables.iter_mut().find(|e| e.name == table) {
                            e.state = TableExportState::Error(error);
                        }
                        running.done_count += 1;
                    }
                    Ok(ExportEvent::Finished {
                        total_tables,
                        total_rows,
                        elapsed_ms,
                    }) => {
                        log_entries.push(LogEntry::ok(
                            format!("Exportación finalizada: {total_tables} tablas"),
                            elapsed_ms,
                            total_rows as usize,
                        ));
                        new_phase = Some(ExportPhase::Done {
                            total_tables,
                            total_rows,
                            elapsed_ms,
                        });
                        break;
                    }
                    Ok(ExportEvent::Cancelled) => {
                        new_phase = Some(ExportPhase::Cancelled);
                        break;
                    }
                    Ok(ExportEvent::Failed(error)) => {
                        log_entries.push(LogEntry::err("Exportación", 0, error.as_str()));
                        new_phase = Some(ExportPhase::Failed(error));
                        break;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        ctx.request_repaint_after(Duration::from_millis(50));
                        break;
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        new_phase = Some(ExportPhase::Failed(
                            "El hilo de exportación terminó de forma inesperada.".to_string(),
                        ));
                        break;
                    }
                }
            }
        }

        for entry in log_entries {
            self.push_log(entry);
        }

        if let Some(p) = new_phase
            && let ModalState::ExportData { phase, .. } = &mut self.modal
        {
            *phase = p;
        }
    }

    /// Tablas de una BBDD para el modal de exportación: usa la caché del
    /// sidebar o, si el árbol aún no se expandió, las pide al backend.
    fn export_tables_for(&mut self, db: &str) -> Vec<String> {
        if let Some(tables) = self.schema.tables_cache.get(db) {
            return tables.clone();
        }
        let ConnState::Connected(backend) = &self.state else {
            return Vec::new();
        };
        match backend.list_tables(db) {
            Ok(tables) => {
                self.schema
                    .tables_cache
                    .insert(db.to_string(), tables.clone());
                tables
            }
            Err(e) => {
                self.push_log(LogEntry::err(
                    format!("SHOW TABLES FROM `{db}`"),
                    0,
                    e.as_str(),
                ));
                Vec::new()
            }
        }
    }

    /// Procesa la acción resultante del modal activo.
    fn handle_modal_action(&mut self, modal_action: ModalAction, sh: &mut Shared) {
        match modal_action {
            ModalAction::None => {}
            ModalAction::Cancel | ModalAction::CloseSession => {
                self.modal = ModalState::None;
            }
            ModalAction::ApplyAndSwitch { new_db, new_table } => {
                self.modal = ModalState::None;
                if self.commit_pending_changes() {
                    self.on_table_selected(sh, new_db, new_table);
                    self.active_tab = ActiveTab::Datos;
                }
            }
            ModalAction::DiscardAndSwitch { new_db, new_table } => {
                self.modal = ModalState::None;
                self.on_table_selected(sh, new_db, new_table);
                self.active_tab = ActiveTab::Datos;
            }
            ModalAction::ApplyAndReload(reload) => {
                self.modal = ModalState::None;
                if self.commit_pending_changes() {
                    self.reload_now(sh, reload);
                }
            }
            ModalAction::DiscardAndReload(reload) => {
                self.modal = ModalState::None;
                self.discard_pending();
                self.reload_now(sh, reload);
            }
            ModalAction::CreateDatabase { name, charset } => {
                let result = match &mut self.state {
                    ConnState::Connected(b) => b.create_database(&name, &charset),
                    _ => Err("No conectado".into()),
                };
                if let Ok(()) = &result {
                    self.push_log(LogEntry::ok(format!("CREATE DATABASE `{name}`"), 0, 0));
                } else if let Err(ref e) = result {
                    self.push_log(LogEntry::err(format!("CREATE DATABASE `{name}`"), 0, e));
                }
                match result {
                    Ok(()) => {
                        self.modal = ModalState::None;
                        self.schema.databases.clear();
                        self.schema.databases_info = None;
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
                let result = match &mut self.state {
                    ConnState::Connected(b) => b.drop_database(&db),
                    _ => Err("No conectado".into()),
                };
                if let Ok(()) = &result {
                    self.push_log(LogEntry::ok(format!("DROP DATABASE `{db}`"), 0, 0));
                } else if let Err(ref e) = result {
                    self.push_log(LogEntry::err(format!("DROP DATABASE `{db}`"), 0, e));
                }
                self.modal = ModalState::None;
                if result.is_ok() {
                    self.invalidate_after_write(Some(&db), None);
                }
            }
            ModalAction::DropTable { db, table } => {
                let result = match &mut self.state {
                    ConnState::Connected(b) => b.drop_table(&db, &table),
                    _ => Err("No conectado".into()),
                };
                if let Ok(()) = &result {
                    self.push_log(LogEntry::ok(format!("DROP TABLE `{db}`.`{table}`"), 0, 0));
                } else if let Err(ref e) = result {
                    self.push_log(LogEntry::err(format!("DROP TABLE `{db}`.`{table}`"), 0, e));
                }
                self.modal = ModalState::None;
                if result.is_ok() {
                    self.invalidate_after_write(Some(&db), Some(&table));
                }
            }
            ModalAction::TruncateTable { db, table } => {
                let result = match &mut self.state {
                    ConnState::Connected(b) => b.truncate_table(&db, &table),
                    _ => Err("No conectado".into()),
                };
                if let Ok(()) = &result {
                    self.push_log(LogEntry::ok(
                        format!("TRUNCATE TABLE `{db}`.`{table}`"),
                        0,
                        0,
                    ));
                } else if let Err(ref e) = result {
                    self.push_log(LogEntry::err(
                        format!("TRUNCATE TABLE `{db}`.`{table}`"),
                        0,
                        e,
                    ));
                }
                let result = result.map(|_| 0u64);
                self.modal = ModalState::None;
                if result.is_ok() {
                    if matches!(
                        &self.schema.selected_node,
                        Some(SelectedNode::Table { db: d, table: t }) if *d == db && *t == table
                    ) {
                        let sql = self.build_sql(sh, &db, &table);
                        self.run_query_async(sql);
                    }
                    self.invalidate_after_write(Some(&db), Some(&table));
                }
            }
            ModalAction::TruncateAllTables(db) => {
                self.modal = ModalState::None;
                let tables = self
                    .schema
                    .tables_cache
                    .get(&db)
                    .cloned()
                    .unwrap_or_default();
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
                self.modal = ModalState::None;
                self.request_reload(sh, Reload::Filter(sql_fragment));
            }
            ModalAction::DeleteRow { db, table, row_idx } => {
                self.modal = ModalState::None;
                let Some(data) = self.table_view.table_data.clone() else {
                    return;
                };
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
                        self.table_view
                            .pending_edits
                            .retain(|e| e.row_idx != row_idx);
                        let sql = self.build_sql(sh, &db, &table);
                        self.run_query_async(sql);
                    }
                }
            }
            ModalAction::CloneTable {
                db,
                src_table,
                new_name,
                copy_data,
            } => {
                let create_sql = format!(
                    "CREATE TABLE `{}`.`{}` LIKE `{}`.`{}`",
                    db.replace('`', "``"),
                    new_name.replace('`', "``"),
                    db.replace('`', "``"),
                    src_table.replace('`', "``")
                );
                match self.run_write(&create_sql) {
                    Ok(_) => {
                        if copy_data {
                            let insert_sql = format!(
                                "INSERT INTO `{}`.`{}` SELECT * FROM `{}`.`{}`",
                                db.replace('`', "``"),
                                new_name.replace('`', "``"),
                                db.replace('`', "``"),
                                src_table.replace('`', "``")
                            );
                            if let Err(e) = self.run_write(&insert_sql) {
                                if let ModalState::CloneTable { error, .. } = &mut self.modal {
                                    *error = Some(e);
                                }
                                return;
                            }
                        }
                        self.modal = ModalState::None;
                        self.invalidate_after_write(Some(&db), None);
                    }
                    Err(e) => {
                        if let ModalState::CloneTable { error, .. } = &mut self.modal {
                            *error = Some(e);
                        }
                    }
                }
            }
            ModalAction::InsertRow {
                db,
                table,
                col_names,
                values,
                nulls,
            } => {
                if self.table_view.table_data.is_none() {
                    return;
                }
                let col_list: Vec<String> = col_names
                    .iter()
                    .map(|c| format!("`{}`", c.replace('`', "``")))
                    .collect();
                let val_list: Vec<String> = values
                    .iter()
                    .zip(nulls.iter())
                    .map(|(val, is_null)| {
                        if *is_null {
                            "NULL".to_string()
                        } else {
                            format!("'{}'", escape_sql(val))
                        }
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
                        self.modal = ModalState::None;
                        let sql = self.build_sql(sh, &db, &table);
                        self.run_query_async(sql);
                    }
                    Err(e) => {
                        if let ModalState::InsertRow { error, .. } = &mut self.modal {
                            *error = Some(e);
                        }
                    }
                }
            }
            ModalAction::StartExport {
                source_db,
                tables,
                destination,
                options,
            } => {
                let cancel = Arc::new(AtomicBool::new(false));
                let rx = crate::db::export::spawn_export(
                    self.params.clone(),
                    source_db,
                    tables,
                    destination,
                    options,
                    Arc::clone(&cancel),
                );
                if let ModalState::ExportData { phase, .. } = &mut self.modal {
                    *phase = ExportPhase::Running(ExportRunning {
                        receiver: rx,
                        cancel_flag: cancel,
                        rows_done: 0,
                        done_count: 0,
                    });
                }
            }
            ModalAction::CancelExport => {
                if let ModalState::ExportData {
                    phase: ExportPhase::Running(running),
                    ..
                } = &mut self.modal
                {
                    running.cancel_flag.store(true, Ordering::Relaxed);
                }
            }
            ModalAction::CloseExport => {
                self.modal = ModalState::None;
            }
        }
    }
}

// ── Frame de la sesión ────────────────────────────────────────────────────────

impl Session {
    /// Sondea los hilos de fondo (conexión, query, exportación). La app lo
    /// llama en todas las sesiones, no solo en la visible.
    pub fn poll(&mut self, ctx: &egui::Context, sh: &mut Shared) {
        self.poll_connection(ctx, sh);
        self.poll_query_receiver(ctx);
        self.poll_export_progress(ctx);
        self.poll_keyring(sh);
    }

    /// Marca los perfiles cuya contraseña ya está guardada en el llavero.
    fn poll_keyring(&mut self, sh: &mut Shared) {
        while let Ok((id, result)) = self.keyring_rx.try_recv() {
            match result {
                Ok(()) => {
                    if let Some(p) = sh.profiles.iter_mut().find(|p| p.id == id) {
                        p.password_in_keyring = true;
                        p.password = None;
                        profiles::save(&sh.profiles);
                    }
                }
                Err(e) => {
                    self.error_message =
                        Some(format!("No se guardó la contraseña en el llavero: {e}"));
                }
            }
        }
    }

    /// Dibuja la sesión y aplica las acciones del frame. Los paneles globales
    /// (pestañas de conexión, barra de estado, log y ajustes) ya los ha
    /// colocado la app.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        sh: &mut Shared,
        keys: FrameKeys,
        show_sidebar: bool,
    ) -> SessionOutcome {
        let ctx = &ui.ctx().clone();
        let FrameKeys {
            f5,
            f9,
            ctrl_enter,
            ctrl_t,
        } = keys;
        let mut outcome = SessionOutcome::default();

        // ── Proactive data loading for current tab ────────────────────────
        if matches!(&self.state, ConnState::Connected(_)) {
            self.maybe_load_tab_data();
        }

        // ── Señales del frame ────────────────────────────────────────────
        let mut connect_clicked = false;
        let mut cancel_connect = false;
        let mut save_profile = false;
        let mut delete_profile = false;
        let mut profile_selected: Option<usize> = None;
        let mut sidebar_action = SidebarAction::None;
        let mut filter_action = filter_bar::FilterAction::None;
        let mut grid_action = GridAction::None;
        let mut apply_bar_action = GridAction::None;
        let mut tab_bar_result = TabBarResult::default();
        let mut db_info_action = DbInfoAction::None;
        let mut sql_action = SqlEditorAction::None;
        let mut should_disconnect = false;

        // El contenido va bajo push_id(sid) para que el estado de los widgets
        // (árbol expandido, scroll, cursores) sea propio de cada sesión.
        let sid = self.id;
        let conn_color = self.color.map(|c| theme::get(ui).conn(c));
        // El grid cambia la ordenación al pulsar una cabecera; se restaura si
        // la recarga no llega a hacerse (query en curso o el usuario cancela).
        let sorts_before = self.table_view.sort_specs.clone();

        // ── Contenido principal según estado ─────────────────────────────
        match &self.state {
            ConnState::Disconnected => {
                egui::Panel::left("conn_favorites")
                    .exact_size(200.0)
                    .show(ui, |ui| {
                        ui.push_id(sid, |ui| {
                            let (sel, connect_now) = connection_panel::show_favorites(
                                ui,
                                &sh.profiles,
                                &self.selected_profile_idx,
                            );
                            profile_selected = sel;
                            if connect_now {
                                connect_clicked = true;
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
                        connect_clicked |= result.connect;
                        save_profile = result.save_profile;
                        delete_profile = result.delete_profile;
                    });
                });
            }

            ConnState::Connecting {
                host,
                status,
                start,
                ..
            } => {
                let host = host.clone();
                let status = status.clone();
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

            state @ (ConnState::Connected(_) | ConnState::QueryRunning) => {
                // Con una query en curso el backend está cedido al hilo: el
                // sidebar trabaja con la caché (None = no hacer queries) y el
                // contenido se muestra sin aplicar sus acciones.
                let backend = match state {
                    ConnState::Connected(b) => Some(b.as_ref()),
                    _ => None,
                };
                let running = backend.is_none();

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
                                    backend,
                                    &mut self.schema.databases,
                                    &mut self.schema.tables_cache,
                                    sel_db,
                                    sel_table,
                                    &mut self.schema.table_search,
                                    &sh.settings.fav_tables,
                                    &mut self.show_only_fav,
                                    &self.params.host,
                                    &self.title,
                                    conn_color,
                                )
                            })
                            .inner
                        });
                    sidebar_action = sidebar_panel.inner;

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
                let has_table =
                    matches!(&self.schema.selected_node, Some(SelectedNode::Table { .. }));

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
                                tab_bar_result = tab_bar::show(
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
                    let pk_missing = self.table_view.table_data.is_some()
                        && self.table_view.primary_keys.is_empty()
                        && !self.table_view.pending_edits.is_empty();
                    let apply_frame =
                        egui::Frame::side_top_panel(ui.style()).fill(theme::get(ui).bg_raised);
                    egui::Panel::bottom("apply_bar")
                        .exact_size(36.0)
                        .frame(apply_frame)
                        .show(ui, |ui| {
                            ui.push_id(sid, |ui| {
                                apply_bar_action = results_grid::show_apply_bar(
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
                                    Some((db.clone(), table.clone()))
                                }
                                _ => None,
                            };
                            if selected_table.is_some() {
                                let action = filter_bar::show(
                                    ui,
                                    &mut self.table_view.filter_text,
                                    f9 && !running,
                                    &self.table_view.filter_history.clone(),
                                );
                                if !running {
                                    filter_action = action;
                                }
                                ui.separator();
                            }
                            let col_info_key = match &self.schema.selected_node {
                                Some(SelectedNode::Table { db, table }) => {
                                    Some((db.clone(), table.clone()))
                                }
                                _ => None,
                            };
                            let empty_cols: Vec<ColumnInfo> = Vec::new();
                            let col_info_slice = col_info_key
                                .as_ref()
                                .and_then(|k| self.schema.columns_info.get(k))
                                .map(|v| v.as_slice())
                                .unwrap_or(empty_cols.as_slice());
                            let action = results_grid::show(
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
                                // Con query en curso: spinner en el grid
                                self.table_view.query_loading || running,
                            );
                            if !running {
                                grid_action = action;
                            } else if matches!(action, GridAction::SortChanged) {
                                self.table_view.sort_specs = sorts_before.clone();
                            }
                        }

                        _ if running => {
                            ui.centered_and_justified(|ui| {
                                ui.spinner();
                                ui.label("Ejecutando consulta…");
                            });
                        }

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
                                let tables = self.schema.tables_info.get(db).unwrap_or(&empty);
                                db_info_action = db_info::show(ui, db, tables);
                            } else {
                                ui.centered_and_justified(|ui| {
                                    ui.label("Selecciona una base de datos en el panel izquierdo");
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

                        ActiveTab::Sql(tab_idx) => {
                            if let Some(tab) = self.sql_tabs.get_mut(tab_idx) {
                                let result_clone = tab.result.clone();
                                let databases_clone = self.schema.databases.clone();
                                sql_action = sql_editor::show(
                                    ui,
                                    &mut tab.sql_text,
                                    &result_clone,
                                    f9,
                                    &mut tab.db_context,
                                    &databases_clone,
                                );
                            }
                        }
                    });
                });
            }
        }

        // ── Modal (sobre todo) ────────────────────────────────────────────
        let modal_action = modals_ui::show(ctx, &mut self.modal);

        // ── Aplicar señales ───────────────────────────────────────────────

        if cancel_connect {
            self.state = ConnState::Disconnected;
        }

        // Handle tab bar result
        if let Some(t) = tab_bar_result.tab_clicked {
            self.active_tab = t;
        }

        // Ctrl+T solo con conexión: en el formulario guardaría pestañas bajo la
        // clave de unos parámetros a medio escribir.
        if tab_bar_result.new_sql_tab || (ctrl_t && self.is_connected()) {
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
            self.save_tabs_state(sh);
        }

        if let Some(close_idx) = tab_bar_result.close_sql_tab
            && close_idx < self.sql_tabs.len()
        {
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
            self.save_tabs_state(sh);
        }

        // Grid action
        match grid_action {
            GridAction::SortChanged => {
                // Re-ejecutar con ORDER BY para sort server-side (LIMIT correcto)
                let sorts = std::mem::replace(&mut self.table_view.sort_specs, sorts_before);
                self.request_reload(sh, Reload::Sort(sorts));
            }
            GridAction::CellEditStarted {
                row,
                col_idx,
                current_value,
            } => {
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
                            && col_idx < ins.values.len()
                        {
                            ins.values[col_idx] = None;
                        }
                    }
                    RowRef::Existing(row_idx) => {
                        let original = self
                            .table_view
                            .table_data
                            .as_ref()
                            .and_then(|d| d.rows.get(row_idx))
                            .and_then(|r| r.get(col_idx))
                            .cloned()
                            .unwrap_or_default();
                        self.table_view
                            .pending_edits
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
            GridAction::CellEditCommitted {
                row,
                col_idx,
                new_value,
            } => {
                self.table_view.editing_cell = None;
                match row {
                    RowRef::Insert(insert_idx) => {
                        if let Some(ins) = self.table_view.pending_inserts.get_mut(insert_idx)
                            && col_idx < ins.values.len()
                        {
                            ins.values[col_idx] = Some(new_value);
                        }
                    }
                    RowRef::Existing(row_idx) => {
                        let original = self
                            .table_view
                            .table_data
                            .as_ref()
                            .and_then(|d| d.rows.get(row_idx))
                            .and_then(|r| r.get(col_idx))
                            .cloned()
                            .unwrap_or_default();

                        if original.as_deref() == Some(new_value.as_str()) {
                            self.table_view
                                .pending_edits
                                .retain(|e| !(e.row_idx == row_idx && e.col_idx == col_idx));
                        } else if let Some(existing) = self
                            .table_view
                            .pending_edits
                            .iter_mut()
                            .find(|e| e.row_idx == row_idx && e.col_idx == col_idx)
                        {
                            if existing.original.as_deref() == Some(new_value.as_str())
                                && !existing.is_null
                            {
                                let idx = self
                                    .table_view
                                    .pending_edits
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
            GridAction::CellEditTabNext {
                row,
                col_idx,
                new_value,
            } => {
                // Commit current value, then move focus to next column
                match row {
                    RowRef::Insert(insert_idx) => {
                        if let Some(ins) = self.table_view.pending_inserts.get_mut(insert_idx)
                            && col_idx < ins.values.len()
                        {
                            ins.values[col_idx] = Some(new_value);
                        }
                    }
                    RowRef::Existing(row_idx) => {
                        let original = self
                            .table_view
                            .table_data
                            .as_ref()
                            .and_then(|d| d.rows.get(row_idx))
                            .and_then(|r| r.get(col_idx))
                            .cloned()
                            .unwrap_or_default();
                        if original.as_deref() != Some(new_value.as_str()) {
                            self.table_view
                                .pending_edits
                                .retain(|e| !(e.row_idx == row_idx && e.col_idx == col_idx));
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
                // Move to next column
                let col_count = self
                    .table_view
                    .table_data
                    .as_ref()
                    .map(|d| d.columns.len())
                    .unwrap_or(1);
                let next_col = (col_idx + 1) % col_count;
                let next_val = match row {
                    RowRef::Insert(insert_idx) => self
                        .table_view
                        .pending_inserts
                        .get(insert_idx)
                        .and_then(|ins| ins.values.get(next_col))
                        .and_then(|v| v.as_deref())
                        .unwrap_or("")
                        .to_string(),
                    RowRef::Existing(row_idx) => self
                        .table_view
                        .table_data
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
                self.apply_pending_changes(sh);
            }
            GridAction::DiscardEdits => self.discard_pending(),
            GridAction::FilterBy { sql_fragment } => {
                self.request_reload(sh, Reload::Filter(sql_fragment));
            }
            GridAction::FilterByClipboard { col_name } => {
                match arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
                    Ok(text) => {
                        let esc_col = format!("`{}`", col_name.replace('`', "``"));
                        let esc_val = escape_sql(&text);
                        let filter = format!("{esc_col} = '{esc_val}'");
                        self.request_reload(sh, Reload::Filter(filter));
                    }
                    Err(e) => {
                        self.table_view.table_error =
                            Some(format!("No se pudo leer el portapapeles: {e}"));
                    }
                }
            }
            GridAction::FilterUserInput { col_name } => {
                self.modal = ModalState::FilterInput {
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
                let col_count = self
                    .table_view
                    .table_data
                    .as_ref()
                    .map(|d| d.columns.len())
                    .unwrap_or(0);
                if col_count == 0 {
                } else {
                    let col_info_key = match &self.schema.selected_node {
                        Some(SelectedNode::Table { db, table }) => {
                            Some((db.clone(), table.clone()))
                        }
                        _ => None,
                    };
                    let empty_cols: Vec<ColumnInfo> = Vec::new();
                    let ci = col_info_key
                        .as_ref()
                        .and_then(|k| self.schema.columns_info.get(k))
                        .map(|v| v.as_slice())
                        .unwrap_or(empty_cols.as_slice());
                    let auto_cols: Vec<bool> = (0..col_count)
                        .map(|i| {
                            ci.get(i)
                                .map(|c| c.extra.contains("auto_increment"))
                                .unwrap_or(false)
                        })
                        .collect();
                    let values: Vec<Option<String>> = (0..col_count)
                        .map(|i| {
                            if auto_cols.get(i).copied().unwrap_or(false) {
                                None
                            } else {
                                ci.get(i)
                                    .and_then(|c| c.default_val.clone())
                                    .map(Some)
                                    .unwrap_or(Some(String::new()))
                            }
                        })
                        .collect();
                    self.table_view
                        .pending_inserts
                        .push(PendingInsert { values, auto_cols });
                }
            }
            GridAction::CloneRow { row_idx } => {
                let col_count = self
                    .table_view
                    .table_data
                    .as_ref()
                    .map(|d| d.columns.len())
                    .unwrap_or(0);
                if col_count == 0 {
                } else {
                    let col_info_key = match &self.schema.selected_node {
                        Some(SelectedNode::Table { db, table }) => {
                            Some((db.clone(), table.clone()))
                        }
                        _ => None,
                    };
                    let empty_cols: Vec<ColumnInfo> = Vec::new();
                    let ci = col_info_key
                        .as_ref()
                        .and_then(|k| self.schema.columns_info.get(k))
                        .map(|v| v.as_slice())
                        .unwrap_or(empty_cols.as_slice());
                    let auto_cols: Vec<bool> = (0..col_count)
                        .map(|i| {
                            ci.get(i)
                                .map(|c| c.extra.contains("auto_increment"))
                                .unwrap_or(false)
                        })
                        .collect();
                    let row_data = self
                        .table_view
                        .table_data
                        .as_ref()
                        .and_then(|d| d.rows.get(row_idx));
                    let values: Vec<Option<String>> = (0..col_count)
                        .map(|i| {
                            if auto_cols.get(i).copied().unwrap_or(false) {
                                None // Skip auto_increment columns
                            } else {
                                row_data.and_then(|r| r.get(i)).cloned().unwrap_or(None)
                            }
                        })
                        .collect();
                    self.table_view
                        .pending_inserts
                        .push(PendingInsert { values, auto_cols });
                }
            }
            GridAction::RemoveInsertRow { insert_idx } => {
                if insert_idx < self.table_view.pending_inserts.len() {
                    self.table_view.pending_inserts.remove(insert_idx);
                }
            }
            GridAction::RowSelected {
                row_idx,
                shift,
                ctrl,
            } => {
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
                    let fmt = sh.settings.copy_format.clone();
                    let tbl = match &self.schema.selected_node {
                        Some(SelectedNode::Table { table, .. }) => Some(table.as_str()),
                        _ => None,
                    };
                    let rows: Vec<&Vec<Option<String>>> = self
                        .table_view
                        .selected_rows
                        .iter()
                        .filter_map(|&i| data.rows.get(i))
                        .collect();
                    if !rows.is_empty() {
                        let text =
                            results_grid::format_rows_for_copy(&data.columns, &rows, &fmt, tbl);
                        ctx.copy_text(text);
                    }
                }
            }
            GridAction::CopyRowsAs { fmt } => {
                if let Some(data) = &self.table_view.table_data {
                    let tbl = match &self.schema.selected_node {
                        Some(SelectedNode::Table { table, .. }) => Some(table.as_str()),
                        _ => None,
                    };
                    let rows: Vec<&Vec<Option<String>>> = self
                        .table_view
                        .selected_rows
                        .iter()
                        .filter_map(|&i| data.rows.get(i))
                        .collect();
                    if !rows.is_empty() {
                        let text =
                            results_grid::format_rows_for_copy(&data.columns, &rows, &fmt, tbl);
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
            GridAction::ApplyEdits => self.apply_pending_changes(sh),
            GridAction::DiscardEdits => self.discard_pending(),
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
                self.switch_to_table(sh, db, table, ActiveTab::Datos);
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
                if sh.settings.fav_tables.contains(&key) {
                    sh.settings.fav_tables.remove(&key);
                } else {
                    sh.settings.fav_tables.insert(key);
                }
                settings::save(&sh.settings);
            }
            SidebarAction::OpenTableStructure { db, table } => {
                self.switch_to_table(sh, db, table, ActiveTab::Tabla);
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
                self.save_tabs_state(sh);
            }
            SidebarAction::CloneTable { db, table } => {
                self.modal = ModalState::CloneTable {
                    db,
                    table,
                    new_name: String::new(),
                    copy_data: true,
                    error: None,
                };
            }
            SidebarAction::ExportDatabase(db) => {
                let tables = self.export_tables_for(&db);
                let dbs = self.schema.databases.clone();
                let profiles = sh.profiles.clone();
                self.modal = ModalState::ExportData {
                    source_db: db,
                    source_table: None,
                    tables: tables
                        .into_iter()
                        .map(|n| TableExportEntry {
                            name: n,
                            selected: true,
                            state: TableExportState::Pending,
                        })
                        .collect(),
                    databases: dbs,
                    profiles,
                    destination: ExportDestination::SqlFile {
                        path: String::new(),
                    },
                    target_db_name: String::new(),
                    profile_idx: 0,
                    options: ExportOptions::default(),
                    phase: ExportPhase::Configuring,
                };
            }
            SidebarAction::ExportTable { db, table } => {
                // Mostrar todas las tablas de la BBDD con solo la pulsada marcada.
                let mut names = self.export_tables_for(&db);
                if names.is_empty() {
                    names.push(table.clone());
                }
                let dbs = self.schema.databases.clone();
                let profiles = sh.profiles.clone();
                self.modal = ModalState::ExportData {
                    source_db: db,
                    source_table: Some(table.clone()),
                    tables: names
                        .into_iter()
                        .map(|n| TableExportEntry {
                            selected: n == table,
                            name: n,
                            state: TableExportState::Pending,
                        })
                        .collect(),
                    databases: dbs,
                    profiles,
                    destination: ExportDestination::SqlFile {
                        path: String::new(),
                    },
                    target_db_name: String::new(),
                    profile_idx: 0,
                    options: ExportOptions::default(),
                    phase: ExportPhase::Configuring,
                };
            }
        }

        if should_disconnect {
            self.disconnect();
        }

        // F5: recargar grid (invalida también caché de estructura de la tabla)
        if f5 {
            self.request_reload(sh, Reload::Refresh);
        }

        // Filter action
        match filter_action {
            filter_bar::FilterAction::Apply => {
                let filter = self.table_view.filter_text.trim().to_string();
                self.request_reload(sh, Reload::Filter(filter));
            }
            filter_bar::FilterAction::Clear => {
                self.request_reload(sh, Reload::Filter(String::new()));
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
                self.switch_to_table(sh, db, table, ActiveTab::Datos);
            }
        }

        // Persistir texto de pestañas SQL al ejecutar
        if matches!(sql_action, SqlEditorAction::Execute { .. }) {
            self.save_tabs_state(sh);
        }

        // SQL editor
        if let SqlEditorAction::Execute { sql, db_context } = sql_action {
            // Ejecutar USE por separado para que el log muestre solo la query del usuario
            if !db_context.is_empty() {
                let use_sql = format!("USE `{}`", db_context.replace('`', "``"));
                let _ = match &mut self.state {
                    ConnState::Connected(b) => b.execute_write(&use_sql),
                    _ => return outcome,
                };
            }
            let start = Instant::now();
            let result = match &mut self.state {
                ConnState::Connected(b) => b.execute_query(&sql),
                _ => return outcome,
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
                && let Some(tab) = self.sql_tabs.get_mut(tab_idx)
            {
                tab.result = query_result.clone();
            }
        }

        // Modal action
        outcome.close = matches!(modal_action, ModalAction::CloseSession);
        self.handle_modal_action(modal_action, sh);

        // Profile selection & management
        if let Some(idx) = profile_selected
            && matches!(self.state, ConnState::Disconnected)
        {
            let profile = &sh.profiles[idx];
            self.params = profile.to_params();
            self.port_str = self.params.port.to_string();
            self.profile_name = profile.name.clone();
            self.save_password = profile.has_password();
            self.color = profile.color;
            self.selected_profile_idx = Some(idx);
            self.error_message = None;
        }

        if save_profile && !self.profile_name.trim().is_empty() {
            if let Ok(p) = self.port_str.parse::<u16>() {
                self.params.port = p;
            }
            let name = self.profile_name.trim().to_string();
            let idx = self
                .selected_profile_idx
                .or_else(|| sh.profiles.iter().position(|p| p.name == name));
            let previous = idx.map(|i| &sh.profiles[i]);
            let had_keyring = previous.is_some_and(|p| p.password_in_keyring);
            let profile = ConnectionProfile {
                id: previous.map_or_else(profiles::new_id, |p| p.id.clone()),
                name,
                host: self.params.host.clone(),
                port: self.params.port,
                user: self.params.user.clone(),
                // Una contraseña nueva se marca cuando el llavero la confirma
                // (poll_keyring). Con el campo vacío se conserva la que había.
                password_in_keyring: had_keyring && self.save_password,
                password: None,
                color: self.color,
            };
            if self.save_password && !self.params.password.is_empty() {
                let id = profile.id.clone();
                let label = profile.keyring_label();
                let password = self.params.password.clone();
                let tx = self.keyring_tx.clone();
                let ctx = ctx.clone();
                std::thread::spawn(move || {
                    let result = secrets::set(&id, &label, &password);
                    let _ = tx.send((id, result));
                    ctx.request_repaint();
                });
            } else if had_keyring && !self.save_password {
                secrets::delete_in_background(profile.id.clone());
            }
            self.params.keyring_id = profile.password_in_keyring.then(|| profile.id.clone());
            match idx {
                Some(i) => sh.profiles[i] = profile,
                None => sh.profiles.push(profile),
            }
            self.selected_profile_idx = Some(idx.unwrap_or(sh.profiles.len() - 1));
            profiles::save(&sh.profiles);
        }

        if delete_profile && let Some(idx) = self.selected_profile_idx {
            let removed = sh.profiles.remove(idx);
            if removed.password_in_keyring {
                secrets::delete_in_background(removed.id);
            }
            self.selected_profile_idx = None;
            self.profile_name.clear();
            profiles::save(&sh.profiles);
            outcome.profile_removed = Some(idx);
        }

        // Iniciar conexión async
        if connect_clicked {
            if let Ok(p) = self.port_str.parse::<u16>() {
                self.params.port = p;
            }
            self.title = self
                .selected_profile_idx
                .and_then(|i| sh.profiles.get(i))
                .map(|p| p.name.clone())
                .unwrap_or_else(|| self.params.host.clone());
            let params = self.params.clone();
            let host = self.params.host.clone();
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                connect_mysql_progress(&params, &tx);
            });
            self.state = ConnState::Connecting {
                receiver: rx,
                host,
                status: "Iniciando...".to_string(),
                start: Instant::now(),
            };
            self.error_message = None;
        }

        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{DbResult, QueryEvent};

    /// Backend sin servidor: las lecturas devuelven vacío y las escrituras, ok.
    struct FakeBackend;

    impl DbBackend for FakeBackend {
        fn list_databases(&self) -> DbResult<Vec<String>> {
            Ok(Vec::new())
        }
        fn list_tables(&self, _: &str) -> DbResult<Vec<String>> {
            Ok(Vec::new())
        }
        fn execute_query(&self, _: &str) -> DbResult<QueryResult> {
            Ok(QueryResult {
                columns: Vec::new(),
                rows: Vec::new(),
            })
        }
        fn get_databases_info(&self) -> DbResult<Vec<DatabaseInfo>> {
            Ok(Vec::new())
        }
        fn get_tables_info(&self, _: &str) -> DbResult<Vec<TableInfo>> {
            Ok(Vec::new())
        }
        fn get_columns_info(&self, _: &str, _: &str) -> DbResult<Vec<ColumnInfo>> {
            Ok(Vec::new())
        }
        fn get_primary_keys(&self, _: &str, _: &str) -> DbResult<Vec<String>> {
            Ok(Vec::new())
        }
        fn execute_write(&mut self, _: &str) -> DbResult<u64> {
            Ok(0)
        }
        fn create_database(&mut self, _: &str, _: &str) -> DbResult<()> {
            Ok(())
        }
        fn drop_database(&mut self, _: &str) -> DbResult<()> {
            Ok(())
        }
        fn drop_table(&mut self, _: &str, _: &str) -> DbResult<()> {
            Ok(())
        }
        fn truncate_table(&mut self, _: &str, _: &str) -> DbResult<()> {
            Ok(())
        }
        fn execute_transaction(&mut self, _: &[String]) -> DbResult<()> {
            Ok(())
        }
    }

    fn shared() -> Shared {
        Shared {
            settings: AppSettings::default(),
            profiles: Vec::new(),
            table_states: TableStates::default(),
        }
    }

    fn data(ids: &[&str]) -> QueryResult {
        QueryResult {
            columns: vec!["id".into()],
            rows: ids.iter().map(|id| vec![Some(id.to_string())]).collect(),
        }
    }

    /// Sesión conectada con `db.t` abierta y sus datos cargados.
    fn session_with_table() -> Session {
        let mut s = Session::new(0);
        s.state = ConnState::Connected(Box::new(FakeBackend));
        s.schema.selected_node = Some(SelectedNode::Table {
            db: "db".into(),
            table: "t".into(),
        });
        s.table_view.table_data = Some(data(&["1", "2"]));
        s.table_view.primary_keys = vec!["id".into()];
        s
    }

    #[test]
    fn recargar_con_cambios_pendientes_pide_confirmacion() {
        let mut s = session_with_table();
        s.table_view.pending_deletes.insert(0);
        s.request_reload(&mut shared(), Reload::Refresh);
        assert!(matches!(
            s.modal,
            ModalState::ConfirmReload {
                pending_count: 1,
                ..
            }
        ));
        assert!(
            matches!(s.state, ConnState::Connected(_)),
            "no debe recargar"
        );
        assert_eq!(s.table_view.pending_deletes.len(), 1);
    }

    #[test]
    fn recargar_sin_cambios_lanza_la_query() {
        let mut s = session_with_table();
        s.request_reload(&mut shared(), Reload::Refresh);
        assert!(matches!(s.modal, ModalState::None));
        assert!(matches!(s.state, ConnState::QueryRunning));
    }

    #[test]
    fn descartar_y_recargar_limpia_los_cambios() {
        let mut s = session_with_table();
        s.table_view.pending_deletes.insert(1);
        s.handle_modal_action(
            ModalAction::DiscardAndReload(Reload::Refresh),
            &mut shared(),
        );
        assert_eq!(s.pending_count(), 0);
        assert!(matches!(s.state, ConnState::QueryRunning));
    }

    #[test]
    fn datos_nuevos_no_heredan_cambios_por_posicion() {
        let mut s = session_with_table();
        s.state = ConnState::QueryRunning;
        s.table_view.pending_deletes.insert(0);
        let (tx, rx) = mpsc::channel();
        s.table_view.query_receiver = Some(rx);
        tx.send(QueryEvent::Done {
            result: Ok(data(&["7", "8"])),
            backend: Box::new(FakeBackend),
            sql: String::new(),
            elapsed_ms: 0,
        })
        .unwrap();
        s.poll_query_receiver(&egui::Context::default());
        assert_eq!(s.pending_count(), 0);
    }
}

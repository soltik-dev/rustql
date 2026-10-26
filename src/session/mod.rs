mod jobs;
mod schema;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use eframe::egui;

use self::jobs::{JobResult, Jobs};
pub use self::schema::SchemaCache;
use self::schema::show_loading;
use crate::db::{
    ConnectEvent, ConnectionParams, DbBackend, DbResult, ExportDestination, ExportEvent,
    ExportOptions, QueryResult, Statement, connect_mysql_progress, escape_ident, escape_sql,
};
use crate::loadable::Loadable;
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
    EditingCell, GridAction, InsertValue, PendingEdit, PendingInsert, RowRef, SortDir, SortSpec,
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
    /// Compartido con los hilos de trabajo.
    Connected(Arc<dyn DbBackend>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum SelectedNode {
    Server,
    Database(String),
    Table { db: String, table: String },
}

// ── Sub-structs de estado ─────────────────────────────────────────────────────

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
    /// Se incrementa con cada consulta de la tabla: los resultados de una
    /// anterior que lleguen tarde se descartan.
    pub data_generation: u64,
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
    /// Trabajos contra la BD en segundo plano.
    jobs: Jobs,
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
            jobs: Jobs::default(),
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

    /// Conectando o cargando la tabla.
    pub fn is_busy(&self) -> bool {
        matches!(self.state, ConnState::Connecting { .. }) || self.table_view.query_loading
    }

    pub fn is_connected(&self) -> bool {
        matches!(self.state, ConnState::Connected(_))
    }

    /// Host y BBDD activa para la barra de estado.
    pub fn status(&self) -> (Option<String>, Option<String>) {
        let host = match &self.state {
            ConnState::Connected(_) => Some(format!("{}:{}", self.params.host, self.params.port)),
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

    /// Vuelve al formulario. La query en curso, si la hay, se descarta: su
    /// resultado ya no corresponde a nada que se esté mostrando.
    fn disconnect(&mut self) {
        self.state = ConnState::Disconnected;
        self.jobs.new_epoch();
        self.table_view.query_loading = false;
    }
}

// ── Métodos auxiliares ────────────────────────────────────────────────────────

impl Session {
    /// Lanza un trabajo en segundo plano; false si no hay conexión.
    fn spawn_job(&self, job: impl FnOnce(&dyn DbBackend) -> JobResult + Send + 'static) -> bool {
        let ConnState::Connected(backend) = &self.state else {
            return false;
        };
        self.jobs.spawn(backend, job);
        true
    }

    /// Carga los datos de la tabla en segundo plano. Sustituye a la consulta
    /// en curso, si la hay.
    fn run_query_async(&mut self, sql: String) {
        if !self.is_connected() {
            return;
        }
        self.table_view.data_generation += 1;
        let generation = self.table_view.data_generation;
        self.table_view.query_loading = true;
        self.spawn_job(move |b| {
            let start = Instant::now();
            let result = b.execute_query(&sql);
            JobResult::TableData {
                generation,
                sql,
                elapsed_ms: start.elapsed().as_millis() as u64,
                result,
            }
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

    /// Tras escribir en `db` (crear, clonar, vaciar…): descarta sus cachés de
    /// esquema para que se vuelvan a leer. No toca la tabla abierta.
    fn invalidate_schema(&mut self, db: &str) {
        self.schema.databases_info = Loadable::NotLoaded;
        // Recargar lista de bases de datos del sidebar
        self.schema.databases = Loadable::NotLoaded;
        self.schema.tables_cache.remove(db);
        self.schema.tables_info.remove(db);
    }

    /// Tras borrar `db` o, con `table`, solo esa tabla: además de invalidar
    /// las cachés, cierra lo borrado si estaba abierto.
    fn on_dropped(&mut self, db: &str, table: Option<&str>) {
        self.invalidate_schema(db);
        let (open_db, open_table) = match &self.schema.selected_node {
            Some(SelectedNode::Database(d)) => (Some(d.as_str()), None),
            Some(SelectedNode::Table { db, table }) => (Some(db.as_str()), Some(table.as_str())),
            _ => (None, None),
        };
        let closes_view = open_db == Some(db) && (table.is_none() || table == open_table);
        match table {
            Some(t) => {
                self.schema
                    .columns_info
                    .remove(&(db.to_string(), t.to_string()));
            }
            None => self.schema.columns_info.retain(|(d, _), _| d != db),
        }
        if !closes_view {
            return;
        }
        // Las pestañas SQL no dependen de la selección: se quedan donde están.
        if table.is_some() {
            self.schema.selected_node = Some(SelectedNode::Database(db.to_string()));
            if matches!(self.active_tab, ActiveTab::Datos | ActiveTab::Tabla) {
                self.active_tab = ActiveTab::Bbdd;
            }
        } else {
            self.schema.selected_node = Some(SelectedNode::Server);
            if !matches!(self.active_tab, ActiveTab::Sql(_)) {
                self.active_tab = ActiveTab::Servidor;
            }
        }
        self.table_view.table_data = None;
        self.table_view.table_error = None;
        self.table_view.primary_keys.clear();
        self.discard_pending();
        self.clear_row_selection();
    }

    /// Tras vaciar tablas de `db`: si la abierta es una de ellas, sus cambios
    /// pendientes ya no tienen filas a las que aplicarse; se recarga vacía.
    fn after_truncate(&mut self, sh: &mut Shared, db: &str, tables: &[String]) {
        self.invalidate_schema(db);
        if let Some(SelectedNode::Table { db: d, table: t }) = &self.schema.selected_node
            && d == db
            && tables.contains(t)
        {
            self.discard_pending();
            self.reload_now(sh, Reload::Refresh);
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

    /// Ejecuta deletes, edits e inserts pendientes en una sola transacción:
    /// o se guardan todos o ninguno. Devuelve true si no queda ninguno. No
    /// recarga: el llamante decide, porque la recarga cede el backend a un
    /// hilo y bloquearía otras operaciones.
    fn commit_pending_changes(&mut self) -> bool {
        if self.pending_count() == 0 {
            return true;
        }
        let stmts = match self.pending_statements() {
            Ok(stmts) => stmts,
            Err(message) => {
                self.modal = ModalState::Error {
                    title: "No se pueden guardar los cambios".into(),
                    message,
                };
                return false;
            }
        };
        let start = Instant::now();
        let result = match &mut self.state {
            ConnState::Connected(b) => b.execute_transaction(&stmts),
            _ => return false,
        };
        let duration_ms = start.elapsed().as_millis() as u64;
        let summary = format!(
            "Guardar cambios: {} eliminación(es), {} edición(es), {} inserción(es) (transacción)",
            self.table_view.pending_deletes.len(),
            self.table_view.pending_edits.len(),
            self.table_view.pending_inserts.len()
        );
        match result {
            Ok(()) => {
                self.push_log(LogEntry::ok(summary, duration_ms, stmts.len()));
                self.discard_pending();
                true
            }
            Err(e) => {
                self.push_log(LogEntry::err(summary, duration_ms, &e));
                self.modal = ModalState::Error {
                    title: "Error al guardar".into(),
                    message: e,
                };
                false
            }
        }
    }

    /// Sentencias de los cambios pendientes: DELETE, UPDATE (uno por fila,
    /// con todas sus celdas editadas) e INSERT, con los valores como
    /// parámetros. Las filas existentes se identifican por su clave primaria.
    fn pending_statements(&self) -> Result<Vec<Statement>, String> {
        let Some(SelectedNode::Table { db, table }) = &self.schema.selected_node else {
            return Err("No hay ninguna tabla abierta.".into());
        };
        let tv = &self.table_view;
        let data = tv.table_data.as_ref().ok_or("No hay datos cargados.")?;
        let target = format!("`{}`.`{}`", escape_ident(db), escape_ident(table));
        let edits_rows = !tv.pending_deletes.is_empty() || !tv.pending_edits.is_empty();
        if edits_rows && self.schema.open_table_columns().is_none() {
            return Err(
                "Aún se está leyendo la estructura de la tabla: inténtalo de nuevo.".into(),
            );
        }
        if edits_rows && tv.primary_keys.is_empty() {
            return Err(
                "La tabla no tiene clave primaria: no se pueden editar ni eliminar filas.".into(),
            );
        }
        let row_key = |row_idx: usize| {
            pk_condition(data, &tv.primary_keys, row_idx)
                .ok_or_else(|| format!("No se encuentra la clave primaria de la fila {row_idx}."))
        };

        let mut stmts = Vec::new();
        for &row_idx in &tv.pending_deletes {
            let (condition, params) = row_key(row_idx)?;
            stmts.push(Statement {
                sql: format!("DELETE FROM {target} WHERE {condition}"),
                params,
            });
        }

        // Un UPDATE por fila: si se edita la clave primaria, el WHERE (que ve
        // la fila original) sigue encontrando la fila para el resto de celdas.
        let mut edits_by_row: std::collections::BTreeMap<usize, Vec<&PendingEdit>> =
            Default::default();
        for edit in &tv.pending_edits {
            edits_by_row.entry(edit.row_idx).or_default().push(edit);
        }
        for (row_idx, edits) in edits_by_row {
            let mut sets = Vec::new();
            let mut params = Vec::new();
            for edit in edits {
                let col = data
                    .columns
                    .get(edit.col_idx)
                    .ok_or_else(|| format!("No existe la columna {}.", edit.col_idx))?;
                sets.push(format!("`{}` = ?", escape_ident(col)));
                params.push((!edit.is_null).then(|| edit.new_value.clone()));
            }
            let (condition, key) = row_key(row_idx)?;
            params.extend(key);
            stmts.push(Statement {
                sql: format!("UPDATE {target} SET {} WHERE {condition}", sets.join(", ")),
                params,
            });
        }

        // Las columnas en DEFAULT se omiten para que el servidor ponga su
        // valor por defecto; si son todas, `INSERT INTO t () VALUES ()`.
        for ins in &tv.pending_inserts {
            let (cols, params): (Vec<String>, Vec<Option<String>>) = data
                .columns
                .iter()
                .zip(&ins.values)
                .filter_map(|(col, value)| {
                    let param = match value {
                        InsertValue::Default => return None,
                        InsertValue::Null => None,
                        InsertValue::Value(v) => Some(v.clone()),
                    };
                    Some((format!("`{}`", escape_ident(col)), param))
                })
                .unzip();
            let placeholders = vec!["?"; cols.len()].join(", ");
            stmts.push(Statement {
                sql: format!(
                    "INSERT INTO {target} ({}) VALUES ({placeholders})",
                    cols.join(", ")
                ),
                params,
            });
        }
        Ok(stmts)
    }

    /// Fila nueva para el grid: vacía (todo DEFAULT, que el servidor
    /// completa) o copia de `source_row`. Las AUTO_INCREMENT, siempre DEFAULT.
    fn new_insert_row(&self, source_row: Option<usize>) -> Option<PendingInsert> {
        let data = self.table_view.table_data.as_ref()?;
        if data.columns.is_empty() {
            return None;
        }
        let columns_info = self.schema.open_table_columns();
        let auto_cols: Vec<bool> = (0..data.columns.len())
            .map(|i| {
                columns_info
                    .and_then(|ci| ci.get(i))
                    .is_some_and(|c| c.extra.contains("auto_increment"))
            })
            .collect();
        let source = source_row.and_then(|r| data.rows.get(r));
        let values = auto_cols
            .iter()
            .enumerate()
            .map(|(i, &auto)| match source {
                Some(row) if !auto => InsertValue::from(row.get(i).cloned().flatten()),
                _ => InsertValue::Default,
            })
            .collect();
        Some(PendingInsert { values, auto_cols })
    }

    /// Registra el valor nuevo de una celda (`None` = NULL). En una fila
    /// existente, volver al valor original quita el cambio pendiente.
    fn record_edit(&mut self, row: RowRef, col_idx: usize, value: Option<String>) {
        let tv = &mut self.table_view;
        match row {
            RowRef::Insert(insert_idx) => {
                if let Some(slot) = tv
                    .pending_inserts
                    .get_mut(insert_idx)
                    .and_then(|ins| ins.values.get_mut(col_idx))
                {
                    *slot = InsertValue::from(value);
                }
            }
            RowRef::Existing(row_idx) => {
                let original = tv
                    .table_data
                    .as_ref()
                    .and_then(|d| d.rows.get(row_idx))
                    .and_then(|r| r.get(col_idx))
                    .cloned()
                    .flatten();
                tv.pending_edits
                    .retain(|e| !(e.row_idx == row_idx && e.col_idx == col_idx));
                if value != original {
                    tv.pending_edits.push(PendingEdit {
                        row_idx,
                        col_idx,
                        is_null: value.is_none(),
                        new_value: value.unwrap_or_default(),
                    });
                }
            }
        }
    }

    /// Texto que muestra el grid en una celda: el del cambio pendiente si lo
    /// hay, si no el de los datos. NULL se edita como texto vacío.
    fn cell_text(&self, row: RowRef, col_idx: usize) -> String {
        let tv = &self.table_view;
        let value = match row {
            RowRef::Insert(insert_idx) => match tv
                .pending_inserts
                .get(insert_idx)
                .and_then(|ins| ins.values.get(col_idx))
            {
                Some(InsertValue::Value(v)) => Some(v.clone()),
                _ => None,
            },
            RowRef::Existing(row_idx) => {
                match tv
                    .pending_edits
                    .iter()
                    .find(|e| e.row_idx == row_idx && e.col_idx == col_idx)
                {
                    Some(edit) => (!edit.is_null).then(|| edit.new_value.clone()),
                    None => tv
                        .table_data
                        .as_ref()
                        .and_then(|d| d.rows.get(row_idx))
                        .and_then(|r| r.get(col_idx))
                        .cloned()
                        .flatten(),
                }
            }
        };
        value.unwrap_or_default()
    }

    /// Columna a la que salta Tab: la siguiente editable (en una fila nueva,
    /// las AUTO_INCREMENT no lo son), volviendo a la primera al final.
    fn next_editable_col(&self, row: RowRef, col_idx: usize) -> usize {
        let tv = &self.table_view;
        let col_count = tv.table_data.as_ref().map_or(1, |d| d.columns.len().max(1));
        let is_auto = |c: usize| match row {
            RowRef::Insert(insert_idx) => tv
                .pending_inserts
                .get(insert_idx)
                .and_then(|ins| ins.auto_cols.get(c))
                .copied()
                .unwrap_or(false),
            RowRef::Existing(_) => false,
        };
        (1..=col_count)
            .map(|step| (col_idx + step) % col_count)
            .find(|&c| !is_auto(c))
            .unwrap_or(col_idx)
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
                // Releer también la estructura: el grid la usa para las
                // claves, los NULL y el AUTO_INCREMENT de las filas nuevas.
                self.schema
                    .columns_info
                    .remove(&(db.clone(), table.clone()));
                self.request_structure(&db, &table);
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

        // 5. Estructura (columnas y claves primarias): de la caché o del servidor
        self.table_view.primary_keys.clear();
        self.request_structure(&db, &table);
        self.sync_primary_keys();

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

    fn on_connected(&mut self, backend: Arc<dyn DbBackend>, sh: &mut Shared) {
        self.state = ConnState::Connected(backend);
        self.jobs.new_epoch();
        self.error_message = None;
        self.schema = SchemaCache::default();
        self.active_tab = ActiveTab::default();
        self.table_view.table_data = None;
        self.table_view.pending_deletes.clear();
        self.table_view.pending_inserts.clear();
        self.table_view.selected_rows.clear();
        self.table_view.last_selected_row = None;
        self.table_view.drag_select_from = None;
        self.table_view.query_loading = false;
        self.table_view.table_error = None;
        self.table_view.sort_specs.clear();
        self.table_view.sort_indices.clear();
        self.table_view.filter_text.clear();
        self.table_view.pending_edits.clear();
        self.table_view.editing_cell = None;
        self.table_view.primary_keys.clear();
        self.modal = ModalState::None;
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

/// Condición `WHERE` que identifica la fila `row_idx` por su clave primaria,
/// con sus valores como parámetros. `None` si falta la fila o alguna columna
/// de la clave: una condición parcial podría afectar a varias filas.
fn pk_condition(
    data: &QueryResult,
    primary_keys: &[String],
    row_idx: usize,
) -> Option<(String, Vec<Option<String>>)> {
    let row = data.rows.get(row_idx)?;
    let mut parts = Vec::new();
    let mut params = Vec::new();
    for pk in primary_keys {
        let col_idx = data.columns.iter().position(|c| c == pk)?;
        parts.push(format!("`{}` = ?", escape_ident(pk)));
        params.push(row.get(col_idx)?.clone());
    }
    Some((parts.join(" AND "), params))
}

// ── Loop principal ────────────────────────────────────────────────────────────

impl Session {
    /// Sondea el canal del hilo de conexión y actualiza el estado si terminó.
    fn poll_connection(&mut self, ctx: &egui::Context, sh: &mut Shared) {
        let mut connect_result: Option<Result<Arc<dyn DbBackend>, String>> = None;
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

    /// Aplica los resultados de los trabajos en segundo plano que hayan
    /// terminado.
    fn poll_jobs(&mut self, ctx: &egui::Context) {
        while let Some(result) = self.jobs.try_recv(ctx) {
            self.apply_job(result);
        }
    }

    fn apply_job(&mut self, result: JobResult) {
        match result {
            JobResult::TableData {
                generation,
                sql,
                elapsed_ms,
                result,
            } => self.apply_table_data(generation, sql, elapsed_ms, result),
            JobResult::Schema(result) => self.apply_schema_job(result),
        }
    }

    fn apply_table_data(
        &mut self,
        generation: u64,
        sql: String,
        elapsed_ms: u64,
        result: DbResult<QueryResult>,
    ) {
        // Otra consulta posterior ya la ha sustituido
        if generation != self.table_view.data_generation {
            return;
        }
        self.table_view.query_loading = false;
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

    /// Tablas de una BBDD: usa la caché del sidebar o, si el árbol aún no
    /// la ha cargado, las pide al backend (y las deja en la caché).
    fn table_names(&mut self, db: &str) -> Result<Vec<String>, String> {
        if let Some(tables) = self.schema.tables(db) {
            return Ok(tables.clone());
        }
        let ConnState::Connected(backend) = &self.state else {
            return Err("No conectado".into());
        };
        match backend.list_tables(db) {
            Ok(tables) => {
                self.schema
                    .tables_cache
                    .insert(db.to_string(), Loadable::Ready(tables.clone()));
                Ok(tables)
            }
            Err(e) => {
                self.push_log(LogEntry::err(
                    format!("SHOW TABLES FROM `{db}`"),
                    0,
                    e.as_str(),
                ));
                Err(e)
            }
        }
    }

    /// Abre el diálogo de exportación de `db`; con `table`, solo esa sale
    /// marcada. Si las tablas aún no están cargadas, se piden y la lista se
    /// rellena al llegar.
    fn open_export(&mut self, sh: &Shared, db: String, table: Option<String>) {
        self.request_tables(&db);
        self.modal = ModalState::ExportData {
            source_db: db.clone(),
            source_table: table,
            tables: Vec::new(),
            databases: self.schema.databases.ready().cloned().unwrap_or_default(),
            profiles: sh.profiles.clone(),
            destination: ExportDestination::SqlFile {
                path: String::new(),
            },
            target_db_name: String::new(),
            profile_idx: 0,
            options: ExportOptions::default(),
            phase: ExportPhase::Configuring,
        };
        self.fill_export_tables(&db);
    }

    /// Rellena la lista de tablas del diálogo de exportación de `db`, si está
    /// abierto y esperándolas.
    pub(super) fn fill_export_tables(&mut self, db: &str) {
        let ModalState::ExportData {
            source_db,
            source_table,
            tables: entries,
            phase: ExportPhase::Configuring,
            ..
        } = &mut self.modal
        else {
            return;
        };
        if source_db != db || !entries.is_empty() {
            return;
        }
        let mut names = match self.schema.tables_cache.get(db) {
            Some(Loadable::Ready(tables)) => tables.clone(),
            Some(Loadable::Failed(_)) => Vec::new(),
            _ => return,
        };
        if names.is_empty()
            && let Some(table) = source_table
        {
            names.push(table.clone());
        }
        *entries = names
            .into_iter()
            .map(|name| TableExportEntry {
                selected: source_table.as_ref().is_none_or(|t| *t == name),
                name,
                state: TableExportState::Pending,
            })
            .collect();
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
                        self.schema.databases = Loadable::NotLoaded;
                        self.schema.databases_info = Loadable::NotLoaded;
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
                        self.invalidate_schema(&db);
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
                    self.on_dropped(&db, None);
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
                    self.on_dropped(&db, Some(&table));
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
                self.modal = ModalState::None;
                if result.is_ok() {
                    self.after_truncate(sh, &db, &[table]);
                }
            }
            ModalAction::TruncateAllTables(db) => {
                self.modal = ModalState::None;
                // El árbol puede no haber cargado aún las tablas de esta BBDD
                let tables = match self.table_names(&db) {
                    Ok(tables) => tables,
                    Err(message) => {
                        self.modal = ModalState::Error {
                            title: format!("No se pudieron listar las tablas de «{db}»"),
                            message,
                        };
                        return;
                    }
                };
                let failed: Vec<String> = tables
                    .iter()
                    .filter_map(|table| {
                        self.run_write(&format!(
                            "TRUNCATE TABLE `{}`.`{}`",
                            escape_ident(&db),
                            escape_ident(table)
                        ))
                        .err()
                        .map(|e| format!("{table}: {e}"))
                    })
                    .collect();
                self.after_truncate(sh, &db, &tables);
                if !failed.is_empty() {
                    self.modal = ModalState::Error {
                        title: format!("No se pudieron vaciar todas las tablas de «{db}»"),
                        message: failed.join("\n"),
                    };
                }
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
                        self.invalidate_schema(&db);
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
        self.poll_jobs(ctx);
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
            self.request_tab_data();
        }

        // ── Señales del frame ────────────────────────────────────────────
        let mut connect_clicked = false;
        let mut cancel_connect = false;
        let mut save_profile = false;
        let mut delete_profile = false;
        let mut profile_selected: Option<usize> = None;
        let mut sidebar_action = SidebarAction::None;
        let mut sidebar_loads: Vec<String> = Vec::new();
        let mut sidebar_loads_all = false;
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

            ConnState::Connected(_) => {
                // Mientras carga la tabla, el grid se muestra sin aplicar sus
                // acciones: irían por posición sobre los datos anteriores.
                let running = self.table_view.query_loading;

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
                                    &self.params.host,
                                    &self.title,
                                    conn_color,
                                )
                            })
                            .inner
                        });
                    let output = sidebar_panel.inner;
                    sidebar_action = output.action;
                    sidebar_loads = output.load_tables;
                    sidebar_loads_all = output.load_all_tables;

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
                            let col_info_slice = self
                                .schema
                                .open_table_columns()
                                .map_or(&[][..], Vec::as_slice);
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
                                running,
                            );
                            if !running {
                                grid_action = action;
                            } else if matches!(action, GridAction::SortChanged) {
                                self.table_view.sort_specs = sorts_before.clone();
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
                                        db_info_action = db_info::show(ui, db, tables);
                                    }
                                    other => {
                                        show_loading(ui, other.unwrap_or(&Loadable::NotLoaded))
                                    }
                                }
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
                                match self.schema.columns_info.get(&key) {
                                    Some(Loadable::Ready(cols)) => {
                                        table_structure::show(ui, db, table, cols);
                                    }
                                    other => {
                                        show_loading(ui, other.unwrap_or(&Loadable::NotLoaded))
                                    }
                                }
                            } else {
                                ui.centered_and_justified(|ui| {
                                    ui.label("Selecciona una tabla en el panel izquierdo");
                                });
                            }
                        }

                        ActiveTab::Sql(tab_idx) => {
                            if let Some(tab) = self.sql_tabs.get_mut(tab_idx) {
                                let result_clone = tab.result.clone();
                                let databases_clone =
                                    self.schema.databases.ready().cloned().unwrap_or_default();
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
                self.record_edit(row, col_idx, None);
            }
            GridAction::CellEditCommitted {
                row,
                col_idx,
                new_value,
            } => {
                self.table_view.editing_cell = None;
                self.record_edit(row, col_idx, Some(new_value));
            }
            GridAction::CellEditCancelled => {
                self.table_view.editing_cell = None;
            }
            GridAction::CellEditTabNext {
                row,
                col_idx,
                new_value,
            } => {
                self.record_edit(row, col_idx, Some(new_value));
                let next_col = self.next_editable_col(row, col_idx);
                self.table_view.editing_cell = Some(EditingCell {
                    row,
                    col_idx: next_col,
                    buffer: self.cell_text(row, next_col),
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
                if let Some(row) = self.new_insert_row(None) {
                    self.table_view.pending_inserts.push(row);
                }
            }
            GridAction::CloneRow { row_idx } => {
                if let Some(row) = self.new_insert_row(Some(row_idx)) {
                    self.table_view.pending_inserts.push(row);
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
            SidebarAction::Refresh => self.schema.clear(),
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
            SidebarAction::ExportDatabase(db) => self.open_export(sh, db, None),
            SidebarAction::ExportTable { db, table } => self.open_export(sh, db, Some(table)),
        }

        // Datos que el sidebar necesita para dibujar el árbol
        for db in &sidebar_loads {
            self.request_tables(db);
        }
        if sidebar_loads_all {
            self.request_all_tables();
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
            let db = (!db_context.is_empty()).then_some(db_context.as_str());
            let start = Instant::now();
            let result = match &self.state {
                ConnState::Connected(b) => b.execute_in_session(db, &sql),
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
    use super::schema::SchemaResult;
    use super::*;
    use crate::db::{ColumnInfo, DatabaseInfo, DbResult, TableInfo};

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
        fn execute_in_session(&self, _: Option<&str>, sql: &str) -> DbResult<QueryResult> {
            self.execute_query(sql)
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
        fn list_all_tables(&self) -> DbResult<Vec<(String, String)>> {
            Ok(Vec::new())
        }
        fn execute_write(&self, _: &str) -> DbResult<u64> {
            Ok(0)
        }
        fn create_database(&self, _: &str, _: &str) -> DbResult<()> {
            Ok(())
        }
        fn drop_database(&self, _: &str) -> DbResult<()> {
            Ok(())
        }
        fn drop_table(&self, _: &str, _: &str) -> DbResult<()> {
            Ok(())
        }
        fn truncate_table(&self, _: &str, _: &str) -> DbResult<()> {
            Ok(())
        }
        fn execute_transaction(&self, _: &[Statement]) -> DbResult<()> {
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

    fn column(name: &str, key: &str, extra: &str) -> ColumnInfo {
        ColumnInfo {
            name: name.into(),
            col_type: "INT".into(),
            is_nullable: true,
            default_val: None,
            column_key: key.into(),
            extra: extra.into(),
        }
    }

    /// Sesión conectada con `db.t` (clave primaria `id`) abierta y sus datos
    /// y estructura cargados.
    fn session_with_table() -> Session {
        let mut s = Session::new(0);
        s.state = ConnState::Connected(Arc::new(FakeBackend));
        s.schema.selected_node = Some(SelectedNode::Table {
            db: "db".into(),
            table: "t".into(),
        });
        s.table_view.table_data = Some(data(&["1", "2"]));
        s.schema.columns_info.insert(
            ("db".into(), "t".into()),
            Loadable::Ready(vec![column("id", "PRI", ""), column("v", "", "")]),
        );
        s.sync_primary_keys();
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
        assert!(s.table_view.query_loading);
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
        assert!(s.table_view.query_loading);
    }

    #[test]
    fn datos_nuevos_no_heredan_cambios_por_posicion() {
        let mut s = session_with_table();
        s.table_view.pending_deletes.insert(0);
        s.apply_job(JobResult::TableData {
            generation: s.table_view.data_generation,
            sql: String::new(),
            elapsed_ms: 0,
            result: Ok(data(&["7", "8"])),
        });
        assert_eq!(s.pending_count(), 0);
    }

    #[test]
    fn los_datos_de_una_consulta_sustituida_se_descartan() {
        let mut s = session_with_table();
        s.request_reload(&mut shared(), Reload::Refresh);
        let stale = s.table_view.data_generation;
        s.request_reload(&mut shared(), Reload::Refresh);
        s.apply_job(JobResult::TableData {
            generation: stale,
            sql: String::new(),
            elapsed_ms: 0,
            result: Ok(data(&["viejo"])),
        });
        assert_eq!(s.table_view.table_data, Some(data(&["1", "2"])));
        assert!(
            s.table_view.query_loading,
            "la consulta nueva sigue en curso"
        );
    }

    fn open_table(s: &Session) -> Option<(&str, &str)> {
        match &s.schema.selected_node {
            Some(SelectedNode::Table { db, table }) => Some((db.as_str(), table.as_str())),
            _ => None,
        }
    }

    #[test]
    fn crear_tabla_conserva_la_vista_y_los_cambios() {
        let mut s = session_with_table();
        s.active_tab = ActiveTab::Datos;
        s.table_view.pending_deletes.insert(0);
        let mut col = NewColumnDef::new();
        col.name = "id".into();
        s.handle_modal_action(
            ModalAction::CreateTable {
                db: "db".into(),
                name: "nueva".into(),
                columns: vec![col],
            },
            &mut shared(),
        );
        assert_eq!(open_table(&s), Some(("db", "t")));
        assert_eq!(s.active_tab, ActiveTab::Datos);
        assert_eq!(s.pending_count(), 1);
    }

    #[test]
    fn borrar_la_tabla_abierta_la_cierra() {
        let mut s = session_with_table();
        s.active_tab = ActiveTab::Datos;
        s.table_view.pending_deletes.insert(0);
        let action = ModalAction::DropTable {
            db: "db".into(),
            table: "t".into(),
        };
        s.handle_modal_action(action, &mut shared());
        assert_eq!(
            s.schema.selected_node,
            Some(SelectedNode::Database("db".into()))
        );
        assert_eq!(s.active_tab, ActiveTab::Bbdd);
        assert!(s.table_view.table_data.is_none());
        assert_eq!(s.pending_count(), 0);
    }

    #[test]
    fn borrar_otra_tabla_no_toca_la_abierta() {
        let mut s = session_with_table();
        s.table_view.pending_deletes.insert(0);
        let action = ModalAction::DropTable {
            db: "db".into(),
            table: "otra".into(),
        };
        s.handle_modal_action(action, &mut shared());
        assert_eq!(open_table(&s), Some(("db", "t")));
        assert_eq!(s.pending_count(), 1);
    }

    #[test]
    fn borrar_la_bbdd_desde_una_pestana_sql_no_cambia_de_pestana() {
        let mut s = session_with_table();
        s.active_tab = ActiveTab::Sql(0);
        s.handle_modal_action(ModalAction::DropDatabase("db".into()), &mut shared());
        assert_eq!(s.schema.selected_node, Some(SelectedNode::Server));
        assert_eq!(s.active_tab, ActiveTab::Sql(0));
    }

    #[test]
    fn vaciar_la_tabla_abierta_la_recarga_sin_cambios() {
        let mut s = session_with_table();
        s.active_tab = ActiveTab::Datos;
        s.table_view.pending_edits.push(PendingEdit {
            row_idx: 0,
            col_idx: 0,
            new_value: "9".into(),
            is_null: false,
        });
        let action = ModalAction::TruncateTable {
            db: "db".into(),
            table: "t".into(),
        };
        s.handle_modal_action(action, &mut shared());
        assert_eq!(open_table(&s), Some(("db", "t")));
        assert_eq!(s.pending_count(), 0);
        assert!(s.table_view.query_loading);
    }

    fn edit(row_idx: usize, col_idx: usize, value: Option<&str>) -> PendingEdit {
        PendingEdit {
            row_idx,
            col_idx,
            new_value: value.unwrap_or_default().into(),
            is_null: value.is_none(),
        }
    }

    #[test]
    fn cambios_pendientes_como_sentencias_con_parametros() {
        let mut s = session_with_table();
        s.table_view.table_data = Some(QueryResult {
            columns: vec!["id".into(), "v".into()],
            rows: vec![
                vec![Some("1".into()), Some("a".into())],
                vec![Some("2".into()), None],
            ],
        });
        s.table_view.pending_deletes.insert(1);
        // Clave primaria y otra columna de la misma fila: un único UPDATE
        s.table_view.pending_edits = vec![edit(0, 0, Some("10")), edit(0, 1, None)];
        s.table_view.pending_inserts.push(PendingInsert {
            values: vec![
                InsertValue::Value("3".into()),
                InsertValue::Value("x'y".into()),
            ],
            auto_cols: vec![false, false],
        });

        let stmts = s.pending_statements().unwrap();
        let p = |v: &[Option<&str>]| v.iter().map(|x| x.map(String::from)).collect::<Vec<_>>();
        assert_eq!(
            stmts,
            [
                Statement {
                    sql: "DELETE FROM `db`.`t` WHERE `id` = ?".into(),
                    params: p(&[Some("2")]),
                },
                Statement {
                    sql: "UPDATE `db`.`t` SET `id` = ?, `v` = ? WHERE `id` = ?".into(),
                    params: p(&[Some("10"), None, Some("1")]),
                },
                Statement {
                    sql: "INSERT INTO `db`.`t` (`id`, `v`) VALUES (?, ?)".into(),
                    params: p(&[Some("3"), Some("x'y")]),
                },
            ]
        );
    }

    #[test]
    fn sin_clave_primaria_no_se_editan_filas() {
        let mut s = session_with_table();
        s.table_view.primary_keys.clear();
        s.table_view.pending_deletes.insert(0);
        assert!(!s.commit_pending_changes());
        assert!(matches!(s.modal, ModalState::Error { .. }));
        assert_eq!(s.pending_count(), 1);
    }

    #[test]
    #[ignore = "necesita RUSTQL_TEST_MYSQL"]
    fn guardar_cambios_contra_el_servidor() {
        let b = crate::db::test_server::backend();
        b.execute_write("DROP DATABASE IF EXISTS rustql_test_grid")
            .unwrap();
        b.execute_write("CREATE DATABASE rustql_test_grid").unwrap();
        b.execute_write("CREATE TABLE rustql_test_grid.t (id VARCHAR(20) PRIMARY KEY, v TEXT)")
            .unwrap();
        b.execute_write(
            "INSERT INTO rustql_test_grid.t VALUES ('a','uno'),('b','dos'),('c','tres')",
        )
        .unwrap();
        let select = "SELECT * FROM rustql_test_grid.t ORDER BY id";

        let mut s = Session::new(0);
        s.table_view.table_data = Some(b.execute_query(select).unwrap());
        let columns = b.get_columns_info("rustql_test_grid", "t").unwrap();
        s.schema.columns_info.insert(
            ("rustql_test_grid".into(), "t".into()),
            Loadable::Ready(columns),
        );
        s.state = ConnState::Connected(Arc::new(b));
        s.schema.selected_node = Some(SelectedNode::Table {
            db: "rustql_test_grid".into(),
            table: "t".into(),
        });
        s.sync_primary_keys();
        s.table_view.pending_deletes.insert(2);
        s.table_view.pending_edits = vec![edit(0, 0, Some("a'\\x")), edit(0, 1, None)];
        s.table_view.pending_inserts.push(PendingInsert {
            values: vec![
                InsertValue::Value("d".into()),
                InsertValue::Value("it's \\ fine".into()),
            ],
            auto_cols: vec![false, false],
        });

        let saved = s.commit_pending_changes();
        let ConnState::Connected(b) = &s.state else {
            unreachable!()
        };
        let rows = b.execute_query(select).map(|r| r.rows);
        b.execute_write("DROP DATABASE rustql_test_grid").unwrap();

        assert!(saved, "{:?}", s.modal);
        let row = |id: &str, v: Option<&str>| vec![Some(id.to_string()), v.map(String::from)];
        assert_eq!(
            rows.unwrap(),
            [
                row("a'\\x", None),
                row("b", Some("dos")),
                row("d", Some("it's \\ fine")),
            ]
        );
    }

    #[test]
    fn volver_al_valor_original_quita_el_cambio() {
        let mut s = session_with_table();
        let cell = RowRef::Existing(0);
        s.record_edit(cell, 0, Some("9".into()));
        assert_eq!(s.cell_text(cell, 0), "9");
        // Tab o Enter con el valor original: antes, Tab dejaba el «9» pendiente
        s.record_edit(cell, 0, Some("1".into()));
        assert!(s.table_view.pending_edits.is_empty());
        assert_eq!(s.cell_text(cell, 0), "1");
    }

    #[test]
    fn null_sobre_null_no_es_un_cambio() {
        let mut s = session_with_table();
        s.table_view.table_data.as_mut().unwrap().rows[0][0] = None;
        s.record_edit(RowRef::Existing(0), 0, None);
        assert!(s.table_view.pending_edits.is_empty());
        s.record_edit(RowRef::Existing(0), 0, Some(String::new()));
        assert_eq!(s.table_view.pending_edits.len(), 1, "NULL → '' sí cambia");
    }

    #[test]
    fn tab_salta_las_columnas_auto_de_una_fila_nueva() {
        let mut s = session_with_table();
        s.table_view.table_data.as_mut().unwrap().columns =
            vec!["id".into(), "a".into(), "b".into()];
        s.table_view.pending_inserts.push(PendingInsert {
            values: vec![InsertValue::Default; 3],
            auto_cols: vec![true, false, false],
        });
        let row = RowRef::Insert(0);
        assert_eq!(s.next_editable_col(row, 1), 2);
        assert_eq!(
            s.next_editable_col(row, 2),
            1,
            "da la vuelta sin pasar por id"
        );
        assert_eq!(s.next_editable_col(RowRef::Existing(0), 2), 0);
    }

    #[test]
    #[ignore = "necesita RUSTQL_TEST_MYSQL"]
    fn vaciar_todas_sin_haber_desplegado_el_arbol() {
        let b = crate::db::test_server::backend();
        for sql in [
            "DROP DATABASE IF EXISTS rustql_test_truncate",
            "CREATE DATABASE rustql_test_truncate",
            "CREATE TABLE rustql_test_truncate.padre (id INT PRIMARY KEY)",
            "CREATE TABLE rustql_test_truncate.hija (id INT, \
             FOREIGN KEY (id) REFERENCES rustql_test_truncate.padre (id))",
            "CREATE TABLE rustql_test_truncate.suelta (id INT)",
            "INSERT INTO rustql_test_truncate.padre VALUES (1)",
            "INSERT INTO rustql_test_truncate.hija VALUES (1)",
            "INSERT INTO rustql_test_truncate.suelta VALUES (1), (2)",
        ] {
            b.execute_write(sql).unwrap();
        }
        let mut s = Session::new(0);
        s.state = ConnState::Connected(Arc::new(b));
        assert!(s.schema.tables_cache.is_empty());

        s.handle_modal_action(
            ModalAction::TruncateAllTables("rustql_test_truncate".into()),
            &mut shared(),
        );
        let ConnState::Connected(b) = &s.state else {
            unreachable!()
        };
        let count = |t: &str| {
            b.execute_query(&format!("SELECT COUNT(*) FROM rustql_test_truncate.{t}"))
                .unwrap()
                .rows[0][0]
                .clone()
        };
        let (suelta, hija, padre) = (count("suelta"), count("hija"), count("padre"));
        b.execute_write("DROP DATABASE rustql_test_truncate")
            .unwrap();

        assert_eq!(suelta, Some("0".into()));
        assert_eq!(hija, Some("0".into()));
        // La referenciada por una clave foránea no se puede truncar: se avisa
        assert_eq!(padre, Some("1".into()));
        assert!(
            matches!(&s.modal, ModalState::Error { message, .. } if message.starts_with("padre:")),
            "{:?}",
            s.modal
        );
    }

    /// `db.t` con columnas id (AUTO_INCREMENT), v y n.
    fn session_with_auto_id() -> Session {
        let mut s = session_with_table();
        s.table_view.table_data = Some(QueryResult {
            columns: vec!["id".into(), "v".into(), "n".into()],
            rows: vec![vec![Some("1".into()), Some("a".into()), None]],
        });
        s.schema.columns_info.insert(
            ("db".into(), "t".into()),
            Loadable::Ready(vec![
                column("id", "PRI", "auto_increment"),
                column("v", "", ""),
                column("n", "", ""),
            ]),
        );
        s
    }

    #[test]
    fn fila_nueva_deja_los_valores_por_defecto_al_servidor() {
        let mut s = session_with_auto_id();
        let row = s.new_insert_row(None).unwrap();
        // Antes: '' en v y n, y el texto «CURRENT_TIMESTAMP» como valor
        assert_eq!(row.values, vec![InsertValue::Default; 3]);
        s.table_view.pending_inserts.push(row);
        s.record_edit(RowRef::Insert(0), 1, Some("x".into()));
        let stmts = s.pending_statements().unwrap();
        assert_eq!(stmts[0].sql, "INSERT INTO `db`.`t` (`v`) VALUES (?)");
        assert_eq!(stmts[0].params, [Some("x".to_string())]);
    }

    #[test]
    fn fila_nueva_sin_tocar_inserta_todo_por_defecto() {
        let mut s = session_with_auto_id();
        let row = s.new_insert_row(None).unwrap();
        s.table_view.pending_inserts.push(row);
        let stmts = s.pending_statements().unwrap();
        assert_eq!(stmts[0].sql, "INSERT INTO `db`.`t` () VALUES ()");
    }

    #[test]
    fn clonar_fila_copia_valores_y_nulls_salvo_el_auto_increment() {
        let s = session_with_auto_id();
        let row = s.new_insert_row(Some(0)).unwrap();
        assert_eq!(
            row.values,
            [
                InsertValue::Default,
                InsertValue::Value("a".into()),
                InsertValue::Null
            ]
        );
    }

    #[test]
    #[ignore = "necesita RUSTQL_TEST_MYSQL"]
    fn fila_nueva_contra_el_servidor() {
        let b = crate::db::test_server::backend();
        b.execute_write("DROP DATABASE IF EXISTS rustql_test_insert")
            .unwrap();
        b.execute_write("CREATE DATABASE rustql_test_insert")
            .unwrap();
        b.execute_write(
            "CREATE TABLE rustql_test_insert.t (id INT AUTO_INCREMENT PRIMARY KEY, \
             creado DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP, n INT NULL, \
             v VARCHAR(10) NOT NULL DEFAULT 'x')",
        )
        .unwrap();
        let select = "SELECT id, creado IS NOT NULL, n, v FROM rustql_test_insert.t";

        let mut s = Session::new(0);
        s.table_view.table_data = Some(QueryResult {
            columns: vec!["id".into(), "creado".into(), "n".into(), "v".into()],
            rows: Vec::new(),
        });
        let key = ("rustql_test_insert".to_string(), "t".to_string());
        let columns = b.get_columns_info(&key.0, &key.1).unwrap();
        s.schema
            .columns_info
            .insert(key.clone(), Loadable::Ready(columns));
        s.schema.selected_node = Some(SelectedNode::Table {
            db: key.0,
            table: key.1,
        });
        s.state = ConnState::Connected(Arc::new(b));
        let row = s.new_insert_row(None).unwrap();
        s.table_view.pending_inserts.push(row);

        let saved = s.commit_pending_changes();
        let ConnState::Connected(b) = &s.state else {
            unreachable!()
        };
        let rows = b.execute_query(select).map(|r| r.rows);
        b.execute_write("DROP DATABASE rustql_test_insert").unwrap();

        assert!(saved, "{:?}", s.modal);
        let some = |v: &str| Some(v.to_string());
        assert_eq!(rows.unwrap(), [[some("1"), some("1"), None, some("x")]]);
    }

    fn ticket<T>(slot: Option<&Loadable<T>>) -> u64 {
        match slot {
            Some(Loadable::Loading(ticket)) => *ticket,
            _ => panic!("no está cargando"),
        }
    }

    #[test]
    fn un_fallo_de_esquema_no_se_reintenta_solo() {
        let mut s = session_with_table();
        s.schema.databases = Loadable::Failed("sin permiso".into());
        s.request_tab_data();
        assert!(matches!(s.schema.databases, Loadable::Failed(_)));
        // Refrescar sí lo vuelve a pedir
        s.schema.clear();
        s.request_tab_data();
        assert!(matches!(s.schema.databases, Loadable::Loading(_)));
    }

    #[test]
    fn la_respuesta_de_una_peticion_invalidada_se_descarta() {
        let mut s = session_with_table();
        s.request_tables("db");
        let old = ticket(s.schema.tables_cache.get("db"));
        s.invalidate_schema("db");
        s.request_tables("db");
        let new = ticket(s.schema.tables_cache.get("db"));
        s.apply_schema_job(SchemaResult::Tables {
            ticket: old,
            db: "db".into(),
            result: Ok(vec!["vieja".into()]),
        });
        assert!(matches!(s.schema.tables_cache["db"], Loadable::Loading(t) if t == new));
        s.apply_schema_job(SchemaResult::Tables {
            ticket: new,
            db: "db".into(),
            result: Ok(vec!["nueva".into()]),
        });
        assert_eq!(s.schema.tables("db"), Some(&vec!["nueva".to_string()]));
    }

    #[test]
    fn el_buscador_pide_todas_las_tablas_en_una_consulta() {
        let mut s = session_with_table();
        s.schema.databases = Loadable::Ready(vec!["a".into(), "b".into(), "c".into()]);
        s.schema
            .tables_cache
            .insert("a".into(), Loadable::Ready(vec!["ya".into()]));
        s.request_all_tables();
        let t = ticket(s.schema.tables_cache.get("b"));
        assert_eq!(
            ticket(s.schema.tables_cache.get("c")),
            t,
            "una sola petición"
        );
        s.apply_schema_job(SchemaResult::AllTables {
            ticket: t,
            result: Ok(vec![("b".into(), "x".into()), ("otra".into(), "y".into())]),
        });
        assert_eq!(s.schema.tables("a"), Some(&vec!["ya".to_string()]));
        assert_eq!(s.schema.tables("b"), Some(&vec!["x".to_string()]));
        assert_eq!(s.schema.tables("c"), Some(&Vec::new()), "BBDD sin tablas");
    }

    #[test]
    fn exportar_rellena_las_tablas_cuando_llegan() {
        let mut s = session_with_table();
        s.open_export(&shared(), "db".into(), Some("t2".into()));
        let t = ticket(s.schema.tables_cache.get("db"));
        s.apply_schema_job(SchemaResult::Tables {
            ticket: t,
            db: "db".into(),
            result: Ok(vec!["t1".into(), "t2".into()]),
        });
        let ModalState::ExportData { tables, .. } = &s.modal else {
            panic!("{:?}", s.modal)
        };
        let marked: Vec<_> = tables
            .iter()
            .map(|e| (e.name.as_str(), e.selected))
            .collect();
        assert_eq!(marked, [("t1", false), ("t2", true)]);
    }

    #[test]
    fn la_clave_primaria_sale_de_la_estructura() {
        let mut s = session_with_table();
        s.reload_now(&mut shared(), Reload::Refresh);
        let t = ticket(s.schema.columns_info.get(&("db".into(), "t".into())));
        s.apply_schema_job(SchemaResult::Structure {
            ticket: t,
            db: "db".into(),
            table: "t".into(),
            result: Ok(vec![column("a", "PRI", ""), column("b", "PRI", "")]),
        });
        assert_eq!(s.table_view.primary_keys, ["a", "b"]);
    }
}

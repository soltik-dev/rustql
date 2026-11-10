mod actions;
mod jobs;
mod schema;
mod view;
mod writes;

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use eframe::egui;

use self::jobs::{JobResult, Jobs};
pub use self::schema::SchemaCache;
use crate::db::{
    ConnectEvent, ConnectionParams, DbBackend, DbResult, ExportDestination, ExportEvent,
    ExportOptions, QueryResult, Statement, qualified, quote_ident,
};
use crate::loadable::Loadable;
use crate::log::LogEntry;
use crate::profiles::{self, ConnColor, ConnectionProfile};
use crate::settings::{self, AppSettings, SavedSqlTab};
use crate::table_state::{self, TableStates};
use crate::ui::modals as modals_ui;
use crate::ui::modals::{
    self, ExportPhase, ModalAction, ModalState, Reload, TableExportEntry, TableExportState,
};
use crate::ui::results_grid::{
    InsertValue, PendingEdit, PendingInsert, RowRef, SortDir, TableViewState,
};
use crate::ui::sql_editor::SqlTab;
use crate::ui::tab_bar::ActiveTab;

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
#[derive(Clone, Copy, Default)]
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
            ctrl_enter: i.modifiers.command && i.key_pressed(egui::Key::Enter),
            ctrl_t: i.modifiers.command && i.key_pressed(egui::Key::T),
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
    /// Escrituras lanzadas y aún sin resultado.
    writes_in_flight: usize,
    /// Ejecuciones del editor SQL lanzadas: identifican su pestaña.
    editor_runs: u64,
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
            writes_in_flight: 0,
            editor_runs: 0,
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
        matches!(self.state, ConnState::Connecting { .. })
            || self.table_view.query_loading
            || self.writes_in_flight > 0
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
        for tab in &mut self.sql_tabs {
            tab.running = None;
        }
    }
}

// ── Métodos auxiliares ────────────────────────────────────────────────────────

impl Session {
    /// Lanza un trabajo en segundo plano; false si no hay conexión.
    fn spawn_job(
        &mut self,
        job: impl FnOnce(&dyn DbBackend) -> JobResult + Send + 'static,
    ) -> bool {
        let ConnState::Connected(backend) = &self.state else {
            return false;
        };
        self.jobs.spawn(backend, job);
        true
    }

    /// Ejecuta SQL del editor en segundo plano; el resultado irá a la pestaña
    /// `tab_idx` (aunque cambie de posición entretanto).
    fn run_editor(&mut self, tab_idx: usize, sql: String, db_context: String, max_rows: usize) {
        self.editor_runs += 1;
        let token = self.editor_runs;
        let spawned = self.spawn_job(move |b| {
            let start = Instant::now();
            let db = (!db_context.is_empty()).then_some(db_context.as_str());
            let result = b.execute_in_session(db, &sql, max_rows);
            JobResult::Editor {
                token,
                sql,
                elapsed_ms: start.elapsed().as_millis() as u64,
                result,
            }
        });
        if spawned && let Some(tab) = self.sql_tabs.get_mut(tab_idx) {
            tab.running = Some(token);
        }
    }

    fn apply_editor_result(
        &mut self,
        token: u64,
        sql: String,
        elapsed_ms: u64,
        result: DbResult<QueryResult>,
    ) {
        match &result {
            Ok(data) => self.push_log(LogEntry::ok(sql, elapsed_ms, data.rows.len())),
            Err(e) => self.push_log(LogEntry::err(sql, elapsed_ms, e.as_str())),
        }
        // La pestaña puede haberse cerrado mientras tanto
        if let Some(tab) = self.sql_tabs.iter_mut().find(|t| t.running == Some(token)) {
            tab.running = None;
            tab.result = Some(result);
        }
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
                    format!("{} {dir}", quote_ident(&s.col_name))
                })
                .collect();
            format!(" ORDER BY {}", parts.join(", "))
        };

        let limit = sh.settings.query_limit;
        if filter.is_empty() {
            format!(
                "SELECT * FROM {}{} LIMIT {limit}",
                qualified(db, table),
                order_by
            )
        } else {
            format!(
                "SELECT * FROM {} WHERE {filter}{} LIMIT {limit}",
                qualified(db, table),
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
        self.table_view.reset();
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

    /// Sentencias de los cambios pendientes: DELETE, UPDATE (uno por fila,
    /// con todas sus celdas editadas) e INSERT, con los valores como
    /// parámetros. Las filas existentes se identifican por su clave primaria.
    fn pending_statements(&self) -> Result<Vec<Statement>, String> {
        let Some(SelectedNode::Table { db, table }) = &self.schema.selected_node else {
            return Err("No hay ninguna tabla abierta.".into());
        };
        let tv = &self.table_view;
        let data = tv.table_data.as_ref().ok_or("No hay datos cargados.")?;
        let target = qualified(db, table);
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
                single_row: true,
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
                sets.push(format!("{} = ?", quote_ident(col)));
                params.push((!edit.is_null).then(|| edit.new_value.clone()));
            }
            let (condition, key) = row_key(row_idx)?;
            params.extend(key);
            stmts.push(Statement {
                sql: format!("UPDATE {target} SET {} WHERE {condition}", sets.join(", ")),
                params,
                single_row: true,
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
                    Some((quote_ident(col), param))
                })
                .unzip();
            let placeholders = vec!["?"; cols.len()].join(", ");
            stmts.push(Statement {
                sql: format!(
                    "INSERT INTO {target} ({}) VALUES ({placeholders})",
                    cols.join(", ")
                ),
                params,
                single_row: false,
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
        // Al terminar de guardar ya se recarga
        if self.table_view.saving {
            return;
        }
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
        self.table_view.selection.clear();
    }

    /// Abre una tabla en la pestaña indicada. Si hay cambios pendientes en la
    /// tabla actual, pide confirmación antes de cambiar.
    fn switch_to_table(&mut self, sh: &mut Shared, db: String, table: String, tab: ActiveTab) {
        // Los cambios que se están guardando son de la tabla abierta
        if self.table_view.saving {
            return;
        }
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
        // 1. Guardar estado de la tabla anterior (sort, filtro, historial) y
        //    vaciar la vista: hasta que lleguen los datos nuevos no se ven los
        //    de la anterior
        self.save_table_state(sh);
        self.table_view.reset();

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
        self.request_structure(&db, &table);
        self.sync_primary_keys();

        // 6. Ejecutar query (server-side sort + filter restaurados)
        let sql = self.build_sql(sh, &db, &table);
        self.run_query_async(sql);
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
        self.table_view.reset();
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
                    ..Default::default()
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
        parts.push(format!("{} = ?", quote_ident(pk)));
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
    fn poll_jobs(&mut self, ctx: &egui::Context, sh: &mut Shared) {
        while let Some(result) = self.jobs.try_recv(ctx) {
            self.apply_job(sh, result);
        }
    }

    fn apply_job(&mut self, sh: &mut Shared, result: JobResult) {
        match result {
            JobResult::TableData {
                generation,
                sql,
                elapsed_ms,
                result,
            } => self.apply_table_data(generation, sql, elapsed_ms, result),
            JobResult::Schema(result) => self.apply_schema_job(result),
            JobResult::Write(done) => self.apply_write(sh, done),
            JobResult::Editor {
                token,
                sql,
                elapsed_ms,
                result,
            } => self.apply_editor_result(token, sql, elapsed_ms, result),
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
                self.table_view.table_data = Some(data);
                self.table_view.table_error = None;
            }
            Err(e) => {
                self.push_log(LogEntry::err(sql, elapsed_ms, &e));
                self.table_view.table_data = None;
                self.table_view.table_error = Some(e);
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
}

// ── Frame de la sesión ────────────────────────────────────────────────────────

impl Session {
    /// Sondea los hilos de fondo (conexión, query, exportación). La app lo
    /// llama en todas las sesiones, no solo en la visible.
    pub fn poll(&mut self, ctx: &egui::Context, sh: &mut Shared) {
        self.poll_connection(ctx, sh);
        self.poll_jobs(ctx, sh);
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
        if self.is_connected() {
            self.request_tab_data();
        }
        // Con un diálogo abierto, el ratón no llega a lo de detrás (lo
        // bloquea el propio diálogo), pero el teclado sí: se ignora.
        let dialog_open = !matches!(self.modal, ModalState::None);
        let keys = if dialog_open {
            FrameKeys::default()
        } else {
            keys
        };
        let mut signals = self.draw(ui, sh, keys, show_sidebar);
        if dialog_open {
            signals = signals.data_requests_only();
        }
        // ── Modal (sobre todo) ────────────────────────────────────────────
        let modal_action = modals_ui::show(ctx, &mut self.modal, self.writes_in_flight > 0);

        let (connect, selected, save, delete) = (
            signals.connect,
            signals.profile_selected,
            signals.save_profile,
            signals.delete_profile,
        );
        self.apply_signals(ctx, sh, signals, keys.f5, keys.ctrl_t);
        let close = matches!(modal_action, ModalAction::CloseSession);
        self.handle_modal_action(modal_action, sh);
        let profile_removed = self.handle_profile_form(ctx, sh, selected, save, delete);
        if connect {
            self.start_connect(sh);
        }
        SessionOutcome {
            close,
            profile_removed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::schema::SchemaResult;
    use super::writes::AfterCommit;
    use super::*;
    use crate::db::{ColumnInfo, DatabaseInfo, DbResult, TableInfo};
    use crate::ui::modals::NewColumnDef;

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
                ..Default::default()
            })
        }
        fn execute_in_session(
            &self,
            _: Option<&str>,
            sql: &str,
            _: usize,
        ) -> DbResult<QueryResult> {
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
            ..Default::default()
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
        s.apply_job(
            &mut shared(),
            JobResult::TableData {
                generation: s.table_view.data_generation,
                sql: String::new(),
                elapsed_ms: 0,
                result: Ok(data(&["7", "8"])),
            },
        );
        assert_eq!(s.pending_count(), 0);
    }

    #[test]
    fn los_datos_de_una_consulta_sustituida_se_descartan() {
        let mut s = session_with_table();
        s.request_reload(&mut shared(), Reload::Refresh);
        let stale = s.table_view.data_generation;
        s.request_reload(&mut shared(), Reload::Refresh);
        s.apply_job(
            &mut shared(),
            JobResult::TableData {
                generation: stale,
                sql: String::new(),
                elapsed_ms: 0,
                result: Ok(data(&["viejo"])),
            },
        );
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
        let mut sh = shared();
        s.handle_modal_action(
            ModalAction::CreateTable {
                db: "db".into(),
                name: "nueva".into(),
                columns: vec![col],
            },
            &mut sh,
        );
        finish_jobs(&mut s, &mut sh);
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
        let mut sh = shared();
        s.handle_modal_action(action, &mut sh);
        finish_jobs(&mut s, &mut sh);
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
        let mut sh = shared();
        s.handle_modal_action(action, &mut sh);
        finish_jobs(&mut s, &mut sh);
        assert_eq!(open_table(&s), Some(("db", "t")));
        assert_eq!(s.pending_count(), 1);
    }

    #[test]
    fn borrar_la_bbdd_desde_una_pestana_sql_no_cambia_de_pestana() {
        let mut s = session_with_table();
        s.active_tab = ActiveTab::Sql(0);
        let mut sh = shared();
        s.handle_modal_action(ModalAction::DropDatabase("db".into()), &mut sh);
        finish_jobs(&mut s, &mut sh);
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
        let mut sh = shared();
        s.handle_modal_action(action, &mut sh);
        finish_jobs(&mut s, &mut sh);
        assert_eq!(open_table(&s), Some(("db", "t")));
        assert_eq!(s.pending_count(), 0);
        // Recargada: FakeBackend devuelve una tabla vacía
        assert_eq!(
            s.table_view.table_data.as_ref().map(|d| d.rows.len()),
            Some(0)
        );
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
            ..Default::default()
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
                    single_row: true,
                },
                Statement {
                    sql: "UPDATE `db`.`t` SET `id` = ?, `v` = ? WHERE `id` = ?".into(),
                    params: p(&[Some("10"), None, Some("1")]),
                    single_row: true,
                },
                Statement {
                    sql: "INSERT INTO `db`.`t` (`id`, `v`) VALUES (?, ?)".into(),
                    params: p(&[Some("3"), Some("x'y")]),
                    single_row: false,
                },
            ]
        );
    }

    #[test]
    fn sin_clave_primaria_no_se_editan_filas() {
        let mut s = session_with_table();
        s.table_view.primary_keys.clear();
        s.table_view.pending_deletes.insert(0);
        s.commit_pending_changes(&mut shared(), AfterCommit::ReloadData);
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

        let mut sh = shared();
        s.commit_pending_changes(&mut sh, AfterCommit::ReloadData);
        finish_jobs(&mut s, &mut sh);
        let saved = s.pending_count() == 0;
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
        assert_eq!(s.table_view.pending_edits.len(), 1);
        s.record_edit(cell, 0, Some("1".into()));
        assert!(s.table_view.pending_edits.is_empty());
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

        let mut sh = shared();
        s.handle_modal_action(
            ModalAction::TruncateAllTables("rustql_test_truncate".into()),
            &mut sh,
        );
        finish_jobs(&mut s, &mut sh);
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
            ..Default::default()
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
            ..Default::default()
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

        let mut sh = shared();
        s.commit_pending_changes(&mut sh, AfterCommit::ReloadData);
        finish_jobs(&mut s, &mut sh);
        let saved = s.pending_count() == 0;
        let ConnState::Connected(b) = &s.state else {
            unreachable!()
        };
        let rows = b.execute_query(select).map(|r| r.rows);
        b.execute_write("DROP DATABASE rustql_test_insert").unwrap();

        assert!(saved, "{:?}", s.modal);
        let some = |v: &str| Some(v.to_string());
        assert_eq!(rows.unwrap(), [[some("1"), some("1"), None, some("x")]]);
    }

    /// Espera a que terminen los trabajos en segundo plano y los aplica.
    fn finish_jobs(s: &mut Session, sh: &mut Shared) {
        let start = Instant::now();
        while s.jobs.pending() > 0 {
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "trabajos sin terminar"
            );
            s.poll_jobs(&egui::Context::default(), sh);
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn tables<'a>(s: &'a Session, db: &str) -> Option<&'a Vec<String>> {
        s.schema.tables_cache.get(db).and_then(Loadable::ready)
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
        assert_eq!(tables(&s, "db"), Some(&vec!["nueva".to_string()]));
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
        assert_eq!(tables(&s, "a"), Some(&vec!["ya".to_string()]));
        assert_eq!(tables(&s, "b"), Some(&vec!["x".to_string()]));
        assert_eq!(tables(&s, "c"), Some(&Vec::new()), "BBDD sin tablas");
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

    #[test]
    fn el_formulario_espera_al_resultado_de_la_escritura() {
        let mut s = session_with_table();
        s.modal = ModalState::CreateDatabase {
            name: "nueva".into(),
            charset: "utf8mb4".into(),
            error: None,
        };
        let mut sh = shared();
        let action = ModalAction::CreateDatabase {
            name: "nueva".into(),
            charset: "utf8mb4".into(),
        };
        s.handle_modal_action(action, &mut sh);
        assert!(matches!(s.modal, ModalState::CreateDatabase { .. }));
        assert!(s.is_busy());
        finish_jobs(&mut s, &mut sh);
        assert!(matches!(s.modal, ModalState::None));
        assert!(!s.is_busy());
    }

    #[test]
    fn mientras_se_guarda_no_se_recarga_ni_se_cambia_de_tabla() {
        let mut s = session_with_table();
        s.table_view.pending_deletes.insert(0);
        let mut sh = shared();
        s.commit_pending_changes(&mut sh, AfterCommit::ReloadData);
        assert!(s.table_view.saving);
        s.request_reload(&mut sh, Reload::Refresh);
        assert!(
            matches!(s.modal, ModalState::None),
            "sin diálogo de recarga"
        );
        s.switch_to_table(&mut sh, "db".into(), "otra".into(), ActiveTab::Datos);
        assert_eq!(open_table(&s), Some(("db", "t")));
        finish_jobs(&mut s, &mut sh);
        assert!(!s.table_view.saving);
        assert_eq!(s.pending_count(), 0);
    }

    #[test]
    fn el_resultado_del_editor_va_a_su_pestana() {
        let mut s = session_with_table();
        s.sql_tabs = vec![SqlTab::default(), SqlTab::default()];
        s.run_editor(1, "SELECT 1".into(), String::new(), 10);
        assert!(s.sql_tabs[1].running.is_some());
        // Se cierra la primera: la que ejecuta pasa a ser la 0
        s.sql_tabs.remove(0);
        finish_jobs(&mut s, &mut shared());
        assert!(s.sql_tabs[0].running.is_none());
        assert!(matches!(s.sql_tabs[0].result, Some(Ok(_))));
        assert_eq!(s.query_log.len(), 1);
    }

    #[test]
    fn desconectar_no_deja_el_editor_esperando() {
        let mut s = session_with_table();
        s.run_editor(0, "SELECT 1".into(), String::new(), 10);
        s.disconnect();
        assert!(s.sql_tabs[0].running.is_none());
    }

    #[test]
    fn al_cambiar_de_tabla_no_se_ven_los_datos_de_la_anterior() {
        let mut s = session_with_table();
        let mut sh = shared();
        s.request_reload(&mut sh, Reload::Refresh);
        let old_query = s.table_view.data_generation;
        s.switch_to_table(&mut sh, "db".into(), "otra".into(), ActiveTab::Datos);
        assert!(s.table_view.table_data.is_none());
        s.apply_job(
            &mut sh,
            JobResult::TableData {
                generation: old_query,
                sql: String::new(),
                elapsed_ms: 0,
                result: Ok(data(&["de la anterior"])),
            },
        );
        assert!(s.table_view.table_data.is_none());
        assert!(s.table_view.query_loading);
    }

    /// Un frame en el que solo se pulsa F5.
    fn press_f5(s: &mut Session, sh: &mut Shared) {
        let ctx = egui::Context::default();
        s.apply_signals(&ctx, sh, Default::default(), true, false);
    }

    #[test]
    fn f5_en_una_pestana_sql_ejecuta_su_consulta() {
        let mut s = session_with_table();
        s.sql_tabs[0].sql_text = "SELECT 1".into();
        s.active_tab = ActiveTab::Sql(0);
        let mut sh = shared();
        press_f5(&mut s, &mut sh);
        assert!(s.sql_tabs[0].running.is_some());
        assert!(!s.table_view.query_loading, "no recarga la tabla abierta");
        finish_jobs(&mut s, &mut sh);
        assert!(matches!(s.sql_tabs[0].result, Some(Ok(_))));
    }

    #[test]
    fn f5_en_estructura_relee_las_columnas_y_no_los_datos() {
        let mut s = session_with_table();
        s.active_tab = ActiveTab::Tabla;
        press_f5(&mut s, &mut shared());
        assert!(s.schema.open_table_columns().is_none());
        assert!(!s.table_view.query_loading);
        s.request_tab_data();
        let key = ("db".to_string(), "t".to_string());
        assert!(matches!(
            s.schema.columns_info.get(&key),
            Some(Loadable::Loading(_))
        ));
    }

    #[test]
    fn f5_en_datos_recarga_la_tabla() {
        let mut s = session_with_table();
        s.active_tab = ActiveTab::Datos;
        press_f5(&mut s, &mut shared());
        assert!(s.table_view.query_loading);
    }
}

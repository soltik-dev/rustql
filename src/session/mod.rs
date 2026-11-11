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
    CancelFlag, ConnectEvent, ConnectionParams, DbBackend, DbResult, Engine, ExportDestination,
    ExportEvent, ExportOptions, ParamList, QueryResult, Statement,
};
use crate::loadable::Loadable;
use crate::log::LogEntry;
use crate::profiles::{self, ConnColor, ConnectionProfile};
use crate::settings::{self, AppSettings};
use crate::table_state::{self, TableStates};
use crate::ui::modals as modals_ui;
use crate::ui::modals::{
    self, ExportPhase, ModalAction, ModalState, Reload, TableExportEntry, TableExportState,
};
use crate::ui::results_grid::{
    InsertValue, PendingEdit, PendingInsert, RowRef, SortDir, TableViewState,
};
use crate::ui::sql_editor::SqlTab;
use crate::ui::tab_bar::{ActiveTab, TabRename};
use crate::ui::toasts::Toasts;

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

impl SelectedNode {
    /// BBDD del nodo: ella misma o la de la tabla.
    pub fn db(&self) -> Option<&str> {
        match self {
            Self::Database(db) | Self::Table { db, .. } => Some(db),
            Self::Server => None,
        }
    }

    /// Si es una tabla, (BBDD, tabla).
    pub fn table(&self) -> Option<(&str, &str)> {
        match self {
            Self::Table { db, table } => Some((db, table)),
            _ => None,
        }
    }
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
    /// Avisos no modales: los lanza cualquier sesión y los dibuja la app.
    pub toasts: Toasts,
}

/// Teclas globales del frame: solo las recibe la sesión activa.
#[derive(Clone, Copy, Default)]
pub struct FrameKeys {
    pub f5: bool,
    pub f9: bool,
    pub ctrl_enter: bool,
    pub ctrl_t: bool,
    /// Cerrar la pestaña SQL activa.
    pub ctrl_w: bool,
    /// Ctrl+Tab y Ctrl+Mayús+Tab: pestaña siguiente (true) o anterior.
    pub cycle_tab: Option<bool>,
}

impl FrameKeys {
    pub fn read(ctx: &egui::Context) -> Self {
        use egui::{Key, Modifiers};
        ctx.input_mut(|i| {
            // Estas se consumen: en un campo de texto, Ctrl+W borraría la
            // palabra anterior y Ctrl+Tab escribiría un tabulador
            let ctrl_w = i.consume_key(Modifiers::COMMAND, Key::W);
            // Primero la combinación con Mayús: la otra también la acepta
            let cycle_tab = if i.consume_key(Modifiers::CTRL | Modifiers::SHIFT, Key::Tab) {
                Some(false)
            } else if i.consume_key(Modifiers::CTRL, Key::Tab) {
                Some(true)
            } else {
                None
            };
            Self {
                f5: i.key_pressed(Key::F5),
                f9: i.key_pressed(Key::F9),
                ctrl_enter: i.modifiers.command && i.key_pressed(Key::Enter),
                ctrl_t: i.modifiers.command && i.key_pressed(Key::T),
                ctrl_w,
                cycle_tab,
            }
        })
    }
}

/// Lista de BBDD del servidor del formulario de conexión, para elegir una.
#[derive(Default)]
pub struct DbPicker {
    pub list: Loadable<Vec<String>>,
    /// Hilo que la está leyendo.
    receiver: Option<Receiver<DbResult<Vec<String>>>>,
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
    /// Pestaña SQL que se está renombrando en la barra, si hay alguna.
    pub sql_tab_rename: Option<TabRename>,

    // Formulario de conexión / perfiles
    pub selected_profile_idx: Option<usize>,
    pub profile_name: String,
    pub save_password: bool,
    pub db_picker: DbPicker,
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
            sql_tab_rename: None,
            selected_profile_idx: None,
            profile_name: String::new(),
            save_password: false,
            db_picker: DbPicker::default(),
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

    /// Motor de la conexión (o del formulario): en su dialecto se escribe
    /// el SQL que compone la sesión.
    pub fn engine(&self) -> Engine {
        self.params.engine
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
        (host, self.selected_db().map(str::to_string))
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
    /// resultado ya no corresponde a nada que se esté mostrando. Las del
    /// editor se cancelan también en el servidor, que si no las terminaría.
    fn disconnect(&mut self) {
        for idx in 0..self.sql_tabs.len() {
            self.cancel_editor(idx);
        }
        self.state = ConnState::Disconnected;
        self.jobs.new_epoch();
        self.table_view.query_loading = false;
        for tab in &mut self.sql_tabs {
            tab.running = None;
            tab.cancel = None;
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
        let cancel = CancelFlag::default();
        let job_cancel = Arc::clone(&cancel);
        let spawned = self.spawn_job(move |b| {
            let start = Instant::now();
            let db = (!db_context.is_empty()).then_some(db_context.as_str());
            let result = b.execute_in_session(db, &sql, max_rows, &job_cancel);
            JobResult::Editor {
                token,
                sql,
                elapsed_ms: start.elapsed().as_millis() as u64,
                result,
            }
        });
        if spawned && let Some(tab) = self.sql_tabs.get_mut(tab_idx) {
            tab.running = Some(token);
            tab.cancel = Some(cancel);
        }
    }

    /// Cancela la consulta que está ejecutando la pestaña `tab_idx`, si hay
    /// alguna. Su resultado llegará como error `QUERY_CANCELLED`.
    fn cancel_editor(&mut self, tab_idx: usize) {
        let Some(cancel) = self.sql_tabs.get(tab_idx).and_then(|t| t.cancel.clone()) else {
            return;
        };
        // Ya pedido: una vez basta
        if cancel.swap(true, Ordering::SeqCst) {
            return;
        }
        self.spawn_job(move |b| JobResult::EditorCancel(b.cancel_in_session(&cancel)));
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
            tab.cancel = None;
            tab.result = Some(result);
            tab.result_run = token;
            tab.elapsed_ms = Some(elapsed_ms);
            // El cursor y la selección eran del resultado anterior
            tab.selection = Default::default();
        }
    }

    /// Carga los datos de una tabla de `db` en segundo plano. Sustituye a la
    /// consulta en curso, si la hay.
    fn run_query_async(&mut self, db: &str, sql: String) {
        if !self.is_connected() {
            return;
        }
        self.table_view.data_generation += 1;
        let generation = self.table_view.data_generation;
        self.table_view.query_loading = true;
        let db = db.to_string();
        self.spawn_job(move |b| {
            let start = Instant::now();
            let result = b.execute_query(Some(&db), &sql);
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

    fn build_sql(&self, db: &str, table: &str) -> String {
        let engine = self.engine();
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
                    format!("{} {dir}", engine.quote_ident(&s.col_name))
                })
                .collect();
            format!(" ORDER BY {}", parts.join(", "))
        };

        // Una fila de más: si llega, es que hay otra página (apply_table_data
        // la quita antes de mostrar los datos)
        let tv = &self.table_view;
        let mut limit = format!(" LIMIT {}", tv.page_size + 1);
        if tv.page > 0 {
            limit.push_str(&format!(" OFFSET {}", tv.page * tv.page_size));
        }
        let from = engine.qualified(db, table);
        if filter.is_empty() {
            format!("SELECT * FROM {from}{order_by}{limit}")
        } else {
            format!("SELECT * FROM {from} WHERE {filter}{order_by}{limit}")
        }
    }

    /// Pide al servidor la página actual de la tabla, con su filtro y su
    /// ordenación. El tamaño de página es el límite de los ajustes; si ha
    /// cambiado, las páginas ya no son las mismas y se vuelve a la primera.
    fn load_table(&mut self, sh: &Shared, db: &str, table: &str) {
        let page_size = sh.settings.query_limit.max(1);
        if self.table_view.page_size != page_size {
            self.table_view.page_size = page_size;
            self.table_view.page = 0;
        }
        let sql = self.build_sql(db, table);
        self.run_query_async(db, sql);
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
        let engine = self.engine();
        let target = engine.qualified(db, table);
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
        // Condición que identifica la fila, con su clave como parámetros
        let row_key = |row_idx: usize, params: &mut ParamList| {
            pk_condition(data, &tv.primary_keys, row_idx, engine, params)
                .ok_or_else(|| format!("No se encuentra la clave primaria de la fila {row_idx}."))
        };

        let mut stmts = Vec::new();
        for &row_idx in &tv.pending_deletes {
            let mut params = ParamList::new(engine);
            let condition = row_key(row_idx, &mut params)?;
            stmts.push(Statement {
                sql: format!("DELETE FROM {target} WHERE {condition}"),
                params: params.into_values(),
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
            let mut params = ParamList::new(engine);
            for edit in edits {
                let col = data
                    .columns
                    .get(edit.col_idx)
                    .ok_or_else(|| format!("No existe la columna {}.", edit.col_idx))?;
                let value = params.push((!edit.is_null).then(|| edit.new_value.clone()));
                sets.push(format!("{} = {value}", engine.quote_ident(col)));
            }
            let condition = row_key(row_idx, &mut params)?;
            stmts.push(Statement {
                sql: format!("UPDATE {target} SET {} WHERE {condition}", sets.join(", ")),
                params: params.into_values(),
                single_row: true,
            });
        }

        // Las columnas en DEFAULT se omiten para que el servidor ponga su
        // valor por defecto; si son todas, la fila entera va por defecto.
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
                    Some((engine.quote_ident(col), param))
                })
                .unzip();
            stmts.push(Statement {
                sql: engine.insert(&target, &cols),
                params,
                single_row: false,
            });
        }
        Ok(stmts)
    }

    /// Fila nueva para el grid: vacía (todo DEFAULT, que el servidor
    /// completa) o copia de `source_row`. Las columnas a las que el servidor
    /// da valor (AUTO_INCREMENT, serial…), siempre DEFAULT.
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
                    .is_some_and(|c| c.is_auto)
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
            // Con otra ordenación u otro filtro las páginas son otras: se
            // vuelve a la primera
            Reload::Sort(specs) => {
                self.table_view.sort_specs = specs;
                self.table_view.page = 0;
                self.clear_row_selection();
                self.save_table_state(sh);
            }
            Reload::Page(page) => {
                self.table_view.page = page;
                self.clear_row_selection();
            }
            Reload::Filter(filter) => {
                if !filter.is_empty() {
                    let history = &mut self.table_view.filter_history;
                    history.retain(|f| f != &filter);
                    history.insert(0, filter.clone());
                    history.truncate(20);
                }
                self.table_view.filter_text = filter;
                self.table_view.page = 0;
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
        self.load_table(sh, &db, &table);
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
        self.load_table(sh, &db, &table);
    }

    /// Guarda el texto de las pestañas SQL de esta conexión. Sin conexión no
    /// hace nada: las pestañas se guardan bajo la clave de la conexión, y en
    /// el formulario esa clave es la de unos parámetros a medio escribir.
    pub fn save_tabs_state(&self, sh: &mut Shared) {
        if !self.is_connected() {
            return;
        }
        let tabs = self.sql_tabs.iter().map(SqlTab::saved).collect();
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
            saved.iter().map(SqlTab::restore).collect()
        };
        if migrated {
            self.save_tabs_state(sh);
        }
        self.query_log.clear();
    }
}

/// Condición `WHERE` que identifica la fila `row_idx` por su clave primaria;
/// sus valores se añaden a `params`. `None` si falta la fila o alguna columna
/// de la clave: una condición parcial podría afectar a varias filas.
fn pk_condition(
    data: &QueryResult,
    primary_keys: &[String],
    row_idx: usize,
    engine: Engine,
    params: &mut ParamList,
) -> Option<String> {
    let row = data.rows.get(row_idx)?;
    // Antes de añadir ningún parámetro: con la clave a medias no se añade
    let mut key = Vec::new();
    for pk in primary_keys {
        let col_idx = data.columns.iter().position(|c| c == pk)?;
        key.push((pk, row.get(col_idx)?.clone()));
    }
    let parts: Vec<String> = key
        .into_iter()
        .map(|(pk, value)| format!("{} = {}", engine.quote_ident(pk), params.push(value)))
        .collect();
    Some(parts.join(" AND "))
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
            JobResult::EditorCancel(Ok(())) => {}
            // La consulta sigue en marcha: que conste por qué
            JobResult::EditorCancel(Err(e)) => {
                sh.toasts
                    .error(format!("No se pudo cancelar la consulta: {e}"));
                self.push_log(LogEntry::err("Cancelar la consulta", 0, e));
            }
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
            Ok(mut data) => {
                // La fila de más solo dice que hay otra página
                let page_size = self.table_view.page_size;
                self.table_view.has_more = page_size > 0 && data.rows.len() > page_size;
                if self.table_view.has_more {
                    data.rows.truncate(page_size);
                }
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
        if !self.engine().supports_export() {
            return;
        }
        // Los servidores de destino son los del mismo motor
        let profiles = sh
            .profiles
            .iter()
            .filter(|p| p.engine == self.engine())
            .cloned()
            .collect();
        self.request_tables(&db);
        self.modal = ModalState::ExportData {
            source_db: db.clone(),
            source_table: table,
            tables: Vec::new(),
            databases: self.schema.databases.ready().cloned().unwrap_or_default(),
            profiles,
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
        self.poll_db_picker();
    }

    /// Recoge la lista de BBDD pedida desde el formulario de conexión.
    fn poll_db_picker(&mut self) {
        let Some(rx) = &self.db_picker.receiver else {
            return;
        };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                Err("No se pudo leer la lista de bases de datos.".into())
            }
        };
        self.db_picker.receiver = None;
        self.db_picker.list = match result {
            Ok(list) => Loadable::Ready(list),
            Err(e) => Loadable::Failed(e),
        };
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
                    // En el formulario y, por si ya no se está en él, en un aviso
                    let message = format!("No se guardó la contraseña en el llavero: {e}");
                    sh.toasts.error(message.clone());
                    self.error_message = Some(message);
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
        let modal_action = modals_ui::show(
            ctx,
            &mut self.modal,
            self.params.engine,
            self.writes_in_flight > 0,
        );

        let (connect, selected, save, delete) = (
            signals.connect,
            signals.profile_selected,
            signals.save_profile,
            signals.delete_profile,
        );
        self.apply_signals(ctx, sh, signals, keys);
        let close = matches!(modal_action, ModalAction::CloseSession);
        let delete_confirmed = matches!(modal_action, ModalAction::DeleteProfile);
        self.handle_modal_action(modal_action, sh);
        // Eliminar un favorito pide confirmación antes
        if delete && let Some(profile) = self.selected_profile_idx.and_then(|i| sh.profiles.get(i))
        {
            self.modal = ModalState::ConfirmDeleteProfile {
                name: profile.name.clone(),
            };
        }
        let profile_removed = self.handle_profile_form(ctx, sh, selected, save, delete_confirmed);
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
mod tests;

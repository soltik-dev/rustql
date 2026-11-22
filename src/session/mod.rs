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
    ExportEvent, ExportOptions, ParamList, QueryResult, Security, Statement, TlsMode,
};
use crate::i18n::t;
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

// ── Connection states ────────────────────────────────────────────────────────

pub enum ConnState {
    Disconnected,
    Connecting {
        receiver: Receiver<ConnectEvent>,
        host: String,
        status: String,
        start: Instant,
    },
    /// Shared with the worker threads.
    Connected(Arc<dyn DbBackend>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum SelectedNode {
    Server,
    Database(String),
    Table { db: String, table: String },
}

impl SelectedNode {
    /// Database of the node: itself or the table's.
    pub fn db(&self) -> Option<&str> {
        match self {
            Self::Database(db) | Self::Table { db, .. } => Some(db),
            Self::Server => None,
        }
    }

    /// If it is a table, (database, table).
    pub fn table(&self) -> Option<(&str, &str)> {
        match self {
            Self::Table { db, table } => Some((db, table)),
            _ => None,
        }
    }
}

// ── State sub-structs ─────────────────────────────────────────────────────────

// ── State shared between sessions ────────────────────────────────────────────

/// What every connection tab shares: settings (favourites, saved SQL
/// tabs…), profiles and per-table persisted state.
pub struct Shared {
    pub settings: AppSettings,
    pub profiles: Vec<ConnectionProfile>,
    /// Per-table persisted state (sort, filter, history)
    pub table_states: TableStates,
    /// Non-modal notifications: any session raises them and the app draws them.
    pub toasts: Toasts,
}

/// Global keys of the frame: only the active session receives them.
#[derive(Clone, Copy, Default)]
pub struct FrameKeys {
    pub f5: bool,
    pub f9: bool,
    pub ctrl_enter: bool,
    pub ctrl_t: bool,
    /// Close the active SQL tab.
    pub ctrl_w: bool,
    /// Ctrl+Tab and Ctrl+Shift+Tab: next (true) or previous tab.
    pub cycle_tab: Option<bool>,
}

impl FrameKeys {
    pub fn read(ctx: &egui::Context) -> Self {
        use egui::{Key, Modifiers};
        ctx.input_mut(|i| {
            // These are consumed: in a text field, Ctrl+W would delete the
            // previous word and Ctrl+Tab would type a tab
            let ctrl_w = i.consume_key(Modifiers::COMMAND, Key::W);
            // The Shift combination first: the other one accepts it too
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

/// What the status bar shows about a session's connection.
pub struct ConnectionStatus {
    /// "host:port", or that it is connecting; `None` without a connection.
    pub host: Option<String>,
    pub database: Option<String>,
    /// How the connection travels and which security level was requested.
    pub security: Option<(Security, TlsMode)>,
}

/// List of databases of the connection form's server, to pick one.
#[derive(Default)]
pub struct DbPicker {
    pub list: Loadable<Vec<String>>,
    /// Thread reading it.
    receiver: Option<Receiver<DbResult<Vec<String>>>>,
}

/// Session requests that only the app can resolve.
#[derive(Default)]
pub struct SessionOutcome {
    /// The user confirmed closing this tab despite the pending changes.
    pub close: bool,
    /// The profile with this index was deleted: the other sessions must
    /// adjust their selected_profile_idx.
    pub profile_removed: Option<usize>,
}

// ── Session ──────────────────────────────────────────────────────────────────

/// A connection tab: the form or a connected server with its own tree,
/// open table, SQL tabs, log and modal.
pub struct Session {
    /// Unique identifier: keeps egui widget state apart between sessions.
    pub id: u64,
    pub state: ConnState,
    pub params: ConnectionParams,
    pub port_str: String,
    pub error_message: Option<String>,
    /// Name of the tab and of the tree root (profile or host), set when connecting.
    pub title: String,
    pub color: Option<ConnColor>,

    pub schema: SchemaCache,
    pub table_view: TableViewState,
    pub modal: ModalState,
    pub query_log: std::collections::VecDeque<LogEntry>,
    pub show_only_fav: bool,

    // Tabs
    pub active_tab: ActiveTab,
    pub sql_tabs: Vec<SqlTab>,
    /// SQL tab being renamed in the bar, if any.
    pub sql_tab_rename: Option<TabRename>,

    // Connection form / profiles
    pub selected_profile_idx: Option<usize>,
    pub profile_name: String,
    pub save_password: bool,
    pub db_picker: DbPicker,
    /// Database jobs in the background.
    jobs: Jobs,
    /// Writes started and still without a result.
    writes_in_flight: usize,
    /// SQL editor runs started: they identify their tab.
    editor_runs: u64,
    /// Passwords a thread finished saving in the keyring: (profile id, result).
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

    /// Text of the connection tab.
    pub fn tab_title(&self) -> std::borrow::Cow<'_, str> {
        match self.state {
            ConnState::Disconnected => t!("conn-tabs-new").into(),
            _ => self.title.as_str().into(),
        }
    }

    /// Connecting or loading the table.
    pub fn is_busy(&self) -> bool {
        matches!(self.state, ConnState::Connecting { .. })
            || self.table_view.query_loading
            || self.writes_in_flight > 0
    }

    /// Engine of the connection (or of the form): the SQL the session builds
    /// is written in its dialect.
    pub fn engine(&self) -> Engine {
        self.params.engine
    }

    pub fn is_connected(&self) -> bool {
        matches!(self.state, ConnState::Connected(_))
    }

    /// Server, active database and connection security, for the status
    /// bar.
    pub fn status(&self) -> ConnectionStatus {
        let (host, security) = match &self.state {
            ConnState::Connected(backend) => (
                Some(format!("{}:{}", self.params.host, self.params.port)),
                Some((backend.security(), self.params.tls)),
            ),
            ConnState::Connecting { host, .. } => {
                (Some(t!("status-connecting", host = host)), None)
            }
            ConnState::Disconnected => (None, None),
        };
        ConnectionStatus {
            host,
            database: self.selected_db().map(str::to_string),
            security,
        }
    }

    /// Before closing the tab: cancels the running export, if any.
    pub fn shutdown(&self) {
        if let ModalState::ExportData {
            phase: ExportPhase::Running(running),
            ..
        } = &self.modal
        {
            running.cancel_flag.store(true, Ordering::Relaxed);
        }
    }

    /// Goes back to the form. The running query, if any, is discarded: its
    /// result no longer matches anything being shown. Editor ones are also
    /// cancelled on the server, which would otherwise finish them.
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

// ── Helper methods ────────────────────────────────────────────────────────────

impl Session {
    /// Runs a job in the background; false if there is no connection.
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

    /// Runs editor SQL in the background; the result will go to tab
    /// `tab_idx` (even if its position changes meanwhile).
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

    /// Cancels the query tab `tab_idx` is running, if any. Its result will
    /// arrive as a `QUERY_CANCELLED` error.
    fn cancel_editor(&mut self, tab_idx: usize) {
        let Some(cancel) = self.sql_tabs.get(tab_idx).and_then(|t| t.cancel.clone()) else {
            return;
        };
        // Already requested: once is enough
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
        // The tab may have been closed meanwhile
        if let Some(tab) = self.sql_tabs.iter_mut().find(|t| t.running == Some(token)) {
            tab.running = None;
            tab.cancel = None;
            tab.result = Some(result);
            tab.result_run = token;
            tab.elapsed_ms = Some(elapsed_ms);
            // The cursor and the selection belonged to the previous result
            tab.selection = Default::default();
        }
    }

    /// Loads the data of a table of `db` in the background. Replaces the
    /// running query, if any.
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

        // One extra row: if it arrives, there is another page (apply_table_data
        // removes it before showing the data)
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

    /// Asks the server for the current page of the table, with its filter and
    /// sorting. The page size is the settings limit; if it has changed, the
    /// pages are no longer the same and it goes back to the first one.
    fn load_table(&mut self, sh: &Shared, db: &str, table: &str) {
        let page_size = sh.settings.query_limit.max(1);
        if self.table_view.page_size != page_size {
            self.table_view.page_size = page_size;
            self.table_view.page = 0;
        }
        let sql = self.build_sql(db, table);
        self.run_query_async(db, sql);
    }

    /// After writing to `db` (create, clone, empty…): discards its schema
    /// caches so they are read again. Does not touch the open table.
    fn invalidate_schema(&mut self, db: &str) {
        self.schema.databases_info = Loadable::NotLoaded;
        // Reload the sidebar's database list
        self.schema.databases = Loadable::NotLoaded;
        self.schema.tables_cache.remove(db);
        self.schema.tables_info.remove(db);
    }

    /// After dropping `db` or, with `table`, only that table: besides
    /// invalidating the caches, closes what was dropped if it was open.
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
        // SQL tabs do not depend on the selection: they stay where they are.
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

    /// After emptying tables of `db`: if the open one is among them, its
    /// pending changes no longer have rows to apply to; it is reloaded empty.
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

    /// Statements for the pending changes: DELETE, UPDATE (one per row, with
    /// all its edited cells) and INSERT, with the values as parameters.
    /// Existing rows are identified by their primary key.
    fn pending_statements(&self) -> Result<Vec<Statement>, String> {
        let Some(SelectedNode::Table { db, table }) = &self.schema.selected_node else {
            return Err(t!("save-no-open-table"));
        };
        let tv = &self.table_view;
        let data = tv.table_data.as_ref().ok_or_else(|| t!("save-no-data"))?;
        let engine = self.engine();
        let target = engine.qualified(db, table);
        let edits_rows = !tv.pending_deletes.is_empty() || !tv.pending_edits.is_empty();
        if edits_rows && self.schema.open_table_columns().is_none() {
            return Err(t!("save-structure-loading"));
        }
        if edits_rows && tv.primary_keys.is_empty() {
            return Err(t!("save-no-primary-key"));
        }
        // Condition identifying the row, with its key as parameters
        let row_key = |row_idx: usize, params: &mut ParamList| {
            pk_condition(data, &tv.primary_keys, row_idx, engine, params)
                .ok_or_else(|| t!("save-row-key-missing", row = row_idx))
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

        // One UPDATE per row: if the primary key is edited, the WHERE (which
        // sees the original row) still finds the row for the rest of the cells.
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
                    .ok_or_else(|| t!("save-no-column", index = edit.col_idx))?;
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

        // Columns set to DEFAULT are left out so the server sets its default
        // value; if they all are, the whole row goes as default.
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

    /// New row for the grid: empty (all DEFAULT, which the server fills in)
    /// or a copy of `source_row`. Columns the server gives a value to
    /// (AUTO_INCREMENT, serial…) are always DEFAULT.
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

    /// Records the new value of a cell (`None` = NULL). In an existing row,
    /// going back to the original value removes the pending change.
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

    /// Persists the sort+filter+history of the currently selected table.
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

    /// Total pending changes (edits + deletes + inserts) in the current table.
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

    /// Reloads the open table. If there are pending changes, first asks what
    /// to do with them: they go by row position and with the new data they
    /// would point to other rows.
    fn request_reload(&mut self, sh: &mut Shared, reload: Reload) {
        let Some(SelectedNode::Table { table, .. }) = &self.schema.selected_node else {
            return;
        };
        // Once saving finishes it already reloads
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
        // The cell being edited also goes by position.
        self.table_view.editing_cell = None;
        match reload {
            // With another sort order or filter the pages are different: it
            // goes back to the first
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
                // Read the structure again too: the grid uses it for the keys,
                // the NULLs and the AUTO_INCREMENT of new rows.
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

    /// Opens a table in the given tab. If the current table has pending
    /// changes, asks for confirmation before switching.
    fn switch_to_table(&mut self, sh: &mut Shared, db: String, table: String, tab: ActiveTab) {
        // The changes being saved belong to the open table
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

    /// Changes the selected table: saves the current state, restores that of
    /// the new table, runs the query with the saved filter and reapplies the
    /// saved sort order.
    fn on_table_selected(&mut self, sh: &mut Shared, db: String, table: String) {
        // 1. Save the previous table's state (sort, filter, history) and
        //    empty the view: until the new data arrives, the old data is
        //    not shown
        self.save_table_state(sh);
        self.table_view.reset();

        // 2. Load the new table's state
        let ts = table_state::get(&sh.table_states, &db, &table);
        self.table_view.filter_text = ts.filter_text.clone();
        self.table_view.filter_history = ts.filter_history.clone();

        // 3. Update the selection
        self.schema.selected_node = Some(SelectedNode::Table {
            db: db.clone(),
            table: table.clone(),
        });

        // 4. Restore the sort BEFORE build_sql so the ORDER BY ends up in the query
        self.table_view.sort_specs = ts.sort_specs.clone();

        // 5. Structure (columns and primary keys): from the cache or the server
        self.request_structure(&db, &table);
        self.sync_primary_keys();

        // 6. Run the query (server-side sort + filter restored)
        self.load_table(sh, &db, &table);
    }

    /// Saves the text of this connection's SQL tabs. Without a connection it
    /// does nothing: tabs are saved under the connection's key, and in the
    /// form that key belongs to half-typed parameters.
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
        // Restore this connection's SQL tabs. Those saved before multiple
        // connections existed are inherited by the first one without its own.
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

/// `WHERE` condition identifying row `row_idx` by its primary key; its
/// values are added to `params`. `None` if the row or a key column is
/// missing: a partial condition could affect several rows.
fn pk_condition(
    data: &QueryResult,
    primary_keys: &[String],
    row_idx: usize,
    engine: Engine,
    params: &mut ParamList,
) -> Option<String> {
    let row = data.rows.get(row_idx)?;
    // Before adding any parameter: with half a key, nothing is added
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

// ── Main loop ─────────────────────────────────────────────────────────────────

impl Session {
    /// Polls the connection thread's channel and updates the state if it finished.
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
                        connect_result = Some(Err(t!("connect-thread-died")));
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

    /// Applies the results of the background jobs that have finished.
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
            JobResult::ViewDefinition {
                db,
                result: Ok(sql),
                ..
            } => self.open_sql_tab(sh, Some(db), sql),
            JobResult::ViewDefinition {
                view,
                result: Err(e),
                ..
            } => {
                sh.toasts
                    .error(t!("view-definition-failed", view = view, error = e));
            }
            JobResult::EditorCancel(Ok(())) => {}
            // The query is still running: record why
            JobResult::EditorCancel(Err(e)) => {
                sh.toasts
                    .error(t!("cancel-query-failed", error = e.as_str()));
                self.push_log(LogEntry::err(t!("log-cancel-query"), 0, e));
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
        // A later query has already replaced it
        if generation != self.table_view.data_generation {
            return;
        }
        self.table_view.query_loading = false;
        match result {
            Ok(mut data) => {
                // The extra row only says there is another page
                let page_size = self.table_view.page_size;
                self.table_view.has_more = page_size > 0 && data.rows.len() > page_size;
                if self.table_view.has_more {
                    data.rows.truncate(page_size);
                }
                // Every reload goes through request_reload, which resolves
                // the pending changes first. If one skips it, better to lose
                // them than to apply them to the rows now in those
                // positions.
                if self.pending_count() > 0 {
                    log::warn!("Data reloaded with pending changes: they are discarded");
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

    /// Consumes events from the export thread and updates the modal's state.
    /// Errors are also recorded in the query log so there is a trace after
    /// the modal is closed.
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
                    }
                    // Rows are counted as their batches arrive
                    Ok(ExportEvent::RowsCopied(rows)) => running.rows_done += rows,
                    Ok(ExportEvent::TableError { table, error }) => {
                        log_entries.push(LogEntry::err(
                            t!("export-title-table", table = &table),
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
                            t!("log-export-finished", count = total_tables),
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
                        log_entries.push(LogEntry::err(t!("log-export"), 0, error.as_str()));
                        new_phase = Some(ExportPhase::Failed(error));
                        break;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        ctx.request_repaint_after(Duration::from_millis(50));
                        break;
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        new_phase = Some(ExportPhase::Failed(t!("export-thread-died")));
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

    /// Opens the export dialog for `db`; with `table`, only that one comes
    /// checked. If the tables are not loaded yet, they are requested and the
    /// list is filled in when they arrive.
    fn open_export(&mut self, sh: &Shared, db: String, table: Option<String>) {
        // The destination servers are those of the same engine
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

    /// Fills in the table list of the export dialog for `db`, if it is open
    /// and waiting for them.
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

// ── Session frame ─────────────────────────────────────────────────────────────

impl Session {
    /// Polls the background threads (connection, query, export). The app
    /// calls it on every session, not just the visible one.
    pub fn poll(&mut self, ctx: &egui::Context, sh: &mut Shared) {
        self.poll_connection(ctx, sh);
        self.poll_jobs(ctx, sh);
        self.poll_export_progress(ctx);
        self.poll_keyring(sh);
        self.poll_db_picker();
    }

    /// Collects the database list requested from the connection form.
    fn poll_db_picker(&mut self) {
        let Some(rx) = &self.db_picker.receiver else {
            return;
        };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Err(t!("database-list-failed")),
        };
        self.db_picker.receiver = None;
        self.db_picker.list = match result {
            Ok(list) => Loadable::Ready(list),
            Err(e) => Loadable::Failed(e),
        };
    }

    /// Flags the profiles whose password is already saved in the keyring.
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
                    // In the form and, in case it is no longer open, in a notification
                    let message = t!("keyring-save-failed", error = e);
                    sh.toasts.error(message.clone());
                    self.error_message = Some(message);
                }
            }
        }
    }

    /// Draws the session and applies the frame's actions. The global panels
    /// (connection tabs, status bar, log and settings) have already been
    /// laid out by the app.
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
        // With a dialog open, the mouse does not reach what is behind it (the
        // dialog blocks it), but the keyboard does: it is ignored.
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
        // ── Modal (on top of everything) ──────────────────────────────────
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
        // Deleting a favourite asks for confirmation first
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

//! Server schema (databases, tables, info and structure): read in the
//! background and cached per session.

use std::collections::HashMap;

use eframe::egui;

use super::jobs::JobResult;
use super::{SelectedNode, Session};
use crate::db::{ColumnInfo, DatabaseInfo, DbBackend, DbResult, TableEntry, TableInfo, TableKind};
use crate::i18n::t;
use crate::loadable::Loadable;
use crate::ui::tab_bar::ActiveTab;
use crate::ui::theme;

/// Result of a schema job, with the number of its request.
pub enum SchemaResult {
    Databases {
        ticket: u64,
        result: DbResult<Vec<String>>,
    },
    Tables {
        ticket: u64,
        db: String,
        result: DbResult<Vec<TableEntry>>,
    },
    /// (database, table) pairs of the whole server.
    AllTables {
        ticket: u64,
        result: DbResult<Vec<(String, TableEntry)>>,
    },
    DatabasesInfo {
        ticket: u64,
        result: DbResult<Vec<DatabaseInfo>>,
    },
    TablesInfo {
        ticket: u64,
        db: String,
        result: DbResult<Vec<TableInfo>>,
    },
    Structure {
        ticket: u64,
        db: String,
        table: String,
        result: DbResult<Vec<ColumnInfo>>,
    },
}

/// Schema tree shown in the sidebar + server info caches.
#[derive(Default)]
pub struct SchemaCache {
    pub databases: Loadable<Vec<String>>,
    pub tables_cache: HashMap<String, Loadable<Vec<String>>>,
    /// For each database, the entries that are not tables (views) and what
    /// they are. Filled in with `tables_cache`.
    pub kinds: HashMap<String, HashMap<String, TableKind>>,
    pub table_search: String,
    pub selected_node: Option<SelectedNode>,
    pub databases_info: Loadable<Vec<DatabaseInfo>>,
    pub tables_info: HashMap<String, Loadable<Vec<TableInfo>>>,
    pub columns_info: HashMap<(String, String), Loadable<Vec<ColumnInfo>>>,
    last_ticket: u64,
}

impl SchemaCache {
    fn ticket(&mut self) -> u64 {
        self.last_ticket += 1;
        self.last_ticket
    }

    /// Columns of `db.table`, if already loaded.
    pub fn columns(&self, db: &str, table: &str) -> Option<&Vec<ColumnInfo>> {
        self.columns_info
            .get(&(db.to_string(), table.to_string()))
            .and_then(Loadable::ready)
    }

    /// Whether `db.table` is a table or a view. Anything unknown (its
    /// database has not been read yet) is treated as a table.
    pub fn kind(&self, db: &str, table: &str) -> TableKind {
        self.kinds
            .get(db)
            .and_then(|kinds| kinds.get(table))
            .copied()
            .unwrap_or_default()
    }

    /// Stores the freshly read tables of `db` and returns their names.
    fn store_kinds(&mut self, db: &str, entries: Vec<TableEntry>) -> Vec<String> {
        let views = entries
            .iter()
            .filter(|entry| entry.kind.is_view())
            .map(|entry| (entry.name.clone(), entry.kind))
            .collect();
        self.kinds.insert(db.to_string(), views);
        entries.into_iter().map(|entry| entry.name).collect()
    }

    /// Selected database (directly or through one of its tables).
    pub fn selected_db(&self) -> Option<&str> {
        self.selected_node.as_ref()?.db()
    }

    /// Open table, as (database, table).
    pub fn selected_table(&self) -> Option<(&str, &str)> {
        self.selected_node.as_ref()?.table()
    }

    /// Columns of the open table, if already loaded.
    pub fn open_table_columns(&self) -> Option<&Vec<ColumnInfo>> {
        let (db, table) = self.selected_table()?;
        self.columns(db, table)
    }

    /// Discards everything loaded (Refresh). The selection is kept.
    pub fn clear(&mut self) {
        self.databases = Loadable::NotLoaded;
        self.tables_cache.clear();
        self.kinds.clear();
        self.databases_info = Loadable::NotLoaded;
        self.tables_info.clear();
        self.columns_info.clear();
    }
}

/// Spinner while `value` loads, or the error if it failed.
pub fn show_loading<T>(ui: &mut egui::Ui, value: &Loadable<T>) {
    match value {
        Loadable::Failed(e) => theme::error_card(ui, &t!("common-error", error = e.as_str())),
        _ => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(t!("common-loading"));
            });
        }
    }
}

// ── Requests ─────────────────────────────────────────────────────────────────
// Each one starts the job only if the data is not loaded or on its way.

impl Session {
    fn spawn_schema_job(
        &mut self,
        job: impl FnOnce(&dyn DbBackend) -> SchemaResult + Send + 'static,
    ) -> bool {
        self.spawn_job(move |b| JobResult::Schema(job(b)))
    }

    pub(super) fn request_databases(&mut self) {
        if !self.schema.databases.needs_load() {
            return;
        }
        let ticket = self.schema.ticket();
        if self.spawn_schema_job(move |b| SchemaResult::Databases {
            ticket,
            result: b.list_databases(),
        }) {
            self.schema.databases = Loadable::Loading(ticket);
        }
    }

    pub(super) fn request_tables(&mut self, db: &str) {
        if !self
            .schema
            .tables_cache
            .get(db)
            .is_none_or(Loadable::needs_load)
        {
            return;
        }
        let ticket = self.schema.ticket();
        let job_db = db.to_string();
        if self.spawn_schema_job(move |b| SchemaResult::Tables {
            ticket,
            result: b.list_tables(&job_db),
            db: job_db,
        }) {
            self.schema
                .tables_cache
                .insert(db.to_string(), Loadable::Loading(ticket));
        }
    }

    /// The missing tables of every database, in a single query (the sidebar
    /// search needs them all).
    pub(super) fn request_all_tables(&mut self) {
        let Some(databases) = self.schema.databases.ready() else {
            return;
        };
        let missing: Vec<String> = databases
            .iter()
            .filter(|db| {
                self.schema
                    .tables_cache
                    .get(*db)
                    .is_none_or(Loadable::needs_load)
            })
            .cloned()
            .collect();
        if missing.is_empty() {
            return;
        }
        let ticket = self.schema.ticket();
        if self.spawn_schema_job(move |b| SchemaResult::AllTables {
            ticket,
            result: b.list_all_tables(),
        }) {
            for db in missing {
                self.schema
                    .tables_cache
                    .insert(db, Loadable::Loading(ticket));
            }
        }
    }

    pub(super) fn request_databases_info(&mut self) {
        if !self.schema.databases_info.needs_load() {
            return;
        }
        let ticket = self.schema.ticket();
        if self.spawn_schema_job(move |b| SchemaResult::DatabasesInfo {
            ticket,
            result: b.get_databases_info(),
        }) {
            self.schema.databases_info = Loadable::Loading(ticket);
        }
    }

    pub(super) fn request_tables_info(&mut self, db: &str) {
        if !self
            .schema
            .tables_info
            .get(db)
            .is_none_or(Loadable::needs_load)
        {
            return;
        }
        let ticket = self.schema.ticket();
        let job_db = db.to_string();
        if self.spawn_schema_job(move |b| SchemaResult::TablesInfo {
            ticket,
            result: b.get_tables_info(&job_db),
            db: job_db,
        }) {
            self.schema
                .tables_info
                .insert(db.to_string(), Loadable::Loading(ticket));
        }
    }

    /// Columns of the table, which also give its primary keys.
    pub(super) fn request_structure(&mut self, db: &str, table: &str) {
        let key = (db.to_string(), table.to_string());
        if !self
            .schema
            .columns_info
            .get(&key)
            .is_none_or(Loadable::needs_load)
        {
            return;
        }
        let ticket = self.schema.ticket();
        let (job_db, job_table) = key.clone();
        if self.spawn_schema_job(move |b| SchemaResult::Structure {
            ticket,
            result: b.get_columns_info(&job_db, &job_table),
            db: job_db,
            table: job_table,
        }) {
            self.schema
                .columns_info
                .insert(key, Loadable::Loading(ticket));
        }
    }

    /// What the active tab needs: called every frame, and only requests what
    /// is not loaded yet.
    pub(super) fn request_tab_data(&mut self) {
        self.request_databases();
        match self.active_tab {
            ActiveTab::Servidor => self.request_databases_info(),
            ActiveTab::Bbdd => {
                if let Some(db) = self.selected_db().map(str::to_string) {
                    self.request_tables_info(&db);
                }
            }
            ActiveTab::Tabla | ActiveTab::Datos => {
                if let Some(SelectedNode::Table { db, table }) = self.schema.selected_node.clone() {
                    self.request_structure(&db, &table);
                }
            }
            ActiveTab::Sql(_) => {}
        }
    }

    /// Selected database (directly or through one of its tables).
    pub(super) fn selected_db(&self) -> Option<&str> {
        self.schema.selected_db()
    }

    /// Primary keys of the open table, according to its structure (the
    /// columns with COLUMN_KEY = 'PRI'), if already loaded.
    pub(super) fn sync_primary_keys(&mut self) {
        let Some(SelectedNode::Table { db, table }) = &self.schema.selected_node else {
            return;
        };
        if let Some(columns) = self.schema.columns(db, table) {
            self.table_view.primary_keys = columns
                .iter()
                .filter(|c| c.is_primary())
                .map(|c| c.name.clone())
                .collect();
        }
    }

    /// Applies the result of a schema job.
    pub(super) fn apply_schema_job(&mut self, result: SchemaResult) {
        let schema = &mut self.schema;
        match result {
            SchemaResult::Databases { ticket, result } => schema.databases.settle(ticket, result),
            SchemaResult::Tables { ticket, db, result } => {
                // Only if the answer is still the one expected
                let current = matches!(
                    schema.tables_cache.get(&db),
                    Some(Loadable::Loading(t)) if *t == ticket
                );
                if current {
                    let names = result.map(|entries| schema.store_kinds(&db, entries));
                    if let Some(slot) = schema.tables_cache.get_mut(&db) {
                        slot.settle(ticket, names);
                    }
                }
                self.fill_export_tables(&db);
            }
            SchemaResult::AllTables { ticket, result } => {
                let mut by_db: HashMap<String, Vec<TableEntry>> = HashMap::new();
                let result = result.map(|pairs| {
                    for (db, table) in pairs {
                        by_db.entry(db).or_default().push(table);
                    }
                });
                let waiting: Vec<String> = schema
                    .tables_cache
                    .iter()
                    .filter(|(_, slot)| matches!(slot, Loadable::Loading(t) if *t == ticket))
                    .map(|(db, _)| db.clone())
                    .collect();
                for db in waiting {
                    let names = result
                        .clone()
                        .map(|()| schema.store_kinds(&db, by_db.remove(&db).unwrap_or_default()));
                    if let Some(slot) = schema.tables_cache.get_mut(&db) {
                        slot.settle(ticket, names);
                    }
                }
            }
            SchemaResult::DatabasesInfo { ticket, result } => {
                schema.databases_info.settle(ticket, result);
            }
            SchemaResult::TablesInfo { ticket, db, result } => {
                if let Some(slot) = schema.tables_info.get_mut(&db) {
                    slot.settle(ticket, result);
                }
            }
            SchemaResult::Structure {
                ticket,
                db,
                table,
                result,
            } => {
                if let Some(slot) = schema.columns_info.get_mut(&(db, table)) {
                    slot.settle(ticket, result);
                }
                self.sync_primary_keys();
            }
        }
    }
}

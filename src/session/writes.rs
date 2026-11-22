//! Database writes in the background (DDL, TRUNCATE and the grid's
//! changes) and what the session has to do when they finish.

use std::time::Instant;

use super::jobs::JobResult;
use super::{SelectedNode, Session, Shared};
use crate::db::{DbBackend, DbResult, Statement};
use crate::i18n::t;
use crate::loadable::Loadable;
use crate::log::LogEntry;
use crate::ui::modals::{ModalState, Reload};
use crate::ui::tab_bar::ActiveTab;

/// What was written, with what is needed to apply the result.
pub enum Write {
    CreateDatabase {
        name: String,
    },
    CreateTable {
        db: String,
        name: String,
    },
    DropDatabase {
        db: String,
    },
    DropTable {
        db: String,
        table: String,
    },
    /// Empty tables of `db`: those attempted and those that failed
    /// ("table: error").
    Truncate {
        db: String,
        tables: Vec<String>,
        failed: Vec<String>,
    },
    CloneTable {
        db: String,
        name: String,
    },
    /// Save the `count` pending changes of `db.table` and then do `then`.
    Commit {
        db: String,
        table: String,
        count: usize,
        then: AfterCommit,
    },
}

/// What waits for the pending changes to be saved.
pub enum AfterCommit {
    /// Read the data again (Save).
    ReloadData,
    Reload(Reload),
    /// Open another table in the Data tab.
    Switch {
        db: String,
        table: String,
    },
}

pub struct WriteResult {
    pub write: Write,
    /// Entries for the query log.
    pub log: Vec<LogEntry>,
    pub result: Result<(), String>,
}

/// Runs `run` and records it in `log` with its duration.
fn logged(
    log: &mut Vec<LogEntry>,
    label: String,
    run: impl FnOnce() -> DbResult<u64>,
) -> DbResult<u64> {
    let start = Instant::now();
    let result = run();
    let ms = start.elapsed().as_millis() as u64;
    log.push(match &result {
        Ok(rows) => LogEntry::ok(label, ms, *rows as usize),
        Err(e) => LogEntry::err(label, ms, e.as_str()),
    });
    result
}

// ── Start writes ─────────────────────────────────────────────────────────────

impl Session {
    fn spawn_write(&mut self, job: impl FnOnce(&dyn DbBackend) -> WriteResult + Send + 'static) {
        if self.spawn_job(move |b| JobResult::Write(job(b))) {
            self.writes_in_flight += 1;
        }
    }

    pub(super) fn create_database(&mut self, name: String, charset: String) {
        self.spawn_write(move |b| {
            let mut log = Vec::new();
            let result = logged(
                &mut log,
                format!("CREATE DATABASE {}", b.engine().quote_ident(&name)),
                || b.create_database(&name, &charset).map(|()| 0),
            );
            WriteResult {
                write: Write::CreateDatabase { name },
                log,
                result: result.map(drop),
            }
        });
    }

    pub(super) fn create_table(&mut self, db: String, name: String, sql: String) {
        self.spawn_write(move |b| {
            let mut log = Vec::new();
            let result = logged(&mut log, sql.clone(), || b.execute_write(Some(&db), &sql));
            WriteResult {
                write: Write::CreateTable { db, name },
                log,
                result: result.map(drop),
            }
        });
    }

    pub(super) fn drop_database(&mut self, db: String) {
        self.spawn_write(move |b| {
            let mut log = Vec::new();
            let result = logged(
                &mut log,
                format!("DROP DATABASE {}", b.engine().quote_ident(&db)),
                || b.drop_database(&db).map(|()| 0),
            );
            WriteResult {
                write: Write::DropDatabase { db },
                log,
                result: result.map(drop),
            }
        });
    }

    /// Drops a table or a view, with the statement for whichever it is.
    pub(super) fn drop_table(&mut self, db: String, table: String) {
        let kind = self.schema.kind(&db, &table);
        self.spawn_write(move |b| {
            let mut log = Vec::new();
            let result = logged(&mut log, b.engine().drop_table(&db, &table, kind), || {
                b.drop_table(&db, &table, kind).map(|()| 0)
            });
            WriteResult {
                write: Write::DropTable { db, table },
                log,
                result: result.map(drop),
            }
        });
    }

    /// Empties `tables` of `db`; without `tables`, all of the database's
    /// (asked from the server: the tree may not have loaded them). Not views:
    /// they have no data of their own.
    pub(super) fn truncate(&mut self, db: String, tables: Option<Vec<String>>) {
        self.spawn_write(move |b| {
            let mut log = Vec::new();
            let tables = match tables {
                Some(tables) => tables,
                None => match b.list_tables(&db) {
                    Ok(tables) => tables
                        .into_iter()
                        .filter(|table| !table.kind.is_view())
                        .map(|table| table.name)
                        .collect(),
                    Err(e) => {
                        return WriteResult {
                            write: Write::Truncate {
                                db,
                                tables: Vec::new(),
                                failed: Vec::new(),
                            },
                            log,
                            result: Err(e),
                        };
                    }
                },
            };
            let failed = tables
                .iter()
                .filter_map(|table| {
                    let label = format!("TRUNCATE TABLE {}", b.engine().qualified(&db, table));
                    logged(&mut log, label, || b.truncate_table(&db, table).map(|()| 0))
                        .err()
                        .map(|e| format!("{table}: {e}"))
                })
                .collect();
            WriteResult {
                write: Write::Truncate { db, tables, failed },
                log,
                result: Ok(()),
            }
        });
    }

    pub(super) fn clone_table(&mut self, db: String, source: String, name: String, data: bool) {
        self.spawn_write(move |b| {
            let mut log = Vec::new();
            let engine = b.engine();
            let (src, dst) = (engine.qualified(&db, &source), engine.qualified(&db, &name));
            let create = engine.create_table_like(&dst, &src);
            let mut result = logged(&mut log, create.clone(), || {
                b.execute_write(Some(&db), &create)
            });
            if result.is_ok() && data {
                let copy = format!("INSERT INTO {dst} SELECT * FROM {src}");
                result = logged(&mut log, copy.clone(), || b.execute_write(Some(&db), &copy));
            }
            WriteResult {
                write: Write::CloneTable { db, name },
                log,
                result: result.map(drop),
            }
        });
    }

    /// Saves the pending changes in a transaction and then does `then`.
    /// Meanwhile the grid accepts no more changes.
    pub(super) fn commit_pending_changes(&mut self, sh: &mut Shared, then: AfterCommit) {
        if self.pending_count() == 0 {
            self.after_commit(sh, then);
            return;
        }
        let Some(SelectedNode::Table { db, table }) = self.schema.selected_node.clone() else {
            return;
        };
        let stmts: Vec<Statement> = match self.pending_statements() {
            Ok(stmts) => stmts,
            Err(message) => {
                self.modal = ModalState::Error {
                    title: t!("save-cannot-title"),
                    message,
                };
                return;
            }
        };
        let summary = t!(
            "log-save-changes",
            deletes = self.table_view.pending_deletes.len(),
            edits = self.table_view.pending_edits.len(),
            inserts = self.table_view.pending_inserts.len()
        );
        let count = self.pending_count();
        self.table_view.editing_cell = None;
        self.table_view.saving = true;
        self.spawn_write(move |b| {
            let mut log = Vec::new();
            let statements = stmts.len() as u64;
            let result = logged(&mut log, summary, || {
                b.execute_transaction(Some(&db), &stmts)
                    .map(|()| statements)
            });
            WriteResult {
                write: Write::Commit {
                    db,
                    table,
                    count,
                    then,
                },
                log,
                result: result.map(drop),
            }
        });
    }

    fn after_commit(&mut self, sh: &mut Shared, then: AfterCommit) {
        match then {
            AfterCommit::ReloadData => {
                if let Some(SelectedNode::Table { db, table }) = self.schema.selected_node.clone() {
                    self.load_table(sh, &db, &table);
                }
            }
            AfterCommit::Reload(reload) => self.reload_now(sh, reload),
            AfterCommit::Switch { db, table } => {
                self.on_table_selected(sh, db, table);
                self.active_tab = ActiveTab::Datos;
            }
        }
    }

    // ── Apply the result ─────────────────────────────────────────────────────

    pub(super) fn apply_write(&mut self, sh: &mut Shared, done: WriteResult) {
        self.writes_in_flight = self.writes_in_flight.saturating_sub(1);
        for entry in done.log {
            self.push_log(entry);
        }
        match (done.write, done.result) {
            (Write::CreateDatabase { name }, Ok(())) => {
                self.close_form(|m| matches!(m, ModalState::CreateDatabase { .. }));
                self.schema.databases = Loadable::NotLoaded;
                self.schema.databases_info = Loadable::NotLoaded;
                sh.toasts.success(t!("database-created", db = name));
            }
            (Write::CreateTable { db, name }, Ok(())) => {
                self.close_form(|m| matches!(m, ModalState::CreateTable { .. }));
                self.invalidate_schema(&db);
                sh.toasts
                    .success(t!("table-created", table = format!("{db}.{name}")));
            }
            (Write::CloneTable { db, name }, Ok(())) => {
                self.close_form(|m| matches!(m, ModalState::CloneTable { .. }));
                self.invalidate_schema(&db);
                sh.toasts
                    .success(t!("table-created", table = format!("{db}.{name}")));
            }
            (
                Write::CreateDatabase { .. } | Write::CreateTable { .. } | Write::CloneTable { .. },
                Err(e),
            ) => {
                self.show_form_error(e);
            }
            (Write::DropDatabase { db }, Ok(())) => {
                self.on_dropped(&db, None);
                sh.toasts.success(t!("database-dropped", db = db));
            }
            (Write::DropTable { db, table }, Ok(())) => {
                self.on_dropped(&db, Some(&table));
                sh.toasts
                    .success(t!("table-dropped", table = format!("{db}.{table}")));
            }
            (Write::DropDatabase { db }, Err(message)) => {
                self.show_error(t!("database-drop-failed", db = db), message);
            }
            (Write::DropTable { db, table }, Err(message)) => {
                self.show_error(
                    t!("table-drop-failed", table = format!("{db}.{table}")),
                    message,
                );
            }
            (Write::Truncate { db, tables, failed }, result) => {
                self.after_truncate(sh, &db, &tables);
                if let Err(message) = result {
                    self.show_error(t!("tables-list-failed", db = db), message);
                } else if !failed.is_empty() {
                    self.show_error(t!("tables-empty-failed", db = db), failed.join("\n"));
                } else {
                    sh.toasts.success(match tables.as_slice() {
                        [table] => t!("table-emptied", table = format!("{db}.{table}")),
                        tables => t!("tables-emptied", count = tables.len(), db = db),
                    });
                }
            }
            (
                Write::Commit {
                    db,
                    table,
                    count,
                    then,
                },
                result,
            ) => {
                self.table_view.saving = false;
                match result {
                    Ok(()) => {
                        self.discard_pending();
                        sh.toasts
                            .success(t!("changes-saved", count = count, table = &table));
                        // If another table was opened meanwhile, what was waiting no longer applies
                        let still_open = matches!(
                            &self.schema.selected_node,
                            Some(SelectedNode::Table { db: d, table: t }) if *d == db && *t == table
                        );
                        if still_open {
                            self.after_commit(sh, then);
                        }
                    }
                    Err(message) => self.show_error(t!("save-failed-title"), message),
                }
            }
        }
    }

    /// Closes the form if it is still open (the user may have closed it
    /// while it ran).
    fn close_form(&mut self, is_form: impl Fn(&ModalState) -> bool) {
        if is_form(&self.modal) {
            self.modal = ModalState::None;
        }
    }

    /// Error of a form: shown in it if it is still open.
    fn show_form_error(&mut self, message: String) {
        match &mut self.modal {
            ModalState::CreateDatabase { error, .. }
            | ModalState::CreateTable { error, .. }
            | ModalState::CloneTable { error, .. } => *error = Some(message),
            _ => self.show_error(t!("error-title"), message),
        }
    }

    fn show_error(&mut self, title: String, message: String) {
        self.modal = ModalState::Error { title, message };
    }
}

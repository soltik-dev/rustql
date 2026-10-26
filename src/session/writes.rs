//! Escrituras contra la BD en segundo plano (DDL, TRUNCATE y los cambios del
//! grid) y lo que hay que hacer en la sesión cuando terminan.

use std::time::Instant;

use super::jobs::JobResult;
use super::{SelectedNode, Session, Shared};
use crate::db::{DbBackend, DbResult, Statement, qualified};
use crate::loadable::Loadable;
use crate::log::LogEntry;
use crate::ui::modals::{ModalState, Reload};
use crate::ui::tab_bar::ActiveTab;

/// Qué se escribió, con lo necesario para aplicar el resultado.
pub enum Write {
    CreateDatabase,
    CreateTable {
        db: String,
    },
    DropDatabase {
        db: String,
    },
    DropTable {
        db: String,
        table: String,
    },
    /// Vaciar tablas de `db`: las que se intentaron y las que fallaron
    /// («tabla: error»).
    Truncate {
        db: String,
        tables: Vec<String>,
        failed: Vec<String>,
    },
    CloneTable {
        db: String,
    },
    /// Guardar los cambios pendientes de `db.table` y después hacer `then`.
    Commit {
        db: String,
        table: String,
        then: AfterCommit,
    },
}

/// Lo que espera a que se guarden los cambios pendientes.
pub enum AfterCommit {
    /// Volver a leer los datos (Guardar).
    ReloadData,
    Reload(Reload),
    /// Abrir otra tabla en la pestaña Datos.
    Switch {
        db: String,
        table: String,
    },
}

pub struct WriteResult {
    pub write: Write,
    /// Entradas para el log de consultas.
    pub log: Vec<LogEntry>,
    pub result: Result<(), String>,
}

/// Ejecuta `run` y deja constancia en `log` con su duración.
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

// ── Lanzar escrituras ────────────────────────────────────────────────────────

impl Session {
    fn spawn_write(&mut self, job: impl FnOnce(&dyn DbBackend) -> WriteResult + Send + 'static) {
        if self.spawn_job(move |b| JobResult::Write(job(b))) {
            self.writes_in_flight += 1;
        }
    }

    pub(super) fn create_database(&mut self, name: String, charset: String) {
        self.spawn_write(move |b| {
            let mut log = Vec::new();
            let result = logged(&mut log, format!("CREATE DATABASE `{name}`"), || {
                b.create_database(&name, &charset).map(|()| 0)
            });
            WriteResult {
                write: Write::CreateDatabase,
                log,
                result: result.map(drop),
            }
        });
    }

    pub(super) fn create_table(&mut self, db: String, sql: String) {
        self.spawn_write(move |b| {
            let mut log = Vec::new();
            let result = logged(&mut log, sql.clone(), || b.execute_write(&sql));
            WriteResult {
                write: Write::CreateTable { db },
                log,
                result: result.map(drop),
            }
        });
    }

    pub(super) fn drop_database(&mut self, db: String) {
        self.spawn_write(move |b| {
            let mut log = Vec::new();
            let result = logged(&mut log, format!("DROP DATABASE `{db}`"), || {
                b.drop_database(&db).map(|()| 0)
            });
            WriteResult {
                write: Write::DropDatabase { db },
                log,
                result: result.map(drop),
            }
        });
    }

    pub(super) fn drop_table(&mut self, db: String, table: String) {
        self.spawn_write(move |b| {
            let mut log = Vec::new();
            let result = logged(&mut log, format!("DROP TABLE `{db}`.`{table}`"), || {
                b.drop_table(&db, &table).map(|()| 0)
            });
            WriteResult {
                write: Write::DropTable { db, table },
                log,
                result: result.map(drop),
            }
        });
    }

    /// Vacía `tables` de `db`; sin `tables`, todas las de la BBDD (se piden
    /// al servidor: el árbol puede no haberlas cargado).
    pub(super) fn truncate(&mut self, db: String, tables: Option<Vec<String>>) {
        self.spawn_write(move |b| {
            let mut log = Vec::new();
            let tables = match tables {
                Some(tables) => tables,
                None => match b.list_tables(&db) {
                    Ok(tables) => tables,
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
                    let label = format!("TRUNCATE TABLE {}", qualified(&db, table));
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
            let (src, dst) = (qualified(&db, &source), qualified(&db, &name));
            let create = format!("CREATE TABLE {dst} LIKE {src}");
            let mut result = logged(&mut log, create.clone(), || b.execute_write(&create));
            if result.is_ok() && data {
                let copy = format!("INSERT INTO {dst} SELECT * FROM {src}");
                result = logged(&mut log, copy.clone(), || b.execute_write(&copy));
            }
            WriteResult {
                write: Write::CloneTable { db },
                log,
                result: result.map(drop),
            }
        });
    }

    /// Guarda los cambios pendientes en una transacción y después hace
    /// `then`. Mientras tanto el grid no admite más cambios.
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
                    title: "No se pueden guardar los cambios".into(),
                    message,
                };
                return;
            }
        };
        let summary = format!(
            "Guardar cambios: {} eliminación(es), {} edición(es), {} inserción(es) (transacción)",
            self.table_view.pending_deletes.len(),
            self.table_view.pending_edits.len(),
            self.table_view.pending_inserts.len()
        );
        self.table_view.editing_cell = None;
        self.table_view.saving = true;
        self.spawn_write(move |b| {
            let mut log = Vec::new();
            let count = stmts.len() as u64;
            let result = logged(&mut log, summary, || {
                b.execute_transaction(&stmts).map(|()| count)
            });
            WriteResult {
                write: Write::Commit { db, table, then },
                log,
                result: result.map(drop),
            }
        });
    }

    fn after_commit(&mut self, sh: &mut Shared, then: AfterCommit) {
        match then {
            AfterCommit::ReloadData => {
                if let Some(SelectedNode::Table { db, table }) = self.schema.selected_node.clone() {
                    let sql = self.build_sql(sh, &db, &table);
                    self.run_query_async(sql);
                }
            }
            AfterCommit::Reload(reload) => self.reload_now(sh, reload),
            AfterCommit::Switch { db, table } => {
                self.on_table_selected(sh, db, table);
                self.active_tab = ActiveTab::Datos;
            }
        }
    }

    // ── Aplicar el resultado ─────────────────────────────────────────────────

    pub(super) fn apply_write(&mut self, sh: &mut Shared, done: WriteResult) {
        self.writes_in_flight = self.writes_in_flight.saturating_sub(1);
        for entry in done.log {
            self.push_log(entry);
        }
        match (done.write, done.result) {
            (Write::CreateDatabase, Ok(())) => {
                self.close_form(|m| matches!(m, ModalState::CreateDatabase { .. }));
                self.schema.databases = Loadable::NotLoaded;
                self.schema.databases_info = Loadable::NotLoaded;
            }
            (Write::CreateTable { db }, Ok(())) => {
                self.close_form(|m| matches!(m, ModalState::CreateTable { .. }));
                self.invalidate_schema(&db);
            }
            (Write::CloneTable { db }, Ok(())) => {
                self.close_form(|m| matches!(m, ModalState::CloneTable { .. }));
                self.invalidate_schema(&db);
            }
            (
                Write::CreateDatabase | Write::CreateTable { .. } | Write::CloneTable { .. },
                Err(e),
            ) => {
                self.show_form_error(e);
            }
            (Write::DropDatabase { db }, Ok(())) => self.on_dropped(&db, None),
            (Write::DropTable { db, table }, Ok(())) => self.on_dropped(&db, Some(&table)),
            (Write::DropDatabase { db }, Err(message)) => {
                self.show_error(format!("No se pudo eliminar «{db}»"), message);
            }
            (Write::DropTable { db, table }, Err(message)) => {
                self.show_error(format!("No se pudo eliminar «{db}.{table}»"), message);
            }
            (Write::Truncate { db, tables, failed }, result) => {
                self.after_truncate(sh, &db, &tables);
                if let Err(message) = result {
                    self.show_error(
                        format!("No se pudieron listar las tablas de «{db}»"),
                        message,
                    );
                } else if !failed.is_empty() {
                    self.show_error(
                        format!("No se pudieron vaciar todas las tablas de «{db}»"),
                        failed.join("\n"),
                    );
                }
            }
            (Write::Commit { db, table, then }, result) => {
                self.table_view.saving = false;
                match result {
                    Ok(()) => {
                        self.discard_pending();
                        // Si entretanto se abrió otra tabla, lo que esperaba ya no aplica
                        let still_open = matches!(
                            &self.schema.selected_node,
                            Some(SelectedNode::Table { db: d, table: t }) if *d == db && *t == table
                        );
                        if still_open {
                            self.after_commit(sh, then);
                        }
                    }
                    Err(message) => self.show_error("Error al guardar".into(), message),
                }
            }
        }
    }

    /// Cierra el formulario si sigue abierto (el usuario puede haberlo
    /// cerrado mientras se ejecutaba).
    fn close_form(&mut self, is_form: impl Fn(&ModalState) -> bool) {
        if is_form(&self.modal) {
            self.modal = ModalState::None;
        }
    }

    /// Error de un formulario: se muestra en él si sigue abierto.
    fn show_form_error(&mut self, message: String) {
        match &mut self.modal {
            ModalState::CreateDatabase { error, .. }
            | ModalState::CreateTable { error, .. }
            | ModalState::CloneTable { error, .. } => *error = Some(message),
            _ => self.show_error("Error".into(), message),
        }
    }

    fn show_error(&mut self, title: String, message: String) {
        self.modal = ModalState::Error { title, message };
    }
}

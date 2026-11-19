//! Database jobs on separate threads: the UI never waits for the server.
//!
//! Each job gets a copy of the backend and returns a `JobResult` through
//! the session's channel, which applies it in `Session::poll`. Results from
//! a previous connection (disconnecting, reconnecting to another server)
//! are discarded: they carry the epoch they were started in.

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};

use eframe::egui;

use super::schema::SchemaResult;
use super::writes::WriteResult;
use crate::db::{DbBackend, DbResult, QueryResult};

/// Result of a job, with what is needed to apply it to the session.
pub enum JobResult {
    /// Data of the open table. `generation` discards that of queries already
    /// replaced by a later one.
    TableData {
        generation: u64,
        sql: String,
        elapsed_ms: u64,
        result: DbResult<QueryResult>,
    },
    Schema(SchemaResult),
    Write(WriteResult),
    /// SQL editor result for the tab that started `token`.
    Editor {
        token: u64,
        sql: String,
        elapsed_ms: u64,
        result: DbResult<QueryResult>,
    },
    /// Definition of a view, to open it in a SQL tab.
    ViewDefinition {
        db: String,
        view: String,
        result: DbResult<String>,
    },
    /// The server was asked to cancel an editor run. If it succeeds, the one
    /// that ends (with an error) is its `Editor`.
    EditorCancel(DbResult<()>),
}

pub struct Jobs {
    tx: Sender<(u64, JobResult)>,
    rx: Receiver<(u64, JobResult)>,
    epoch: u64,
    /// Started in this epoch and not collected yet.
    pending: usize,
    /// To repaint as soon as a job finishes (updated on every poll).
    repaint: egui::Context,
}

impl Default for Jobs {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            tx,
            rx,
            epoch: 0,
            pending: 0,
            repaint: egui::Context::default(),
        }
    }
}

impl Jobs {
    /// Runs `job` on a thread with its own reference to the backend.
    pub fn spawn(
        &mut self,
        backend: &Arc<dyn DbBackend>,
        job: impl FnOnce(&dyn DbBackend) -> JobResult + Send + 'static,
    ) {
        let backend = Arc::clone(backend);
        let tx = self.tx.clone();
        let repaint = self.repaint.clone();
        let epoch = self.epoch;
        self.pending += 1;
        std::thread::spawn(move || {
            let result = job(backend.as_ref());
            let _ = tx.send((epoch, result));
            repaint.request_repaint();
        });
    }

    /// Next result of the current epoch, if there is one.
    pub fn try_recv(&mut self, ctx: &egui::Context) -> Option<JobResult> {
        self.repaint = ctx.clone();
        loop {
            let (epoch, result) = self.rx.try_recv().ok()?;
            if epoch == self.epoch {
                self.pending -= 1;
                return Some(result);
            }
        }
    }

    /// Discards the results of the jobs already started (when connecting or
    /// disconnecting).
    pub fn new_epoch(&mut self) {
        self.epoch += 1;
        self.pending = 0;
    }

    /// Jobs of this epoch not finished yet.
    #[cfg(test)]
    pub fn pending(&self) -> usize {
        self.pending
    }
}

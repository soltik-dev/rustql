//! Trabajos contra la BD en hilos aparte: la UI nunca espera al servidor.
//!
//! Cada trabajo recibe una copia del backend y devuelve un `JobResult` por
//! el canal de la sesión, que lo aplica en `Session::poll`. Los resultados de
//! una conexión anterior (desconectar, reconectar a otro servidor) se
//! descartan: llevan la época en la que se lanzaron.

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};

use eframe::egui;

use super::schema::SchemaResult;
use super::writes::WriteResult;
use crate::db::{DbBackend, DbResult, QueryResult};

/// Resultado de un trabajo, con lo necesario para aplicarlo a la sesión.
pub enum JobResult {
    /// Datos de la tabla abierta. `generation` descarta los de consultas
    /// que otra posterior ya ha sustituido.
    TableData {
        generation: u64,
        sql: String,
        elapsed_ms: u64,
        result: DbResult<QueryResult>,
    },
    Schema(SchemaResult),
    Write(WriteResult),
    /// Resultado del editor SQL para la pestaña que lanzó `token`.
    Editor {
        token: u64,
        sql: String,
        elapsed_ms: u64,
        result: DbResult<QueryResult>,
    },
    /// Definición de una vista, para abrirla en una pestaña SQL.
    ViewDefinition {
        db: String,
        view: String,
        result: DbResult<String>,
    },
    /// Se pidió al servidor cancelar una ejecución del editor. Si lo
    /// consigue, quien termina (con error) es su `Editor`.
    EditorCancel(DbResult<()>),
}

pub struct Jobs {
    tx: Sender<(u64, JobResult)>,
    rx: Receiver<(u64, JobResult)>,
    epoch: u64,
    /// Lanzados en esta época y aún sin recoger.
    pending: usize,
    /// Para repintar en cuanto termina un trabajo (se actualiza en cada poll).
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
    /// Lanza `job` en un hilo con su propia referencia al backend.
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

    /// Siguiente resultado de la época actual, si hay alguno.
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

    /// Descarta los resultados de los trabajos ya lanzados (al conectar o
    /// desconectar).
    pub fn new_epoch(&mut self) {
        self.epoch += 1;
        self.pending = 0;
    }

    /// Trabajos de esta época aún sin terminar.
    #[cfg(test)]
    pub fn pending(&self) -> usize {
        self.pending
    }
}

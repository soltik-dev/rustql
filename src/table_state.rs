use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::ui::results_grid::SortSpec;

/// Estado persistido por tabla: ordenación activa, filtro WHERE e historial.
#[derive(Clone, Serialize, Deserialize, Default)]
pub struct TableState {
    pub sort_specs: Vec<SortSpec>,
    pub filter_text: String,
    /// Historial de filtros, más reciente primero. Máximo 20 entradas.
    pub filter_history: Vec<String>,
}

pub type TableStates = HashMap<String, TableState>;

fn key(db: &str, table: &str) -> String {
    format!("{}/{}", db, table)
}

fn path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("rustql").join("table_states.json"))
}

pub fn load() -> TableStates {
    let Some(p) = path() else {
        return HashMap::new();
    };
    let Ok(data) = std::fs::read_to_string(p) else {
        return HashMap::new();
    };
    serde_json::from_str(&data).unwrap_or_default()
}

pub fn save(states: &TableStates) {
    let Some(p) = path() else { return };
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(data) = serde_json::to_string_pretty(states) {
        let _ = std::fs::write(p, data);
    }
}

pub fn get(states: &TableStates, db: &str, table: &str) -> TableState {
    states.get(&key(db, table)).cloned().unwrap_or_default()
}

pub fn set(states: &mut TableStates, db: &str, table: &str, state: TableState) {
    states.insert(key(db, table), state);
}

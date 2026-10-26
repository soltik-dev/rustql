use std::collections::HashMap;

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

/// Clave «bd/tabla» de una tabla: la del estado guardado y la de los favoritos.
pub fn table_key(db: &str, table: &str) -> String {
    format!("{db}/{table}")
}

const TABLE_STATES_FILE: &str = "table_states.json";

pub fn load() -> TableStates {
    crate::config_file::load(TABLE_STATES_FILE)
}

pub fn save(states: &TableStates) {
    crate::config_file::save(TABLE_STATES_FILE, states, false);
}

pub fn get(states: &TableStates, db: &str, table: &str) -> TableState {
    states
        .get(&table_key(db, table))
        .cloned()
        .unwrap_or_default()
}

pub fn set(states: &mut TableStates, db: &str, table: &str, state: TableState) {
    states.insert(table_key(db, table), state);
}

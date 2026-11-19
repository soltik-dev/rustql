use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::ui::results_grid::SortSpec;

/// Persisted per-table state: active sort, WHERE filter and history.
#[derive(Clone, Serialize, Deserialize, Default)]
pub struct TableState {
    pub sort_specs: Vec<SortSpec>,
    pub filter_text: String,
    /// Filter history, most recent first. At most 20 entries.
    pub filter_history: Vec<String>,
}

pub type TableStates = HashMap<String, TableState>;

/// "db/table" key of a table: that of the saved state and of favourites.
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

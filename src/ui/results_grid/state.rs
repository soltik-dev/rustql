//! Estado y tipos del grid: ordenación, cambios pendientes, edición y
//! selección, y las acciones que devuelve.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::db::QueryResult;

// ── Tipos de ordenación ───────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SortDir {
    Asc,
    Desc,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SortSpec {
    pub col_idx: usize,
    pub col_name: String,
    pub dir: SortDir,
}

// ── Tipos de edición ─────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct PendingEdit {
    /// Índice real en `QueryResult.rows` (no el índice de visualización con sort).
    pub row_idx: usize,
    pub col_idx: usize,
    pub new_value: String,
    /// Si true, el UPDATE usa `= NULL` en lugar de `= 'valor'`.
    pub is_null: bool,
}

/// Identifies a row unambiguously: either an existing data row or a pending insert row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowRef {
    /// Index into `QueryResult.rows`.
    Existing(usize),
    /// Index into `pending_inserts`.
    Insert(usize),
}

/// Valor de una celda de una fila nueva.
#[derive(Debug, Clone, PartialEq)]
pub enum InsertValue {
    /// Se omite del INSERT: el servidor pone el DEFAULT de la columna (o el
    /// siguiente AUTO_INCREMENT).
    Default,
    Null,
    Value(String),
}

impl From<Option<String>> for InsertValue {
    fn from(value: Option<String>) -> Self {
        value.map_or(Self::Null, Self::Value)
    }
}

/// A pending insert row at the bottom of the grid.
#[derive(Debug, Clone)]
pub struct PendingInsert {
    /// Valor por columna; las no tocadas, `Default`.
    pub values: Vec<InsertValue>,
    /// Which columns are auto_increment (shown as "(AUTO)", not editable)
    pub auto_cols: Vec<bool>,
}

#[derive(Debug, Clone)]
pub struct EditingCell {
    pub row: RowRef,
    pub col_idx: usize,
    /// Buffer de edición mutable mientras el usuario escribe.
    pub buffer: String,
    /// Solicitar foco al TextEdit en el primer frame de edición.
    pub needs_focus: bool,
}

#[derive(Debug, Default)]
pub enum GridAction {
    #[default]
    None,
    /// La ordenación que pide el usuario (aún sin aplicar).
    SortChanged(Vec<SortSpec>),
    CellEditStarted {
        row: RowRef,
        col_idx: usize,
        current_value: String,
    },
    CellEditCommitted {
        row: RowRef,
        col_idx: usize,
        new_value: String,
    },
    CellSetNull {
        row: RowRef,
        col_idx: usize,
    },
    CellEditCancelled,
    CellEditTabNext {
        row: RowRef,
        col_idx: usize,
        new_value: String,
    },
    ApplyEdits,
    DiscardEdits,
    // Filtrado desde menú contextual
    FilterBy {
        sql_fragment: String,
    },
    FilterByClipboard {
        col_name: String,
    },
    FilterUserInput {
        col_name: String,
    },
    // Inline delete/insert/clone
    MarkDeleteRow {
        row_idx: usize,
    },
    InsertRow,
    CloneRow {
        row_idx: usize,
    },
    RemoveInsertRow {
        insert_idx: usize,
    },
    // Row selection
    RowSelected {
        row_idx: usize,
        shift: bool,
        ctrl: bool,
    },
    CopySelectedRows,
    CopyRowsAs {
        fmt: crate::settings::CopyFormat,
    },
    // Cell selection via plain click
    CellSelected {
        row_idx: usize,
        col_idx: usize,
    },
    // Mark multiple rows for delete (used when selected_rows is non-empty)
    MarkDeleteRows {
        row_indices: Vec<usize>,
    },
}

/// Estado de la tabla abierta: datos, ordenación y filtro, cambios
/// pendientes, edición y selección.
#[derive(Default)]
pub struct TableViewState {
    pub table_data: Option<QueryResult>,
    pub table_error: Option<String>,
    pub sort_specs: Vec<SortSpec>,
    pub filter_text: String,
    pub filter_history: Vec<String>,
    pub pending_edits: Vec<PendingEdit>,
    pub editing_cell: Option<EditingCell>,
    pub primary_keys: Vec<String>,
    pub pending_deletes: BTreeSet<usize>,
    pub pending_inserts: Vec<PendingInsert>,
    pub selected_cell: Option<(usize, usize)>,
    pub selected_rows: BTreeSet<usize>,
    pub last_selected_row: Option<usize>,
    pub drag_select_from: Option<usize>,
    pub query_loading: bool,
    /// Guardando los cambios pendientes: el grid no admite más hasta saber
    /// el resultado.
    pub saving: bool,
    /// Se incrementa con cada consulta de la tabla: los resultados de una
    /// anterior que lleguen tarde se descartan.
    pub data_generation: u64,
}

impl TableViewState {
    /// Vacía la vista (otra tabla, otra conexión, tabla borrada). La
    /// generación avanza: los datos de consultas anteriores que sigan en
    /// camino se descartan.
    pub fn reset(&mut self) {
        *self = Self {
            data_generation: self.data_generation + 1,
            ..Self::default()
        };
    }
}

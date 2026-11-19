//! Grid state and types: sorting, pending changes, editing and selection,
//! and the actions it returns.

use std::collections::BTreeSet;

use eframe::egui;
use serde::{Deserialize, Serialize};

use crate::db::QueryResult;
use crate::settings::CopyFormat;

// ── Sort types ────────────────────────────────────────────────────────────────

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

// ── Edit types ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct PendingEdit {
    /// Real index in `QueryResult.rows` (not the display index with sorting).
    pub row_idx: usize,
    pub col_idx: usize,
    pub new_value: String,
    /// If true, the UPDATE uses `= NULL` instead of `= 'value'`.
    pub is_null: bool,
}

/// Identifies a grid row: one of the data or a new unsaved one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowRef {
    /// Index in `QueryResult.rows`.
    Existing(usize),
    /// Index in `pending_inserts`.
    Insert(usize),
}

impl RowRef {
    /// Row at position `display` of the grid: the `existing` rows of the data
    /// come first and, below them, the new ones.
    pub fn from_display(display: usize, existing: usize) -> Self {
        if display < existing {
            Self::Existing(display)
        } else {
            Self::Insert(display - existing)
        }
    }

    /// Position of the row in the grid (inverse of `from_display`).
    pub fn display(self, existing: usize) -> usize {
        match self {
            Self::Existing(i) => i,
            Self::Insert(i) => existing + i,
        }
    }
}

/// A grid cell, of an existing row or a new one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellRef {
    pub row: RowRef,
    pub col: usize,
}

/// Value of a cell of a new row.
#[derive(Debug, Clone, PartialEq)]
pub enum InsertValue {
    /// Left out of the INSERT: the server sets the column's DEFAULT (or the
    /// next AUTO_INCREMENT).
    Default,
    Null,
    Value(String),
}

impl From<Option<String>> for InsertValue {
    fn from(value: Option<String>) -> Self {
        value.map_or(Self::Null, Self::Value)
    }
}

/// New row, at the end of the grid, waiting to be inserted.
#[derive(Debug, Clone)]
pub struct PendingInsert {
    /// Value per column; untouched ones are `Default`.
    pub values: Vec<InsertValue>,
    /// AUTO_INCREMENT columns: shown as "(AUTO)" and not edited.
    pub auto_cols: Vec<bool>,
}

/// Cell being edited.
#[derive(Debug, Clone)]
pub struct EditingCell {
    pub cell: CellRef,
    /// Editor text while the user types.
    pub buffer: String,
    /// Text the cell had when editing started. Confirming without changing it
    /// is not an edit: NULL and DEFAULT are edited as empty text, and tabbing
    /// through them would turn them into ''.
    pub original: String,
    /// Request focus for the editor on its first frame.
    pub needs_focus: bool,
}

impl EditingCell {
    /// Editor that starts with the cell's text.
    pub fn new(cell: CellRef, text: String) -> Self {
        Self {
            cell,
            buffer: text.clone(),
            original: text,
            needs_focus: true,
        }
    }

    /// Editor that starts with what the user just typed over the cell,
    /// instead of its text (`original`).
    pub fn replacing(cell: CellRef, original: String, typed: String) -> Self {
        Self {
            cell,
            buffer: typed,
            original,
            needs_focus: true,
        }
    }
}

/// What the user asks to do with the data. The selection, the cursor and
/// the cell being edited are the grid's own state and it changes them.
#[derive(Debug, Default)]
pub enum GridAction {
    #[default]
    None,
    /// The sort order the user asks for (not applied yet).
    SortChanged(Vec<SortSpec>),
    /// New value of a cell; `None` is NULL.
    CellEdited {
        row: RowRef,
        col_idx: usize,
        value: Option<String>,
    },
    /// Cell of a new row back to DEFAULT.
    CellSetDefault {
        insert_idx: usize,
        col_idx: usize,
    },
    // Filtering from the context menu
    FilterBy {
        sql_fragment: String,
    },
    FilterByClipboard {
        col_name: String,
    },
    FilterUserInput {
        col_name: String,
    },
    /// Marks for deletion the rows that are not marked and unmarks those that are.
    ToggleDeleteRows(Vec<usize>),
    InsertRow,
    CloneRow {
        row_idx: usize,
    },
    RemoveInsertRow {
        insert_idx: usize,
    },
    /// Copies the selected rows; without a format, in the one from the settings.
    CopyRows(Option<CopyFormat>),
    /// Go to another page of the table (0 = the first).
    PageChanged(usize),
}

// ── Selection ────────────────────────────────────────────────────────────────

/// Cursor and selected rows of the grid, and what it needs to bring the
/// view to the cursor when it moves with the keyboard.
#[derive(Debug, Default)]
pub struct GridSelection {
    /// Cursor cell: the one that receives the keys.
    pub cell: Option<CellRef>,
    /// Existing rows selected as a whole (to copy or delete them).
    pub rows: BTreeSet<usize>,
    /// Where the selection extends from with Shift+click.
    anchor_row: Option<usize>,
    /// Row where the drag that selects a range started.
    pub(super) drag_from: Option<usize>,
    /// The cursor moved without the mouse: the grid has to scroll to it.
    pub(super) reveal: bool,
    /// Times it has already been tried (see `table::show`).
    pub(super) reveal_tries: u8,
    /// Go back to the top (new data with another order or filter).
    pub(super) scroll_top: bool,
    /// Vertical scroll requested with the grid's bar.
    pub(super) scroll_to_y: Option<f32>,
    /// Where everything was in the previous frame.
    pub(super) layout: Option<GridLayout>,
    /// Id the grid receives keyboard focus with.
    pub focus_id: Option<egui::Id>,
}

/// Grid geometry in the last frame drawn. Row positions are measured on
/// what was drawn instead of assumed: asking for a scroll a few points
/// past the real maximum makes egui treat it as a bounce and bring it
/// back with an animation.
#[derive(Debug, Clone, Default)]
pub(super) struct GridLayout {
    /// Horizontal and vertical scroll.
    pub offset: egui::Vec2,
    /// Size of the visible area of the rows.
    pub size: egui::Vec2,
    /// Maximum scroll on each axis.
    pub max_offset: egui::Vec2,
    /// Where row 0 starts, measured from the start of the content.
    pub row_origin: f32,
    /// Distance between the start of a row and the start of the next.
    pub row_pitch: f32,
    pub row_h: f32,
    pub col_widths: Vec<f32>,
    /// Spacing between columns.
    pub col_gap: f32,
}

impl GridLayout {
    /// Vertical span of the row shown at position `display`.
    pub fn row_span(&self, display: usize) -> (f32, f32) {
        let top = self.row_origin + display as f32 * self.row_pitch;
        (top, top + self.row_h)
    }

    /// Horizontal span of column `col`, if it exists.
    pub fn col_span(&self, col: usize) -> Option<(f32, f32)> {
        let width = *self.col_widths.get(col)?;
        let left: f32 = self.col_widths[..col]
            .iter()
            .map(|w| w + self.col_gap)
            .sum();
        Some((left, left + width))
    }

    /// Scroll (x, y) that brings the cell into view, without going past the
    /// maximum; `None` on an axis that does not need to move.
    pub fn reveal(&self, display: usize, col: usize) -> (Option<f32>, Option<f32>) {
        let axis = |span: (f32, f32), offset: f32, viewport: f32, max: f32| {
            scroll_into_view(span.0, span.1, offset, viewport)
                .map(|target| target.clamp(0.0, max))
                .filter(|target| (target - offset).abs() > 0.5)
        };
        let y = axis(
            self.row_span(display),
            self.offset.y,
            self.size.y,
            self.max_offset.y,
        );
        let x = self
            .col_span(col)
            .and_then(|span| axis(span, self.offset.x, self.size.x, self.max_offset.x));
        (x, y)
    }
}

impl GridSelection {
    /// Puts the cursor on `cell` (a click): no rows stay selected.
    pub fn select_cell(&mut self, cell: CellRef) {
        self.cell = Some(cell);
        self.rows.clear();
        self.anchor_row = None;
    }

    /// Like `select_cell`, and the grid also scrolls to the cell (keyboard,
    /// or a freshly added row).
    pub fn move_to(&mut self, cell: CellRef) {
        self.select_cell(cell);
        self.reveal = true;
        self.reveal_tries = 0;
    }

    /// Selects whole rows: only `row`, the range from the previous one
    /// (Shift), or adds or removes it (Ctrl). The cursor stays on the clicked
    /// cell.
    pub fn select_row(&mut self, row: usize, col: usize, shift: bool, ctrl: bool) {
        if shift {
            let cursor_row = match self.cell {
                Some(CellRef {
                    row: RowRef::Existing(r),
                    ..
                }) => Some(r),
                _ => None,
            };
            let anchor = self.anchor_row.or(cursor_row).unwrap_or(row);
            self.rows = (anchor.min(row)..=anchor.max(row)).collect();
            self.anchor_row = Some(anchor);
        } else if ctrl {
            if !self.rows.remove(&row) {
                self.rows.insert(row);
                self.anchor_row = Some(row);
            }
        } else {
            self.rows = BTreeSet::from([row]);
            self.anchor_row = Some(row);
        }
        self.cell = Some(CellRef {
            row: RowRef::Existing(row),
            col,
        });
    }

    /// Removes the cursor and the selection, and goes back to the top: the
    /// incoming data (another order, another filter) is no longer the same.
    pub fn clear(&mut self) {
        self.deselect();
        self.scroll_top = true;
    }

    /// Removes the cursor and the selection (Escape).
    pub fn deselect(&mut self) {
        self.cell = None;
        self.rows.clear();
        self.anchor_row = None;
        self.drag_from = None;
        self.reveal = false;
    }

    /// Fits the selection to the rows and columns there are now: after a
    /// reload or removing a new row they may no longer exist.
    pub(super) fn clamp(&mut self, existing: usize, inserts: usize, cols: usize) {
        self.rows.retain(|&r| r < existing);
        self.cell = self.cell.and_then(|cell| {
            if cell.col >= cols {
                return None;
            }
            let row = match cell.row {
                RowRef::Existing(r) if r < existing => RowRef::Existing(r),
                RowRef::Insert(i) if inserts > 0 => RowRef::Insert(i.min(inserts - 1)),
                _ => return None,
            };
            Some(CellRef { row, col: cell.col })
        });
    }
}

/// New scroll that keeps span `start..end` inside a window of size
/// `viewport` that now starts at `offset`. `None` if it is already fully
/// visible.
pub(super) fn scroll_into_view(start: f32, end: f32, offset: f32, viewport: f32) -> Option<f32> {
    if start < offset {
        Some(start)
    } else if end > offset + viewport {
        // If the span does not fit, its start wins
        Some((end - viewport).min(start))
    } else {
        None
    }
}

// ── State of the open table ──────────────────────────────────────────────────

/// State of the open table: data, sort and filter, pending changes,
/// editing and selection.
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
    pub selection: GridSelection,
    pub query_loading: bool,
    /// Saving the pending changes: the grid accepts no more until the result
    /// is known.
    pub saving: bool,
    /// Incremented with each query of the table: late results of an earlier
    /// one are discarded.
    pub data_generation: u64,
    /// Page of the table being shown (0 = the first).
    pub page: usize,
    /// Rows per page the data was requested with.
    pub page_size: usize,
    /// There are more rows after this page.
    pub has_more: bool,
}

impl TableViewState {
    /// Empties the view (another table, another connection, table dropped).
    /// The generation moves on: data from earlier queries still on its way is
    /// discarded.
    pub fn reset(&mut self) {
        *self = Self {
            data_generation: self.data_generation + 1,
            ..Self::default()
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(row: RowRef, col: usize) -> CellRef {
        CellRef { row, col }
    }

    #[test]
    fn las_filas_nuevas_van_debajo_de_las_existentes() {
        assert_eq!(RowRef::from_display(2, 3), RowRef::Existing(2));
        assert_eq!(RowRef::from_display(3, 3), RowRef::Insert(0));
        assert_eq!(RowRef::Insert(1).display(3), 4);
        assert_eq!(RowRef::Existing(1).display(3), 1);
    }

    #[test]
    fn mayus_clic_selecciona_el_rango_desde_el_cursor() {
        let mut sel = GridSelection::default();
        sel.select_cell(cell(RowRef::Existing(4), 0));
        sel.select_row(1, 2, true, false);
        assert_eq!(sel.rows, BTreeSet::from([1, 2, 3, 4]));
        assert_eq!(sel.cell, Some(cell(RowRef::Existing(1), 2)));
        // Another Shift+click starts from the same origin
        sel.select_row(6, 0, true, false);
        assert_eq!(sel.rows, BTreeSet::from([4, 5, 6]));
    }

    #[test]
    fn ctrl_clic_anade_y_quita_filas() {
        let mut sel = GridSelection::default();
        sel.select_row(1, 0, false, false);
        sel.select_row(3, 0, false, true);
        assert_eq!(sel.rows, BTreeSet::from([1, 3]));
        sel.select_row(1, 0, false, true);
        assert_eq!(sel.rows, BTreeSet::from([3]));
    }

    #[test]
    fn mover_el_cursor_deselecciona_las_filas() {
        let mut sel = GridSelection::default();
        sel.select_row(1, 0, false, false);
        sel.move_to(cell(RowRef::Existing(2), 0));
        assert!(sel.rows.is_empty());
        assert!(sel.reveal);
    }

    #[test]
    fn la_seleccion_se_ajusta_a_lo_que_queda() {
        let mut sel = GridSelection::default();
        sel.select_row(5, 0, false, false);
        sel.clamp(3, 0, 2);
        assert!(sel.rows.is_empty());
        assert_eq!(sel.cell, None, "la fila 5 ya no existe");

        // When the last new row is removed, the cursor moves to the previous one
        sel.select_cell(cell(RowRef::Insert(2), 1));
        sel.clamp(3, 2, 2);
        assert_eq!(sel.cell, Some(cell(RowRef::Insert(1), 1)));
        sel.clamp(3, 0, 2);
        assert_eq!(sel.cell, None);
    }

    #[test]
    fn desplazar_solo_lo_necesario() {
        // Window of 100 starting at 50: 50..150 is visible
        assert_eq!(scroll_into_view(60.0, 80.0, 50.0, 100.0), None);
        assert_eq!(scroll_into_view(30.0, 50.0, 50.0, 100.0), Some(30.0));
        assert_eq!(scroll_into_view(140.0, 160.0, 50.0, 100.0), Some(60.0));
        // Wider than the window: its start is aligned
        assert_eq!(scroll_into_view(200.0, 400.0, 50.0, 100.0), Some(200.0));
    }
}

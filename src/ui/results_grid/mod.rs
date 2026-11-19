//! Data grid of the open table, with cell editing.
//!
//! The grid owns its view state (cursor, selected rows, cell being edited,
//! scroll) and changes it itself. Whatever affects the data (editing,
//! inserting, deleting, sorting, filtering) it returns as a `GridAction`
//! for the session to apply.

mod apply_bar;
mod copy;
mod keys;
mod menu;
mod state;
mod table;
#[cfg(test)]
mod tests;

use std::collections::BTreeSet;

use eframe::egui;

pub use self::apply_bar::{ApplyBarAction, show_apply_bar};
use self::copy::format_rows_for_copy;
pub use self::state::{
    CellRef, EditingCell, GridAction, GridSelection, InsertValue, PendingEdit, PendingInsert,
    RowRef, SortDir, SortSpec, TableViewState,
};
use crate::db::{ColumnInfo, Engine, QueryResult, TableKind};
use crate::settings::CopyFormat;
use crate::ui::icons::{PLUS, VIEW};
use crate::ui::{CMD, theme};

// ── What the grid draws ──────────────────────────────────────────────────────

/// Data, structure and pending changes the grid is drawn with.
struct Grid<'a> {
    /// Identifies the grid: the focus id and the scroll ids come from it.
    id: egui::Id,
    data: &'a QueryResult,
    column_info: &'a [ColumnInfo],
    sorts: &'a [SortSpec],
    pending_edits: &'a [PendingEdit],
    pending_deletes: &'a BTreeSet<usize>,
    pending_inserts: &'a [PendingInsert],
    /// Cells can be edited and rows inserted or deleted.
    editable: bool,
    /// Existing rows can be identified (the table has a primary key): without
    /// that they are not edited or deleted, only inserted.
    keyed: bool,
    /// The data is a table's: it can be sorted and filtered by re-running its
    /// query.
    queryable: bool,
    /// Engine of the connection: the menu's filters are written in its dialect.
    engine: Engine,
    /// Page of the table being shown, if the data comes in pages.
    pager: Option<Pager>,
}

/// Page of the table the grid shows.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Pager {
    /// 0 = the first.
    page: usize,
    page_size: usize,
    has_more: bool,
}

impl Pager {
    /// There is more than one page: it makes sense to show which one it is on.
    fn paged(&self) -> bool {
        self.page > 0 || self.has_more
    }

    fn previous(&self) -> Option<usize> {
        self.page.checked_sub(1)
    }

    fn next(&self) -> Option<usize> {
        self.has_more.then_some(self.page + 1)
    }
}

/// What a cell shows.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Shown<'a> {
    Value(&'a str),
    Null,
    /// New row: the server will set the default value.
    Default,
    /// New row: AUTO_INCREMENT column.
    Auto,
}

impl<'a> Grid<'a> {
    fn focus_id(&self) -> egui::Id {
        self.id.with("focus")
    }

    /// Rows of the data; new ones come after.
    fn existing(&self) -> usize {
        self.data.rows.len()
    }

    fn total_rows(&self) -> usize {
        self.existing() + self.pending_inserts.len()
    }

    fn cols(&self) -> usize {
        self.data.columns.len()
    }

    fn pending_edit(&self, row: usize, col: usize) -> Option<&'a PendingEdit> {
        self.pending_edits
            .iter()
            .find(|e| e.row_idx == row && e.col_idx == col)
    }

    fn is_deleted(&self, row: RowRef) -> bool {
        matches!(row, RowRef::Existing(r) if self.pending_deletes.contains(&r))
    }

    /// Value saved on the server (without the pending changes).
    fn stored(&self, row: usize, col: usize) -> Option<&'a str> {
        self.data.rows.get(row)?.get(col)?.as_deref()
    }

    /// What the cell shows: the pending change if there is one and, if not,
    /// the data.
    fn shown(&self, cell: CellRef) -> Shown<'a> {
        match cell.row {
            RowRef::Existing(row) => match self.pending_edit(row, cell.col) {
                Some(edit) if edit.is_null => Shown::Null,
                Some(edit) => Shown::Value(&edit.new_value),
                None => self.stored(row, cell.col).map_or(Shown::Null, Shown::Value),
            },
            RowRef::Insert(idx) => {
                let Some(ins) = self.pending_inserts.get(idx) else {
                    return Shown::Default;
                };
                if ins.auto_cols.get(cell.col).copied().unwrap_or(false) {
                    return Shown::Auto;
                }
                match ins.values.get(cell.col) {
                    Some(InsertValue::Value(v)) => Shown::Value(v),
                    Some(InsertValue::Null) => Shown::Null,
                    _ => Shown::Default,
                }
            }
        }
    }

    /// Value highlighted in the other cells that show it: that of the
    /// cursor's cell, unless it is empty, NULL, or the DEFAULT or AUTO of a
    /// new row (they are not a value to look for).
    fn highlighted_value(&self, cursor: Option<CellRef>) -> Option<&'a str> {
        match self.shown(cursor?) {
            Shown::Value(v) if !v.is_empty() => Some(v),
            _ => None,
        }
    }

    /// Text the cell editor starts with: NULL and DEFAULT are edited as
    /// empty text.
    fn edit_text(&self, cell: CellRef) -> String {
        match self.shown(cell) {
            Shown::Value(v) => v.to_string(),
            _ => String::new(),
        }
    }

    /// Rows marked for deletion, AUTO_INCREMENT columns of a new row and,
    /// without a primary key, existing rows are not edited.
    fn can_edit(&self, cell: CellRef) -> bool {
        self.editable
            && cell.col < self.cols()
            && self.can_change_row(cell.row)
            && !self.is_deleted(cell.row)
            && self.shown(cell) != Shown::Auto
    }

    /// Whether the row can be edited or deleted: new ones always; existing
    /// ones only if they can be identified when saving.
    fn can_change_row(&self, row: RowRef) -> bool {
        self.editable && (self.keyed || matches!(row, RowRef::Insert(_)))
    }

    /// Whether the column allows NULL. Without its structure (still loading)
    /// it is assumed so: the server will reject it on save if not.
    fn nullable(&self, col: usize) -> bool {
        self.column_info.get(col).is_none_or(|c| c.is_nullable)
    }

    /// Next editable cell in reading order (or the previous one): the one
    /// Tab moves to while editing.
    fn next_editable(&self, from: CellRef, forward: bool) -> Option<CellRef> {
        let cols = self.cols();
        let total = self.total_rows() * cols;
        let mut idx = from.row.display(self.existing()) * cols + from.col;
        loop {
            idx = if forward {
                idx + 1
            } else {
                idx.checked_sub(1)?
            };
            if idx >= total {
                return None;
            }
            let cell = CellRef {
                row: RowRef::from_display(idx / cols, self.existing()),
                col: idx % cols,
            };
            if self.can_edit(cell) {
                return Some(cell);
            }
        }
    }
}

// ── Grid of the open table ───────────────────────────────────────────────────

/// Shows the data of the open table. Returns what the user asks to do
/// with it this frame. With `busy` (loading or saving) editing and
/// changing pages are not possible.
pub fn show(
    ui: &mut egui::Ui,
    selected_table: Option<(&str, &str)>,
    view: &mut TableViewState,
    column_info: &[ColumnInfo],
    engine: Engine,
    kind: TableKind,
    busy: bool,
) -> GridAction {
    let Some((db, table)) = selected_table else {
        ui.centered_and_justified(|ui| {
            ui.label("Selecciona una tabla en el panel izquierdo");
        });
        return GridAction::None;
    };
    if let Some(err) = &view.table_error {
        ui.label(format!("{db}.{table}"));
        ui.separator();
        theme::error_card(ui, &format!("Error: {err}"));
        return GridAction::None;
    }
    let Some(data) = &view.table_data else {
        ui.centered_and_justified(|ui| {
            if busy {
                ui.spinner();
                ui.label("Ejecutando consulta…");
            } else {
                ui.label("Cargando…");
            }
        });
        return GridAction::None;
    };

    // Without the structure (still loading) a key is assumed: if there is
    // none, it will be known before anything can be saved
    let keyed = column_info.is_empty() || column_info.iter().any(ColumnInfo::is_primary);
    let pager = Pager {
        page: view.page,
        page_size: view.page_size,
        has_more: view.has_more,
    };
    let bar = InfoBar {
        name: format!("{db}.{table}"),
        data,
        sorts: &view.sort_specs,
        busy,
        pager,
        keyed,
        kind,
    };
    let bar_action = info_bar(ui, &bar);
    ui.separator();

    if data.columns.is_empty() {
        ui.label("(sin columnas)");
        return bar_action;
    }

    let grid = Grid {
        id: ui.make_persistent_id(("grid", db, table)),
        data,
        column_info,
        sorts: &view.sort_specs,
        pending_edits: &view.pending_edits,
        pending_deletes: &view.pending_deletes,
        pending_inserts: &view.pending_inserts,
        // A view can be seen, sorted and filtered, but is not edited here
        editable: !busy && !kind.is_view(),
        keyed,
        queryable: true,
        engine,
        // While loading, the page shown is no longer the one requested
        pager: (!busy).then_some(pager),
    };
    let out = show_grid(ui, &grid, &mut view.selection, &mut view.editing_cell);

    // Editing takes priority over sorting
    match (out.action, out.clicked_col) {
        (GridAction::None, Some(col)) => {
            GridAction::SortChanged(toggle_sort(&view.sort_specs, col, &data.columns[col]))
        }
        (GridAction::None, None) => bar_action,
        (action, _) => action,
    }
}

/// Read-only grid for the result of a free query: the same cursor,
/// keyboard and copy as a table's, without editing, sorting or filters.
/// `result_id` must change with each result (its columns are different).
/// The only action it can return is `CopyRows`.
pub fn show_read_only(
    ui: &mut egui::Ui,
    result_id: u64,
    data: &QueryResult,
    sel: &mut GridSelection,
) -> GridAction {
    let no_deletes = BTreeSet::new();
    let grid = Grid {
        id: ui.make_persistent_id(("result_grid", result_id)),
        data,
        column_info: &[],
        sorts: &[],
        pending_edits: &[],
        pending_deletes: &no_deletes,
        pending_inserts: &[],
        editable: false,
        keyed: false,
        queryable: false,
        engine: Engine::default(),
        pager: None,
    };
    let mut editing = None;
    show_grid(ui, &grid, sel, &mut editing).action
}

/// Copies rows `rows` of `data` to the clipboard in format `fmt`.
/// `table` is the name the INSERTs use. Returns how many it copied.
pub fn copy_rows(
    ctx: &egui::Context,
    data: &QueryResult,
    rows: &BTreeSet<usize>,
    fmt: &CopyFormat,
    table: Option<&str>,
    engine: Engine,
) -> usize {
    let rows: Vec<&Vec<Option<String>>> = rows.iter().filter_map(|&i| data.rows.get(i)).collect();
    if !rows.is_empty() {
        ctx.copy_text(format_rows_for_copy(
            &data.columns,
            &rows,
            fmt,
            table,
            engine,
        ));
    }
    rows.len()
}

/// Keyboard and table of a grid, whether a table's or a read-only one.
fn show_grid(
    ui: &mut egui::Ui,
    grid: &Grid,
    sel: &mut GridSelection,
    editing: &mut Option<EditingCell>,
) -> table::TableOut {
    sel.clamp(grid.existing(), grid.pending_inserts.len(), grid.cols());
    if editing.as_ref().is_some_and(|e| !grid.can_edit(e.cell)) {
        *editing = None;
    }

    let focus_id = grid.focus_id();
    sel.focus_id = Some(focus_id);
    let focused = ui.memory(|m| m.has_focus(focus_id));
    if focused {
        // Tab and the arrows move the grid cursor, not egui's focus
        let filter = egui::EventFilter {
            tab: true,
            horizontal_arrows: true,
            vertical_arrows: true,
            escape: false,
        };
        ui.memory_mut(|m| m.set_focus_lock_filter(focus_id, filter));
    }

    let key_action = if focused && editing.is_none() {
        keys::handle(ui, grid, sel, editing)
    } else {
        GridAction::None
    };

    let mut out = table::show(ui, grid, sel, editing, focused);
    if !matches!(key_action, GridAction::None) {
        out.action = key_action;
    }
    out
}

/// What the grid's information bar shows.
struct InfoBar<'a> {
    /// "db.table".
    name: String,
    data: &'a QueryResult,
    sorts: &'a [SortSpec],
    busy: bool,
    pager: Pager,
    keyed: bool,
    kind: TableKind,
}

/// Table name, row count, new row button and the active sort order.
/// Returns what the user asks for from it: insert a row or remove columns
/// from the sort order.
fn info_bar(ui: &mut egui::Ui, bar: &InfoBar) -> GridAction {
    let InfoBar {
        data,
        sorts,
        busy,
        pager,
        ..
    } = *bar;
    let mut action = GridAction::None;
    let p = theme::get(ui);

    ui.horizontal_wrapped(|ui| {
        ui.strong(&bar.name);
        if pager.paged() {
            // The table does not fit in one page: which ones are shown and how to move
            let first = pager.page * pager.page_size + 1;
            let rows = if data.rows.is_empty() {
                "— ninguna fila en esta página".to_string()
            } else {
                format!("— filas {first}–{}", first + data.rows.len() - 1)
            };
            ui.label(egui::RichText::new(rows).color(p.text_muted))
                .on_hover_text(format!(
                    "Se piden {} filas por página (se cambia en Ajustes).",
                    pager.page_size
                ));
            let mut go = |ui: &mut egui::Ui, label: &str, hint: &str, page: Option<usize>| {
                let button = ui
                    .add_enabled(!busy && page.is_some(), egui::Button::new(label).small())
                    .on_hover_text(hint);
                if button.clicked()
                    && let Some(page) = page
                {
                    action = GridAction::PageChanged(page);
                }
            };
            go(
                ui,
                "‹ Anterior",
                "Página anterior  Alt+Re Pág",
                pager.previous(),
            );
            go(
                ui,
                "Siguiente ›",
                "Página siguiente  Alt+Av Pág",
                pager.next(),
            );
        } else {
            ui.label(
                egui::RichText::new(format!("— {} filas", data.rows.len())).color(p.text_muted),
            );
        }
        if busy {
            ui.separator();
            ui.spinner();
            ui.label("Cargando…");
        } else if bar.kind.is_view() {
            // No data of its own: neither new rows nor the key warning
            ui.separator();
            ui.label(
                egui::RichText::new(format!("{VIEW} Vista · solo lectura")).color(p.text_muted),
            )
            .on_hover_text(
                "Los datos de una vista son los de sus tablas: se editan en ellas.\n\
                     Su definición se abre desde el menú de la vista en el árbol.",
            );
        } else if !data.columns.is_empty()
            && ui
                .small_button(format!("{PLUS} Fila"))
                .on_hover_text(format!("Insertar una fila  {CMD}Ins"))
                .clicked()
        {
            // In view even when there are no rows to open the menu on
            action = GridAction::InsertRow;
        }
        if !bar.keyed && !bar.kind.is_view() {
            // Before someone edits only to find out when saving
            ui.separator();
            ui.label(egui::RichText::new("⚠ Sin clave primaria").color(p.warning))
                .on_hover_text(
                    "Sin clave primaria no hay forma segura de identificar una fila:\n\
                     las que ya existen no se pueden editar ni eliminar desde aquí.\n\
                     Sí se pueden añadir filas nuevas.",
                );
        }

        if sorts.is_empty() {
            return;
        }
        ui.separator();
        ui.label("Orden:");
        for (i, spec) in sorts.iter().enumerate() {
            let arrow = if spec.dir == SortDir::Asc {
                "↑"
            } else {
                "↓"
            };
            egui::Frame::NONE
                .fill(p.bg_surface)
                .stroke(egui::Stroke::new(1.0, p.border_subtle))
                .corner_radius(egui::CornerRadius::same(theme::RADIUS_LG))
                .inner_margin(egui::Margin::symmetric(6, 2))
                .show(ui, |ui| {
                    ui.label(format!("{} {arrow}", spec.col_name));
                    if ui
                        .small_button("x")
                        .on_hover_text(format!("Quitar '{}'", spec.col_name))
                        .clicked()
                    {
                        let mut specs = sorts.to_vec();
                        specs.remove(i);
                        action = GridAction::SortChanged(specs);
                    }
                });
        }
        if sorts.len() > 1
            && ui
                .small_button("Limpiar todo")
                .on_hover_text("Eliminar toda la ordenación")
                .clicked()
        {
            action = GridAction::SortChanged(Vec::new());
        }
    });

    action
}

/// Sort order after clicking the header of `col`: ascending, descending
/// and off, in that order.
fn toggle_sort(sorts: &[SortSpec], col: usize, name: &str) -> Vec<SortSpec> {
    let mut specs = sorts.to_vec();
    match specs.iter().position(|s| s.col_idx == col) {
        None => specs.push(SortSpec {
            col_idx: col,
            col_name: name.to_string(),
            dir: SortDir::Asc,
        }),
        Some(i) if specs[i].dir == SortDir::Asc => specs[i].dir = SortDir::Desc,
        Some(i) => {
            specs.remove(i);
        }
    }
    specs
}

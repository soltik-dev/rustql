//! Context menu of a cell: copy, edit, filter by its value and the
//! operations on its row.

use eframe::egui;

use super::{CellRef, EditingCell, Grid, GridAction, GridSelection, RowRef, Shown};
use crate::i18n::t;
use crate::settings::CopyFormat;
use crate::ui::icons::{CLOCK, CLONE, COPY, EDIT, PLUS, SEARCH, TRASH, UNDO};
use crate::ui::{CMD, ellipsize, theme};

/// Menu option: returns true (and closes the menu) when clicked.
fn item(ui: &mut egui::Ui, label: impl Into<egui::WidgetText>) -> bool {
    let clicked = ui.button(label).clicked();
    if clicked {
        ui.close();
    }
    clicked
}

/// Detects whether a text looks like a date (YYYY-MM-DD…).
fn looks_like_datetime(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b[..4].iter().all(u8::is_ascii_digit)
        && b[5..7].iter().all(u8::is_ascii_digit)
        && b[8..10].iter().all(u8::is_ascii_digit)
}

/// Current local date and time as "YYYY-MM-DD HH:MM:SS".
fn current_datetime() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

/// Whether the column stores dates, by its type or, without structure, by
/// the value.
fn is_datetime(g: &Grid, col: usize, shown: Shown) -> bool {
    let by_type = g.column_info.get(col).is_some_and(|c| {
        let ty = c.col_type.to_ascii_lowercase();
        ty.starts_with("date") || ty.starts_with("timestamp")
    });
    by_type || matches!(shown, Shown::Value(v) if looks_like_datetime(v))
}

/// Content of the menu of `cell`. Returns the chosen action, if any; the
/// cell edit one is applied right here.
pub(super) fn cell(
    ui: &mut egui::Ui,
    g: &Grid,
    sel: &mut GridSelection,
    editing: &mut Option<EditingCell>,
    cell: CellRef,
) -> Option<GridAction> {
    let mut action = None;
    let shown = g.shown(cell);
    let col = cell.col;
    let edited = |value: Option<String>| GridAction::CellEdited {
        row: cell.row,
        col_idx: col,
        value,
    };

    // ── Copy ─────────────────────────────────────────────────────────────
    if sel.rows.is_empty() {
        if item(ui, format!("{COPY}  {}", t!("menu-copy-value"))) {
            let text = match shown {
                Shown::Value(v) => v,
                _ => "",
            };
            ui.ctx().copy_text(text.to_string());
        }
    } else {
        for (label, fmt) in [
            (t!("menu-copy-csv"), CopyFormat::Csv),
            (t!("menu-copy-tsv"), CopyFormat::Tsv),
            (t!("menu-copy-inserts"), CopyFormat::SqlInserts),
        ] {
            if item(ui, format!("{COPY}  {label}")) {
                action = Some(GridAction::CopyRows(Some(fmt)));
            }
        }
    }

    // ── Cell value ───────────────────────────────────────────────────────
    if g.can_edit(cell) {
        ui.separator();
        if item(ui, format!("{EDIT}  {}  F2", t!("menu-edit-cell"))) {
            sel.select_cell(cell);
            *editing = Some(EditingCell::new(cell, g.edit_text(cell)));
        }
        if g.nullable(col)
            && shown != Shown::Null
            && item(ui, format!("∅  {}  {CMD}N", t!("menu-set-null")))
        {
            action = Some(edited(None));
        }
        if shown != Shown::Value("") && item(ui, format!("\"\"  {}", t!("menu-set-empty"))) {
            action = Some(edited(Some(String::new())));
        }
        if is_datetime(g, col, shown) && item(ui, format!("{CLOCK}  {}", t!("menu-now"))) {
            action = Some(edited(Some(current_datetime())));
        }
        if let RowRef::Insert(insert_idx) = cell.row
            && shown != Shown::Default
            && item(
                ui,
                format!("{UNDO}  {}  {}", t!("menu-default-value"), t!("key-delete")),
            )
        {
            action = Some(GridAction::CellSetDefault {
                insert_idx,
                col_idx: col,
            });
        }
        // Going back to the saved value removes the pending change
        if let RowRef::Existing(row) = cell.row
            && g.pending_edit(row, col).is_some()
            && item(ui, format!("{UNDO}  {}", t!("menu-undo-change")))
        {
            action = Some(edited(g.stored(row, col).map(str::to_string)));
        }
    }

    // ── Filter by the saved value ────────────────────────────────────────
    if g.queryable
        && let RowRef::Existing(row) = cell.row
    {
        ui.separator();
        let col_name = &g.data.columns[col];
        let esc_col = g.engine.quote_ident(col_name);
        let filter = |sql_fragment: String| Some(GridAction::FilterBy { sql_fragment });
        if let Some(value) = g.stored(row, col) {
            let display = ellipsize(value, 22);
            let literal = g.engine.literal(value);
            if item(ui, format!("{SEARCH}  = '{display}'")) {
                action = filter(format!("{esc_col} = {literal}"));
            }
            if item(ui, format!("{SEARCH}  ≠ '{display}'")) {
                action = filter(format!("{esc_col} != {literal}"));
            }
            if item(ui, format!("{SEARCH}  LIKE '%{display}%'")) {
                action = filter(g.engine.like(col_name, &format!("%{value}%")));
            }
        }
        if item(ui, format!("{SEARCH}  IS NULL ({col_name})")) {
            action = filter(format!("{esc_col} IS NULL"));
        }
        if item(ui, format!("{SEARCH}  IS NOT NULL ({col_name})")) {
            action = filter(format!("{esc_col} IS NOT NULL"));
        }
        if item(ui, format!("{SEARCH}  {}", t!("menu-filter-clipboard"))) {
            action = Some(GridAction::FilterByClipboard {
                col_name: col_name.clone(),
            });
        }
        if item(ui, format!("{SEARCH}  {}", t!("menu-filter-text"))) {
            action = Some(GridAction::FilterUserInput {
                col_name: col_name.clone(),
            });
        }
    }

    // ── Row ──────────────────────────────────────────────────────────────
    if g.editable {
        ui.separator();
        if item(ui, format!("{PLUS}  {}  {CMD}Ins", t!("menu-insert-row"))) {
            action = Some(GridAction::InsertRow);
        }
        match cell.row {
            RowRef::Existing(row_idx) => {
                if item(ui, format!("{CLONE}  {}  {CMD}D", t!("menu-clone-row"))) {
                    action = Some(GridAction::CloneRow { row_idx });
                }
                // Inside a row selection, all of them are deleted
                let rows: Vec<usize> = if sel.rows.contains(&row_idx) {
                    sel.rows.iter().copied().collect()
                } else {
                    vec![row_idx]
                };
                let label = if g.is_deleted(cell.row) {
                    format!("{UNDO}  {}", t!("menu-undelete"))
                } else {
                    format!(
                        "{TRASH}  {}  {CMD}{}",
                        t!("menu-delete-rows", count = rows.len()),
                        t!("key-delete")
                    )
                };
                // Without a primary key there would be no way to know which row to delete
                if g.can_change_row(cell.row) && theme::danger_button(ui, label).clicked() {
                    action = Some(GridAction::ToggleDeleteRows(rows));
                    ui.close();
                }
            }
            RowRef::Insert(insert_idx) => {
                if item(
                    ui,
                    format!(
                        "{TRASH}  {}  {CMD}{}",
                        t!("grid-remove-new-row"),
                        t!("key-delete")
                    ),
                ) {
                    action = Some(GridAction::RemoveInsertRow { insert_idx });
                }
            }
        }
    }

    action
}

#[cfg(test)]
mod tests {
    use super::looks_like_datetime;

    #[test]
    fn reconoce_fechas() {
        assert!(looks_like_datetime("2026-11-10"));
        assert!(looks_like_datetime("2026-11-10 12:30:00"));
        assert!(!looks_like_datetime("10/11/2026"));
        assert!(!looks_like_datetime("2026-11"));
    }
}

//! Grid keyboard: move the cursor, start editing and the shortcuts on the
//! cursor's cell or row. Only handled with the focus on the grid and no
//! cell being edited.

use eframe::egui;

use super::{CellRef, EditingCell, Grid, GridAction, GridSelection, RowRef, Shown};

/// Cursor movement.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Move {
    Up,
    Down,
    Left,
    Right,
    PageUp,
    PageDown,
    /// First or last column of the row.
    RowStart,
    RowEnd,
    /// First or last cell of the grid.
    First,
    Last,
    /// Next or previous in reading order (Tab).
    Next,
    Prev,
}

fn read_move(i: &egui::InputState) -> Option<Move> {
    use egui::Key;
    let m = i.modifiers;
    if m.alt {
        return None;
    }
    // Ctrl+Tab switches tabs: it does not belong to the grid
    if i.key_pressed(Key::Tab) && !m.command {
        return Some(if m.shift { Move::Prev } else { Move::Next });
    }
    let keys: &[(Key, Move)] = if m.command {
        &[(Key::Home, Move::First), (Key::End, Move::Last)]
    } else {
        &[
            (Key::ArrowUp, Move::Up),
            (Key::ArrowDown, Move::Down),
            (Key::ArrowLeft, Move::Left),
            (Key::ArrowRight, Move::Right),
            (Key::PageUp, Move::PageUp),
            (Key::PageDown, Move::PageDown),
            (Key::Home, Move::RowStart),
            (Key::End, Move::RowEnd),
        ]
    };
    keys.iter()
        .find(|(key, _)| i.key_pressed(*key))
        .map(|&(_, mv)| mv)
}

/// Position (displayed row, column) after moving the cursor from `from` in
/// a grid of `rows` × `cols`, without leaving it. `page` is the number of
/// rows that fit on screen.
fn apply_move(
    mv: Move,
    from: (usize, usize),
    rows: usize,
    cols: usize,
    page: usize,
) -> (usize, usize) {
    let (row, col) = from;
    let (last_row, last_col) = (rows - 1, cols - 1);
    match mv {
        Move::Up => (row.saturating_sub(1), col),
        Move::Down => ((row + 1).min(last_row), col),
        Move::Left => (row, col.saturating_sub(1)),
        Move::Right => (row, (col + 1).min(last_col)),
        Move::PageUp => (row.saturating_sub(page), col),
        Move::PageDown => ((row + page).min(last_row), col),
        Move::RowStart => (row, 0),
        Move::RowEnd => (row, last_col),
        Move::First => (0, 0),
        Move::Last => (last_row, last_col),
        Move::Next if col < last_col => (row, col + 1),
        Move::Next if row < last_row => (row + 1, 0),
        Move::Next => (row, col),
        Move::Prev if col > 0 => (row, col - 1),
        Move::Prev if row > 0 => (row - 1, last_col),
        Move::Prev => (row, col),
    }
}

pub(super) fn handle(
    ui: &egui::Ui,
    g: &Grid,
    sel: &mut GridSelection,
    editing: &mut Option<EditingCell>,
) -> GridAction {
    let (rows, cols) = (g.total_rows(), g.cols());
    let ctrl = ui.input(|i| i.modifiers.command);
    let pressed = |key: egui::Key| ui.input(|i| i.key_pressed(key));

    // Insert needs no cursor: it is the way to start on an empty table
    if g.editable && ctrl && pressed(egui::Key::Insert) {
        return GridAction::InsertRow;
    }
    // Alt+Page Down and Alt+Page Up: next and previous page of the table
    if let Some(pager) = g.pager
        && ui.input(|i| i.modifiers.alt)
    {
        let target = if pressed(egui::Key::PageDown) {
            pager.next()
        } else if pressed(egui::Key::PageUp) {
            pager.previous()
        } else {
            None
        };
        if let Some(page) = target {
            return GridAction::PageChanged(page);
        }
    }
    if rows == 0 || cols == 0 {
        return GridAction::None;
    }

    if let Some(mv) = ui.input(read_move) {
        let target = match sel.cell {
            Some(cell) => {
                // Rows that fit on screen, for Page Up and Page Down
                let page = sel
                    .layout
                    .as_ref()
                    .map_or(10, |l| (l.size.y / l.row_pitch) as usize)
                    .max(1);
                let from = (cell.row.display(g.existing()), cell.col);
                apply_move(mv, from, rows, cols, page)
            }
            // Without a cursor, any movement puts it on the first cell
            None => (0, 0),
        };
        sel.move_to(CellRef {
            row: RowRef::from_display(target.0, g.existing()),
            col: target.1,
        });
        return GridAction::None;
    }

    let Some(cell) = sel.cell else {
        return GridAction::None;
    };
    let shown = g.shown(cell);

    // Copy: the selected rows or, if there are none, the cell's value
    let copy = ui.input(|i| {
        i.events.iter().any(|e| matches!(e, egui::Event::Copy))
            || (i.modifiers.command && i.key_pressed(egui::Key::C))
    });
    if copy {
        if !sel.rows.is_empty() {
            return GridAction::CopyRows(None);
        }
        let text = match shown {
            Shown::Value(v) => v,
            _ => "",
        };
        ui.ctx().copy_text(text.to_string());
        return GridAction::None;
    }

    if pressed(egui::Key::Escape) {
        sel.deselect();
        return GridAction::None;
    }

    if g.editable {
        // Ctrl+Delete: delete the row (or the selected ones); a new row is
        // simply removed
        if ctrl && pressed(egui::Key::Delete) && g.can_change_row(cell.row) {
            return match cell.row {
                RowRef::Existing(row) if sel.rows.is_empty() => {
                    GridAction::ToggleDeleteRows(vec![row])
                }
                RowRef::Existing(_) => {
                    GridAction::ToggleDeleteRows(sel.rows.iter().copied().collect())
                }
                RowRef::Insert(insert_idx) => GridAction::RemoveInsertRow { insert_idx },
            };
        }
        if ctrl
            && pressed(egui::Key::D)
            && let RowRef::Existing(row_idx) = cell.row
        {
            return GridAction::CloneRow { row_idx };
        }
    }

    if !g.can_edit(cell) {
        return GridAction::None;
    }
    let edited = |value: Option<String>| GridAction::CellEdited {
        row: cell.row,
        col_idx: cell.col,
        value,
    };

    if ctrl && pressed(egui::Key::N) {
        return edited(None);
    }
    // Delete empties the cell: NULL if the column allows it; in a new row,
    // back to the default value
    if !ctrl && pressed(egui::Key::Delete) {
        return match cell.row {
            RowRef::Insert(insert_idx) => GridAction::CellSetDefault {
                insert_idx,
                col_idx: cell.col,
            },
            RowRef::Existing(_) if g.nullable(cell.col) => edited(None),
            RowRef::Existing(_) => edited(Some(String::new())),
        };
    }

    let pasted = ui.input(|i| {
        i.events.iter().find_map(|e| match e {
            egui::Event::Paste(text) => Some(text.clone()),
            _ => None,
        })
    });
    if let Some(text) = pasted {
        return edited(Some(text));
    }

    // Enter or F2 edit what is there; typing replaces it
    let enter = ui.input(|i| i.modifiers.is_none() && i.key_pressed(egui::Key::Enter));
    if enter || pressed(egui::Key::F2) {
        *editing = Some(EditingCell::new(cell, g.edit_text(cell)));
        return GridAction::None;
    }
    let typed = ui.input(|i| {
        if i.modifiers.command || i.modifiers.alt {
            return None;
        }
        i.events.iter().find_map(|e| match e {
            egui::Event::Text(text) if !text.is_empty() => Some(text.clone()),
            _ => None,
        })
    });
    if let Some(text) = typed {
        *editing = Some(EditingCell::replacing(cell, g.edit_text(cell), text));
    }
    GridAction::None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_cursor_no_se_sale_del_grid() {
        // 3 rows × 2 columns, 10 rows per page
        let mv = |m, from| apply_move(m, from, 3, 2, 10);
        assert_eq!(mv(Move::Up, (0, 1)), (0, 1));
        assert_eq!(mv(Move::Down, (2, 0)), (2, 0));
        assert_eq!(mv(Move::Left, (1, 0)), (1, 0));
        assert_eq!(mv(Move::Right, (1, 1)), (1, 1));
        assert_eq!(mv(Move::PageDown, (0, 1)), (2, 1));
        assert_eq!(mv(Move::PageUp, (2, 1)), (0, 1));
        assert_eq!(mv(Move::RowEnd, (1, 0)), (1, 1));
        assert_eq!(mv(Move::Last, (0, 0)), (2, 1));
        assert_eq!(mv(Move::First, (2, 1)), (0, 0));
    }

    #[test]
    fn tab_pasa_de_fila_al_llegar_al_final() {
        let mv = |m, from| apply_move(m, from, 3, 2, 10);
        assert_eq!(mv(Move::Next, (0, 0)), (0, 1));
        assert_eq!(mv(Move::Next, (0, 1)), (1, 0));
        assert_eq!(mv(Move::Next, (2, 1)), (2, 1), "última celda");
        assert_eq!(mv(Move::Prev, (1, 0)), (0, 1));
        assert_eq!(mv(Move::Prev, (0, 0)), (0, 0), "primera celda");
    }
}

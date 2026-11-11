//! Teclado del grid: mover el cursor, empezar a editar y los atajos sobre la
//! celda o la fila del cursor. Solo se atiende con el foco en el grid y sin
//! ninguna celda en edición.

use eframe::egui;

use super::{CellRef, EditingCell, Grid, GridAction, GridSelection, RowRef, Shown};

/// Movimiento del cursor.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Move {
    Up,
    Down,
    Left,
    Right,
    PageUp,
    PageDown,
    /// Primera o última columna de la fila.
    RowStart,
    RowEnd,
    /// Primera o última celda del grid.
    First,
    Last,
    /// Siguiente o anterior en orden de lectura (Tab).
    Next,
    Prev,
}

fn read_move(i: &egui::InputState) -> Option<Move> {
    use egui::Key;
    let m = i.modifiers;
    if m.alt {
        return None;
    }
    // Ctrl+Tab cambia de pestaña: no es del grid
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

/// Posición (fila mostrada, columna) tras mover el cursor desde `from` en un
/// grid de `rows` × `cols`, sin salirse de él. `page` son las filas que
/// caben en pantalla.
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

    // Insertar no necesita cursor: es la forma de empezar en una tabla vacía
    if g.editable && ctrl && pressed(egui::Key::Insert) {
        return GridAction::InsertRow;
    }
    // Alt+Av Pág y Alt+Re Pág: página siguiente y anterior de la tabla
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
                // Filas que caben en pantalla, para Re Pág y Av Pág
                let page = sel
                    .layout
                    .as_ref()
                    .map_or(10, |l| (l.size.y / l.row_pitch) as usize)
                    .max(1);
                let from = (cell.row.display(g.existing()), cell.col);
                apply_move(mv, from, rows, cols, page)
            }
            // Sin cursor, cualquier movimiento lo pone en la primera celda
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

    // Copiar: las filas seleccionadas o, si no hay, el valor de la celda
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
        // Ctrl+Supr: eliminar la fila (o las seleccionadas); una fila nueva
        // simplemente se quita
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
    // Supr vacía la celda: NULL si la columna lo admite; en una fila nueva,
    // de vuelta al valor por defecto
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

    // Enter o F2 editan lo que hay; escribir lo sustituye
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
        // 3 filas × 2 columnas, 10 filas por página
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

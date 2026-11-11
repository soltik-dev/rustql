//! Menú contextual de una celda: copiar, editar, filtrar por su valor y las
//! operaciones sobre su fila.

use eframe::egui;

use super::{CellRef, EditingCell, Grid, GridAction, GridSelection, RowRef, Shown};
use crate::db::{escape_sql, quote_ident};
use crate::settings::CopyFormat;
use crate::ui::icons::{CLOCK, CLONE, COPY, EDIT, PLUS, SEARCH, TRASH, UNDO};
use crate::ui::{CMD, ellipsize, theme};

/// Opción del menú: devuelve true (y cierra el menú) al pulsarla.
fn item(ui: &mut egui::Ui, label: impl Into<egui::WidgetText>) -> bool {
    let clicked = ui.button(label).clicked();
    if clicked {
        ui.close();
    }
    clicked
}

/// Detecta si un texto parece una fecha (YYYY-MM-DD…).
fn looks_like_datetime(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b[..4].iter().all(u8::is_ascii_digit)
        && b[5..7].iter().all(u8::is_ascii_digit)
        && b[8..10].iter().all(u8::is_ascii_digit)
}

/// Fecha y hora local actual como «YYYY-MM-DD HH:MM:SS».
fn current_datetime() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

/// Si la columna guarda fechas, por su tipo o, sin estructura, por el valor.
fn is_datetime(g: &Grid, col: usize, shown: Shown) -> bool {
    let by_type = g.column_info.get(col).is_some_and(|c| {
        let ty = c.col_type.to_ascii_lowercase();
        ty.starts_with("date") || ty.starts_with("timestamp")
    });
    by_type || matches!(shown, Shown::Value(v) if looks_like_datetime(v))
}

/// Contenido del menú de `cell`. Devuelve la acción elegida, si la hay; la
/// de editar la celda se aplica aquí mismo.
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

    // ── Copiar ───────────────────────────────────────────────────────────
    if sel.rows.is_empty() {
        if item(ui, format!("{COPY}  Copiar valor")) {
            let text = match shown {
                Shown::Value(v) => v,
                _ => "",
            };
            ui.ctx().copy_text(text.to_string());
        }
    } else {
        for (label, fmt) in [
            ("Copiar en CSV", CopyFormat::Csv),
            ("Copiar en TSV", CopyFormat::Tsv),
            ("Copiar SQL Insert", CopyFormat::SqlInserts),
        ] {
            if item(ui, format!("{COPY}  {label}")) {
                action = Some(GridAction::CopyRows(Some(fmt)));
            }
        }
    }

    // ── Valor de la celda ────────────────────────────────────────────────
    if g.can_edit(cell) {
        ui.separator();
        if item(ui, format!("{EDIT}  Editar celda  F2")) {
            sel.select_cell(cell);
            *editing = Some(EditingCell::new(cell, g.edit_text(cell)));
        }
        if g.nullable(col)
            && shown != Shown::Null
            && item(ui, format!("∅  Establecer NULL  {CMD}N"))
        {
            action = Some(edited(None));
        }
        if shown != Shown::Value("") && item(ui, "\"\"  Establecer vacío") {
            action = Some(edited(Some(String::new())));
        }
        if is_datetime(g, col, shown) && item(ui, format!("{CLOCK}  Fecha/hora actual")) {
            action = Some(edited(Some(current_datetime())));
        }
        if let RowRef::Insert(insert_idx) = cell.row
            && shown != Shown::Default
            && item(ui, format!("{UNDO}  Valor por defecto  Supr"))
        {
            action = Some(GridAction::CellSetDefault {
                insert_idx,
                col_idx: col,
            });
        }
        // Volver al valor guardado quita el cambio pendiente
        if let RowRef::Existing(row) = cell.row
            && g.pending_edit(row, col).is_some()
            && item(ui, format!("{UNDO}  Deshacer el cambio"))
        {
            action = Some(edited(g.stored(row, col).map(str::to_string)));
        }
    }

    // ── Filtrar por el valor guardado ────────────────────────────────────
    if g.queryable
        && let RowRef::Existing(row) = cell.row
    {
        ui.separator();
        let col_name = &g.data.columns[col];
        let esc_col = quote_ident(col_name);
        let filter = |sql_fragment: String| Some(GridAction::FilterBy { sql_fragment });
        if let Some(value) = g.stored(row, col) {
            let display = ellipsize(value, 22);
            let escaped = escape_sql(value);
            if item(ui, format!("{SEARCH}  = '{display}'")) {
                action = filter(format!("{esc_col} = '{escaped}'"));
            }
            if item(ui, format!("{SEARCH}  ≠ '{display}'")) {
                action = filter(format!("{esc_col} != '{escaped}'"));
            }
            if item(ui, format!("{SEARCH}  LIKE '%{display}%'")) {
                action = filter(format!("{esc_col} LIKE '%{escaped}%'"));
            }
        }
        if item(ui, format!("{SEARCH}  IS NULL ({col_name})")) {
            action = filter(format!("{esc_col} IS NULL"));
        }
        if item(ui, format!("{SEARCH}  IS NOT NULL ({col_name})")) {
            action = filter(format!("{esc_col} IS NOT NULL"));
        }
        if item(ui, format!("{SEARCH}  Por portapapeles")) {
            action = Some(GridAction::FilterByClipboard {
                col_name: col_name.clone(),
            });
        }
        if item(ui, format!("{SEARCH}  Por texto...")) {
            action = Some(GridAction::FilterUserInput {
                col_name: col_name.clone(),
            });
        }
    }

    // ── Fila ─────────────────────────────────────────────────────────────
    if g.editable {
        ui.separator();
        if item(ui, format!("{PLUS}  Insertar fila  {CMD}Ins")) {
            action = Some(GridAction::InsertRow);
        }
        match cell.row {
            RowRef::Existing(row_idx) => {
                if item(ui, format!("{CLONE}  Clonar fila  {CMD}D")) {
                    action = Some(GridAction::CloneRow { row_idx });
                }
                // Dentro de una selección de filas, se eliminan todas
                let rows: Vec<usize> = if sel.rows.contains(&row_idx) {
                    sel.rows.iter().copied().collect()
                } else {
                    vec![row_idx]
                };
                let label = if g.is_deleted(cell.row) {
                    format!("{UNDO}  Desmarcar eliminación")
                } else if rows.len() > 1 {
                    format!("{TRASH}  Eliminar {} filas  {CMD}Supr", rows.len())
                } else {
                    format!("{TRASH}  Eliminar fila  {CMD}Supr")
                };
                // Sin clave primaria no se sabría qué fila eliminar
                if g.can_change_row(cell.row) && theme::danger_button(ui, label).clicked() {
                    action = Some(GridAction::ToggleDeleteRows(rows));
                    ui.close();
                }
            }
            RowRef::Insert(insert_idx) => {
                if item(ui, format!("{TRASH}  Quitar la fila nueva  {CMD}Supr")) {
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

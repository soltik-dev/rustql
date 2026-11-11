//! Grid de datos de la tabla abierta, con edición de celdas.
//!
//! El grid es dueño de su estado de vista (cursor, filas seleccionadas, celda
//! en edición, desplazamiento) y lo cambia él mismo. Lo que afecta a los
//! datos —editar, insertar, eliminar, ordenar, filtrar— lo devuelve como
//! `GridAction` y lo aplica la sesión.

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
use crate::db::{ColumnInfo, QueryResult};
use crate::settings::CopyFormat;
use crate::ui::icons::PLUS;
use crate::ui::{CMD, theme};

// ── Lo que el grid dibuja ────────────────────────────────────────────────────

/// Datos, estructura y cambios pendientes con los que se dibuja el grid.
struct Grid<'a> {
    /// Identifica el grid: de él salen el id del foco y los del scroll.
    id: egui::Id,
    data: &'a QueryResult,
    column_info: &'a [ColumnInfo],
    sorts: &'a [SortSpec],
    pending_edits: &'a [PendingEdit],
    pending_deletes: &'a BTreeSet<usize>,
    pending_inserts: &'a [PendingInsert],
    /// Se pueden editar celdas e insertar o eliminar filas.
    editable: bool,
    /// Los datos son los de una tabla: se puede ordenar y filtrar relanzando
    /// su consulta.
    queryable: bool,
}

/// Lo que muestra una celda.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Shown<'a> {
    Value(&'a str),
    Null,
    /// Fila nueva: el servidor pondrá el valor por defecto.
    Default,
    /// Fila nueva: columna AUTO_INCREMENT.
    Auto,
}

impl<'a> Grid<'a> {
    fn focus_id(&self) -> egui::Id {
        self.id.with("focus")
    }

    /// Filas de los datos; las nuevas van después.
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

    /// Valor guardado en el servidor (sin los cambios pendientes).
    fn stored(&self, row: usize, col: usize) -> Option<&'a str> {
        self.data.rows.get(row)?.get(col)?.as_deref()
    }

    /// Lo que se ve en la celda: el cambio pendiente si lo hay y, si no, el
    /// dato.
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

    /// Texto con el que arranca el editor de la celda: NULL y DEFAULT se
    /// editan como texto vacío.
    fn edit_text(&self, cell: CellRef) -> String {
        match self.shown(cell) {
            Shown::Value(v) => v.to_string(),
            _ => String::new(),
        }
    }

    /// No se editan las filas marcadas para eliminar ni las columnas
    /// AUTO_INCREMENT de una fila nueva.
    fn can_edit(&self, cell: CellRef) -> bool {
        self.editable
            && cell.col < self.cols()
            && !self.is_deleted(cell.row)
            && self.shown(cell) != Shown::Auto
    }

    /// Si la columna admite NULL. Sin su estructura (aún cargando) se supone
    /// que sí: el servidor lo rechazará al guardar si no.
    fn nullable(&self, col: usize) -> bool {
        self.column_info.get(col).is_none_or(|c| c.is_nullable)
    }

    /// Siguiente celda editable en orden de lectura (o la anterior): la que
    /// toca al pulsar Tab mientras se edita.
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

// ── Grid de la tabla abierta ─────────────────────────────────────────────────

/// Muestra los datos de la tabla abierta. Devuelve lo que el usuario pide
/// hacer con ellos en este frame. Con `busy` (cargando o guardando) no se
/// puede editar. `row_limit` es el LIMIT con el que se pidieron los datos.
pub fn show(
    ui: &mut egui::Ui,
    selected_table: Option<(&str, &str)>,
    view: &mut TableViewState,
    column_info: &[ColumnInfo],
    busy: bool,
    row_limit: usize,
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

    let bar_action = info_bar(ui, db, table, data, &view.sort_specs, busy, row_limit);
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
        editable: !busy,
        queryable: true,
    };
    let out = show_grid(ui, &grid, &mut view.selection, &mut view.editing_cell);

    // La edición tiene prioridad sobre la ordenación
    match (out.action, out.clicked_col) {
        (GridAction::None, Some(col)) => {
            GridAction::SortChanged(toggle_sort(&view.sort_specs, col, &data.columns[col]))
        }
        (GridAction::None, None) => bar_action,
        (action, _) => action,
    }
}

/// Grid de solo lectura para el resultado de una consulta libre: el mismo
/// cursor, teclado y copia que el de una tabla, sin edición, ordenación ni
/// filtros. `result_id` debe cambiar con cada resultado (sus columnas son
/// otras). La única acción que puede devolver es `CopyRows`.
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
        queryable: false,
    };
    let mut editing = None;
    show_grid(ui, &grid, sel, &mut editing).action
}

/// Copia al portapapeles las filas `rows` de `data` en el formato `fmt`.
/// `table` es el nombre que llevan los INSERT.
pub fn copy_rows(
    ctx: &egui::Context,
    data: &QueryResult,
    rows: &BTreeSet<usize>,
    fmt: &CopyFormat,
    table: Option<&str>,
) {
    let rows: Vec<&Vec<Option<String>>> = rows.iter().filter_map(|&i| data.rows.get(i)).collect();
    if !rows.is_empty() {
        ctx.copy_text(format_rows_for_copy(&data.columns, &rows, fmt, table));
    }
}

/// Teclado y tabla de un grid, sea el de una tabla o uno de solo lectura.
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
        // Tab y las flechas mueven el cursor del grid, no el foco de egui
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

/// Nombre de la tabla, número de filas, botón de fila nueva y la ordenación
/// activa. Devuelve lo que el usuario pide desde ella: insertar una fila o
/// quitar columnas de la ordenación.
fn info_bar(
    ui: &mut egui::Ui,
    db: &str,
    table: &str,
    data: &QueryResult,
    sorts: &[SortSpec],
    busy: bool,
    row_limit: usize,
) -> GridAction {
    let mut action = GridAction::None;
    let p = theme::get(ui);

    ui.horizontal_wrapped(|ui| {
        ui.strong(format!("{db}.{table}"));
        if data.rows.len() >= row_limit {
            // La consulta lleva LIMIT: lo que se ve puede no ser todo
            ui.label(
                egui::RichText::new(format!("— primeras {} filas", data.rows.len()))
                    .color(p.warning),
            )
            .on_hover_text(format!(
                "La tabla puede tener más: solo se piden {row_limit} filas.\n\
                 Filtra para acotar o sube el límite en Ajustes."
            ));
        } else {
            ui.label(
                egui::RichText::new(format!("— {} filas", data.rows.len())).color(p.text_muted),
            );
        }
        if busy {
            ui.separator();
            ui.spinner();
            ui.label("Cargando…");
        } else if !data.columns.is_empty()
            && ui
                .small_button(format!("{PLUS} Fila"))
                .on_hover_text(format!("Insertar una fila  {CMD}Ins"))
                .clicked()
        {
            // A la vista aunque no haya filas sobre las que abrir el menú
            action = GridAction::InsertRow;
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

/// Ordenación tras pulsar la cabecera de `col`: ascendente, descendente y
/// fuera, por ese orden.
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

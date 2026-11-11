//! Estado y tipos del grid: ordenación, cambios pendientes, edición y
//! selección, y las acciones que devuelve.

use std::collections::BTreeSet;

use eframe::egui;
use serde::{Deserialize, Serialize};

use crate::db::QueryResult;
use crate::settings::CopyFormat;

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

/// Identifica una fila del grid: una de los datos o una nueva sin guardar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowRef {
    /// Índice en `QueryResult.rows`.
    Existing(usize),
    /// Índice en `pending_inserts`.
    Insert(usize),
}

impl RowRef {
    /// Fila que ocupa la posición `display` del grid: primero van las
    /// `existing` filas de los datos y, debajo, las nuevas.
    pub fn from_display(display: usize, existing: usize) -> Self {
        if display < existing {
            Self::Existing(display)
        } else {
            Self::Insert(display - existing)
        }
    }

    /// Posición de la fila en el grid (inversa de `from_display`).
    pub fn display(self, existing: usize) -> usize {
        match self {
            Self::Existing(i) => i,
            Self::Insert(i) => existing + i,
        }
    }
}

/// Una celda del grid, de una fila existente o de una nueva.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellRef {
    pub row: RowRef,
    pub col: usize,
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

/// Fila nueva, al final del grid, pendiente de insertar.
#[derive(Debug, Clone)]
pub struct PendingInsert {
    /// Valor por columna; las no tocadas, `Default`.
    pub values: Vec<InsertValue>,
    /// Columnas AUTO_INCREMENT: se muestran como «(AUTO)» y no se editan.
    pub auto_cols: Vec<bool>,
}

/// Celda que se está editando.
#[derive(Debug, Clone)]
pub struct EditingCell {
    pub cell: CellRef,
    /// Texto del editor mientras el usuario escribe.
    pub buffer: String,
    /// Texto que tenía la celda al empezar. Confirmar sin haberlo cambiado
    /// no es una edición: NULL y DEFAULT se editan como texto vacío, y pasar
    /// por ellos con Tab los convertiría en ''.
    pub original: String,
    /// Pedir el foco para el editor en su primer frame.
    pub needs_focus: bool,
}

impl EditingCell {
    /// Editor que arranca con el texto de la celda.
    pub fn new(cell: CellRef, text: String) -> Self {
        Self {
            cell,
            buffer: text.clone(),
            original: text,
            needs_focus: true,
        }
    }

    /// Editor que arranca con lo que el usuario acaba de teclear sobre la
    /// celda, en lugar de su texto (`original`).
    pub fn replacing(cell: CellRef, original: String, typed: String) -> Self {
        Self {
            cell,
            buffer: typed,
            original,
            needs_focus: true,
        }
    }
}

/// Lo que el usuario pide hacer con los datos. La selección, el cursor y la
/// celda en edición son estado del propio grid y los cambia él.
#[derive(Debug, Default)]
pub enum GridAction {
    #[default]
    None,
    /// La ordenación que pide el usuario (aún sin aplicar).
    SortChanged(Vec<SortSpec>),
    /// Valor nuevo de una celda; `None` es NULL.
    CellEdited {
        row: RowRef,
        col_idx: usize,
        value: Option<String>,
    },
    /// Celda de una fila nueva de vuelta a DEFAULT.
    CellSetDefault {
        insert_idx: usize,
        col_idx: usize,
    },
    // Filtrado desde el menú contextual
    FilterBy {
        sql_fragment: String,
    },
    FilterByClipboard {
        col_name: String,
    },
    FilterUserInput {
        col_name: String,
    },
    /// Marca para eliminar las filas que no lo estén y desmarca las que sí.
    ToggleDeleteRows(Vec<usize>),
    InsertRow,
    CloneRow {
        row_idx: usize,
    },
    RemoveInsertRow {
        insert_idx: usize,
    },
    /// Copia las filas seleccionadas; sin formato, en el de los ajustes.
    CopyRows(Option<CopyFormat>),
    /// Ir a otra página de la tabla (0 = la primera).
    PageChanged(usize),
}

// ── Selección ────────────────────────────────────────────────────────────────

/// Cursor y filas seleccionadas del grid, y lo que necesita para llevar la
/// vista hasta el cursor cuando se mueve con el teclado.
#[derive(Debug, Default)]
pub struct GridSelection {
    /// Celda del cursor: la que reciben las teclas.
    pub cell: Option<CellRef>,
    /// Filas existentes seleccionadas enteras (para copiarlas o eliminarlas).
    pub rows: BTreeSet<usize>,
    /// Desde dónde se extiende la selección con Mayús+clic.
    anchor_row: Option<usize>,
    /// Fila en la que empezó el arrastre que selecciona un rango.
    pub(super) drag_from: Option<usize>,
    /// El cursor se movió sin el ratón: hay que desplazar el grid hasta él.
    pub(super) reveal: bool,
    /// Veces que se ha intentado ya (ver `table::show`).
    pub(super) reveal_tries: u8,
    /// Volver arriba (datos nuevos con otro orden o filtro).
    pub(super) scroll_top: bool,
    /// Desplazamiento vertical pedido con la barra del grid.
    pub(super) scroll_to_y: Option<f32>,
    /// Dónde quedó cada cosa en el frame anterior.
    pub(super) layout: Option<GridLayout>,
    /// Id con el que el grid recibe el foco del teclado.
    pub focus_id: Option<egui::Id>,
}

/// Geometría del grid en el último frame dibujado. Las posiciones de las
/// filas se miden sobre lo dibujado en lugar de suponerlas: pedir un
/// desplazamiento unos puntos más allá del máximo real hace que egui lo trate
/// como un rebote y lo devuelva con una animación.
#[derive(Debug, Clone, Default)]
pub(super) struct GridLayout {
    /// Desplazamiento horizontal y vertical.
    pub offset: egui::Vec2,
    /// Tamaño de la zona visible de las filas.
    pub size: egui::Vec2,
    /// Desplazamiento máximo en cada eje.
    pub max_offset: egui::Vec2,
    /// Dónde empieza la fila 0, medido desde el principio del contenido.
    pub row_origin: f32,
    /// Distancia entre el comienzo de una fila y el de la siguiente.
    pub row_pitch: f32,
    pub row_h: f32,
    pub col_widths: Vec<f32>,
    /// Separación entre columnas.
    pub col_gap: f32,
}

impl GridLayout {
    /// Tramo vertical que ocupa la fila mostrada en la posición `display`.
    pub fn row_span(&self, display: usize) -> (f32, f32) {
        let top = self.row_origin + display as f32 * self.row_pitch;
        (top, top + self.row_h)
    }

    /// Tramo horizontal de la columna `col`, si existe.
    pub fn col_span(&self, col: usize) -> Option<(f32, f32)> {
        let width = *self.col_widths.get(col)?;
        let left: f32 = self.col_widths[..col]
            .iter()
            .map(|w| w + self.col_gap)
            .sum();
        Some((left, left + width))
    }

    /// Desplazamiento (x, y) con el que la celda queda a la vista, sin
    /// pasar del máximo; `None` en el eje que no hay que mover.
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
    /// Pone el cursor en `cell` (un clic): deja de haber filas seleccionadas.
    pub fn select_cell(&mut self, cell: CellRef) {
        self.cell = Some(cell);
        self.rows.clear();
        self.anchor_row = None;
    }

    /// Como `select_cell`, y además el grid se desplaza hasta la celda
    /// (teclado, o una fila recién añadida).
    pub fn move_to(&mut self, cell: CellRef) {
        self.select_cell(cell);
        self.reveal = true;
        self.reveal_tries = 0;
    }

    /// Selecciona filas enteras: solo `row`, el rango desde la anterior
    /// (Mayús) o la añade o quita (Ctrl). El cursor queda en la celda pulsada.
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

    /// Quita el cursor y la selección, y vuelve al principio: los datos que
    /// vienen (otro orden, otro filtro) ya no son los mismos.
    pub fn clear(&mut self) {
        self.deselect();
        self.scroll_top = true;
    }

    /// Quita el cursor y la selección (Escape).
    pub fn deselect(&mut self) {
        self.cell = None;
        self.rows.clear();
        self.anchor_row = None;
        self.drag_from = None;
        self.reveal = false;
    }

    /// Ajusta la selección a las filas y columnas que hay ahora: tras
    /// recargar o quitar una fila nueva pueden haber dejado de existir.
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

/// Nuevo desplazamiento con el que el tramo `start..end` queda dentro de una
/// ventana de tamaño `viewport` que ahora empieza en `offset`. `None` si ya
/// se ve entero.
pub(super) fn scroll_into_view(start: f32, end: f32, offset: f32, viewport: f32) -> Option<f32> {
    if start < offset {
        Some(start)
    } else if end > offset + viewport {
        // Si el tramo no cabe, manda su principio
        Some((end - viewport).min(start))
    } else {
        None
    }
}

// ── Estado de la tabla abierta ───────────────────────────────────────────────

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
    pub selection: GridSelection,
    pub query_loading: bool,
    /// Guardando los cambios pendientes: el grid no admite más hasta saber
    /// el resultado.
    pub saving: bool,
    /// Se incrementa con cada consulta de la tabla: los resultados de una
    /// anterior que lleguen tarde se descartan.
    pub data_generation: u64,
    /// Página de la tabla que se muestra (0 = la primera).
    pub page: usize,
    /// Filas por página con las que se pidieron los datos.
    pub page_size: usize,
    /// Hay más filas después de esta página.
    pub has_more: bool,
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
        // Otro Mayús+clic parte del mismo origen
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

        // Al quitar la última fila nueva, el cursor pasa a la anterior
        sel.select_cell(cell(RowRef::Insert(2), 1));
        sel.clamp(3, 2, 2);
        assert_eq!(sel.cell, Some(cell(RowRef::Insert(1), 1)));
        sel.clamp(3, 0, 2);
        assert_eq!(sel.cell, None);
    }

    #[test]
    fn desplazar_solo_lo_necesario() {
        // Ventana de 100 que empieza en 50: se ve 50..150
        assert_eq!(scroll_into_view(60.0, 80.0, 50.0, 100.0), None);
        assert_eq!(scroll_into_view(30.0, 50.0, 50.0, 100.0), Some(30.0));
        assert_eq!(scroll_into_view(140.0, 160.0, 50.0, 100.0), Some(60.0));
        // Más ancho que la ventana: se alinea su principio
        assert_eq!(scroll_into_view(200.0, 400.0, 50.0, 100.0), Some(200.0));
    }
}

use std::cell::RefCell;
use std::collections::BTreeSet;

use eframe::egui;
use egui_extras::{Column, TableBuilder};
use serde::{Deserialize, Serialize};

use crate::db::{ColumnInfo, QueryResult, quote_ident};
use crate::ui::{CMD, ellipsize, theme};

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

#[derive(Debug)]
pub enum GridAction {
    None,
    SortChanged,
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

// ── Barra de apply ────────────────────────────────────────────────────────────

/// Renderiza la barra inferior de cambios pendientes.
pub fn show_apply_bar(
    ui: &mut egui::Ui,
    pending_edits: &[PendingEdit],
    pending_deletes: usize,
    pending_inserts: usize,
    ctrl_enter: bool,
    pk_missing: bool,
) -> GridAction {
    let mut action = GridAction::None;
    let p = theme::get(ui);

    ui.horizontal(|ui| {
        if pk_missing {
            ui.colored_label(p.warning, "⚠ Sin clave primaria — no se puede guardar");
        } else {
            if !pending_edits.is_empty() {
                theme::badge(
                    ui,
                    &format!("● {} edición(es)", pending_edits.len()),
                    p.warning,
                );
            }
            if pending_deletes > 0 {
                theme::badge(
                    ui,
                    &format!(
                        "{} {} eliminación(es)",
                        crate::ui::icons::TRASH,
                        pending_deletes
                    ),
                    p.danger,
                );
            }
            if pending_inserts > 0 {
                theme::badge(
                    ui,
                    &format!(
                        "{} {} inserción(es)",
                        crate::ui::icons::PLUS,
                        pending_inserts
                    ),
                    p.success,
                );
            }
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Descartar").clicked() {
                action = GridAction::DiscardEdits;
            }
            if !pk_missing {
                let apply_btn = theme::primary_button(ui, format!("Guardar  {CMD}Enter"));
                if apply_btn.clicked() || ctrl_enter {
                    action = GridAction::ApplyEdits;
                }
            }
        });
    });

    action
}

// ── Utilidades de fecha ───────────────────────────────────────────────────────

/// Detecta si un string parece fecha (YYYY-MM-DD) o datetime (YYYY-MM-DD HH:MM:SS).
fn looks_like_datetime(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b[..4].iter().all(|c| c.is_ascii_digit())
        && b[5..7].iter().all(|c| c.is_ascii_digit())
        && b[8..10].iter().all(|c| c.is_ascii_digit())
}

/// Devuelve la fecha/hora local actual formateada como "YYYY-MM-DD HH:MM:SS".
fn current_datetime() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

// ── Helpers para filtros ──────────────────────────────────────────────────────

/// Pinta texto en `painter` dentro de `clip_rect` truncando con "…" si no cabe.
fn paint_text_truncated(
    ui: &egui::Ui,
    painter: &egui::Painter,
    pos: egui::Pos2,
    text: &str,
    color: egui::Color32,
    clip_width: f32,
) {
    use egui::text::{LayoutJob, TextFormat};
    let font_id = egui::TextStyle::Monospace.resolve(ui.style());
    let mut job = LayoutJob {
        wrap: egui::text::TextWrapping {
            max_width: clip_width,
            max_rows: 1,
            break_anywhere: false,
            overflow_character: Some('…'),
        },
        ..Default::default()
    };
    job.append(
        text,
        0.0,
        TextFormat {
            font_id,
            color,
            ..Default::default()
        },
    );
    let galley = painter.layout_job(job);
    painter.galley(pos, galley, color);
}

// ── Ancho inteligente de columnas ────────────────────────────────────────────

fn compute_col_widths(ui: &egui::Ui, data: &QueryResult) -> Vec<f32> {
    // Las celdas se pintan en monospace (JetBrains Mono, avance ~0.6 em)
    let font_size = egui::TextStyle::Monospace.resolve(ui.style()).size;
    let char_w = font_size * 0.62;
    let sort_reserve = font_size * 1.8;
    let padding = 8.0;

    let sample = data.rows.len().min(200);

    data.columns
        .iter()
        .enumerate()
        .map(|(col_i, col_name)| {
            let header_w = col_name.len() as f32 * char_w + sort_reserve + padding;

            let max_content = data.rows[..sample]
                .iter()
                .map(|row| {
                    row.get(col_i)
                        .and_then(|v| v.as_deref())
                        .map_or(0, str::len)
                })
                .max()
                .unwrap_or(0);
            let content_w = max_content as f32 * char_w + 8.0;

            header_w.max(content_w).clamp(50.0, 400.0)
        })
        .collect()
}

// ── Format rows for copy ──────────────────────────────────────────────────────

pub fn format_rows_for_copy(
    columns: &[String],
    rows: &[&Vec<Option<String>>],
    fmt: &crate::settings::CopyFormat,
    table_name: Option<&str>,
) -> String {
    match fmt {
        crate::settings::CopyFormat::Csv => {
            let header = columns
                .iter()
                .map(|c| format!("\"{}\"", c.replace('"', "\"\"")))
                .collect::<Vec<_>>()
                .join(",");
            let data_rows: Vec<String> = rows
                .iter()
                .map(|row| {
                    row.iter()
                        .map(|cell| match cell {
                            None => "NULL".to_string(),
                            Some(v) => format!("\"{}\"", v.replace('"', "\"\"")),
                        })
                        .collect::<Vec<_>>()
                        .join(",")
                })
                .collect();
            std::iter::once(header)
                .chain(data_rows)
                .collect::<Vec<_>>()
                .join("\n")
        }
        crate::settings::CopyFormat::Tsv => rows
            .iter()
            .map(|row| {
                row.iter()
                    .map(|cell| cell.as_deref().unwrap_or(""))
                    .collect::<Vec<_>>()
                    .join("\t")
            })
            .collect::<Vec<_>>()
            .join("\n"),
        crate::settings::CopyFormat::SqlInserts => {
            let col_list = columns
                .iter()
                .map(|c| quote_ident(c))
                .collect::<Vec<_>>()
                .join(", ");
            let tbl = quote_ident(table_name.unwrap_or("tabla"));
            rows.iter()
                .map(|row| {
                    let vals: Vec<String> = row
                        .iter()
                        .map(|cell| match cell {
                            None => "NULL".to_string(),
                            Some(v) => format!("'{}'", crate::db::escape_sql(v)),
                        })
                        .collect();
                    format!(
                        "INSERT INTO {tbl} ({col_list}) VALUES ({});",
                        vals.join(", ")
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
    }
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

// ── Grid principal ────────────────────────────────────────────────────────────

/// Muestra el grid de datos con soporte de edición de celdas.
/// Retorna la acción del usuario este frame.
pub fn show(
    ui: &mut egui::Ui,
    selected_table: Option<(&str, &str)>,
    view: &mut TableViewState,
    column_info: &[ColumnInfo],
    query_loading: bool,
) -> GridAction {
    let TableViewState {
        table_data: result,
        table_error: error,
        sort_specs: sorts,
        pending_edits,
        pending_deletes,
        pending_inserts,
        editing_cell,
        selected_cell,
        selected_rows,
        drag_select_from,
        ..
    } = view;
    match (selected_table, &*result, &*error) {
        (None, _, _) => {
            ui.centered_and_justified(|ui| {
                ui.label("Selecciona una tabla en el panel izquierdo");
            });
            return GridAction::None;
        }
        (Some((db, table)), _, Some(err)) => {
            ui.label(format!("{db}.{table}"));
            ui.separator();
            theme::error_card(ui, &format!("Error: {err}"));
            return GridAction::None;
        }
        (Some(_), None, None) => {
            ui.centered_and_justified(|ui| {
                if query_loading {
                    ui.spinner();
                    ui.label("Ejecutando consulta…");
                } else {
                    ui.label("Cargando…");
                }
            });
            return GridAction::None;
        }
        _ => {}
    }

    let (db, table) = selected_table.unwrap();
    let data = result.as_ref().unwrap();

    // Barra de info + chips de ordenación activa
    let mut remove_sort_idx: Option<usize> = None;
    let mut clear_all_sorts = false;

    ui.horizontal_wrapped(|ui| {
        ui.strong(format!("{db}.{table}"));
        ui.label(
            egui::RichText::new(format!("— {} filas", data.rows.len()))
                .color(theme::get(ui).text_muted),
        );
        if query_loading {
            ui.separator();
            ui.spinner();
            ui.label("Cargando…");
        }

        if !sorts.is_empty() {
            ui.separator();
            ui.label("Orden:");
            for (i, spec) in sorts.iter().enumerate() {
                let arrow = if spec.dir == SortDir::Asc {
                    "↑"
                } else {
                    "↓"
                };
                let p = theme::get(ui);
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
                            remove_sort_idx = Some(i);
                        }
                    });
            }
            if sorts.len() > 1
                && ui
                    .small_button("Limpiar todo")
                    .on_hover_text("Eliminar toda la ordenación")
                    .clicked()
            {
                clear_all_sorts = true;
            }
        }
    });

    let mut sort_changed = false;
    if clear_all_sorts {
        sorts.clear();
        sort_changed = true;
    } else if let Some(idx) = remove_sort_idx {
        sorts.remove(idx);
        sort_changed = true;
    }
    ui.separator();

    if data.columns.is_empty() {
        ui.label("(tabla vacía)");
        return if sort_changed {
            GridAction::SortChanged
        } else {
            GridAction::None
        };
    }

    let col_count = data.columns.len();
    // El servidor devuelve los datos ya ordenados: la fila mostrada N es la
    // fila N de los datos.
    let real_row_count = data.rows.len();
    let total_row_count = real_row_count + pending_inserts.len();

    // Señales fuera de closures para evitar conflictos de borrow
    let mut clicked_col: Option<usize> = None;

    // Usamos RefCell para mutabilidad interior en closures anidados de egui
    let edit_cell_rc: RefCell<Option<EditingCell>> = RefCell::new(editing_cell.take());
    let grid_action_rc: RefCell<GridAction> = RefCell::new(GridAction::None);
    // Track drag state
    let drag_current_row_rc: RefCell<Option<usize>> = RefCell::new(None);

    // Altura de fila fija para evitar salto de layout al comenzar a editar
    let row_h = 22.0;

    // Anchos inteligentes por columna basados en cabecera + muestra de contenido
    let col_widths = compute_col_widths(ui, data);
    let total_width: f32 = col_widths.iter().sum::<f32>() + 24.0;

    // Atajos de teclado para la celda seleccionada
    let mut keyboard_action = GridAction::None;
    let no_widget_focused = ui.memory(|m| m.focused().is_none());

    // Ctrl+C: sin foco en widget de texto y sin estar editando una celda.
    // Usamos Event::Copy (idiomático en egui) + fallback raw key para mayor compatibilidad.
    let copy_requested = edit_cell_rc.borrow().is_none()
        && no_widget_focused
        && ui.input(|i| {
            i.events.iter().any(|e| matches!(e, egui::Event::Copy))
                || (i.modifiers.command && i.key_pressed(egui::Key::C))
        });
    if copy_requested {
        if !selected_rows.is_empty() {
            keyboard_action = GridAction::CopySelectedRows;
        } else if let Some((sel_row, sel_col)) = *selected_cell
            && let Some(row) = data.rows.get(sel_row)
            && let Some(cell) = row.get(sel_col)
        {
            ui.ctx()
                .copy_text(cell.as_deref().unwrap_or("").to_string());
        }
    }

    if no_widget_focused && edit_cell_rc.borrow().is_none() {
        let arrow_up = ui.input(|i| i.key_pressed(egui::Key::ArrowUp));
        let arrow_down = ui.input(|i| i.key_pressed(egui::Key::ArrowDown));
        let arrow_left = ui.input(|i| i.key_pressed(egui::Key::ArrowLeft));
        let arrow_right = ui.input(|i| i.key_pressed(egui::Key::ArrowRight));
        if arrow_up || arrow_down || arrow_left || arrow_right {
            let (r, c) = selected_cell.unwrap_or((0, 0));
            let new_r = if arrow_up {
                r.saturating_sub(1)
            } else if arrow_down {
                (r + 1).min(real_row_count.saturating_sub(1))
            } else {
                r
            };
            let new_c = if arrow_left {
                c.saturating_sub(1)
            } else if arrow_right {
                (c + 1).min(col_count.saturating_sub(1))
            } else {
                c
            };
            *selected_cell = Some((new_r, new_c));
        }

        let ctrl = ui.input(|i| i.modifiers.command);

        if let Some((sel_row, sel_col)) = *selected_cell
            && sel_row < data.rows.len()
            && data.rows[sel_row].get(sel_col).is_some()
        {
            let cell_str = data.rows[sel_row][sel_col]
                .as_deref()
                .unwrap_or("")
                .to_string();

            // Ctrl+C handled above
            // Ctrl+N: NULL
            if ctrl && ui.input(|i| i.key_pressed(egui::Key::N)) {
                keyboard_action = GridAction::CellSetNull {
                    row: RowRef::Existing(sel_row),
                    col_idx: sel_col,
                };
            }
            // Ctrl+Del: mark/toggle delete row(s)
            if ctrl && ui.input(|i| i.key_pressed(egui::Key::Delete)) {
                if selected_rows.is_empty() {
                    keyboard_action = GridAction::MarkDeleteRow { row_idx: sel_row };
                } else {
                    keyboard_action = GridAction::MarkDeleteRows {
                        row_indices: selected_rows.iter().cloned().collect(),
                    };
                }
            }
            // Ctrl+Ins: insert row
            if ctrl && ui.input(|i| i.key_pressed(egui::Key::Insert)) {
                keyboard_action = GridAction::InsertRow;
            }
            // Ctrl+D: clone row
            if ctrl && ui.input(|i| i.key_pressed(egui::Key::D)) {
                keyboard_action = GridAction::CloneRow { row_idx: sel_row };
            }
            // Delete: set NULL if nullable, else ""
            if !ctrl && ui.input(|i| i.key_pressed(egui::Key::Delete)) {
                let is_nullable = column_info
                    .get(sel_col)
                    .map(|c| c.is_nullable)
                    .unwrap_or(false);
                if is_nullable {
                    keyboard_action = GridAction::CellSetNull {
                        row: RowRef::Existing(sel_row),
                        col_idx: sel_col,
                    };
                } else {
                    keyboard_action = GridAction::CellEditCommitted {
                        row: RowRef::Existing(sel_row),
                        col_idx: sel_col,
                        new_value: String::new(),
                    };
                }
            }
            // F2 o Enter: editar
            if ui.input(|i| i.key_pressed(egui::Key::F2)) {
                keyboard_action = GridAction::CellEditStarted {
                    row: RowRef::Existing(sel_row),
                    col_idx: sel_col,
                    current_value: cell_str.clone(),
                };
            }
            // Ctrl+V: pegar
            let paste = ui.input(|i| {
                i.events.iter().find_map(|e| {
                    if let egui::Event::Paste(s) = e {
                        Some(s.clone())
                    } else {
                        None
                    }
                })
            });
            if let Some(text) = paste {
                keyboard_action = GridAction::CellEditCommitted {
                    row: RowRef::Existing(sel_row),
                    col_idx: sel_col,
                    new_value: text,
                };
            }
            // Escape: deseleccionar
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                *selected_cell = None;
                selected_rows.clear();
            }
        }
    }

    let sort_hash: u64 = sorts.iter().fold(0u64, |acc, s| {
        acc.wrapping_add(s.col_idx as u64)
            .wrapping_mul(31)
            .wrapping_add(if s.dir == SortDir::Asc { 0 } else { 1 })
    });
    let table_id = format!("{}/{}/{}", db, table, sort_hash);

    egui::ScrollArea::horizontal()
        .id_salt(format!("{}_hscroll", table_id))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.set_min_width(total_width);

            // Banda de fondo de la cabecera (se pinta antes de construir la tabla)
            let header_band = egui::Rect::from_min_size(
                ui.cursor().min,
                egui::vec2(ui.available_width().max(total_width), 24.0),
            );
            ui.painter()
                .rect_filled(header_band, 0.0, theme::get(ui).grid_header_bg);

            let mut builder = TableBuilder::new(ui)
                .id_salt(&table_id)
                .striped(true)
                .resizable(true)
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center));
            for &w in &col_widths {
                builder =
                    builder.column(Column::initial(w).resizable(true).clip(true).at_least(40.0));
            }
            builder
                .header(24.0, |mut header| {
                    for (col_i, col_name) in data.columns.iter().enumerate() {
                        header.col(|ui| {
                            let sort_pos = sorts.iter().position(|s| s.col_idx == col_i);

                            // Build header label with icons
                            let icon = if let Some(ci) = column_info.get(col_i) {
                                match ci.column_key.as_str() {
                                    "PRI" => "🔑 ",
                                    "UNI" => "U ",
                                    "MUL" => "I ",
                                    _ => "",
                                }
                            } else {
                                ""
                            };

                            let mut label = format!("{}{}", icon, col_name);
                            if let Some(pos) = sort_pos {
                                let arrow = if sorts[pos].dir == SortDir::Asc {
                                    " ↑"
                                } else {
                                    " ↓"
                                };
                                label.push_str(arrow);
                                if sorts.len() > 1 {
                                    label.push_str(&format!("{}", pos + 1));
                                }
                            }

                            let hint = match sort_pos {
                                None => "Click: ordenar ↑",
                                Some(i) if sorts[i].dir == SortDir::Asc => "Click: ordenar ↓",
                                _ => "Click: quitar ordenación",
                            };

                            let p = theme::get(ui);
                            let header_color = if sort_pos.is_some() {
                                p.accent
                            } else {
                                p.text_muted
                            };
                            if ui
                                .add(
                                    egui::Button::new(
                                        egui::RichText::new(label).color(header_color),
                                    )
                                    .frame(false),
                                )
                                .on_hover_text(hint)
                                .clicked()
                            {
                                clicked_col = Some(col_i);
                            }
                        });
                    }
                })
                .body(|body| {
                    body.rows(row_h, total_row_count, |mut row| {
                        let row_display_idx = row.index();

                        if row_display_idx < real_row_count {
                            // ── Real data row ─────────────────────────────────────
                            let real_idx = row_display_idx;
                            let row_data = &data.rows[real_idx];

                            let is_pending_delete = pending_deletes.contains(&real_idx);
                            let is_selected_row =
                                selected_rows.contains(&real_idx) && !is_pending_delete;
                            let row_modified = pending_edits.iter().any(|e| e.row_idx == real_idx);

                            for (col_i, cell_opt) in row_data.iter().enumerate().take(col_count) {
                                row.col(|ui| {
                                    let is_editing =
                                        edit_cell_rc.borrow().as_ref().is_some_and(|e| {
                                            e.row == RowRef::Existing(real_idx)
                                                && e.col_idx == col_i
                                        });

                                    let pending = pending_edits
                                        .iter()
                                        .find(|e| e.row_idx == real_idx && e.col_idx == col_i);

                                    let cell_rect = ui.available_rect_before_wrap();

                                    let p = theme::get(ui);
                                    // Fondos de fila (opacos: no se mezclan con el rayado)
                                    if is_pending_delete {
                                        ui.painter().rect_filled(cell_rect, 0.0, p.grid_delete_bg);
                                    } else if is_selected_row {
                                        ui.painter().rect_filled(cell_rect, 0.0, p.grid_sel_row_bg);
                                    }

                                    if is_editing {
                                        // ── Celda en edición ──────────────────────────
                                        let mut ec = edit_cell_rc.borrow_mut();
                                        let edit = ec.as_mut().unwrap();
                                        let resp = ui.add(
                                            egui::TextEdit::singleline(&mut edit.buffer)
                                                .desired_width(f32::INFINITY),
                                        );

                                        if edit.needs_focus {
                                            resp.request_focus();
                                            edit.needs_focus = false;
                                        }

                                        let tab_pressed = ui.input_mut(|i| {
                                            i.consume_key(egui::Modifiers::NONE, egui::Key::Tab)
                                        });
                                        let enter_pressed = resp.has_focus()
                                            && ui.input(|i| i.key_pressed(egui::Key::Enter));
                                        let committed = resp.lost_focus() || enter_pressed;
                                        let cancelled =
                                            ui.input(|i| i.key_pressed(egui::Key::Escape));

                                        if tab_pressed && resp.has_focus() {
                                            *grid_action_rc.borrow_mut() =
                                                GridAction::CellEditTabNext {
                                                    row: RowRef::Existing(real_idx),
                                                    col_idx: col_i,
                                                    new_value: edit.buffer.clone(),
                                                };
                                        } else if cancelled {
                                            *grid_action_rc.borrow_mut() =
                                                GridAction::CellEditCancelled;
                                        } else if committed {
                                            *grid_action_rc.borrow_mut() =
                                                GridAction::CellEditCommitted {
                                                    row: RowRef::Existing(real_idx),
                                                    col_idx: col_i,
                                                    new_value: edit.buffer.clone(),
                                                };
                                        }
                                    } else if let Some(edit) = pending {
                                        // ── Celda con cambio pendiente ────────────────
                                        if !is_pending_delete {
                                            ui.painter().rect_filled(
                                                cell_rect,
                                                0.0,
                                                p.grid_edit_bg,
                                            );
                                            let pts = vec![
                                                cell_rect.right_top(),
                                                cell_rect.right_top() - egui::vec2(8.0, 0.0),
                                                cell_rect.right_top() + egui::vec2(0.0, 8.0),
                                            ];
                                            ui.painter().add(egui::Shape::convex_polygon(
                                                pts,
                                                p.warning,
                                                egui::Stroke::NONE,
                                            ));
                                        }
                                        let resp =
                                            ui.allocate_rect(cell_rect, egui::Sense::click());
                                        if ui.is_rect_visible(cell_rect) {
                                            let text_clip = cell_rect.shrink2(egui::vec2(4.0, 0.0));
                                            let painter = ui.painter().with_clip_rect(text_clip);
                                            let pos = egui::pos2(
                                                text_clip.min.x,
                                                text_clip.center().y
                                                    - ui.text_style_height(&egui::TextStyle::Body)
                                                        * 0.5,
                                            );
                                            if is_pending_delete {
                                                let display = if edit.is_null {
                                                    "NULL".to_string()
                                                } else {
                                                    edit.new_value.clone()
                                                };
                                                paint_text_truncated(
                                                    ui,
                                                    &painter,
                                                    pos,
                                                    &display,
                                                    p.grid_delete_fg,
                                                    text_clip.width(),
                                                );
                                            } else if edit.is_null {
                                                paint_text_truncated(
                                                    ui,
                                                    &painter,
                                                    pos,
                                                    "NULL",
                                                    p.grid_edit_fg,
                                                    text_clip.width(),
                                                );
                                            } else {
                                                paint_text_truncated(
                                                    ui,
                                                    &painter,
                                                    pos,
                                                    &edit.new_value,
                                                    p.grid_edit_fg,
                                                    text_clip.width(),
                                                );
                                            }
                                        }

                                        if !is_pending_delete && resp.double_clicked() {
                                            let start_value = if edit.is_null {
                                                String::new()
                                            } else {
                                                edit.new_value.clone()
                                            };
                                            *grid_action_rc.borrow_mut() =
                                                GridAction::CellEditStarted {
                                                    row: RowRef::Existing(real_idx),
                                                    col_idx: col_i,
                                                    current_value: start_value,
                                                };
                                        }

                                        // Handle click for row selection
                                        if resp.clicked() {
                                            let shift = ui.input(|i| i.modifiers.shift);
                                            let ctrl = ui.input(|i| i.modifiers.command);
                                            if shift || ctrl {
                                                *grid_action_rc.borrow_mut() =
                                                    GridAction::RowSelected {
                                                        row_idx: real_idx,
                                                        shift,
                                                        ctrl,
                                                    };
                                            } else {
                                                *grid_action_rc.borrow_mut() =
                                                    GridAction::CellSelected {
                                                        row_idx: real_idx,
                                                        col_idx: col_i,
                                                    };
                                            }
                                        }
                                    } else {
                                        // ── Celda normal ──────────────────────────────
                                        let is_null_cell = cell_opt.is_none();
                                        let cell_text: &str = cell_opt.as_deref().unwrap_or("");

                                        // Fondo tenue si la fila tiene cambios en otras celdas (no delete)
                                        if row_modified && !is_pending_delete {
                                            ui.painter().rect_filled(
                                                cell_rect,
                                                0.0,
                                                p.grid_edit_bg.gamma_multiply(0.5),
                                            );
                                        }

                                        // Fondo de celda seleccionada
                                        let is_selected_cell = selected_cell
                                            .as_ref()
                                            .is_some_and(|&(r, c)| r == real_idx && c == col_i);
                                        if is_selected_cell
                                            && !is_pending_delete
                                            && !is_selected_row
                                        {
                                            ui.painter().rect_filled(
                                                cell_rect,
                                                0.0,
                                                ui.visuals().selection.bg_fill,
                                            );
                                        }

                                        let resp = ui.allocate_rect(
                                            cell_rect,
                                            egui::Sense::click_and_drag(),
                                        );

                                        if ui.is_rect_visible(cell_rect) {
                                            let text_clip = cell_rect.shrink2(egui::vec2(4.0, 0.0));
                                            let painter = ui.painter().with_clip_rect(text_clip);
                                            let pos = egui::pos2(
                                                text_clip.min.x,
                                                text_clip.center().y
                                                    - ui.text_style_height(&egui::TextStyle::Body)
                                                        * 0.5,
                                            );
                                            if is_pending_delete {
                                                let display_text =
                                                    if is_null_cell { "NULL" } else { cell_text };
                                                paint_text_truncated(
                                                    ui,
                                                    &painter,
                                                    pos,
                                                    display_text,
                                                    p.grid_delete_fg,
                                                    text_clip.width(),
                                                );
                                            } else if is_null_cell {
                                                paint_text_truncated(
                                                    ui,
                                                    &painter,
                                                    pos,
                                                    "NULL",
                                                    p.null_text,
                                                    text_clip.width(),
                                                );
                                            } else {
                                                paint_text_truncated(
                                                    ui,
                                                    &painter,
                                                    pos,
                                                    cell_text,
                                                    ui.visuals().text_color(),
                                                    text_clip.width(),
                                                );
                                            }
                                        }

                                        // Drag selection
                                        if resp.drag_started() {
                                            *drag_current_row_rc.borrow_mut() = Some(real_idx);
                                            *grid_action_rc.borrow_mut() =
                                                GridAction::RowSelected {
                                                    row_idx: real_idx,
                                                    shift: false,
                                                    ctrl: false,
                                                };
                                        }

                                        // While dragging, track current row by pointer position
                                        if ui.ctx().input(|i| i.pointer.is_decidedly_dragging())
                                            && let Some(pointer_pos) =
                                                ui.ctx().input(|i| i.pointer.hover_pos())
                                            && pointer_pos.y >= cell_rect.min.y
                                            && pointer_pos.y <= cell_rect.max.y
                                        {
                                            *drag_current_row_rc.borrow_mut() = Some(real_idx);
                                        }

                                        if resp.clicked() {
                                            let shift = ui.input(|i| i.modifiers.shift);
                                            let ctrl = ui.input(|i| i.modifiers.command);
                                            if shift || ctrl {
                                                *grid_action_rc.borrow_mut() =
                                                    GridAction::RowSelected {
                                                        row_idx: real_idx,
                                                        shift,
                                                        ctrl,
                                                    };
                                            } else {
                                                *grid_action_rc.borrow_mut() =
                                                    GridAction::CellSelected {
                                                        row_idx: real_idx,
                                                        col_idx: col_i,
                                                    };
                                            }
                                        }

                                        if resp.double_clicked() && !is_pending_delete {
                                            *grid_action_rc.borrow_mut() =
                                                GridAction::CellEditStarted {
                                                    row: RowRef::Existing(real_idx),
                                                    col_idx: col_i,
                                                    current_value: cell_text.to_string(),
                                                };
                                        }

                                        // Context menu
                                        let is_dt = looks_like_datetime(cell_text);
                                        let ga_rc = &grid_action_rc;
                                        let col_name = &data.columns[col_i];
                                        let has_multi_selection = !selected_rows.is_empty();
                                        resp.context_menu(|ui| {
                                            if has_multi_selection {
                                                if ui.button("📋  Copiar en CSV").clicked() {
                                                    *ga_rc.borrow_mut() = GridAction::CopyRowsAs {
                                                        fmt: crate::settings::CopyFormat::Csv,
                                                    };
                                                    ui.close();
                                                }
                                                if ui.button("📋  Copiar en TSV").clicked() {
                                                    *ga_rc.borrow_mut() = GridAction::CopyRowsAs {
                                                        fmt: crate::settings::CopyFormat::Tsv,
                                                    };
                                                    ui.close();
                                                }
                                                if ui.button("📋  Copiar SQL Insert").clicked() {
                                                    *ga_rc.borrow_mut() = GridAction::CopyRowsAs {
                                                        fmt:
                                                            crate::settings::CopyFormat::SqlInserts,
                                                    };
                                                    ui.close();
                                                }
                                            } else {
                                                if ui.button("📋  Copiar valor").clicked() {
                                                    ui.ctx().copy_text(cell_text.to_string());
                                                    ui.close();
                                                }
                                            }
                                            ui.separator();
                                            if !is_pending_delete {
                                                if ui.button("✏  Editar celda").clicked() {
                                                    *ga_rc.borrow_mut() =
                                                        GridAction::CellEditStarted {
                                                            row: RowRef::Existing(real_idx),
                                                            col_idx: col_i,
                                                            current_value: cell_text.to_string(),
                                                        };
                                                    ui.close();
                                                }
                                                if ui.button("∅  Establecer NULL").clicked() {
                                                    *ga_rc.borrow_mut() = GridAction::CellSetNull {
                                                        row: RowRef::Existing(real_idx),
                                                        col_idx: col_i,
                                                    };
                                                    ui.close();
                                                }
                                                if ui.button("\"\"  Establecer vacío").clicked() {
                                                    *ga_rc.borrow_mut() =
                                                        GridAction::CellEditCommitted {
                                                            row: RowRef::Existing(real_idx),
                                                            col_idx: col_i,
                                                            new_value: String::new(),
                                                        };
                                                    ui.close();
                                                }
                                                if is_dt
                                                    && ui.button("🕐  Fecha/hora actual").clicked()
                                                {
                                                    *ga_rc.borrow_mut() =
                                                        GridAction::CellEditCommitted {
                                                            row: RowRef::Existing(real_idx),
                                                            col_idx: col_i,
                                                            new_value: current_datetime(),
                                                        };
                                                    ui.close();
                                                }
                                                ui.separator();
                                            }

                                            // Filtrar
                                            let esc_col = quote_ident(col_name);
                                            let val_display = ellipsize(cell_text, 22);
                                            let val_escaped = crate::db::escape_sql(cell_text);

                                            if ui.button(format!("🔍  = '{val_display}'")).clicked()
                                            {
                                                *ga_rc.borrow_mut() = GridAction::FilterBy {
                                                    sql_fragment: format!(
                                                        "{esc_col} = '{val_escaped}'"
                                                    ),
                                                };
                                                ui.close();
                                            }
                                            if ui.button(format!("🔍  ≠ '{val_display}'")).clicked()
                                            {
                                                *ga_rc.borrow_mut() = GridAction::FilterBy {
                                                    sql_fragment: format!(
                                                        "{esc_col} != '{val_escaped}'"
                                                    ),
                                                };
                                                ui.close();
                                            }
                                            if ui
                                                .button(format!("🔍  LIKE '%{val_display}%'"))
                                                .clicked()
                                            {
                                                *ga_rc.borrow_mut() = GridAction::FilterBy {
                                                    sql_fragment: format!(
                                                        "{esc_col} LIKE '%{val_escaped}%'"
                                                    ),
                                                };
                                                ui.close();
                                            }
                                            if ui
                                                .button(format!("🔍  IS NULL ({col_name})"))
                                                .clicked()
                                            {
                                                *ga_rc.borrow_mut() = GridAction::FilterBy {
                                                    sql_fragment: format!("{esc_col} IS NULL"),
                                                };
                                                ui.close();
                                            }
                                            if ui
                                                .button(format!("🔍  IS NOT NULL ({col_name})"))
                                                .clicked()
                                            {
                                                *ga_rc.borrow_mut() = GridAction::FilterBy {
                                                    sql_fragment: format!("{esc_col} IS NOT NULL"),
                                                };
                                                ui.close();
                                            }
                                            if ui.button("🔍  Por portapapeles").clicked() {
                                                *ga_rc.borrow_mut() =
                                                    GridAction::FilterByClipboard {
                                                        col_name: col_name.clone(),
                                                    };
                                                ui.close();
                                            }
                                            if ui.button("🔍  Por texto...").clicked() {
                                                *ga_rc.borrow_mut() = GridAction::FilterUserInput {
                                                    col_name: col_name.clone(),
                                                };
                                                ui.close();
                                            }

                                            // Fila
                                            ui.separator();
                                            if ui
                                                .button(format!("➕  Insertar fila  {CMD}Ins"))
                                                .clicked()
                                            {
                                                *ga_rc.borrow_mut() = GridAction::InsertRow;
                                                ui.close();
                                            }
                                            if ui
                                                .button(format!("📋  Clonar fila  {CMD}D"))
                                                .clicked()
                                            {
                                                *ga_rc.borrow_mut() =
                                                    GridAction::CloneRow { row_idx: real_idx };
                                                ui.close();
                                            }
                                            let del_label = if pending_deletes.contains(&real_idx) {
                                                "↩  Desmarcar eliminación".to_string()
                                            } else {
                                                format!("🗑  Eliminar fila  {CMD}Supr")
                                            };
                                            if theme::danger_button(ui, del_label).clicked() {
                                                *ga_rc.borrow_mut() =
                                                    GridAction::MarkDeleteRow { row_idx: real_idx };
                                                ui.close();
                                            }
                                        });
                                    }
                                });
                            }
                        } else {
                            // ── Pending insert row ────────────────────────────────
                            let insert_idx = row_display_idx - real_row_count;
                            let ins = match pending_inserts.get(insert_idx) {
                                Some(i) => i,
                                None => return,
                            };

                            for col_i in 0..col_count {
                                row.col(|ui| {
                                    let cell_rect = ui.available_rect_before_wrap();
                                    let p = theme::get(ui);

                                    // Fondo verde de fila de inserción
                                    ui.painter().rect_filled(cell_rect, 0.0, p.grid_insert_bg);

                                    let is_auto =
                                        ins.auto_cols.get(col_i).copied().unwrap_or(false);

                                    let is_editing =
                                        edit_cell_rc.borrow().as_ref().is_some_and(|e| {
                                            e.row == RowRef::Insert(insert_idx)
                                                && e.col_idx == col_i
                                        });

                                    if is_auto {
                                        // Show "(AUTO)" in muted color, no editing
                                        if ui.is_rect_visible(cell_rect) {
                                            let text_clip = cell_rect.shrink2(egui::vec2(4.0, 0.0));
                                            let painter = ui.painter().with_clip_rect(text_clip);
                                            let pos = egui::pos2(
                                                text_clip.min.x,
                                                text_clip.center().y
                                                    - ui.text_style_height(&egui::TextStyle::Body)
                                                        * 0.5,
                                            );
                                            paint_text_truncated(
                                                ui,
                                                &painter,
                                                pos,
                                                "(AUTO)",
                                                p.null_text,
                                                text_clip.width(),
                                            );
                                        }
                                        ui.allocate_rect(cell_rect, egui::Sense::hover());
                                    } else if is_editing {
                                        // Editing an insert cell
                                        let mut ec = edit_cell_rc.borrow_mut();
                                        let edit = ec.as_mut().unwrap();
                                        let resp = ui.add(
                                            egui::TextEdit::singleline(&mut edit.buffer)
                                                .desired_width(f32::INFINITY),
                                        );

                                        if edit.needs_focus {
                                            resp.request_focus();
                                            edit.needs_focus = false;
                                        }

                                        let tab_pressed = ui.input_mut(|i| {
                                            i.consume_key(egui::Modifiers::NONE, egui::Key::Tab)
                                        });
                                        let enter_pressed = resp.has_focus()
                                            && ui.input(|i| i.key_pressed(egui::Key::Enter));
                                        let committed = resp.lost_focus() || enter_pressed;
                                        let cancelled =
                                            ui.input(|i| i.key_pressed(egui::Key::Escape));

                                        if tab_pressed && resp.has_focus() {
                                            *grid_action_rc.borrow_mut() =
                                                GridAction::CellEditTabNext {
                                                    row: RowRef::Insert(insert_idx),
                                                    col_idx: col_i,
                                                    new_value: edit.buffer.clone(),
                                                };
                                        } else if cancelled {
                                            *grid_action_rc.borrow_mut() =
                                                GridAction::CellEditCancelled;
                                        } else if committed {
                                            *grid_action_rc.borrow_mut() =
                                                GridAction::CellEditCommitted {
                                                    row: RowRef::Insert(insert_idx),
                                                    col_idx: col_i,
                                                    new_value: edit.buffer.clone(),
                                                };
                                        }
                                    } else {
                                        // Show insert value, double-click to edit
                                        let (val, val_color) = match ins.values.get(col_i) {
                                            Some(InsertValue::Value(v)) => {
                                                (v.as_str(), p.grid_insert_fg)
                                            }
                                            Some(InsertValue::Null) => ("NULL", p.null_text),
                                            _ => ("(DEFAULT)", p.null_text),
                                        };
                                        let resp =
                                            ui.allocate_rect(cell_rect, egui::Sense::click());

                                        if ui.is_rect_visible(cell_rect) {
                                            let text_clip = cell_rect.shrink2(egui::vec2(4.0, 0.0));
                                            let painter = ui.painter().with_clip_rect(text_clip);
                                            let pos = egui::pos2(
                                                text_clip.min.x,
                                                text_clip.center().y
                                                    - ui.text_style_height(&egui::TextStyle::Body)
                                                        * 0.5,
                                            );
                                            paint_text_truncated(
                                                ui,
                                                &painter,
                                                pos,
                                                val,
                                                val_color,
                                                text_clip.width(),
                                            );
                                        }

                                        if resp.double_clicked() {
                                            let current_value = match ins.values.get(col_i) {
                                                Some(InsertValue::Value(v)) => v.clone(),
                                                _ => String::new(),
                                            };
                                            *grid_action_rc.borrow_mut() =
                                                GridAction::CellEditStarted {
                                                    row: RowRef::Insert(insert_idx),
                                                    col_idx: col_i,
                                                    current_value,
                                                };
                                        }

                                        // Show × button in the last column
                                        if col_i == col_count - 1 {
                                            let btn_size = egui::vec2(16.0, 16.0);
                                            let btn_pos = egui::pos2(
                                                cell_rect.max.x - btn_size.x - 2.0,
                                                cell_rect.center().y - btn_size.y * 0.5,
                                            );
                                            let btn_rect =
                                                egui::Rect::from_min_size(btn_pos, btn_size);
                                            let btn_resp =
                                                ui.allocate_rect(btn_rect, egui::Sense::click());
                                            if btn_resp.hovered() {
                                                ui.painter().rect_filled(btn_rect, 3.0, p.danger);
                                            }
                                            ui.painter().text(
                                                btn_rect.center(),
                                                egui::Align2::CENTER_CENTER,
                                                "×",
                                                egui::FontId::proportional(12.0),
                                                if btn_resp.hovered() {
                                                    p.on_accent
                                                } else {
                                                    p.danger
                                                },
                                            );
                                            if btn_resp.clicked() {
                                                *grid_action_rc.borrow_mut() =
                                                    GridAction::RemoveInsertRow { insert_idx };
                                            }
                                        }
                                    }
                                });
                            }
                        }
                    });
                });
        }); // cierra ScrollArea::horizontal

    // Restaurar el editing_cell
    *editing_cell = edit_cell_rc.into_inner();

    // Handle drag selection update
    let drag_row = drag_current_row_rc.into_inner();
    if ui.ctx().input(|i| i.pointer.is_decidedly_dragging()) {
        if let (Some(start), Some(current)) = (*drag_select_from, drag_row) {
            let (lo, hi) = (start.min(current), start.max(current));
            *selected_rows = (lo..=hi).collect();
        }
    } else {
        // Drag ended
        if drag_select_from.is_some() && !ui.ctx().input(|i| i.pointer.any_down()) {
            *drag_select_from = None;
        }
    }

    // Aplicar cambio de ordenación tras los closures
    if let Some(col_i) = clicked_col {
        match sorts.iter().position(|s| s.col_idx == col_i) {
            None => sorts.push(SortSpec {
                col_idx: col_i,
                col_name: data.columns[col_i].clone(),
                dir: SortDir::Asc,
            }),
            Some(i) if sorts[i].dir == SortDir::Asc => sorts[i].dir = SortDir::Desc,
            Some(i) => {
                sorts.remove(i);
            }
        }
        sort_changed = true;
    }

    let grid_action = grid_action_rc.into_inner();

    // Priorizar acciones de teclado
    if !matches!(keyboard_action, GridAction::None) {
        return keyboard_action;
    }

    // Priorizar la acción de edición sobre el sort
    match grid_action {
        GridAction::None if sort_changed => GridAction::SortChanged,
        other => other,
    }
}

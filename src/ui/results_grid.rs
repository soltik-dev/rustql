use std::cell::RefCell;

use eframe::egui;
use egui_extras::{Column, TableBuilder};

use crate::db::QueryResult;

// ── Tipos de ordenación (sin cambios) ────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum SortDir {
    Asc,
    Desc,
}

#[derive(Debug, Clone)]
pub struct SortSpec {
    pub col_idx: usize,
    pub col_name: String,
    pub dir: SortDir,
}

/// Ordena `rows` según `sorts` y devuelve un vector de índices.
pub fn compute_sort_indices(rows: &[Vec<String>], sorts: &[SortSpec]) -> Vec<usize> {
    let mut indices: Vec<usize> = (0..rows.len()).collect();
    if sorts.is_empty() {
        return indices;
    }
    indices.sort_by(|&a, &b| {
        for spec in sorts {
            let va = &rows[a][spec.col_idx];
            let vb = &rows[b][spec.col_idx];
            let cmp = compare_values(va, vb);
            let cmp = if spec.dir == SortDir::Desc { cmp.reverse() } else { cmp };
            if cmp != std::cmp::Ordering::Equal {
                return cmp;
            }
        }
        std::cmp::Ordering::Equal
    });
    indices
}

fn compare_values(a: &str, b: &str) -> std::cmp::Ordering {
    match (a.parse::<f64>(), b.parse::<f64>()) {
        (Ok(na), Ok(nb)) => na.partial_cmp(&nb).unwrap_or(std::cmp::Ordering::Equal),
        _ => a.cmp(b),
    }
}

// ── Tipos de edición ─────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct PendingEdit {
    /// Índice real en `QueryResult.rows` (no el índice de visualización con sort).
    pub row_idx: usize,
    pub col_idx: usize,
    pub original: String,
    pub new_value: String,
}

#[derive(Debug, Clone)]
pub struct EditingCell {
    pub row_idx: usize,
    pub col_idx: usize,
    /// Buffer de edición mutable mientras el usuario escribe.
    pub buffer: String,
}

#[derive(Debug)]
pub enum GridAction {
    None,
    SortChanged,
    CellEditStarted { row_idx: usize, col_idx: usize, current_value: String },
    CellEditCommitted { row_idx: usize, col_idx: usize, new_value: String },
    CellEditCancelled,
    ApplyEdits,
    DiscardEdits,
}

// ── Barra de apply ────────────────────────────────────────────────────────────

/// Renderiza la barra inferior de cambios pendientes.
/// Retorna `GridAction::ApplyEdits` o `GridAction::DiscardEdits` si se pulsó.
pub fn show_apply_bar(
    ui: &mut egui::Ui,
    pending_edits: &[PendingEdit],
    ctrl_enter: bool,
    pk_missing: bool,
) -> GridAction {
    let mut action = GridAction::None;

    ui.horizontal(|ui| {
        if pk_missing {
            ui.colored_label(
                egui::Color32::from_rgb(220, 100, 50),
                "⚠ Sin clave primaria — no se puede guardar",
            );
        } else {
            ui.colored_label(
                egui::Color32::from_rgb(200, 150, 0),
                format!("● {} cambio(s) sin guardar", pending_edits.len()),
            );
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Descartar").clicked() {
                action = GridAction::DiscardEdits;
            }
            if !pk_missing {
                let apply_btn = ui.button("Guardar  Ctrl+Enter");
                if apply_btn.clicked() || ctrl_enter {
                    action = GridAction::ApplyEdits;
                }
            }
        });
    });

    action
}

// ── Grid principal ────────────────────────────────────────────────────────────

/// Muestra el grid de datos con soporte de edición de celdas.
/// Retorna la acción del usuario este frame.
pub fn show(
    ui: &mut egui::Ui,
    selected_table: &Option<(String, String)>,
    result: &Option<QueryResult>,
    error: &Option<String>,
    sorts: &mut Vec<SortSpec>,
    sort_indices: &[usize],
    pending_edits: &[PendingEdit],
    editing_cell: &mut Option<EditingCell>,
) -> GridAction {
    match (selected_table, result, error) {
        (None, _, _) => {
            ui.centered_and_justified(|ui| {
                ui.label("Selecciona una tabla en el panel izquierdo");
            });
            return GridAction::None;
        }
        (Some((db, table)), _, Some(err)) => {
            ui.label(format!("{db}.{table}"));
            ui.separator();
            ui.colored_label(egui::Color32::RED, format!("Error: {err}"));
            return GridAction::None;
        }
        (Some(_), None, None) => {
            ui.centered_and_justified(|ui| {
                ui.label("Cargando…");
            });
            return GridAction::None;
        }
        _ => {}
    }

    let (db, table) = selected_table.as_ref().unwrap();
    let data = result.as_ref().unwrap();

    // Barra de info + chips de ordenación activa
    let mut remove_sort_idx: Option<usize> = None;
    let mut clear_all_sorts = false;

    ui.horizontal_wrapped(|ui| {
        ui.strong(format!("{db}.{table}"));
        ui.label(format!("— {} filas", data.rows.len()));

        if !sorts.is_empty() {
            ui.separator();
            ui.label("Orden:");
            for (i, spec) in sorts.iter().enumerate() {
                let arrow = if spec.dir == SortDir::Asc { "↑" } else { "↓" };
                egui::Frame::group(ui.style())
                    .inner_margin(egui::Margin::same(2.0))
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
        return if sort_changed { GridAction::SortChanged } else { GridAction::None };
    }

    let col_count = data.columns.len();
    let row_count = if sort_indices.is_empty() {
        data.rows.len()
    } else {
        sort_indices.len()
    };

    // Señales fuera de closures para evitar conflictos de borrow
    let mut clicked_col: Option<usize> = None;

    // Usamos RefCell para mutabilidad interior en closures anidados de egui
    let edit_cell_rc: RefCell<Option<EditingCell>> = RefCell::new(editing_cell.take());
    let grid_action_rc: RefCell<GridAction> = RefCell::new(GridAction::None);

    // Altura de fila ligeramente mayor cuando hay edición activa
    let row_h = if edit_cell_rc.borrow().is_some() { 22.0 } else { 20.0 };

    TableBuilder::new(ui)
        .striped(true)
        .resizable(true)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .columns(Column::auto().clip(true).at_least(60.0), col_count)
        .header(24.0, |mut header| {
            for (col_i, col_name) in data.columns.iter().enumerate() {
                header.col(|ui| {
                    let sort_pos = sorts.iter().position(|s| s.col_idx == col_i);

                    let mut label = col_name.clone();
                    if let Some(pos) = sort_pos {
                        let arrow =
                            if sorts[pos].dir == SortDir::Asc { " ↑" } else { " ↓" };
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

                    if ui
                        .add(
                            egui::Button::new(egui::RichText::new(label).strong())
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
            body.rows(row_h, row_count, |mut row| {
                let real_idx = if sort_indices.is_empty() {
                    row.index()
                } else {
                    sort_indices[row.index()]
                };
                let row_data = &data.rows[real_idx];

                // ¿Esta fila tiene algún cambio pendiente?
                let row_modified = pending_edits.iter().any(|e| e.row_idx == real_idx);

                for col_i in 0..col_count {
                    row.col(|ui| {
                        let is_editing = edit_cell_rc
                            .borrow()
                            .as_ref()
                            .map_or(false, |e| e.row_idx == real_idx && e.col_idx == col_i);

                        let pending = pending_edits
                            .iter()
                            .find(|e| e.row_idx == real_idx && e.col_idx == col_i);

                        if is_editing {
                            // ── Celda en edición ──────────────────────────
                            let mut ec = edit_cell_rc.borrow_mut();
                            let edit = ec.as_mut().unwrap();
                            let resp = ui.add(
                                egui::TextEdit::singleline(&mut edit.buffer)
                                    .desired_width(f32::INFINITY),
                            );

                            let enter_pressed = resp.has_focus()
                                && ui.input(|i| i.key_pressed(egui::Key::Enter));
                            let committed = resp.lost_focus() || enter_pressed;
                            let cancelled =
                                ui.input(|i| i.key_pressed(egui::Key::Escape));

                            if cancelled {
                                *grid_action_rc.borrow_mut() = GridAction::CellEditCancelled;
                            } else if committed {
                                *grid_action_rc.borrow_mut() = GridAction::CellEditCommitted {
                                    row_idx: real_idx,
                                    col_idx: col_i,
                                    new_value: edit.buffer.clone(),
                                };
                            }
                        } else if let Some(edit) = pending {
                            // ── Celda con cambio pendiente ────────────────
                            let bg_rect = ui.available_rect_before_wrap();
                            // Fondo amarillo
                            ui.painter().rect_filled(
                                bg_rect,
                                0.0,
                                egui::Color32::from_rgb(255, 235, 100),
                            );
                            // Texto del nuevo valor
                            let resp = ui.add(
                                egui::Label::new(&edit.new_value)
                                    .truncate()
                                    .sense(egui::Sense::click()),
                            );
                            // Triángulo naranja en la esquina superior derecha
                            let pts = vec![
                                bg_rect.right_top(),
                                bg_rect.right_top() - egui::vec2(8.0, 0.0),
                                bg_rect.right_top() + egui::vec2(0.0, 8.0),
                            ];
                            ui.painter().add(egui::Shape::convex_polygon(
                                pts,
                                egui::Color32::from_rgb(220, 120, 0),
                                egui::Stroke::NONE,
                            ));

                            if resp.double_clicked() {
                                *grid_action_rc.borrow_mut() = GridAction::CellEditStarted {
                                    row_idx: real_idx,
                                    col_idx: col_i,
                                    current_value: edit.new_value.clone(),
                                };
                            }
                        } else {
                            // ── Celda normal ──────────────────────────────
                            let cell_text = &row_data[col_i];

                            // Si la fila tiene cambios en otras celdas, fondo tenue
                            if row_modified {
                                let bg_rect = ui.available_rect_before_wrap();
                                ui.painter().rect_filled(
                                    bg_rect,
                                    0.0,
                                    egui::Color32::from_rgba_unmultiplied(255, 235, 100, 60),
                                );
                            }

                            let resp = ui.add(
                                egui::Label::new(cell_text)
                                    .truncate()
                                    .sense(egui::Sense::click()),
                            );
                            if resp.double_clicked() {
                                *grid_action_rc.borrow_mut() = GridAction::CellEditStarted {
                                    row_idx: real_idx,
                                    col_idx: col_i,
                                    current_value: cell_text.clone(),
                                };
                            }
                        }
                    });
                }
            });
        });

    // Restaurar el editing_cell (puede haber sido modificado por el TextEdit)
    *editing_cell = edit_cell_rc.into_inner();

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

    // Priorizar la acción de edición sobre el sort
    match grid_action {
        GridAction::None if sort_changed => GridAction::SortChanged,
        other => other,
    }
}

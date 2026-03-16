use std::cell::RefCell;

use eframe::egui;
use egui_extras::{Column, TableBuilder};
use serde::{Deserialize, Serialize};

use crate::db::QueryResult;

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
    pub original: String,
    pub new_value: String,
    /// Si true, el UPDATE usa `= NULL` en lugar de `= 'valor'`.
    pub is_null: bool,
}

#[derive(Debug, Clone)]
pub struct EditingCell {
    pub row_idx: usize,
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
    CellEditStarted { row_idx: usize, col_idx: usize, current_value: String },
    CellEditCommitted { row_idx: usize, col_idx: usize, new_value: String },
    CellSetNull { row_idx: usize, col_idx: usize },
    CellEditCancelled,
    ApplyEdits,
    DiscardEdits,
    // Filtrado desde menú contextual
    FilterBy { sql_fragment: String },
    FilterByClipboard { col_name: String },
    FilterUserInput { col_name: String },
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

/// Devuelve la fecha/hora actual formateada como "YYYY-MM-DD HH:MM:SS".
fn current_datetime() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // Desglose manual de timestamp Unix en componentes de fecha/hora UTC
    let s = secs % 60;
    let m = (secs / 60) % 60;
    let h = (secs / 3600) % 24;
    let days = secs / 86400; // días desde 1970-01-01

    // Calcular año, mes, día a partir de días
    let mut year = 1970u32;
    let mut remaining = days as u32;
    loop {
        let days_in_year = if is_leap(year) { 366 } else { 365 };
        if remaining < days_in_year {
            break;
        }
        remaining -= days_in_year;
        year += 1;
    }
    let months = [31, if is_leap(year) { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut month = 1u32;
    for &days_in_month in &months {
        if remaining < days_in_month {
            break;
        }
        remaining -= days_in_month;
        month += 1;
    }
    let day = remaining + 1;

    format!("{year:04}-{month:02}-{day:02} {h:02}:{m:02}:{s:02}")
}

fn is_leap(year: u32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

// ── Helpers para filtros ──────────────────────────────────────────────────────

/// Escapa un valor para usarlo en una literal de string SQL ('…').
fn esc_sql_val(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
}

/// Escapa un identificador SQL entre backticks.
fn esc_ident(s: &str) -> String {
    format!("`{}`", s.replace('`', "``"))
}

/// Trunca `s` para mostrar en el label de un botón de menú.
fn trunc_display(s: &str, max: usize) -> String {
    let mut chars = s.chars();
    let t: String = chars.by_ref().take(max).collect();
    if chars.next().is_some() { format!("{t}…") } else { t }
}

// ── Ancho inteligente de columnas ────────────────────────────────────────────

/// Calcula el ancho inicial de cada columna en base al nombre de la cabecera
/// y al contenido de las primeras 200 filas. Siempre reserva espacio para el
/// indicador de ordenación ("↑1") para que no se trunque al ordenar.
fn compute_col_widths(ui: &egui::Ui, data: &QueryResult) -> Vec<f32> {
    let font_size = ui.text_style_height(&egui::TextStyle::Body);
    // Ancho estimado por carácter para fuente proporcional
    let char_w = font_size * 0.60;
    // Espacio extra siempre reservado para " ↑1" (peor caso de indicador de orden)
    let sort_reserve = font_size * 1.8;
    // Padding horizontal de celda (botón de cabecera) + scroll margin
    let padding = 8.0;

    let sample = data.rows.len().min(200);

    data.columns
        .iter()
        .enumerate()
        .map(|(col_i, col_name)| {
            let header_w = col_name.len() as f32 * char_w + sort_reserve + padding;

            let max_content = data.rows[..sample]
                .iter()
                .map(|row| row.get(col_i).map_or(0, |v| v.len()))
                .max()
                .unwrap_or(0);
            let content_w = max_content as f32 * char_w + 8.0;

            header_w.max(content_w).clamp(50.0, 400.0)
        })
        .collect()
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
    selected_cell: &mut Option<(usize, usize)>,
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
                    .inner_margin(egui::Margin::same(2))
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

    // Anchos inteligentes por columna basados en cabecera + muestra de contenido
    let col_widths = compute_col_widths(ui, data);
    let total_width: f32 = col_widths.iter().sum::<f32>() + 24.0; // margen para scrollbar

    // Atajos de teclado para la celda seleccionada
    let mut keyboard_action = GridAction::None;
    let no_widget_focused = ui.memory(|m| m.focused().is_none());
    if no_widget_focused && editing_cell.is_none() {
        if let Some((sel_row, sel_col)) = *selected_cell {
            if sel_row < data.rows.len() {
                if let Some(cell_val) = data.rows[sel_row].get(sel_col).cloned() {
                    let ctrl = ui.input(|i| i.modifiers.ctrl);

                    // Ctrl+C: copiar
                    if ctrl && ui.input(|i| i.key_pressed(egui::Key::C)) {
                        ui.ctx().copy_text(cell_val.clone());
                    }
                    // Ctrl+N: NULL
                    if ctrl && ui.input(|i| i.key_pressed(egui::Key::N)) {
                        keyboard_action = GridAction::CellSetNull { row_idx: sel_row, col_idx: sel_col };
                    }
                    // Delete: vacío
                    if ui.input(|i| i.key_pressed(egui::Key::Delete)) {
                        keyboard_action = GridAction::CellEditCommitted {
                            row_idx: sel_row, col_idx: sel_col, new_value: String::new(),
                        };
                    }
                    // F2 o Enter: editar
                    if ui.input(|i| i.key_pressed(egui::Key::F2)) {
                        keyboard_action = GridAction::CellEditStarted {
                            row_idx: sel_row, col_idx: sel_col, current_value: cell_val.clone(),
                        };
                    }
                    // Ctrl+V: pegar
                    let paste = ui.input(|i| {
                        i.events.iter().find_map(|e| {
                            if let egui::Event::Paste(s) = e { Some(s.clone()) } else { None }
                        })
                    });
                    if let Some(text) = paste {
                        keyboard_action = GridAction::CellEditCommitted {
                            row_idx: sel_row, col_idx: sel_col, new_value: text,
                        };
                    }
                    // Escape: deseleccionar
                    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        *selected_cell = None;
                    }
                }
            }
        }
    }

    // Usar un id por tabla para que egui recuerde anchos ajustados por el usuario
    // y los recalcule al cambiar de tabla
    let table_id = format!("{}/{}", db, table);

    // ScrollArea horizontal externa — TableBuilder maneja el scroll vertical interno
    egui::ScrollArea::horizontal()
        .id_salt(format!("{}_hscroll", table_id))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // Forzar ancho mínimo para que el scrollbar aparezca si es necesario
            ui.set_min_width(total_width);

    let mut builder = TableBuilder::new(ui)
        .id_salt(&table_id)
        .striped(true)
        .resizable(true)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center));
    for &w in &col_widths {
        builder = builder.column(Column::initial(w).resizable(true).clip(true).at_least(40.0));
    }
    builder.header(24.0, |mut header| {
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

                            // Auto-foco en el primer frame de edición
                            if edit.needs_focus {
                                resp.request_focus();
                                edit.needs_focus = false;
                            }

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
                            // Texto del nuevo valor (o "(NULL)" si es un SET NULL)
                            let display_text = if edit.is_null {
                                egui::RichText::new("(NULL)").italics().color(
                                    egui::Color32::from_rgb(120, 80, 0),
                                )
                            } else {
                                egui::RichText::new(&edit.new_value)
                            };
                            let resp = ui.add(
                                egui::Label::new(display_text)
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
                                let start_value = if edit.is_null {
                                    String::new()
                                } else {
                                    edit.new_value.clone()
                                };
                                *grid_action_rc.borrow_mut() = GridAction::CellEditStarted {
                                    row_idx: real_idx,
                                    col_idx: col_i,
                                    current_value: start_value,
                                };
                            }
                        } else {
                            // ── Celda normal ──────────────────────────────
                            let cell_text = row_data[col_i].clone();
                            let cell_rect = ui.available_rect_before_wrap();

                            // Fondo tenue si la fila tiene cambios en otras celdas
                            if row_modified {
                                ui.painter().rect_filled(
                                    cell_rect,
                                    0.0,
                                    egui::Color32::from_rgba_unmultiplied(255, 235, 100, 60),
                                );
                            }

                            // Fondo de celda seleccionada
                            let is_selected_cell = selected_cell.as_ref()
                                .map_or(false, |&(r, c)| r == real_idx && c == col_i);
                            if is_selected_cell {
                                ui.painter().rect_filled(
                                    cell_rect, 0.0,
                                    ui.visuals().selection.bg_fill,
                                );
                            }

                            // Texto siempre izquierda — el layout nunca cambia en hover
                            let resp = ui.add(
                                egui::Label::new(&cell_text)
                                    .truncate()
                                    .sense(egui::Sense::click()),
                            );
                            if resp.clicked() {
                                *selected_cell = Some((real_idx, col_i));
                            }
                            if resp.double_clicked() {
                                *grid_action_rc.borrow_mut() = GridAction::CellEditStarted {
                                    row_idx: real_idx,
                                    col_idx: col_i,
                                    current_value: cell_text.clone(),
                                };
                            }

                            // Menú contextual (botón derecho) — estándar en todos los
                            // clientes SQL: HeidiSQL, DataGrip, TablePlus, etc.
                            let is_dt = looks_like_datetime(&cell_text);
                            let ctx = ui.ctx().clone();
                            let ga_rc = &grid_action_rc;
                            let col_name = data.columns[col_i].clone();
                            resp.context_menu(|ui| {
                                if ui.button("📋  Copiar valor").clicked() {
                                    ctx.copy_text(cell_text.clone());
                                    ui.close();
                                }
                                ui.separator();
                                if ui.button("✏  Editar celda").clicked() {
                                    *ga_rc.borrow_mut() = GridAction::CellEditStarted {
                                        row_idx: real_idx, col_idx: col_i,
                                        current_value: cell_text.clone(),
                                    };
                                    ui.close();
                                }
                                if ui.button("∅  Establecer NULL").clicked() {
                                    *ga_rc.borrow_mut() = GridAction::CellSetNull {
                                        row_idx: real_idx, col_idx: col_i,
                                    };
                                    ui.close();
                                }
                                if ui.button("\"\"  Establecer vacío").clicked() {
                                    *ga_rc.borrow_mut() = GridAction::CellEditCommitted {
                                        row_idx: real_idx, col_idx: col_i,
                                        new_value: String::new(),
                                    };
                                    ui.close();
                                }
                                if is_dt && ui.button("🕐  Fecha/hora actual").clicked() {
                                    *ga_rc.borrow_mut() = GridAction::CellEditCommitted {
                                        row_idx: real_idx, col_idx: col_i,
                                        new_value: current_datetime(),
                                    };
                                    ui.close();
                                }

                                // ── Filtrar ──────────────────────────────
                                ui.separator();
                                let esc_col = esc_ident(&col_name);
                                let val_display = trunc_display(&cell_text, 22);
                                let val_escaped = esc_sql_val(&cell_text);

                                if ui.button(format!("🔍  = '{val_display}'")).clicked() {
                                    let frag = format!("{esc_col} = '{val_escaped}'");
                                    *ga_rc.borrow_mut() = GridAction::FilterBy {
                                        sql_fragment: frag,
                                    };
                                    ui.close();
                                }
                                if ui.button(format!("🔍  ≠ '{val_display}'")).clicked() {
                                    let frag = format!("{esc_col} != '{val_escaped}'");
                                    *ga_rc.borrow_mut() = GridAction::FilterBy {
                                        sql_fragment: frag,
                                    };
                                    ui.close();
                                }
                                if ui.button(format!("🔍  LIKE '%{val_display}%'")).clicked() {
                                    let frag = format!("{esc_col} LIKE '%{val_escaped}%'");
                                    *ga_rc.borrow_mut() = GridAction::FilterBy {
                                        sql_fragment: frag,
                                    };
                                    ui.close();
                                }
                                if ui.button(format!("🔍  IS NULL ({col_name})")).clicked() {
                                    let frag = format!("{esc_col} IS NULL");
                                    *ga_rc.borrow_mut() = GridAction::FilterBy {
                                        sql_fragment: frag,
                                    };
                                    ui.close();
                                }
                                if ui.button(format!("🔍  IS NOT NULL ({col_name})")).clicked() {
                                    let frag = format!("{esc_col} IS NOT NULL");
                                    *ga_rc.borrow_mut() = GridAction::FilterBy {
                                        sql_fragment: frag,
                                    };
                                    ui.close();
                                }
                                if ui.button("🔍  Por portapapeles").clicked() {
                                    *ga_rc.borrow_mut() = GridAction::FilterByClipboard {
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
                            });
                        }
                    });
                }
            });
        });

    }); // cierra ScrollArea::horizontal

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

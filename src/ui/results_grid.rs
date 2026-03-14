use eframe::egui;
use egui_extras::{Column, TableBuilder};

use crate::db::QueryResult;

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

/// Muestra el grid de datos.
/// Retorna `true` si el estado de ordenación cambió este frame.
pub fn show(
    ui: &mut egui::Ui,
    selected_table: &Option<(String, String)>,
    result: &Option<QueryResult>,
    error: &Option<String>,
    sorts: &mut Vec<SortSpec>,
    sort_indices: &[usize],
) -> bool {
    let mut sort_changed = false;

    match (selected_table, result, error) {
        (None, _, _) => {
            ui.centered_and_justified(|ui| {
                ui.label("Selecciona una tabla en el panel izquierdo");
            });
        }
        (Some((db, table)), _, Some(err)) => {
            ui.label(format!("{db}.{table}"));
            ui.separator();
            ui.colored_label(egui::Color32::RED, format!("Error: {err}"));
        }
        (Some((db, table)), Some(data), None) => {
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
                return sort_changed;
            }

            let col_count = data.columns.len();
            let row_count = if sort_indices.is_empty() {
                data.rows.len()
            } else {
                sort_indices.len()
            };

            // Recogemos el click de columna fuera de los closures para evitar
            // conflictos de borrow con `sorts` mutable.
            let mut clicked_col: Option<usize> = None;

            TableBuilder::new(ui)
                .striped(true)
                .resizable(true)
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .columns(Column::auto().clip(true).at_least(60.0), col_count)
                .header(24.0, |mut header| {
                    for (col_i, col_name) in data.columns.iter().enumerate() {
                        header.col(|ui| {
                            let sort_pos =
                                sorts.iter().position(|s| s.col_idx == col_i);

                            // Texto de cabecera con indicador de dirección y prioridad
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

                            let clicked = ui
                                .add(
                                    egui::Button::new(egui::RichText::new(label).strong())
                                        .frame(false),
                                )
                                .on_hover_text(hint)
                                .clicked();

                            if clicked {
                                clicked_col = Some(col_i);
                            }
                        });
                    }
                })
                .body(|body| {
                    body.rows(18.0, row_count, |mut row| {
                        let data_idx = if sort_indices.is_empty() {
                            row.index()
                        } else {
                            sort_indices[row.index()]
                        };
                        let row_data = &data.rows[data_idx];
                        for cell in row_data {
                            row.col(|ui| {
                                // truncate() ya muestra el texto completo en tooltip
                                // cuando está cortado — no añadir on_hover_text aquí
                                ui.add(egui::Label::new(cell).truncate());
                            });
                        }
                    });
                });

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
        }
        (Some(_), None, None) => {
            ui.centered_and_justified(|ui| {
                ui.label("Cargando…");
            });
        }
    }

    sort_changed
}

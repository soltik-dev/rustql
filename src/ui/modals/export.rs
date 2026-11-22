//! Export dialog: configure the destination, follow the progress table by
//! table and show the result.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use eframe::egui;

use super::{ModalAction, dialog};
use crate::db::Engine;
use crate::db::export::{ExportDestination, ExportEvent, ExportOptions};
use crate::i18n::t;
use crate::profiles::ConnectionProfile;
use crate::ui::icons::*;
use crate::ui::theme;

#[derive(Debug, Clone)]
pub struct TableExportEntry {
    pub name: String,
    pub selected: bool,
    pub state: TableExportState,
}

#[derive(Debug, Clone)]
pub enum TableExportState {
    Pending,
    InProgress,
    Done { rows: u64 },
    Error(String),
}

/// In-flight data while the export is running.
/// Receiver<ExportEvent> does not implement Debug, so a wrapper is used.
pub struct ExportRunning {
    pub receiver: std::sync::mpsc::Receiver<ExportEvent>,
    pub cancel_flag: Arc<AtomicBool>,
    pub rows_done: u64,
    pub done_count: usize,
}

impl std::fmt::Debug for ExportRunning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ExportRunning {{ done: {}, rows: {} }}",
            self.done_count, self.rows_done
        )
    }
}

#[derive(Debug)]
pub enum ExportPhase {
    Configuring,
    Running(ExportRunning),
    Done {
        total_tables: usize,
        total_rows: u64,
        elapsed_ms: u64,
    },
    Cancelled,
    /// Fatal error that aborted the whole export.
    Failed(String),
}

#[allow(clippy::too_many_arguments)]
pub(super) fn show_export_data(
    ctx: &egui::Context,
    engine: Engine,
    source_db: &str,
    _source_table: &Option<String>,
    tables: &mut [TableExportEntry],
    databases: &[String],
    profiles: &[ConnectionProfile],
    destination: &mut ExportDestination,
    target_db_name: &mut String,
    profile_idx: &mut usize,
    options: &mut ExportOptions,
    phase: &mut ExportPhase,
) -> ModalAction {
    let mut action = ModalAction::None;
    let mut open = true;

    let title = format!("{EXPORT}  {}", t!("export-title", db = source_db));
    // While running it only closes with Cancel: closing it would leave the
    // export running with nothing showing it
    let closable = !matches!(phase, ExportPhase::Running(_));

    dialog(ctx, &title, 620.0, closable.then_some(&mut open), |ui| {
        // Fixed width: without it, the phases with StripBuilder may get an
        // unbounded width from auto-size and propagate NaN to the layout.
        ui.set_width(600.0);
        match phase {
            ExportPhase::Configuring => {
                action = render_export_config(
                    ui,
                    engine,
                    source_db,
                    tables,
                    databases,
                    profiles,
                    destination,
                    target_db_name,
                    profile_idx,
                    options,
                );
            }
            ExportPhase::Running(running) => {
                action = render_export_running(ui, tables, running, destination);
            }
            ExportPhase::Done {
                total_tables,
                total_rows,
                elapsed_ms,
            } => {
                action = render_export_done(
                    ui,
                    tables,
                    *total_tables,
                    *total_rows,
                    *elapsed_ms,
                    destination,
                );
            }
            ExportPhase::Cancelled => {
                action = render_export_cancelled(ui, tables);
            }
            ExportPhase::Failed(error) => {
                action = render_export_failed(ui, error);
            }
        }
    });

    if !open {
        action = ModalAction::CloseExport;
    }
    action
}

#[allow(clippy::too_many_arguments)]
fn render_export_config(
    ui: &mut egui::Ui,
    engine: Engine,
    source_db: &str,
    tables: &mut [TableExportEntry],
    databases: &[String],
    profiles: &[ConnectionProfile],
    destination: &mut ExportDestination,
    target_db_name: &mut String,
    profile_idx: &mut usize,
    options: &mut ExportOptions,
) -> ModalAction {
    let mut action = ModalAction::None;
    let selected_count = tables.iter().filter(|t| t.selected).count();

    // ── Two-column layout ──────────────────────────────────────────────────
    // Bounded height: the strip would fill all the available height and,
    // with the footer below, the window would auto-grow every frame forever.
    ui.scope(|ui| {
        ui.set_max_height(350.0);
        egui_extras::StripBuilder::new(ui)
            .size(egui_extras::Size::exact(190.0))
            .size(egui_extras::Size::exact(8.0))
            .size(egui_extras::Size::remainder())
            .horizontal(|mut strip| {
                // ── Left column: table list ────────────────────────────────────
                strip.cell(|ui| {
                    ui.heading(t!(
                        "export-tables-heading",
                        selected = selected_count,
                        total = tables.len()
                    ));
                    ui.add_space(2.0);
                    ui.horizontal(|ui| {
                        if ui.small_button(t!("export-select-all")).clicked() {
                            tables.iter_mut().for_each(|t| t.selected = true);
                        }
                        if ui.small_button(t!("export-select-none")).clicked() {
                            tables.iter_mut().for_each(|t| t.selected = false);
                        }
                    });
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .max_height(280.0)
                        .show(ui, |ui| {
                            for entry in tables.iter_mut() {
                                ui.checkbox(&mut entry.selected, &entry.name);
                            }
                        });
                });

                // ── Separator ──────────────────────────────────────────────────
                strip.cell(|ui| {
                    ui.separator();
                });

                // ── Right column: destination + options ────────────────────────
                strip.cell(|ui| {
                    ui.heading(t!("export-destination"));
                    ui.add_space(4.0);

                    // Radio buttons for the destination type
                    let is_sqlfile = matches!(destination, ExportDestination::SqlFile { .. });
                    let is_folder = matches!(destination, ExportDestination::Folder { .. });
                    let is_samedb = matches!(destination, ExportDestination::SameServerDb { .. });
                    let is_remote = matches!(destination, ExportDestination::RemoteServer { .. });

                    if ui.radio(is_sqlfile, t!("export-to-file")).clicked() && !is_sqlfile {
                        *destination = ExportDestination::SqlFile {
                            path: String::new(),
                        };
                    }
                    if is_sqlfile {
                        if let ExportDestination::SqlFile { path } = destination {
                            ui.horizontal(|ui| {
                                ui.add(
                                    egui::TextEdit::singleline(path)
                                        .hint_text("/ruta/archivo.sql")
                                        .desired_width(200.0),
                                );
                                if ui.small_button("…").clicked()
                                    && let Some(p) = rfd::FileDialog::new()
                                        .add_filter("SQL", &["sql"])
                                        .set_file_name(format!("{source_db}.sql"))
                                        .save_file()
                                {
                                    *path = p.to_string_lossy().to_string();
                                }
                            });
                        }
                        ui.add_space(4.0);
                    }

                    if ui.radio(is_folder, t!("export-to-folder")).clicked() && !is_folder {
                        *destination = ExportDestination::Folder {
                            path: String::new(),
                        };
                    }
                    if is_folder {
                        if let ExportDestination::Folder { path } = destination {
                            ui.horizontal(|ui| {
                                ui.add(
                                    egui::TextEdit::singleline(path)
                                        .hint_text("/ruta/carpeta/")
                                        .desired_width(200.0),
                                );
                                if ui.small_button("…").clicked()
                                    && let Some(p) = rfd::FileDialog::new().pick_folder()
                                {
                                    *path = p.to_string_lossy().to_string();
                                }
                            });
                        }
                        ui.add_space(4.0);
                    }

                    if ui.radio(is_samedb, t!("export-to-database")).clicked() && !is_samedb {
                        // Preselect the first different database if there is one
                        let first_other = databases
                            .iter()
                            .find(|d| d.as_str() != source_db)
                            .cloned()
                            .unwrap_or_default();
                        *destination = ExportDestination::SameServerDb {
                            target_db: first_other,
                        };
                    }
                    if is_samedb && let ExportDestination::SameServerDb { target_db } = destination
                    {
                        egui::ComboBox::from_id_salt("export_dest_db")
                            .selected_text(if target_db.is_empty() {
                                t!("export-choose")
                            } else {
                                target_db.clone()
                            })
                            .show_ui(ui, |ui| {
                                for db in databases.iter().filter(|d| d.as_str() != source_db) {
                                    ui.selectable_value(target_db, db.clone(), db);
                                }
                            });
                        ui.add_space(4.0);
                    }

                    if ui.radio(is_remote, t!("export-to-server")).clicked() && !is_remote {
                        let params = profiles.first().map(|p| p.to_params()).unwrap_or_default();
                        *destination = ExportDestination::RemoteServer {
                            params,
                            target_db: source_db.to_string(),
                        };
                        *profile_idx = 0;
                        *target_db_name = source_db.to_string();
                    }
                    if is_remote {
                        egui::ComboBox::from_id_salt("export_dest_profile")
                            .selected_text(
                                profiles
                                    .get(*profile_idx)
                                    .map_or_else(|| t!("export-no-profiles"), |p| p.name.clone()),
                            )
                            .show_ui(ui, |ui| {
                                for (i, prof) in profiles.iter().enumerate() {
                                    if ui.selectable_label(*profile_idx == i, &prof.name).clicked()
                                    {
                                        *profile_idx = i;
                                        if let ExportDestination::RemoteServer { params, .. } =
                                            destination
                                        {
                                            *params = prof.to_params();
                                        }
                                    }
                                }
                            });
                        if profiles.is_empty() {
                            ui.label(
                                egui::RichText::new(t!("export-no-profiles-hint"))
                                    .color(theme::get(ui).text_muted)
                                    .small(),
                            );
                        }
                        ui.horizontal(|ui| {
                            ui.label(t!("export-target-database"));
                            ui.add(egui::TextEdit::singleline(target_db_name).desired_width(130.0));
                        });
                        if let ExportDestination::RemoteServer { target_db, .. } = destination {
                            *target_db = target_db_name.clone();
                        }
                        ui.add_space(4.0);
                    }

                    ui.separator();
                    ui.heading(t!("export-options"));
                    ui.add_space(2.0);
                    ui.checkbox(&mut options.include_create, t!("export-include-create"));
                    ui.checkbox(&mut options.include_data, t!("export-include-data"));
                    ui.checkbox(&mut options.drop_if_exists, "DROP TABLE IF EXISTS");
                    // What does not fit, where what to export is chosen
                    if engine == Engine::Postgres {
                        ui.add_space(6.0);
                        ui.label(
                            egui::RichText::new(t!("export-postgres-scope"))
                                .small()
                                .color(theme::get(ui).text_muted),
                        );
                    }
                });
            });
    });

    // ── Footer ─────────────────────────────────────────────────────────────
    ui.separator();
    ui.horizontal(|ui| {
        if ui.button(t!("common-cancel")).clicked() {
            action = ModalAction::CloseExport;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let can_export =
                selected_count > 0 && is_destination_valid(destination, !profiles.is_empty());
            let btn = theme::primary_button_enabled(
                ui,
                can_export,
                format!("{EXPORT}  {} →", t!("export-button")),
            );
            if btn.clicked() {
                let sel_tables = tables
                    .iter()
                    .filter(|t| t.selected)
                    .map(|t| t.name.clone())
                    .collect();
                action = ModalAction::StartExport {
                    source_db: source_db.to_string(),
                    tables: sel_tables,
                    destination: destination.clone(),
                    options: options.clone(),
                };
            }
            if !can_export && selected_count == 0 {
                ui.label(
                    egui::RichText::new(t!("export-select-a-table"))
                        .color(theme::get(ui).text_muted)
                        .small(),
                );
            }
        });
    });

    action
}

/// Without profiles, the remote destination points to no chosen server
/// (its parameters would be the defaults, root@127.0.0.1).
fn is_destination_valid(dest: &ExportDestination, has_profiles: bool) -> bool {
    match dest {
        ExportDestination::SqlFile { path } => !path.is_empty(),
        ExportDestination::Folder { path } => !path.is_empty(),
        ExportDestination::SameServerDb { target_db } => !target_db.is_empty(),
        ExportDestination::RemoteServer { target_db, .. } => has_profiles && !target_db.is_empty(),
    }
}

fn destination_label(dest: &ExportDestination) -> String {
    match dest {
        ExportDestination::SqlFile { path } => path.clone(),
        ExportDestination::Folder { path } => path.clone(),
        ExportDestination::SameServerDb { target_db } => t!("db-info-title", db = target_db),
        ExportDestination::RemoteServer { target_db, params } => {
            format!("{}:{} → {target_db}", params.host, params.port)
        }
    }
}

fn render_export_running(
    ui: &mut egui::Ui,
    tables: &[TableExportEntry],
    running: &ExportRunning,
    destination: &ExportDestination,
) -> ModalAction {
    let mut action = ModalAction::None;
    let total = tables.iter().filter(|t| t.selected).count();
    let fraction = if total > 0 {
        running.done_count as f32 / total as f32
    } else {
        0.0
    };

    // Progress bar
    ui.add(egui::ProgressBar::new(fraction).text(t!(
        "export-progress",
        done = running.done_count,
        total = total
    )));
    ui.add_space(6.0);

    ui.scope(|ui| {
        ui.set_max_height(260.0);
        egui_extras::StripBuilder::new(ui)
            .size(egui_extras::Size::exact(220.0))
            .size(egui_extras::Size::exact(8.0))
            .size(egui_extras::Size::remainder())
            .horizontal(|mut strip| {
                // Table list with status
                strip.cell(|ui| {
                    egui::ScrollArea::vertical()
                        .max_height(240.0)
                        .show(ui, |ui| {
                            for entry in tables.iter().filter(|t| t.selected) {
                                let (icon, color) = match &entry.state {
                                    TableExportState::Pending => ("◌", theme::get(ui).text_muted),
                                    TableExportState::InProgress => ("⟳", theme::get(ui).warning),
                                    TableExportState::Done { .. } => ("✓", theme::get(ui).success),
                                    TableExportState::Error(_) => ("✗", theme::get(ui).danger),
                                };
                                ui.horizontal(|ui| {
                                    ui.colored_label(color, icon);
                                    let name = egui::RichText::new(&entry.name);
                                    ui.label(match &entry.state {
                                        TableExportState::Done { .. } => {
                                            name.color(theme::get(ui).success)
                                        }
                                        _ => name,
                                    });
                                    if let TableExportState::Done { rows } = &entry.state {
                                        ui.label(
                                            egui::RichText::new(format_rows(*rows))
                                                .color(theme::get(ui).text_muted)
                                                .small(),
                                        );
                                    }
                                    if let TableExportState::Error(e) = &entry.state {
                                        ui.label(
                                            egui::RichText::new(e)
                                                .color(theme::get(ui).danger)
                                                .small(),
                                        )
                                        .on_hover_text(e);
                                    }
                                });
                            }
                        });
                });

                strip.cell(|ui| {
                    ui.separator();
                });

                // Right panel: statistics
                strip.cell(|ui| {
                    ui.label(egui::RichText::new(t!("export-destination-label")).strong());
                    ui.label(destination_label(destination));
                    ui.add_space(8.0);
                    ui.label(t!(
                        "export-tables-done",
                        done = running.done_count,
                        total = total
                    ));
                    ui.label(t!(
                        "export-rows-done",
                        rows = format_rows(running.rows_done)
                    ));
                });
            });
    });

    ui.separator();
    ui.horizontal(|ui| {
        if ui.button(t!("export-cancel")).clicked() {
            action = ModalAction::CancelExport;
        }
    });
    action
}

fn render_export_done(
    ui: &mut egui::Ui,
    tables: &[TableExportEntry],
    total_tables: usize,
    total_rows: u64,
    elapsed_ms: u64,
    destination: &ExportDestination,
) -> ModalAction {
    let mut action = ModalAction::None;
    let error_count = tables
        .iter()
        .filter(|t| t.selected && matches!(t.state, TableExportState::Error(_)))
        .count();

    if error_count == 0 {
        ui.add(egui::ProgressBar::new(1.0).text(format!("✓ {}", t!("export-completed"))));
    } else {
        ui.add(
            egui::ProgressBar::new(1.0)
                .text(format!(
                    "⚠ {}",
                    t!("export-completed-errors", count = error_count)
                ))
                .fill(theme::get(ui).warning),
        );
    }
    ui.add_space(6.0);

    ui.scope(|ui| {
        ui.set_max_height(260.0);
        egui_extras::StripBuilder::new(ui)
            .size(egui_extras::Size::exact(220.0))
            .size(egui_extras::Size::exact(8.0))
            .size(egui_extras::Size::remainder())
            .horizontal(|mut strip| {
                strip.cell(|ui| {
                    egui::ScrollArea::vertical()
                        .max_height(240.0)
                        .show(ui, |ui| {
                            for entry in tables.iter().filter(|t| t.selected) {
                                let (icon, color) = match &entry.state {
                                    TableExportState::Done { .. } => ("✓", theme::get(ui).success),
                                    TableExportState::Error(_) => ("✗", theme::get(ui).danger),
                                    _ => ("◌", theme::get(ui).text_muted),
                                };
                                ui.horizontal(|ui| {
                                    ui.colored_label(color, icon);
                                    ui.label(&entry.name);
                                    if let TableExportState::Done { rows } = &entry.state {
                                        ui.label(
                                            egui::RichText::new(format_rows(*rows))
                                                .color(theme::get(ui).text_muted)
                                                .small(),
                                        );
                                    }
                                    if let TableExportState::Error(e) = &entry.state {
                                        ui.label(
                                            egui::RichText::new(e)
                                                .color(theme::get(ui).danger)
                                                .small(),
                                        )
                                        .on_hover_text(e);
                                    }
                                });
                            }
                        });
                });

                strip.cell(|ui| {
                    ui.separator();
                });

                strip.cell(|ui| {
                    if error_count == 0 {
                        ui.label(
                            egui::RichText::new(t!("export-done"))
                                .color(theme::get(ui).success)
                                .strong(),
                        );
                    } else {
                        ui.label(
                            egui::RichText::new(t!("export-done-errors"))
                                .color(theme::get(ui).warning)
                                .strong(),
                        );
                    }
                    ui.add_space(8.0);
                    ui.label(t!("export-summary-tables", count = total_tables));
                    if error_count > 0 {
                        ui.label(
                            egui::RichText::new(t!("export-summary-errors", count = error_count))
                                .color(theme::get(ui).danger),
                        );
                    }
                    ui.label(t!("export-summary-rows", rows = format_rows(total_rows)));
                    ui.label(t!(
                        "export-summary-time",
                        seconds = format!("{:.1}", elapsed_ms as f64 / 1000.0)
                    ));
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new(t!("export-destination-label")).strong());
                    ui.label(destination_label(destination));
                });
            });
    });

    ui.separator();
    ui.horizontal(|ui| {
        if ui.button(t!("common-close")).clicked() {
            action = ModalAction::CloseExport;
        }
    });
    action
}

fn render_export_cancelled(ui: &mut egui::Ui, tables: &[TableExportEntry]) -> ModalAction {
    let mut action = ModalAction::None;

    ui.add(egui::ProgressBar::new(0.0).text(format!("✗ {}", t!("export-cancelled"))));
    ui.add_space(6.0);

    egui::ScrollArea::vertical()
        .max_height(260.0)
        .show(ui, |ui| {
            for entry in tables.iter().filter(|t| t.selected) {
                let (icon, color) = match &entry.state {
                    TableExportState::Done { .. } => ("✓", theme::get(ui).success),
                    TableExportState::Error(_) => ("✗", theme::get(ui).danger),
                    _ => ("◌", theme::get(ui).text_muted),
                };
                ui.horizontal(|ui| {
                    ui.colored_label(color, icon);
                    ui.label(&entry.name);
                });
            }
        });

    ui.separator();
    ui.horizontal(|ui| {
        if ui.button(t!("common-close")).clicked() {
            action = ModalAction::CloseExport;
        }
    });
    action
}

fn render_export_failed(ui: &mut egui::Ui, error: &str) -> ModalAction {
    let mut action = ModalAction::None;

    ui.add(egui::ProgressBar::new(0.0).text(format!("✗ {}", t!("error-title"))));
    ui.add_space(6.0);

    ui.label(
        egui::RichText::new(t!("export-failed"))
            .color(theme::get(ui).danger)
            .strong(),
    );
    ui.add_space(4.0);
    ui.label(error.to_string());

    ui.separator();
    ui.horizontal(|ui| {
        if ui.button(t!("common-close")).clicked() {
            action = ModalAction::CloseExport;
        }
    });
    action
}

fn format_rows(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M {}", n as f64 / 1_000_000.0, t!("export-rows-unit"))
    } else if n >= 1_000 {
        format!("{:.1}K {}", n as f64 / 1_000.0, t!("export-rows-unit"))
    } else {
        format!("{n} {}", t!("export-rows-unit"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destino_remoto_sin_perfiles_no_es_valido() {
        let dest = ExportDestination::RemoteServer {
            params: Default::default(),
            target_db: "copia".into(),
        };
        assert!(!is_destination_valid(&dest, false));
        assert!(is_destination_valid(&dest, true));
    }
}

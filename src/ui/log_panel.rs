use std::collections::VecDeque;

use eframe::egui;

use crate::i18n::t;
use crate::log::LogEntry;
use crate::ui::icons::{CODE, COPY};
use crate::ui::theme;

/// What the user asks for from the menu of a log entry.
#[derive(Debug, Default, PartialEq)]
pub enum LogAction {
    #[default]
    None,
    /// Open the query in a new SQL tab.
    OpenInSql(String),
}

/// The user cancelled the query: it is not an error to explain.
fn cancelled(entry: &LogEntry) -> bool {
    entry.error.as_deref() == Some(crate::db::QUERY_CANCELLED)
}

/// Query log of the connection. Each entry has a context menu to copy its
/// query (or its error) and to reopen it in the editor.
pub fn show(ui: &mut egui::Ui, log: &VecDeque<LogEntry>) -> LogAction {
    let mut action = LogAction::None;
    let p = theme::get(ui);
    ui.horizontal(|ui| {
        theme::section_label(ui, &t!("log-title"));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                egui::RichText::new(t!("log-entry-count", count = log.len())).color(p.text_muted),
            );
        });
    });
    ui.separator();

    egui::ScrollArea::vertical()
        .stick_to_bottom(true)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (idx, entry) in log.iter().enumerate() {
                let row = ui.horizontal(|ui| {
                    // Time
                    ui.monospace(egui::RichText::new(&entry.time).color(p.text_muted));
                    ui.separator();

                    // Status
                    if cancelled(entry) {
                        theme::badge(ui, &t!("log-cancelled"), p.warning);
                    } else if entry.error.is_some() {
                        theme::badge(ui, &t!("log-error"), p.danger);
                    } else {
                        let rows = entry.rows.unwrap_or(0);
                        theme::badge(ui, &t!("log-rows", count = rows), p.success);
                    }

                    // Duration
                    ui.separator();
                    ui.label(
                        egui::RichText::new(format!("{}ms", entry.duration_ms)).color(p.text_muted),
                    );
                    ui.separator();

                    // Truncated query: in full, in the row's tooltip
                    ui.add(egui::Label::new(&entry.query).truncate());
                });
                // The whole row responds to right clicks. It sits above its
                // labels, so it provides the tooltip itself
                ui.interact(
                    row.response.rect,
                    ui.make_persistent_id(("log_entry", idx)),
                    egui::Sense::click(),
                )
                .on_hover_text(&entry.query)
                .context_menu(|ui| {
                    if ui
                        .button(format!("{COPY}  {}", t!("log-copy-query")))
                        .clicked()
                    {
                        ui.ctx().copy_text(entry.query.clone());
                        ui.close();
                    }
                    if let Some(err) = &entry.error
                        && ui
                            .button(format!("{COPY}  {}", t!("log-copy-error")))
                            .clicked()
                    {
                        ui.ctx().copy_text(err.clone());
                        ui.close();
                    }
                    ui.separator();
                    if ui
                        .button(format!("{CODE}  {}", t!("log-open-in-sql-tab")))
                        .clicked()
                    {
                        action = LogAction::OpenInSql(entry.query.clone());
                        ui.close();
                    }
                });

                // Expanded error
                if let Some(err) = &entry.error
                    && !cancelled(entry)
                {
                    ui.indent("log_err", |ui| {
                        ui.colored_label(p.danger, err);
                    });
                }
            }
        });
    action
}

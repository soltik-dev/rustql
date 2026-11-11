use std::collections::VecDeque;

use eframe::egui;

use crate::log::LogEntry;
use crate::ui::icons::{CODE, COPY};
use crate::ui::theme;

/// Lo que el usuario pide desde el menú de una entrada del log.
#[derive(Debug, Default, PartialEq)]
pub enum LogAction {
    #[default]
    None,
    /// Abrir la consulta en una pestaña SQL nueva.
    OpenInSql(String),
}

/// La consulta la canceló el usuario: no es un error que explicar.
fn cancelled(entry: &LogEntry) -> bool {
    entry.error.as_deref() == Some(crate::db::QUERY_CANCELLED)
}

/// Log de consultas de la conexión. Cada entrada tiene un menú contextual
/// para copiar su consulta (o su error) y para reabrirla en el editor.
pub fn show(ui: &mut egui::Ui, log: &VecDeque<LogEntry>) -> LogAction {
    let mut action = LogAction::None;
    let p = theme::get(ui);
    ui.horizontal(|ui| {
        theme::section_label(ui, "Log de consultas");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(egui::RichText::new(format!("{} entradas", log.len())).color(p.text_muted));
        });
    });
    ui.separator();

    egui::ScrollArea::vertical()
        .stick_to_bottom(true)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (idx, entry) in log.iter().enumerate() {
                let row = ui.horizontal(|ui| {
                    // Hora
                    ui.monospace(egui::RichText::new(&entry.time).color(p.text_muted));
                    ui.separator();

                    // Estado
                    if cancelled(entry) {
                        theme::badge(ui, "CANCELADA", p.warning);
                    } else if entry.error.is_some() {
                        theme::badge(ui, "ERROR", p.danger);
                    } else {
                        let rows = entry.rows.unwrap_or(0);
                        theme::badge(ui, &format!("{rows} filas"), p.success);
                    }

                    // Duración
                    ui.separator();
                    ui.label(
                        egui::RichText::new(format!("{}ms", entry.duration_ms)).color(p.text_muted),
                    );
                    ui.separator();

                    // Query truncada: entera, en el tooltip de la fila
                    ui.add(egui::Label::new(&entry.query).truncate());
                });
                // Toda la fila responde al clic derecho. Queda por encima de
                // sus etiquetas, así que el tooltip lo pone ella
                ui.interact(
                    row.response.rect,
                    ui.make_persistent_id(("log_entry", idx)),
                    egui::Sense::click(),
                )
                .on_hover_text(&entry.query)
                .context_menu(|ui| {
                    if ui.button(format!("{COPY}  Copiar la consulta")).clicked() {
                        ui.ctx().copy_text(entry.query.clone());
                        ui.close();
                    }
                    if let Some(err) = &entry.error
                        && ui.button(format!("{COPY}  Copiar el error")).clicked()
                    {
                        ui.ctx().copy_text(err.clone());
                        ui.close();
                    }
                    ui.separator();
                    if ui
                        .button(format!("{CODE}  Abrir en una pestaña SQL"))
                        .clicked()
                    {
                        action = LogAction::OpenInSql(entry.query.clone());
                        ui.close();
                    }
                });

                // Error expandido
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

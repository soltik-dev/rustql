use std::collections::VecDeque;

use eframe::egui;

use crate::log::LogEntry;
use crate::ui::theme;

pub fn show(ui: &mut egui::Ui, log: &VecDeque<LogEntry>) {
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
            for entry in log {
                ui.horizontal(|ui| {
                    // Hora
                    ui.monospace(egui::RichText::new(&entry.time).color(p.text_muted));
                    ui.separator();

                    // Estado
                    if entry.error.is_some() {
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

                    // Query (truncada; egui ya muestra el tooltip automáticamente cuando se trunca)
                    ui.add(egui::Label::new(&entry.query).truncate());
                });

                // Error expandido
                if let Some(err) = &entry.error {
                    ui.indent("log_err", |ui| {
                        ui.colored_label(p.danger, err);
                    });
                }
            }
        });
}

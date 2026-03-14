use eframe::egui;

use crate::log::LogEntry;

pub fn show(ui: &mut egui::Ui, log: &[LogEntry]) {
    ui.horizontal(|ui| {
        ui.strong("Log de consultas");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(format!("{} entradas", log.len()));
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
                    ui.monospace(&entry.time);
                    ui.separator();

                    // Estado
                    if entry.error.is_some() {
                        ui.colored_label(egui::Color32::from_rgb(220, 60, 60), "ERROR");
                    } else {
                        let rows = entry.rows.unwrap_or(0);
                        ui.colored_label(
                            egui::Color32::from_rgb(80, 200, 120),
                            format!("{rows} filas"),
                        );
                    }

                    // Duración
                    ui.separator();
                    ui.label(format!("{}ms", entry.duration_ms));
                    ui.separator();

                    // Query (truncada, tooltip con texto completo)
                    ui.add(egui::Label::new(&entry.query).truncate())
                        .on_hover_text(&entry.query);
                });

                // Error expandido
                if let Some(err) = &entry.error {
                    ui.indent("log_err", |ui| {
                        ui.colored_label(egui::Color32::from_rgb(220, 60, 60), err);
                    });
                }
            }
        });
}

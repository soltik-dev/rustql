use eframe::egui;

use crate::ui::theme;

const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Barra inferior estilo VSCode con botones de panel, versión y estado de conexión.
pub fn show(
    ui: &mut egui::Ui,
    show_sidebar: &mut bool,
    show_log: &mut bool,
    show_settings: &mut bool,
    host: Option<&str>,    // Some("host:port") si hay conexión, None si no
    db_name: Option<&str>, // BD activa si la hay
) {
    let p = theme::get(ui);
    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.toggle_value(show_sidebar, "Esquema");
        ui.separator();
        ui.toggle_value(show_log, "Log");

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.toggle_value(show_settings, "Ajustes");
            ui.separator();
            // Versión
            ui.label(egui::RichText::new(format!("v{APP_VERSION}")).color(p.text_disabled));
            ui.separator();
            // Estado de conexión (texto primero: en RTL queda a la derecha del punto)
            match host {
                Some(h) => {
                    let label = if let Some(db) = db_name {
                        format!("{h}  /  {db}")
                    } else {
                        h.to_owned()
                    };
                    ui.label(egui::RichText::new(label).color(p.text_muted));
                    status_dot(ui, p.success);
                }
                None => {
                    ui.label(egui::RichText::new("Sin conexión").color(p.text_muted));
                    status_dot(ui, p.danger);
                }
            }
        });
    });
}

fn status_dot(ui: &mut egui::Ui, color: egui::Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(9.0, 9.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 3.5, color);
}

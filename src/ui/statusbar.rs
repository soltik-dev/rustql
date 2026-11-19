use eframe::egui;

use crate::db::Security;
use crate::session::ConnectionStatus;
use crate::ui::icons::{LOCK, UNLOCK};
use crate::ui::theme;

const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// VSCode-style bottom bar with panel buttons, version and connection status.
pub fn show(
    ui: &mut egui::Ui,
    show_sidebar: &mut bool,
    show_log: &mut bool,
    show_settings: &mut bool,
    status: &ConnectionStatus,
) {
    let p = theme::get(ui);
    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.toggle_value(show_sidebar, "Esquema");
        ui.separator();
        ui.toggle_value(show_log, "Log");

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.toggle_value(show_settings, "Ajustes");
            ui.separator();
            // Version
            ui.label(egui::RichText::new(format!("v{APP_VERSION}")).color(p.text_disabled));
            ui.separator();
            // Connection status (text first: in RTL it ends up to the right of the dot)
            match &status.host {
                Some(h) => {
                    let label = if let Some(db) = &status.database {
                        format!("{h}  /  {db}")
                    } else {
                        h.clone()
                    };
                    ui.label(egui::RichText::new(label).color(p.text_muted));
                    // Padlock: closed if the connection is encrypted
                    if let Some((security, mode)) = status.security {
                        let (icon, color) = match security {
                            Security::Encrypted => (LOCK, p.success),
                            Security::Local => (UNLOCK, p.text_disabled),
                            Security::Plain => (UNLOCK, p.warning),
                        };
                        ui.label(egui::RichText::new(icon).color(color))
                            .on_hover_text(security.description(mode));
                    }
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

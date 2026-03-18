use eframe::egui;

const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Barra inferior estilo VSCode con botones de panel, versión y estado de conexión.
pub fn show(
    ui: &mut egui::Ui,
    show_sidebar: &mut bool,
    show_log: &mut bool,
    show_settings: &mut bool,
    host: Option<&str>,   // Some("host:port") si hay conexión, None si no
    db_name: Option<&str>, // BD activa si la hay
) {
    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.toggle_value(show_sidebar, "Esquema");
        ui.separator();
        ui.toggle_value(show_log, "Log");

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.toggle_value(show_settings, "Ajustes");
            ui.separator();
            // Versión
            ui.label(format!("v{APP_VERSION}"));
            ui.separator();
            // Estado de conexión
            match host {
                Some(h) => {
                    let color = egui::Color32::from_rgb(100, 210, 120);
                    let label = if let Some(db) = db_name {
                        format!("🟢 {h}  /  {db}")
                    } else {
                        format!("🟢 {h}")
                    };
                    ui.colored_label(color, label);
                }
                None => {
                    ui.colored_label(egui::Color32::from_rgb(180, 80, 80), "● Sin conexión");
                }
            }
        });
    });
}

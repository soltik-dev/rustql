use eframe::egui;

/// Barra inferior estilo VSCode con botones de panel.
pub fn show(
    ui: &mut egui::Ui,
    show_sidebar: &mut bool,
    show_log: &mut bool,
    show_settings: &mut bool,
) {
    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.toggle_value(show_sidebar, "Esquema");
        ui.separator();
        ui.toggle_value(show_log, "Log");

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.toggle_value(show_settings, "Ajustes");
        });
    });
}

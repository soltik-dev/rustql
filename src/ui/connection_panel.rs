use eframe::egui;

use crate::db::ConnectionParams;

/// Muestra el formulario de conexión.
/// Retorna `true` si el usuario pulsó el botón Conectar.
pub fn show(
    ui: &mut egui::Ui,
    params: &mut ConnectionParams,
    port_str: &mut String,
    error_message: &Option<String>,
) -> bool {
    let mut connect_clicked = false;

    ui.add_space(60.0);

    ui.vertical_centered(|ui| {
        ui.heading("RustQL");
        ui.add_space(4.0);
        ui.label("Conectar a base de datos");
        ui.add_space(24.0);

        egui::Grid::new("connection_form")
            .num_columns(2)
            .spacing([16.0, 10.0])
            .min_col_width(80.0)
            .show(ui, |ui| {
                ui.label("Host:");
                ui.add(egui::TextEdit::singleline(&mut params.host).min_size(egui::vec2(200.0, 0.0)));
                ui.end_row();

                ui.label("Puerto:");
                ui.add(egui::TextEdit::singleline(port_str).min_size(egui::vec2(200.0, 0.0)));
                ui.end_row();

                ui.label("Usuario:");
                ui.add(egui::TextEdit::singleline(&mut params.user).min_size(egui::vec2(200.0, 0.0)));
                ui.end_row();

                ui.label("Contraseña:");
                ui.add(
                    egui::TextEdit::singleline(&mut params.password)
                        .password(true)
                        .min_size(egui::vec2(200.0, 0.0)),
                );
                ui.end_row();
            });

        ui.add_space(12.0);

        if let Some(err) = error_message {
            ui.colored_label(egui::Color32::RED, err);
            ui.add_space(8.0);
        }

        if ui.button("  Conectar  ").clicked() {
            connect_clicked = true;
        }
    });

    connect_clicked
}

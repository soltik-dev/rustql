use eframe::egui;

use crate::db::ConnectionParams;
use crate::profiles::ConnectionProfile;

/// Resultado de la interacción con el formulario de conexión.
#[derive(Default)]
pub struct FormResult {
    pub connect: bool,
    pub save_profile: bool,
    pub delete_profile: bool,
}

/// Panel izquierdo: lista de favoritos guardados.
/// Retorna (índice del perfil seleccionado, true si fue doble-click para conectar).
pub fn show_favorites(
    ui: &mut egui::Ui,
    profiles: &[ConnectionProfile],
    selected_idx: &Option<usize>,
) -> (Option<usize>, bool) {
    let mut clicked = None;
    let mut connect_now = false;

    ui.add_space(8.0);
    ui.heading("Favoritos");
    ui.separator();

    if profiles.is_empty() {
        ui.add_space(8.0);
        ui.label("Sin conexiones guardadas.");
        ui.add_space(4.0);
        ui.label("Rellena el formulario\ny pulsa Guardar.");
    } else {
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (i, profile) in profiles.iter().enumerate() {
                let is_selected = selected_idx.is_some_and(|s| s == i);
                let resp = ui.selectable_label(is_selected, &profile.name);
                if resp.clicked() {
                    clicked = Some(i);
                }
                if resp.double_clicked() {
                    clicked = Some(i);
                    connect_now = true;
                }
            }
        });
    }

    (clicked, connect_now)
}

/// Panel derecho: formulario de conexión + botones de gestión.
pub fn show_form(
    ui: &mut egui::Ui,
    params: &mut ConnectionParams,
    port_str: &mut String,
    error_message: &Option<String>,
    profile_name: &mut String,
    save_password: &mut bool,
    selected_idx: &Option<usize>,
) -> FormResult {
    let mut result = FormResult::default();

    ui.add_space(24.0);
    ui.vertical_centered(|ui| {
        ui.heading("RustQL");
        ui.label("Conectar a base de datos");
    });
    ui.add_space(16.0);
    ui.separator();
    ui.add_space(12.0);

    egui::Grid::new("connection_form")
        .num_columns(2)
        .spacing([16.0, 8.0])
        .min_col_width(70.0)
        .show(ui, |ui| {
            ui.label("Host:");
            ui.add(
                egui::TextEdit::singleline(&mut params.host)
                    .min_size(egui::vec2(220.0, 0.0)),
            );
            ui.end_row();

            ui.label("Puerto:");
            ui.add(
                egui::TextEdit::singleline(port_str)
                    .min_size(egui::vec2(220.0, 0.0)),
            );
            ui.end_row();

            ui.label("Usuario:");
            ui.add(
                egui::TextEdit::singleline(&mut params.user)
                    .min_size(egui::vec2(220.0, 0.0)),
            );
            ui.end_row();

            ui.label("Contraseña:");
            ui.add(
                egui::TextEdit::singleline(&mut params.password)
                    .password(true)
                    .min_size(egui::vec2(220.0, 0.0)),
            );
            ui.end_row();

            ui.label("");
            ui.vertical(|ui| {
                ui.checkbox(save_password, "Guardar contraseña");
                if *save_password {
                    ui.colored_label(
                        egui::Color32::from_rgb(200, 140, 0),
                        "⚠ Se guarda sin cifrar en ~/.config/rustql/connections.json",
                    );
                }
            });
            ui.end_row();
        });

    ui.add_space(16.0);
    ui.separator();
    ui.add_space(8.0);

    // Sección guardar/eliminar favorito
    egui::Grid::new("profile_form")
        .num_columns(2)
        .spacing([16.0, 8.0])
        .min_col_width(70.0)
        .show(ui, |ui| {
            ui.label("Nombre:");
            ui.add(
                egui::TextEdit::singleline(profile_name)
                    .hint_text("Mi servidor...")
                    .min_size(egui::vec2(220.0, 0.0)),
            );
            ui.end_row();
        });

    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if ui.button("Guardar").clicked() {
            result.save_profile = true;
        }
        if selected_idx.is_some()
            && ui.button("Eliminar").clicked()
        {
            result.delete_profile = true;
        }
    });

    ui.add_space(16.0);
    ui.separator();
    ui.add_space(12.0);

    if let Some(err) = error_message {
        ui.colored_label(egui::Color32::RED, err);
        ui.add_space(8.0);
    }

    ui.vertical_centered(|ui| {
        if ui
            .add_sized([160.0, 32.0], egui::Button::new("Conectar"))
            .clicked()
        {
            result.connect = true;
        }
    });

    result
}

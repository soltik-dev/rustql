use eframe::egui;

use crate::db::ConnectionParams;
use crate::profiles::ConnectionProfile;
use crate::ui::theme;

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
    theme::section_label(ui, "Favoritos");
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
    let p = theme::get(ui);

    ui.add_space(32.0);
    ui.vertical_centered(|ui| {
        ui.heading("RustQL");
        ui.label(egui::RichText::new("Conectar a base de datos").color(p.text_muted));
    });
    ui.add_space(20.0);

    ui.vertical_centered(|ui| {
        ui.set_max_width(420.0);
        egui::Frame::NONE
            .fill(p.bg_raised)
            .stroke(egui::Stroke::new(1.0, p.border_subtle))
            .corner_radius(egui::CornerRadius::same(theme::RADIUS_LG))
            .inner_margin(egui::Margin::same(20))
            .show(ui, |ui| {
                show_form_fields(
                    ui,
                    params,
                    port_str,
                    error_message,
                    profile_name,
                    save_password,
                    selected_idx,
                    &mut result,
                );
            });
    });

    result
}

#[allow(clippy::too_many_arguments)]
fn show_form_fields(
    ui: &mut egui::Ui,
    params: &mut ConnectionParams,
    port_str: &mut String,
    error_message: &Option<String>,
    profile_name: &mut String,
    save_password: &mut bool,
    selected_idx: &Option<usize>,
    result: &mut FormResult,
) {
    let p = theme::get(ui);

    egui::Grid::new("connection_form")
        .num_columns(2)
        .spacing([16.0, 8.0])
        .min_col_width(70.0)
        .show(ui, |ui| {
            ui.label("Host:");
            ui.add(egui::TextEdit::singleline(&mut params.host).min_size(egui::vec2(220.0, 0.0)));
            ui.end_row();

            ui.label("Puerto:");
            ui.add(egui::TextEdit::singleline(port_str).min_size(egui::vec2(220.0, 0.0)));
            ui.end_row();

            ui.label("Usuario:");
            ui.add(egui::TextEdit::singleline(&mut params.user).min_size(egui::vec2(220.0, 0.0)));
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
                        p.warning,
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
        if selected_idx.is_some() && ui.button("Eliminar").clicked() {
            result.delete_profile = true;
        }
    });

    ui.add_space(16.0);
    ui.separator();
    ui.add_space(12.0);

    if let Some(err) = error_message {
        theme::error_card(ui, err);
        ui.add_space(8.0);
    }

    ui.vertical_centered(|ui| {
        ui.spacing_mut().button_padding = egui::vec2(48.0, 8.0);
        if theme::primary_button(ui, "Conectar").clicked() {
            result.connect = true;
        }
    });
}

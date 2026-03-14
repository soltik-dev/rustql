use eframe::egui;

use crate::settings::{AppSettings, Theme};

/// Devuelve true si algún ajuste cambió este frame.
pub fn show(ui: &mut egui::Ui, settings: &mut AppSettings) -> bool {
    let mut changed = false;

    ui.add_space(8.0);
    ui.heading("Ajustes");
    ui.separator();
    ui.add_space(8.0);

    ui.label("Tema:");
    changed |= ui
        .radio_value(&mut settings.theme, Theme::System, "Sistema (SO)")
        .changed();
    changed |= ui
        .radio_value(&mut settings.theme, Theme::Light, "Claro")
        .changed();
    changed |= ui
        .radio_value(&mut settings.theme, Theme::Dark, "Oscuro")
        .changed();

    ui.add_space(12.0);
    ui.separator();
    ui.add_space(8.0);

    ui.label("Tamaño de fuente:");
    changed |= ui
        .add(egui::Slider::new(&mut settings.font_size, 10.0..=22.0).step_by(0.5))
        .changed();

    ui.add_space(4.0);
    ui.label(
        egui::RichText::new("Texto de muestra / Sample text")
            .size(settings.font_size),
    );

    changed
}

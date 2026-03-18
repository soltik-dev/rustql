use eframe::egui;

use crate::settings::{AppSettings, CopyFormat, Theme};

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

    ui.add_space(12.0);
    ui.separator();
    ui.add_space(8.0);

    ui.label("Formato de copia (multi-fila):");
    changed |= ui
        .radio_value(&mut settings.copy_format, CopyFormat::Csv, "CSV")
        .changed();
    changed |= ui
        .radio_value(&mut settings.copy_format, CopyFormat::Tsv, "TSV")
        .changed();
    changed |= ui
        .radio_value(&mut settings.copy_format, CopyFormat::SqlInserts, "SQL INSERT")
        .changed();

    ui.add_space(12.0);
    ui.separator();
    ui.add_space(8.0);

    ui.label("Límite de filas por consulta:");
    changed |= ui
        .add(egui::Slider::new(&mut settings.query_limit, 100..=10000).step_by(100.0))
        .changed();

    changed
}

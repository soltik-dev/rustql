use eframe::egui;

use crate::settings::{self, AppSettings, CopyFormat, Theme};
use crate::ui::theme;

/// Un deslizador da su valor por cambiado al soltarlo (o al teclearlo), no
/// en cada frame del arrastre: aplicar un ajuste rehace los estilos y lo
/// guarda en disco, y con el tamaño de fuente la interfaz se movería bajo
/// el puntero.
fn slider(ui: &mut egui::Ui, slider: egui::Slider<'_>) -> bool {
    let resp = ui.add(slider);
    resp.drag_stopped() || (resp.changed() && !resp.dragged())
}

/// Devuelve true si algún ajuste cambió este frame.
pub fn show(ui: &mut egui::Ui, settings: &mut AppSettings) -> bool {
    let mut changed = false;

    ui.add_space(8.0);
    ui.heading("Ajustes");
    ui.separator();
    ui.add_space(8.0);

    theme::section_label(ui, "Tema");
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

    theme::section_label(ui, "Fuente");
    let available_fonts = settings::available_system_fonts();
    egui::ComboBox::from_id_salt("font_family")
        .selected_text(&settings.font_family)
        .width(200.0)
        .show_ui(ui, |ui| {
            changed |= ui
                .selectable_value(
                    &mut settings.font_family,
                    settings::EMBEDDED_FONT_NAME.to_owned(),
                    settings::EMBEDDED_FONT_NAME,
                )
                .changed();
            for (name, _) in available_fonts {
                changed |= ui
                    .selectable_value(&mut settings.font_family, name.clone(), name)
                    .changed();
            }
        });

    ui.add_space(12.0);
    ui.separator();
    ui.add_space(8.0);

    theme::section_label(ui, "Tamaño de fuente");
    changed |= slider(
        ui,
        egui::Slider::new(&mut settings.font_size, 10.0..=22.0).step_by(0.5),
    );

    ui.add_space(4.0);
    ui.label(egui::RichText::new("Texto de muestra / Sample text").size(settings.font_size));

    ui.add_space(12.0);
    ui.separator();
    ui.add_space(8.0);

    theme::section_label(ui, "Formato de copia (multi-fila)");
    changed |= ui
        .radio_value(&mut settings.copy_format, CopyFormat::Csv, "CSV")
        .changed();
    changed |= ui
        .radio_value(&mut settings.copy_format, CopyFormat::Tsv, "TSV")
        .changed();
    changed |= ui
        .radio_value(
            &mut settings.copy_format,
            CopyFormat::SqlInserts,
            "SQL INSERT",
        )
        .changed();

    ui.add_space(12.0);
    ui.separator();
    ui.add_space(8.0);

    theme::section_label(ui, "Límite de filas por consulta");
    changed |= slider(
        ui,
        egui::Slider::new(&mut settings.query_limit, 100..=10000).step_by(100.0),
    );

    theme::section_label(ui, "Máx. filas en el editor SQL");
    changed |= slider(
        ui,
        egui::Slider::new(&mut settings.sql_editor_max_rows, 1_000..=100_000).step_by(1_000.0),
    );

    changed
}

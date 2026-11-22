use eframe::egui;

use crate::i18n::{self, t};
use crate::settings::{self, AppSettings, CopyFormat, Theme};
use crate::ui::theme;

/// A slider reports its value as changed when released (or typed), not on
/// every frame of the drag: applying a setting rebuilds the styles and
/// saves it to disk, and with the font size the interface would move
/// under the pointer.
fn slider(ui: &mut egui::Ui, slider: egui::Slider<'_>) -> bool {
    let resp = ui.add(slider);
    resp.drag_stopped() || (resp.changed() && !resp.dragged())
}

/// Returns true if any setting changed this frame.
pub fn show(ui: &mut egui::Ui, settings: &mut AppSettings) -> bool {
    let mut changed = false;

    ui.add_space(8.0);
    ui.heading(t!("settings-title"));
    ui.separator();
    ui.add_space(8.0);

    theme::section_label(ui, &t!("settings-language"));
    let system = t!("settings-language-system");
    let current = match &settings.language {
        Some(code) => i18n::LANGUAGES
            .iter()
            .find(|(c, _)| c == code)
            .map_or(code.as_str(), |(_, name)| name),
        None => system.as_str(),
    };
    egui::ComboBox::from_id_salt("language")
        .selected_text(current)
        .width(200.0)
        .show_ui(ui, |ui| {
            changed |= ui
                .selectable_value(&mut settings.language, None, &system)
                .changed();
            for (code, name) in i18n::LANGUAGES {
                changed |= ui
                    .selectable_value(&mut settings.language, Some((*code).to_owned()), *name)
                    .changed();
            }
        });

    ui.add_space(12.0);
    ui.separator();
    ui.add_space(8.0);

    theme::section_label(ui, &t!("settings-theme"));
    changed |= ui
        .radio_value(
            &mut settings.theme,
            Theme::System,
            t!("settings-theme-system"),
        )
        .changed();
    changed |= ui
        .radio_value(
            &mut settings.theme,
            Theme::Light,
            t!("settings-theme-light"),
        )
        .changed();
    changed |= ui
        .radio_value(&mut settings.theme, Theme::Dark, t!("settings-theme-dark"))
        .changed();

    ui.add_space(12.0);
    ui.separator();
    ui.add_space(8.0);

    theme::section_label(ui, &t!("settings-font"));
    let available_fonts = settings::available_system_fonts();
    let embedded = t!("settings-font-embedded");
    let font_label = if settings.font_family == settings::EMBEDDED_FONT_NAME {
        embedded.as_str()
    } else {
        settings.font_family.as_str()
    };
    egui::ComboBox::from_id_salt("font_family")
        .selected_text(font_label)
        .width(200.0)
        .show_ui(ui, |ui| {
            changed |= ui
                .selectable_value(
                    &mut settings.font_family,
                    settings::EMBEDDED_FONT_NAME.to_owned(),
                    &embedded,
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

    theme::section_label(ui, &t!("settings-font-size"));
    changed |= slider(
        ui,
        egui::Slider::new(&mut settings.font_size, 10.0..=22.0).step_by(0.5),
    );

    ui.add_space(4.0);
    ui.label(egui::RichText::new(t!("settings-font-sample")).size(settings.font_size));

    ui.add_space(12.0);
    ui.separator();
    ui.add_space(8.0);

    theme::section_label(ui, &t!("settings-copy-format"));
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

    theme::section_label(ui, &t!("settings-page-rows"));
    changed |= slider(
        ui,
        egui::Slider::new(&mut settings.query_limit, 100..=10000).step_by(100.0),
    );

    theme::section_label(ui, &t!("settings-editor-max-rows"));
    changed |= slider(
        ui,
        egui::Slider::new(&mut settings.sql_editor_max_rows, 1_000..=100_000).step_by(1_000.0),
    );

    changed
}

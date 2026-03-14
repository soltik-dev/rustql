use std::path::PathBuf;

use eframe::egui;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub enum Theme {
    System,
    Light,
    Dark,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub theme: Theme,
    pub font_size: f32,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            theme: Theme::System,
            font_size: 13.0,
        }
    }
}

fn settings_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("rustql").join("settings.json"))
}

pub fn load() -> AppSettings {
    let Some(path) = settings_path() else {
        return AppSettings::default();
    };
    let Ok(data) = std::fs::read_to_string(path) else {
        return AppSettings::default();
    };
    serde_json::from_str(&data).unwrap_or_default()
}

pub fn save(s: &AppSettings) {
    let Some(path) = settings_path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(data) = serde_json::to_string_pretty(s) {
        let _ = std::fs::write(path, data);
    }
}

/// Aplica el tema y el tamaño de fuente al contexto egui.
pub fn apply(ctx: &egui::Context, s: &AppSettings) {
    let visuals = match s.theme {
        Theme::Dark => egui::Visuals::dark(),
        Theme::Light => egui::Visuals::light(),
        Theme::System => match dark_light::detect() {
            dark_light::Mode::Light => egui::Visuals::light(),
            _ => egui::Visuals::dark(), // Dark o Default → oscuro
        },
    };
    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    for font_id in style.text_styles.values_mut() {
        font_id.size = s.font_size;
    }
    ctx.set_style(style);
}

use std::path::PathBuf;

use eframe::egui;
use serde::{Deserialize, Serialize};

/// Pestaña SQL guardada (sin resultado, solo texto y contexto de BD).
#[derive(Clone, Serialize, Deserialize, Default)]
pub struct SavedSqlTab {
    pub db_context: String,
    pub sql_text: String,
}

#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub enum Theme {
    System,
    Light,
    Dark,
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub theme: Theme,
    pub font_size: f32,
    #[serde(default = "default_true")]
    pub show_sidebar: bool,
    #[serde(default = "default_true")]
    pub show_log: bool,
    #[serde(default)]
    pub fav_tables: std::collections::HashSet<String>,
    #[serde(default)]
    pub pinned_tables: Vec<(String, String)>,
    #[serde(default)]
    pub sql_tabs: Vec<SavedSqlTab>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            theme: Theme::System,
            font_size: 13.0,
            show_sidebar: true,
            show_log: true,
            fav_tables: std::collections::HashSet::new(),
            pinned_tables: Vec::new(),
            sql_tabs: Vec::new(),
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

fn dark_visuals() -> egui::Visuals {
    let mut v = egui::Visuals::dark();

    // Fondo general más oscuro para mayor contraste
    v.panel_fill = egui::Color32::from_rgb(18, 18, 18);
    v.window_fill = egui::Color32::from_rgb(24, 24, 24);
    v.faint_bg_color = egui::Color32::from_rgb(28, 28, 28);
    v.extreme_bg_color = egui::Color32::from_rgb(10, 10, 10);

    // Texto casi blanco (frente a los grises tenues por defecto)
    let text_bright = egui::Color32::from_rgb(220, 220, 220);
    let text_dim = egui::Color32::from_rgb(160, 160, 160);

    // Widgets no interactivos (etiquetas, separadores…)
    v.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, text_bright);
    v.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(28, 28, 28);
    v.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(55, 55, 55));

    // Widgets inactivos (botones sin hover, campos sin foco)
    v.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, text_bright);
    v.widgets.inactive.bg_fill = egui::Color32::from_rgb(38, 38, 38);
    v.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(65, 65, 65));

    // Hover
    v.widgets.hovered.fg_stroke = egui::Stroke::new(1.5, egui::Color32::WHITE);
    v.widgets.hovered.bg_fill = egui::Color32::from_rgb(55, 55, 75);
    v.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(100, 120, 200));

    // Active (pulsado)
    v.widgets.active.fg_stroke = egui::Stroke::new(2.0, egui::Color32::WHITE);
    v.widgets.active.bg_fill = egui::Color32::from_rgb(45, 70, 130);

    // Open (desplegables abiertos)
    v.widgets.open.fg_stroke = egui::Stroke::new(1.0, text_bright);
    v.widgets.open.bg_fill = egui::Color32::from_rgb(35, 35, 50);

    // Selección (texto seleccionado)
    v.selection.bg_fill = egui::Color32::from_rgb(40, 80, 160);
    v.selection.stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(100, 150, 255));

    // Texto hyperlink
    v.hyperlink_color = egui::Color32::from_rgb(100, 160, 255);

    // Líneas tenues (separadores, bordes de panel)
    v.window_stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(55, 55, 55));

    // Stripe de tabla: fondo alterno más visible
    v.faint_bg_color = egui::Color32::from_rgb(28, 28, 32);

    let _ = text_dim; // por si se usa en el futuro
    v
}

/// Aplica el tema y el tamaño de fuente al contexto egui.
pub fn apply(ctx: &egui::Context, s: &AppSettings) {
    let visuals = match s.theme {
        Theme::Dark => dark_visuals(),
        Theme::Light => egui::Visuals::light(),
        Theme::System => match dark_light::detect() {
            dark_light::Mode::Light => egui::Visuals::light(),
            _ => dark_visuals(),
        },
    };
    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    for font_id in style.text_styles.values_mut() {
        font_id.size = s.font_size;
    }
    ctx.set_style(style);
}

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum CopyFormat {
    #[default]
    Csv,
    Tsv,
    SqlInserts,
}

fn default_true() -> bool {
    true
}

fn default_query_limit() -> usize {
    500
}

fn default_font_family() -> String {
    "Liberation Sans".to_owned()
}

#[derive(Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub theme: Theme,
    pub font_size: f32,
    #[serde(default = "default_font_family")]
    pub font_family: String,
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
    #[serde(default)]
    pub copy_format: CopyFormat,
    #[serde(default = "default_query_limit")]
    pub query_limit: usize,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            theme: Theme::System,
            font_size: 13.0,
            font_family: default_font_family(),
            show_sidebar: true,
            show_log: true,
            fav_tables: std::collections::HashSet::new(),
            pinned_tables: Vec::new(),
            sql_tabs: Vec::new(),
            copy_format: CopyFormat::Csv,
            query_limit: 500,
        }
    }
}

/// Descubre todas las fuentes TTF/OTF del sistema disponibles para la UI.
/// Cada entrada es (nombre_display, path). Ordenadas alfabéticamente por nombre.
/// Excluye variantes Bold/Italic/etc. para no saturar el listado.
pub fn available_system_fonts() -> Vec<(String, String)> {
    #[cfg(target_os = "windows")]
    {
        windows_fonts()
    }
    #[cfg(not(target_os = "windows"))]
    {
        fc_list_fonts()
    }
}

#[cfg(not(target_os = "windows"))]
fn fc_list_fonts() -> Vec<(String, String)> {
    // fc-list --format "%{family[0]}\t%{file}\n" — una línea por fuente
    let output = std::process::Command::new("fc-list")
        .args(["--format", "%{family[0]}\t%{file}\n"])
        .output();

    let Ok(out) = output else { return vec![] };
    let text = String::from_utf8_lossy(&out.stdout);

    let mut fonts: Vec<(String, String)> = text
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(2, '\t');
            let name = parts.next()?.trim().to_string();
            let path = parts.next()?.trim().to_string();
            // Solo TTF/OTF que egui puede leer (no .ttc compuesto ni Variable Font bracket)
            if !path.ends_with(".ttf") && !path.ends_with(".otf") {
                return None;
            }
            // Excluir variantes: Bold, Italic, Oblique, Heavy, Black, Light, Thin, Medium, Condensed
            let low = path.to_lowercase();
            for variant in &["bold", "italic", "oblique", "heavy", "black", "light", "thin", "medium", "condensed", "narrow", "semibold", "extrabold", "demi"] {
                if low.contains(variant) { return None; }
            }
            // Excluir fuentes de símbolo/emoji/icon que no son texto
            for exclude in &["emoji", "symbol", "awesome", "nerd", "icon", "dingbat", "webdings", "wingding"] {
                if low.contains(exclude) || name.to_lowercase().contains(exclude) { return None; }
            }
            Some((name, path))
        })
        .collect();

    // Deduplicar por nombre (fc-list puede repetir la misma familia)
    fonts.sort_by(|a, b| a.0.cmp(&b.0));
    fonts.dedup_by(|a, b| a.0 == b.0);
    fonts
}

#[cfg(target_os = "windows")]
fn windows_fonts() -> Vec<(String, String)> {
    let fonts_dir = std::path::Path::new("C:\\Windows\\Fonts");
    let Ok(entries) = std::fs::read_dir(fonts_dir) else { return vec![] };

    let mut fonts: Vec<(String, String)> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let path = e.path();
            let ext = path.extension()?.to_str()?.to_lowercase();
            if ext != "ttf" && ext != "otf" { return None; }
            let filename = path.file_stem()?.to_str()?.to_string();
            let low = filename.to_lowercase();
            for variant in &["bold", "italic", "oblique", "heavy", "black", "light", "thin", "medium", "condensed", "narrow", "semibold"] {
                if low.contains(variant) { return None; }
            }
            Some((filename, path.to_string_lossy().to_string()))
        })
        .collect();

    fonts.sort_by(|a, b| a.0.cmp(&b.0));
    fonts
}

/// Carga fuentes: iconos embebidos + fuente del sistema elegida + emoji fallback.
/// Llamar al inicio y cada vez que el usuario cambie la fuente.
pub fn setup_fonts(ctx: &egui::Context, s: &AppSettings) {
    static ICONS_TTF: &[u8] = include_bytes!("../assets/fonts/rustql.ttf");

    let emoji_candidates: &[&str] = &[
        "/usr/share/fonts/google-noto-emoji-fonts/NotoEmoji-Regular.ttf",
        "/usr/share/fonts/noto-emoji/NotoEmoji-Regular.ttf",
        "/usr/share/fonts/truetype/noto/NotoEmoji-Regular.ttf",
        "/usr/share/fonts/gdouros-symbola/Symbola.ttf",
        "/usr/share/fonts/truetype/ancient-scripts/Symbola_hint.ttf",
        "C:\\Windows\\Fonts\\seguiemj.ttf",
        "/System/Library/Fonts/Apple Color Emoji.ttc",
    ];

    let mut fonts = egui::FontDefinitions::default();

    // 1. Iconos propios embebidos — primera posición (prioridad en U+E800..)
    fonts.font_data.insert(
        "rustql_icons".to_owned(),
        egui::FontData::from_static(ICONS_TTF).into(),
    );
    fonts
        .families
        .get_mut(&egui::FontFamily::Proportional)
        .unwrap()
        .insert(0, "rustql_icons".to_owned());

    // 2. Fuente del sistema elegida por el usuario
    let available = available_system_fonts();
    let chosen_path = available
        .iter()
        .find(|(name, _)| name == &s.font_family)
        .or_else(|| available.first())
        .map(|(_, path)| path.clone());

    if let Some(path) = chosen_path
        && let Ok(data) = std::fs::read(&path) {
            fonts.font_data.insert(
                "system_font".to_owned(),
                egui::FontData::from_owned(data).into(),
            );
            fonts
                .families
                .get_mut(&egui::FontFamily::Proportional)
                .unwrap()
                .insert(1, "system_font".to_owned());
        }

    // 3. Emoji/símbolo como último fallback
    for path in emoji_candidates {
        if let Ok(data) = std::fs::read(path) {
            fonts.font_data.insert(
                "emoji_fallback".to_owned(),
                egui::FontData::from_owned(data).into(),
            );
            fonts
                .families
                .get_mut(&egui::FontFamily::Proportional)
                .unwrap()
                .push("emoji_fallback".to_owned());
            fonts
                .families
                .get_mut(&egui::FontFamily::Monospace)
                .unwrap()
                .push("emoji_fallback".to_owned());
            break;
        }
    }

    ctx.set_fonts(fonts);
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
    v.widgets.hovered.bg_fill = egui::Color32::from_rgb(50, 52, 60);
    v.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(80, 90, 110));

    // Active (pulsado / toggle activado)
    v.widgets.active.fg_stroke = egui::Stroke::new(1.5, egui::Color32::WHITE);
    v.widgets.active.bg_fill = egui::Color32::from_rgb(42, 48, 64);

    // Open (desplegables abiertos)
    v.widgets.open.fg_stroke = egui::Stroke::new(1.0, text_bright);
    v.widgets.open.bg_fill = egui::Color32::from_rgb(35, 37, 48);

    // Selección (item seleccionado en sidebar, texto seleccionado)
    v.selection.bg_fill = egui::Color32::from_rgba_unmultiplied(60, 90, 160, 120);
    v.selection.stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(130, 160, 220));

    // Texto hyperlink
    v.hyperlink_color = egui::Color32::from_rgb(100, 160, 255);

    // Líneas tenues (separadores, bordes de panel)
    v.window_stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(55, 55, 55));

    // Stripe de tabla: fondo alterno más visible
    v.faint_bg_color = egui::Color32::from_rgb(28, 28, 32);

    v
}

/// Aplica el tema y el tamaño de fuente al contexto egui.
pub fn apply(ctx: &egui::Context, s: &AppSettings) {
    let visuals = match s.theme {
        Theme::Dark => dark_visuals(),
        Theme::Light => egui::Visuals::light(),
        Theme::System => match dark_light::detect() {
            Ok(dark_light::Mode::Light) => egui::Visuals::light(),
            _ => dark_visuals(),
        },
    };
    ctx.set_visuals(visuals);

    let mut style = (*ctx.global_style()).clone();
    for font_id in style.text_styles.values_mut() {
        font_id.size = s.font_size;
    }
    ctx.set_global_style(style);
}

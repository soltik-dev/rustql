use std::collections::HashMap;

use eframe::egui;
use serde::{Deserialize, Serialize};

/// Pestaña SQL guardada (sin resultado, solo texto y contexto de BD).
#[derive(Clone, Serialize, Deserialize, Default)]
pub struct SavedSqlTab {
    /// Nombre que le dio el usuario (ver SqlTab::name).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub db_context: String,
    pub sql_text: String,
    /// Alto que el usuario le dio al editor (ver SqlTab::editor_height).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub editor_height: Option<f32>,
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

/// Nombre mostrado para la fuente de UI embebida (Inter).
pub const EMBEDDED_FONT_NAME: &str = "Inter (integrada)";

fn default_font_family() -> String {
    EMBEDDED_FONT_NAME.to_owned()
}

/// Los campos que falten en el JSON (ficheros de versiones anteriores) toman
/// su valor de `AppSettings::default()`.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
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
    /// Pestañas SQL de antes de las conexiones múltiples: las hereda la primera
    /// conexión que se abra y después deja de guardarse.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sql_tabs: Vec<SavedSqlTab>,
    /// Pestañas SQL guardadas por conexión (clave: ConnectionParams::key).
    #[serde(default)]
    pub sql_tabs_by_conn: HashMap<String, Vec<SavedSqlTab>>,
    #[serde(default)]
    pub copy_format: CopyFormat,
    #[serde(default = "default_query_limit")]
    pub query_limit: usize,
    /// Tope de filas que lee el editor SQL; el resto se descarta.
    pub sql_editor_max_rows: usize,
    /// Tamaño de la ventana (sin maximizar) al salir la última vez.
    pub window_size: Option<[f32; 2]>,
    pub window_maximized: bool,
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
            sql_tabs: Vec::new(),
            sql_tabs_by_conn: HashMap::new(),
            copy_format: CopyFormat::Csv,
            query_limit: 500,
            sql_editor_max_rows: 10_000,
            window_size: None,
            window_maximized: false,
        }
    }
}

/// Fuentes TTF/OTF del sistema disponibles para la UI, como (nombre, ruta)
/// ordenadas por nombre y sin las variantes Bold/Italic/etc. Se buscan una
/// sola vez: listarlas lanza `fc-list` (o recorre el directorio de fuentes),
/// y el panel de ajustes las pide en cada frame.
pub fn available_system_fonts() -> &'static [(String, String)] {
    static FONTS: std::sync::OnceLock<Vec<(String, String)>> = std::sync::OnceLock::new();
    FONTS.get_or_init(|| {
        #[cfg(target_os = "windows")]
        {
            windows_fonts()
        }
        #[cfg(not(target_os = "windows"))]
        {
            fc_list_fonts()
        }
    })
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
            for variant in &[
                "bold",
                "italic",
                "oblique",
                "heavy",
                "black",
                "light",
                "thin",
                "medium",
                "condensed",
                "narrow",
                "semibold",
                "extrabold",
                "demi",
            ] {
                if low.contains(variant) {
                    return None;
                }
            }
            // Excluir fuentes de símbolo/emoji/icon que no son texto
            for exclude in &[
                "emoji", "symbol", "awesome", "nerd", "icon", "dingbat", "webdings", "wingding",
            ] {
                if low.contains(exclude) || name.to_lowercase().contains(exclude) {
                    return None;
                }
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
    let Ok(entries) = std::fs::read_dir(fonts_dir) else {
        return vec![];
    };

    let mut fonts: Vec<(String, String)> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let path = e.path();
            let ext = path.extension()?.to_str()?.to_lowercase();
            if ext != "ttf" && ext != "otf" {
                return None;
            }
            let filename = path.file_stem()?.to_str()?.to_string();
            let low = filename.to_lowercase();
            for variant in &[
                "bold",
                "italic",
                "oblique",
                "heavy",
                "black",
                "light",
                "thin",
                "medium",
                "condensed",
                "narrow",
                "semibold",
            ] {
                if low.contains(variant) {
                    return None;
                }
            }
            Some((filename, path.to_string_lossy().to_string()))
        })
        .collect();

    fonts.sort_by(|a, b| a.0.cmp(&b.0));
    fonts
}

/// Carga fuentes: iconos embebidos + Inter/JetBrains Mono embebidas +
/// fuente del sistema elegida (override) + emoji fallback.
/// Llamar al inicio y cada vez que el usuario cambie la fuente.
pub fn setup_fonts(ctx: &egui::Context, s: &AppSettings) {
    static ICONS_TTF: &[u8] = include_bytes!("../assets/fonts/rustql.ttf");
    static INTER_REGULAR: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.otf");
    static INTER_MEDIUM: &[u8] = include_bytes!("../assets/fonts/Inter-Medium.otf");
    static JETBRAINS_MONO: &[u8] = include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf");

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

    // 2. Inter embebida como fuente de UI base
    fonts.font_data.insert(
        "inter_regular".to_owned(),
        egui::FontData::from_static(INTER_REGULAR).into(),
    );
    fonts
        .families
        .get_mut(&egui::FontFamily::Proportional)
        .unwrap()
        .insert(1, "inter_regular".to_owned());

    // 3. Fuente del sistema elegida por el usuario (override, delante de Inter)
    if s.font_family != EMBEDDED_FONT_NAME {
        let available = available_system_fonts();
        let chosen_path = available
            .iter()
            .find(|(name, _)| name == &s.font_family)
            .map(|(_, path)| path.clone());

        if let Some(path) = chosen_path
            && let Ok(data) = std::fs::read(&path)
        {
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
    }

    // 4. JetBrains Mono embebida como monospace principal (SQL, celdas de datos)
    fonts.font_data.insert(
        "jetbrains_mono".to_owned(),
        egui::FontData::from_static(JETBRAINS_MONO).into(),
    );
    fonts
        .families
        .get_mut(&egui::FontFamily::Monospace)
        .unwrap()
        .insert(0, "jetbrains_mono".to_owned());

    // 5. Familia con peso medio para etiquetas de sección (theme::section_label)
    fonts.font_data.insert(
        "inter_medium".to_owned(),
        egui::FontData::from_static(INTER_MEDIUM).into(),
    );
    fonts.families.insert(
        egui::FontFamily::Name(crate::ui::theme::MEDIUM_FAMILY.into()),
        vec!["inter_medium".to_owned(), "inter_regular".to_owned()],
    );

    // 6. Emoji/símbolo como último fallback
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

const SETTINGS_FILE: &str = "settings.json";

pub fn load() -> AppSettings {
    let mut s: AppSettings = crate::config_file::load(SETTINGS_FILE);
    // "Liberation Sans" era el default auto-inyectado antes de embeber Inter,
    // no una elección del usuario: migrarlo a la fuente integrada.
    if s.font_family == "Liberation Sans" {
        s.font_family = EMBEDDED_FONT_NAME.to_owned();
    }
    s
}

pub fn save(s: &AppSettings) {
    crate::config_file::save(SETTINGS_FILE, s, false);
}

/// Aplica el tema y el tamaño de fuente al contexto egui.
/// Registra el estilo de ambos temas (set_visuals solo escribiría el activo)
/// y luego selecciona cuál usar según los ajustes.
pub fn apply(ctx: &egui::Context, s: &AppSettings) {
    use crate::ui::theme;

    ctx.set_style_of(egui::Theme::Dark, theme::style(true, s.font_size));
    ctx.set_style_of(egui::Theme::Light, theme::style(false, s.font_size));

    let pref = match s.theme {
        Theme::Dark => egui::ThemePreference::Dark,
        Theme::Light => egui::ThemePreference::Light,
        Theme::System => match detect_system_mode() {
            Some(dark_light::Mode::Light) => egui::ThemePreference::Light,
            _ => egui::ThemePreference::Dark,
        },
    };
    ctx.set_theme(pref);
}

/// Tema del sistema con tope de espera. dark-light 3 consulta el portal por
/// D-Bus sin timeout; si el portal no responde, no debe congelar la UI.
fn detect_system_mode() -> Option<dark_light::Mode> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(dark_light::detect());
    });
    rx.recv_timeout(std::time::Duration::from_millis(250))
        .ok()?
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ajustes_incompletos_toman_los_valores_por_defecto() {
        // Antes, sin font_size el JSON no se podía leer y se perdía todo
        let s: AppSettings = serde_json::from_str(r#"{"theme": "Dark"}"#).unwrap();
        assert!(s.theme == Theme::Dark);
        assert_eq!(s.font_size, 13.0);
        assert_eq!(s.query_limit, 500);
        assert_eq!(s.sql_editor_max_rows, 10_000);
        assert!(s.show_sidebar);
    }
}

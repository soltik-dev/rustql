#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod db;
mod log;
mod profiles;
mod settings;
mod table_state;
mod ui;

use app::RustqlApp;
use eframe::egui;

fn main() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 680.0])
            .with_min_inner_size([800.0, 500.0])
            .with_title("RustQL"),
        ..Default::default()
    };

    eframe::run_native(
        "RustQL",
        native_options,
        Box::new(|cc| {
            setup_fonts(&cc.egui_ctx);
            // Aplicar settings al arrancar para que el primer frame ya tenga el tema correcto
            let loaded = settings::load();
            settings::apply(&cc.egui_ctx, &loaded);
            Ok(Box::new(RustqlApp::default()))
        }),
    )
}

/// Carga fuentes del sistema como fallback para glifos no presentes
/// en Ubuntu-Light (el font por defecto de egui): flechas, emojis, símbolos, etc.
fn setup_fonts(ctx: &egui::Context) {
    let text_candidates: &[&str] = &[
        // Fedora / RHEL
        "/usr/share/fonts/dejavu-sans-fonts/DejaVuSans.ttf",
        // Debian / Ubuntu
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        // Arch / Manjaro
        "/usr/share/fonts/TTF/DejaVuSans.ttf",
        "/usr/share/fonts/dejavu/DejaVuSans.ttf",
        // Noto (si instalado)
        "/usr/share/fonts/google-noto/NotoSans-Regular.ttf",
        "/usr/share/fonts/noto/NotoSans-Regular.ttf",
        // Windows
        "C:\\Windows\\Fonts\\seguisym.ttf",
        "C:\\Windows\\Fonts\\segoeui.ttf",
        "C:\\Windows\\Fonts\\arial.ttf",
        // macOS
        "/System/Library/Fonts/Supplemental/Arial Unicode MS.ttf",
        "/Library/Fonts/Arial Unicode.ttf",
    ];

    // Fuentes emoji/símbolo — se prueban en orden y se usa la primera disponible
    let emoji_candidates: &[&str] = &[
        // Noto Emoji (sin color, compatible con egui rasterizer)
        "/usr/share/fonts/google-noto-emoji-fonts/NotoEmoji-Regular.ttf",
        "/usr/share/fonts/noto-emoji/NotoEmoji-Regular.ttf",
        "/usr/share/fonts/truetype/noto/NotoEmoji-Regular.ttf",
        // Symbola (cobertura Unicode muy amplia)
        "/usr/share/fonts/gdouros-symbola/Symbola.ttf",
        "/usr/share/fonts/truetype/ancient-scripts/Symbola_hint.ttf",
        // Segoe UI Emoji (Windows)
        "C:\\Windows\\Fonts\\seguiemj.ttf",
        // Apple Color Emoji (macOS)
        "/System/Library/Fonts/Apple Color Emoji.ttc",
    ];

    let mut fonts = egui::FontDefinitions::default();

    // 1. Fuente de texto general
    for path in text_candidates {
        if let Ok(data) = std::fs::read(path) {
            fonts.font_data.insert(
                "system_fallback".to_owned(),
                egui::FontData::from_owned(data).into(),
            );
            fonts
                .families
                .get_mut(&egui::FontFamily::Proportional)
                .unwrap()
                .push("system_fallback".to_owned());
            break;
        }
    }

    // 2. Fuente emoji/símbolo como último fallback (cubre 🔑 ★ ⚠ etc.)
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
            // También disponible en Monospace (editor SQL, log, etc.)
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

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
            Ok(Box::new(RustqlApp::default()))
        }),
    )
}

/// Carga una fuente del sistema como fallback para glifos no presentes
/// en Ubuntu-Light (el font por defecto de egui): flechas, símbolos, etc.
fn setup_fonts(ctx: &egui::Context) {
    let candidates: &[&str] = &[
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

    for path in candidates {
        if let Ok(data) = std::fs::read(path) {
            let mut fonts = egui::FontDefinitions::default();
            fonts.font_data.insert(
                "system_fallback".to_owned(),
                egui::FontData::from_owned(data).into(),
            );
            // Añadir al final de la lista proporcional: se usa solo como fallback
            // para glifos que Ubuntu-Light no tiene (flechas, símbolos, etc.)
            fonts
                .families
                .get_mut(&egui::FontFamily::Proportional)
                .unwrap()
                .push("system_fallback".to_owned());
            ctx.set_fonts(fonts);
            return;
        }
    }
    // Si no hay fuente del sistema, queda Ubuntu-Light y los indicadores
    // ASCII como fallback (ver resultados_grid.rs y demás)
}

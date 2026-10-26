#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod db;
mod log;
mod profiles;
mod secrets;
mod session;
mod settings;
mod table_state;
mod ui;

use app::RustqlApp;
use eframe::egui;

fn main() -> eframe::Result<()> {
    // Muestra en stderr los errores de las dependencias (rfd, eframe, …),
    // que usan el crate `log` y sin logger se pierden en silencio.
    env_logger::init();

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
            let loaded = settings::load();
            settings::setup_fonts(&cc.egui_ctx, &loaded);
            settings::apply(&cc.egui_ctx, &loaded);
            Ok(Box::new(RustqlApp::new(&cc.egui_ctx)))
        }),
    )
}


#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod db;
mod ui;

use app::RustqlApp;
use eframe::egui;

fn main() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1024.0, 768.0])
            .with_title("RustQL"),
        ..Default::default()
    };

    eframe::run_native(
        "RustQL",
        native_options,
        Box::new(|_cc| Ok(Box::new(RustqlApp::default()))),
    )
}

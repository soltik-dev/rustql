#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod config_file;
mod db;
mod loadable;
mod log;
mod profiles;
mod secrets;
mod session;
mod settings;
mod table_state;
mod ui;

use app::RustqlApp;
use eframe::egui;

/// Identificador de la aplicación para el escritorio: en Wayland asocia la
/// ventana a su `.desktop` (icono y nombre en el panel) y en X11 es su clase.
const APP_ID: &str = "rustql";

/// Icono de la ventana (lo genera assets/icon/make_icon.py).
const ICON_PNG: &[u8] = include_bytes!("../assets/icon/icon.png");

const DEFAULT_WINDOW_SIZE: [f32; 2] = [1100.0, 680.0];
const MIN_WINDOW_SIZE: [f32; 2] = [800.0, 500.0];

fn main() -> eframe::Result<()> {
    // Muestra en stderr los errores de las dependencias (rfd, eframe, …),
    // que usan el crate `log` y sin logger se pierden en silencio.
    env_logger::init();

    let loaded = settings::load();
    let mut viewport = egui::ViewportBuilder::default()
        .with_app_id(APP_ID)
        .with_inner_size(window_size(loaded.window_size))
        .with_min_inner_size(MIN_WINDOW_SIZE)
        .with_maximized(loaded.window_maximized)
        .with_title("RustQL");
    match eframe::icon_data::from_png_bytes(ICON_PNG) {
        Ok(icon) => viewport = viewport.with_icon(icon),
        Err(e) => ::log::warn!("No se pudo leer el icono de la ventana: {e}"),
    }
    let native_options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "RustQL",
        native_options,
        Box::new(|cc| {
            settings::setup_fonts(&cc.egui_ctx, &loaded);
            settings::apply(&cc.egui_ctx, &loaded);
            Ok(Box::new(RustqlApp::new(&cc.egui_ctx, loaded)))
        }),
    )
}

/// Tamaño con el que se abre la ventana: el guardado al salir la última vez
/// o, si no hay o no es válido, el inicial. Nunca menor que el mínimo.
fn window_size(saved: Option<[f32; 2]>) -> [f32; 2] {
    let [w, h] = saved
        .filter(|size| size.iter().all(|v| v.is_finite()))
        .unwrap_or(DEFAULT_WINDOW_SIZE);
    [w.max(MIN_WINDOW_SIZE[0]), h.max(MIN_WINDOW_SIZE[1])]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_ventana_se_abre_con_el_tamano_guardado() {
        assert_eq!(window_size(None), DEFAULT_WINDOW_SIZE);
        assert_eq!(window_size(Some([1600.0, 900.0])), [1600.0, 900.0]);
        // Un fichero de ajustes estropeado no deja la ventana inservible
        assert_eq!(window_size(Some([10.0, 900.0])), [800.0, 900.0]);
        assert_eq!(window_size(Some([f32::NAN, 900.0])), DEFAULT_WINDOW_SIZE);
    }
}

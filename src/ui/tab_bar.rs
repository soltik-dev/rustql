use eframe::egui;

use crate::ui::icons::*;
use crate::ui::{CMD, ellipsize, theme};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ActiveTab {
    #[default]
    Servidor,
    Bbdd,
    Tabla,
    Datos,
    Sql(usize), // index into sql_tabs Vec
}

#[derive(Default)]
pub struct TabBarResult {
    pub tab_clicked: Option<ActiveTab>,
    pub new_sql_tab: bool,
    pub close_sql_tab: Option<usize>,
}

#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut egui::Ui,
    active: ActiveTab,
    has_db: bool,
    has_table: bool,
    server_name: &str,
    db_name: Option<&str>,
    table_name: Option<&str>,
    sql_tab_count: usize,
) -> TabBarResult {
    let mut result = TabBarResult::default();

    ui.spacing_mut().button_padding = egui::vec2(10.0, 5.0);
    ui.spacing_mut().item_spacing = egui::vec2(3.0, 3.0);

    // Pestañas de contexto + pestañas SQL, con salto de línea si no caben
    ui.horizontal_wrapped(|ui| {
        // Servidor tab
        let srv_label = format!("{PLUG} {}", ellipsize(server_name, 18));
        if tab_btn(ui, &srv_label, active == ActiveTab::Servidor, true).clicked() {
            result.tab_clicked = Some(ActiveTab::Servidor);
        }

        // BBDD tab
        let db_label = format!("{DATABASE} {}", ellipsize(db_name.unwrap_or("BBDD"), 18));
        if tab_btn(ui, &db_label, active == ActiveTab::Bbdd, has_db).clicked() {
            result.tab_clicked = Some(ActiveTab::Bbdd);
        }

        // Estructura y Datos son de la misma tabla: su nombre va solo en
        // Datos (antes las dos pestañas decían lo mismo y solo las
        // distinguía el icono)
        let struct_label = format!("{EDIT} Estructura");
        let mut struct_tab = tab_btn(ui, &struct_label, active == ActiveTab::Tabla, has_table);
        if let Some(table) = table_name {
            struct_tab = struct_tab.on_hover_text(format!("Estructura de {table}"));
        }
        if struct_tab.clicked() {
            result.tab_clicked = Some(ActiveTab::Tabla);
        }

        // Datos tab
        let datos_label = format!("{GRID} {}", ellipsize(table_name.unwrap_or("Datos"), 18));
        if tab_btn(ui, &datos_label, active == ActiveTab::Datos, has_table).clicked() {
            result.tab_clicked = Some(ActiveTab::Datos);
        }

        tab_separator(ui);

        // SQL tabs
        for i in 0..sql_tab_count {
            let is_active = active == ActiveTab::Sql(i);
            let label = format!("{CODE} SQL {}", i + 1);
            let tab = tab_btn(ui, &label, is_active, true);
            if tab.clicked() {
                result.tab_clicked = Some(ActiveTab::Sql(i));
            }
            let close_resp = ui
                .add(egui::Button::new("×").small().frame(false))
                .on_hover_text("Cerrar pestaña SQL (o clic central en la pestaña)");
            // Clic central, como en las pestañas de conexión
            if close_resp.clicked() || tab.middle_clicked() {
                result.close_sql_tab = Some(i);
            }
        }

        // New SQL tab button
        if ui
            .add(egui::Button::new("＋").small().frame(false))
            .on_hover_text(format!("Nueva pestaña SQL  {CMD}T"))
            .clicked()
        {
            result.new_sql_tab = true;
        }
    });

    result
}

fn tab_separator(ui: &mut egui::Ui) {
    let h = ui.spacing().interact_size.y * 0.6;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(1.0, h), egui::Sense::hover());
    ui.painter().rect_filled(
        rect,
        0.0,
        ui.visuals().widgets.noninteractive.bg_stroke.color,
    );
}

/// Pestaña: fondo transparente, hover en superficie y subrayado de acento si
/// está activa.
fn tab_btn(ui: &mut egui::Ui, label: &str, active: bool, enabled: bool) -> egui::Response {
    let p = theme::get(ui);
    if !enabled {
        let text = egui::RichText::new(label).color(p.text_disabled);
        return ui.add_enabled(false, egui::Button::new(text).frame(false));
    }

    let text_color = if active { p.text_primary } else { p.text_muted };

    let resp = ui
        .scope(|ui| {
            let w = &mut ui.style_mut().visuals.widgets;
            w.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
            w.hovered.weak_bg_fill = p.bg_surface;
            w.active.weak_bg_fill = p.bg_surface;
            for state in [&mut w.inactive, &mut w.hovered, &mut w.active] {
                state.bg_stroke = egui::Stroke::NONE;
            }
            ui.add(egui::Button::new(
                egui::RichText::new(label).color(text_color),
            ))
        })
        .inner;

    if active {
        let rect = resp.rect;
        ui.painter().hline(
            egui::Rangef::new(rect.left() + 2.0, rect.right() - 2.0),
            rect.bottom() - 1.0,
            egui::Stroke::new(2.0, p.accent),
        );
    }
    resp
}

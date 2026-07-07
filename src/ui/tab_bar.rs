use eframe::egui;

use crate::ui::icons::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ActiveTab {
    #[default]
    Servidor,
    Bbdd,
    Tabla,
    Datos,
    Sql(usize),    // index into sql_tabs Vec
    Pinned(usize), // index into pinned_tables Vec
}

#[derive(Default)]
pub struct TabBarResult {
    pub tab_clicked: Option<ActiveTab>,
    pub new_sql_tab: bool,
    pub close_sql_tab: Option<usize>,
    pub close_pinned: Option<usize>,
}

fn clip_text(s: &str, max_chars: usize) -> String {
    let mut chars = s.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{truncated}…")
    } else {
        truncated
    }
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
    pinned_tables: &[(String, String)],
    pinned_pending_counts: &[usize],
) -> TabBarResult {
    let mut result = TabBarResult::default();

    ui.spacing_mut().button_padding = egui::vec2(8.0, 4.0);
    ui.spacing_mut().item_spacing = egui::vec2(3.0, 3.0);

    // Row 1: main context tabs + SQL tabs, all wrapping
    ui.horizontal_wrapped(|ui| {
        // Servidor tab
        let srv_label = format!("{DATABASE} {}", clip_text(server_name, 18));
        if tab_btn(ui, &srv_label, active == ActiveTab::Servidor, true) {
            result.tab_clicked = Some(ActiveTab::Servidor);
        }

        // BBDD tab
        let db_label = format!("{DATABASE} {}", clip_text(db_name.unwrap_or("BBDD"), 18));
        if tab_btn(ui, &db_label, active == ActiveTab::Bbdd, has_db) {
            result.tab_clicked = Some(ActiveTab::Bbdd);
        }

        // Estructura tab
        let struct_label = format!("{EDIT} {}", clip_text(table_name.unwrap_or("Estructura"), 18));
        if tab_btn(ui, &struct_label, active == ActiveTab::Tabla, has_table) {
            result.tab_clicked = Some(ActiveTab::Tabla);
        }

        // Datos tab
        let datos_label = format!("{GRID} {}", clip_text(table_name.unwrap_or("Datos"), 18));
        if tab_btn(ui, &datos_label, active == ActiveTab::Datos, has_table) {
            result.tab_clicked = Some(ActiveTab::Datos);
        }

        tab_separator(ui);

        // SQL tabs
        for i in 0..sql_tab_count {
            let is_active = active == ActiveTab::Sql(i);
            let label = format!("{CODE} SQL {}", i + 1);
            if tab_btn(ui, &label, is_active, true) {
                result.tab_clicked = Some(ActiveTab::Sql(i));
            }
            let close_resp = ui
                .add(egui::Button::new("×").small().frame(false))
                .on_hover_text("Cerrar pestaña SQL");
            if close_resp.clicked() {
                result.close_sql_tab = Some(i);
            }
        }

        // New SQL tab button
        if ui
            .add(egui::Button::new("＋").small().frame(false))
            .on_hover_text("Nueva pestaña SQL  Ctrl+T")
            .clicked()
        {
            result.new_sql_tab = true;
        }
    });

    // Row 2: pinned tabs (only if any)
    if !pinned_tables.is_empty() {
        ui.horizontal_wrapped(|ui| {
            ui.label(PIN_FILLED);
            for (i, (_db, table)) in pinned_tables.iter().enumerate() {
                let is_active = active == ActiveTab::Pinned(i);
                let has_changes = pinned_pending_counts.get(i).copied().unwrap_or(0) > 0;
                let label = if has_changes {
                    format!("● {}", clip_text(table, 16))
                } else {
                    clip_text(table, 18)
                };
                if tab_btn_pinned(ui, &label, is_active, true, has_changes) {
                    result.tab_clicked = Some(ActiveTab::Pinned(i));
                }
                let close_resp = ui
                    .add(egui::Button::new("×").small().frame(false))
                    .on_hover_text("Quitar fijado");
                if close_resp.clicked() {
                    result.close_pinned = Some(i);
                }
            }
        });
    }

    result
}

fn tab_separator(ui: &mut egui::Ui) {
    let h = ui.spacing().interact_size.y * 0.6;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(1.0, h), egui::Sense::hover());
    ui.painter().rect_filled(rect, 0.0, ui.visuals().widgets.noninteractive.bg_stroke.color);
}

fn tab_btn(ui: &mut egui::Ui, label: &str, active: bool, enabled: bool) -> bool {
    if !enabled {
        let text = egui::RichText::new(label)
            .color(ui.visuals().noninteractive().text_color());
        ui.add_enabled(false, egui::Button::new(text).frame(true));
        return false;
    }

    let text = if active {
        egui::RichText::new(label).strong()
    } else {
        egui::RichText::new(label)
    };

    let fill = if active {
        ui.visuals().widgets.active.bg_fill
    } else {
        ui.visuals().widgets.inactive.bg_fill
    };

    let resp = ui.add(
        egui::Button::new(text)
            .fill(fill)
            .frame(true)
            .selected(active),
    );
    resp.clicked()
}

/// Variante para tabs fijados: muestra un color cálido cuando tiene cambios pendientes.
fn tab_btn_pinned(
    ui: &mut egui::Ui,
    label: &str,
    active: bool,
    enabled: bool,
    has_changes: bool,
) -> bool {
    if !enabled {
        let text = egui::RichText::new(label)
            .color(ui.visuals().noninteractive().text_color());
        ui.add_enabled(false, egui::Button::new(text).frame(true));
        return false;
    }

    let text = if active {
        egui::RichText::new(label).strong()
    } else {
        egui::RichText::new(label)
    };

    let fill = if has_changes {
        // Tono ámbar para indicar cambios pendientes
        if ui.visuals().dark_mode {
            egui::Color32::from_rgb(100, 70, 10)
        } else {
            egui::Color32::from_rgb(255, 220, 100)
        }
    } else if active {
        ui.visuals().widgets.active.bg_fill
    } else {
        ui.visuals().widgets.inactive.bg_fill
    };

    let resp = ui.add(
        egui::Button::new(text)
            .fill(fill)
            .frame(true)
            .selected(active),
    );
    resp.clicked()
}

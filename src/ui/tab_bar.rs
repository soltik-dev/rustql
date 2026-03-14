use eframe::egui;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ActiveTab {
    #[default]
    Servidor,
    Bbdd,
    Tabla,
    Datos,
    Sql,
}

/// Renderiza la barra de pestañas. Retorna el tab solicitado si el usuario
/// hizo click en uno diferente al activo.
pub fn show(
    ui: &mut egui::Ui,
    active: ActiveTab,
    has_db: bool,
    has_table: bool,
) -> Option<ActiveTab> {
    let mut requested = None;

    ui.horizontal(|ui| {
        ui.style_mut().spacing.button_padding = egui::vec2(8.0, 4.0);

        if tab_btn(ui, "Servidor", active == ActiveTab::Servidor, true) {
            requested = Some(ActiveTab::Servidor);
        }
        if tab_btn(ui, "BBDD", active == ActiveTab::Bbdd, has_db) {
            requested = Some(ActiveTab::Bbdd);
        }
        if tab_btn(ui, "Tabla", active == ActiveTab::Tabla, has_table) {
            requested = Some(ActiveTab::Tabla);
        }
        if tab_btn(ui, "Datos", active == ActiveTab::Datos, has_table) {
            requested = Some(ActiveTab::Datos);
        }
        if tab_btn(ui, "SQL", active == ActiveTab::Sql, true) {
            requested = Some(ActiveTab::Sql);
        }
    });

    // Ignorar si el usuario pulsó el tab ya activo
    requested.filter(|&t| t != active)
}

fn tab_btn(ui: &mut egui::Ui, label: &str, active: bool, enabled: bool) -> bool {
    if !enabled {
        let text = egui::RichText::new(label)
            .color(ui.visuals().noninteractive().text_color());
        ui.add_enabled(false, egui::Button::new(text).frame(false));
        return false;
    }

    let text = if active {
        egui::RichText::new(label).strong()
    } else {
        egui::RichText::new(label)
    };

    let resp = ui.add(egui::Button::new(text).frame(false).selected(active));
    resp.clicked()
}

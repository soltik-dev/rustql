use eframe::egui;

pub enum FilterAction {
    None,
    Apply,
    Clear,
}

/// Barra de filtro WHERE encima del grid.
/// Retorna la acción que el usuario tomó este frame.
/// F9 se detecta en app.rs y se pasa como `f9_pressed`.
pub fn show(ui: &mut egui::Ui, filter_text: &mut String, f9_pressed: bool) -> FilterAction {
    let mut action = FilterAction::None;

    ui.horizontal(|ui| {
        ui.label("WHERE");

        let resp = ui.add(
            egui::TextEdit::singleline(filter_text)
                .hint_text("id = 1 AND nombre LIKE 'foo%'")
                .desired_width(ui.available_width() - 160.0),
        );

        // Enter en el campo también aplica
        if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            action = FilterAction::Apply;
        }

        if ui.button("Aplicar  F9").clicked() || f9_pressed {
            action = FilterAction::Apply;
        }

        if !filter_text.is_empty() {
            if ui.button("[x]").on_hover_text("Limpiar filtro").clicked() {
                filter_text.clear();
                action = FilterAction::Clear;
            }
        }
    });

    action
}

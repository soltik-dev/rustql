use eframe::egui;

use super::PendingEdit;
use crate::ui::icons::{PLUS, TRASH};
use crate::ui::{CMD, theme};

/// Lo que el usuario pulsa en la barra de cambios pendientes.
#[derive(Debug, Default, PartialEq)]
pub enum ApplyBarAction {
    #[default]
    None,
    Apply,
    Discard,
}

/// Barra inferior de cambios pendientes: cuántos hay y los botones de
/// guardarlos o descartarlos.
pub fn show_apply_bar(
    ui: &mut egui::Ui,
    pending_edits: &[PendingEdit],
    pending_deletes: usize,
    pending_inserts: usize,
    ctrl_enter: bool,
    pk_missing: bool,
) -> ApplyBarAction {
    let mut action = ApplyBarAction::None;
    let p = theme::get(ui);

    ui.horizontal(|ui| {
        if pk_missing {
            ui.colored_label(p.warning, "⚠ Sin clave primaria — no se puede guardar");
        } else {
            if !pending_edits.is_empty() {
                theme::badge(
                    ui,
                    &format!("● {} edición(es)", pending_edits.len()),
                    p.warning,
                );
            }
            if pending_deletes > 0 {
                theme::badge(
                    ui,
                    &format!("{} {} eliminación(es)", TRASH, pending_deletes),
                    p.danger,
                );
            }
            if pending_inserts > 0 {
                theme::badge(
                    ui,
                    &format!("{} {} inserción(es)", PLUS, pending_inserts),
                    p.success,
                );
            }
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Descartar").clicked() {
                action = ApplyBarAction::Discard;
            }
            if !pk_missing {
                let apply_btn = theme::primary_button(ui, format!("Guardar  {CMD}Enter"));
                if apply_btn.clicked() || ctrl_enter {
                    action = ApplyBarAction::Apply;
                }
            }
        });
    });

    action
}

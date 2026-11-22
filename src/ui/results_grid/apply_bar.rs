use eframe::egui;

use super::PendingEdit;
use crate::i18n::t;
use crate::ui::icons::{PLUS, TRASH};
use crate::ui::{CMD, theme};

/// What the user clicks in the pending changes bar.
#[derive(Debug, Default, PartialEq)]
pub enum ApplyBarAction {
    #[default]
    None,
    Apply,
    Discard,
}

/// Bottom bar of pending changes: how many there are and the buttons to
/// save or discard them.
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
            ui.colored_label(p.warning, format!("⚠ {}", t!("apply-no-primary-key")));
        } else {
            if !pending_edits.is_empty() {
                theme::badge(
                    ui,
                    &format!("● {}", t!("apply-edits", count = pending_edits.len())),
                    p.warning,
                );
            }
            if pending_deletes > 0 {
                theme::badge(
                    ui,
                    &format!("{TRASH} {}", t!("apply-deletes", count = pending_deletes)),
                    p.danger,
                );
            }
            if pending_inserts > 0 {
                theme::badge(
                    ui,
                    &format!("{PLUS} {}", t!("apply-inserts", count = pending_inserts)),
                    p.success,
                );
            }
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button(t!("apply-discard")).clicked() {
                action = ApplyBarAction::Discard;
            }
            if !pk_missing {
                let apply_btn =
                    theme::primary_button(ui, format!("{}  {CMD}Enter", t!("apply-save")));
                if apply_btn.clicked() || ctrl_enter {
                    action = ApplyBarAction::Apply;
                }
            }
        });
    });

    action
}

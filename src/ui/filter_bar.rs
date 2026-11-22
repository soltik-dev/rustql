use eframe::egui;

use crate::i18n::t;
use crate::ui::theme;

#[derive(Default)]
pub enum FilterAction {
    #[default]
    None,
    Apply,
    Clear,
}

/// WHERE filter bar above the grid.
/// `filter_history` holds the last filters applied (most recent first).
/// Returns the action the user took this frame.
pub fn show(
    ui: &mut egui::Ui,
    filter_text: &mut String,
    f9_pressed: bool,
    filter_history: &[String],
) -> FilterAction {
    let mut action = FilterAction::None;

    ui.horizontal(|ui| {
        // ── History button ───────────────────────────────────────────────
        let hist_btn = ui.button("▾").on_hover_text(t!("filter-history-hint"));
        let popup_id = ui.make_persistent_id("filter_history_popup");

        if hist_btn.clicked() {
            egui::Popup::toggle_id(ui.ctx(), popup_id);
        }

        let selected = egui::Popup::from_response(&hist_btn)
            .id(popup_id)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .open_memory(None::<egui::SetOpenCommand>)
            .show(|ui| {
                ui.set_min_width(280.0);
                let mut picked: Option<String> = None;
                if filter_history.is_empty() {
                    ui.label(t!("filter-no-history"));
                } else {
                    egui::ScrollArea::vertical()
                        .max_height(200.0)
                        .show(ui, |ui| {
                            for entry in filter_history {
                                if ui.selectable_label(false, entry).clicked() {
                                    picked = Some(entry.clone());
                                    egui::Popup::close_id(ui.ctx(), popup_id);
                                }
                            }
                        });
                }
                picked
            });

        if let Some(inner) = selected
            && let Some(entry) = inner.inner
        {
            *filter_text = entry;
            action = FilterAction::Apply;
        }

        // ── Label + text field ───────────────────────────────────────────
        ui.monospace(egui::RichText::new("WHERE").color(theme::get(ui).sql_keyword));

        let resp = ui
            .add(
                egui::TextEdit::singleline(filter_text)
                    .hint_text(t!("filter-hint"))
                    .desired_width(ui.available_width() - 160.0),
            )
            .on_hover_text(t!("filter-field-hint"));

        // Enter in the field also applies
        if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            action = FilterAction::Apply;
        }

        if theme::primary_button(ui, format!("{}  F9", t!("filter-apply"))).clicked() || f9_pressed
        {
            action = FilterAction::Apply;
        }

        if !filter_text.is_empty()
            && ui
                .add(egui::Button::new("×").frame(false))
                .on_hover_text(t!("filter-clear"))
                .clicked()
        {
            filter_text.clear();
            action = FilterAction::Clear;
        }
    });

    action
}

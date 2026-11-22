use eframe::egui;

use crate::i18n::t;
use crate::ui::icons::*;
use crate::ui::{CMD, ellipsize, theme};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ActiveTab {
    #[default]
    Servidor,
    Bbdd,
    Tabla,
    Datos,
    /// SQL tab, by its position in `Session::sql_tabs`.
    Sql(usize),
}

#[derive(Default)]
pub struct TabBarResult {
    pub tab_clicked: Option<ActiveTab>,
    pub new_sql_tab: bool,
    pub close_sql_tab: Option<usize>,
    /// New name of a SQL tab (empty: back to "SQL n").
    pub renamed: Option<(usize, String)>,
}

/// SQL tab whose name is being edited in the bar itself.
#[derive(Debug)]
pub struct TabRename {
    pub idx: usize,
    pub buffer: String,
    /// First frame: the field still has to take focus.
    needs_focus: bool,
}

impl TabRename {
    pub fn new(idx: usize, name: String) -> Self {
        Self {
            idx,
            buffer: name,
            needs_focus: true,
        }
    }
}

/// What the bar shows: the active tab and what each one refers to.
pub struct TabBarView<'a> {
    pub active: ActiveTab,
    pub server_name: &'a str,
    /// Selected database; without it its tab is disabled.
    pub db_name: Option<&'a str>,
    /// Open table; without it Structure and Data are disabled.
    pub table_name: Option<&'a str>,
    /// Title of each SQL tab.
    pub sql_titles: &'a [String],
}

/// Tab bar of the session. `rename` is its own state: the SQL tab whose
/// name is being edited.
pub fn show(ui: &mut egui::Ui, view: &TabBarView, rename: &mut Option<TabRename>) -> TabBarResult {
    let TabBarView {
        active,
        server_name,
        db_name,
        table_name,
        sql_titles,
    } = *view;
    let (has_db, has_table) = (db_name.is_some(), table_name.is_some());
    let mut result = TabBarResult::default();
    // The tab being renamed no longer exists
    if rename.as_ref().is_some_and(|r| r.idx >= sql_titles.len()) {
        *rename = None;
    }

    ui.spacing_mut().button_padding = egui::vec2(10.0, 5.0);
    ui.spacing_mut().item_spacing = egui::vec2(3.0, 3.0);

    // Context tabs + SQL tabs, wrapping to a new line if they do not fit
    ui.horizontal_wrapped(|ui| {
        // Server
        let srv_label = format!("{PLUG} {}", ellipsize(server_name, 18));
        if tab_btn(ui, &srv_label, active == ActiveTab::Servidor, true).clicked() {
            result.tab_clicked = Some(ActiveTab::Servidor);
        }

        // Database
        let db_label = format!(
            "{DATABASE} {}",
            ellipsize(db_name.unwrap_or(&t!("tab-database")), 18)
        );
        if tab_btn(ui, &db_label, active == ActiveTab::Bbdd, has_db).clicked() {
            result.tab_clicked = Some(ActiveTab::Bbdd);
        }

        // Structure and Data belong to the same table: its name only goes in
        // Data (both tabs used to say the same and only the icon told them
        // apart)
        let struct_label = format!("{EDIT} {}", t!("tab-structure"));
        let mut struct_tab = tab_btn(ui, &struct_label, active == ActiveTab::Tabla, has_table);
        if let Some(table) = table_name {
            struct_tab = struct_tab.on_hover_text(t!("tab-structure-of", table = table));
        }
        if struct_tab.clicked() {
            result.tab_clicked = Some(ActiveTab::Tabla);
        }

        // Data
        let datos_label = format!(
            "{GRID} {}",
            ellipsize(table_name.unwrap_or(&t!("tab-data")), 18)
        );
        if tab_btn(ui, &datos_label, active == ActiveTab::Datos, has_table).clicked() {
            result.tab_clicked = Some(ActiveTab::Datos);
        }

        tab_separator(ui);

        // SQL tabs
        for (i, title) in sql_titles.iter().enumerate() {
            if rename.as_ref().is_some_and(|r| r.idx == i) {
                rename_field(ui, rename, &mut result);
                continue;
            }
            let is_active = active == ActiveTab::Sql(i);
            let label = format!("{CODE} {}", ellipsize(title, 24));
            let tab = tab_btn(ui, &label, is_active, true);
            if tab.clicked() {
                result.tab_clicked = Some(ActiveTab::Sql(i));
            }
            if tab.double_clicked() {
                *rename = Some(TabRename::new(i, title.clone()));
            }
            tab.context_menu(|ui| {
                if ui.button(format!("{EDIT}  {}", t!("tab-rename"))).clicked() {
                    *rename = Some(TabRename::new(i, title.clone()));
                    ui.close();
                }
                if ui
                    .button(format!("×  {}  {CMD}W", t!("tab-close")))
                    .clicked()
                {
                    result.close_sql_tab = Some(i);
                    ui.close();
                }
            });
            let close_resp = ui
                .add(egui::Button::new("×").small().frame(false))
                .on_hover_text(t!("tab-close-sql-hint", shortcut = format!("{CMD}W")));
            // Middle click, as on the connection tabs
            if close_resp.clicked() || tab.middle_clicked() {
                result.close_sql_tab = Some(i);
            }
        }

        // New SQL tab
        if ui
            .add(egui::Button::new("＋").small().frame(false))
            .on_hover_text(format!("{}  {CMD}T", t!("tab-new-sql")))
            .clicked()
        {
            result.new_sql_tab = true;
        }
    });

    result
}

/// Field where the name of a SQL tab is edited, in its place in the bar.
/// Enter or leaving the field confirms it; Escape leaves it as it was.
fn rename_field(ui: &mut egui::Ui, rename: &mut Option<TabRename>, result: &mut TabBarResult) {
    let Some(state) = rename else {
        return;
    };
    let resp = ui.add(
        egui::TextEdit::singleline(&mut state.buffer)
            .desired_width(130.0)
            .hint_text(t!("tab-name-hint")),
    );
    if state.needs_focus {
        state.needs_focus = false;
        resp.request_focus();
        return;
    }
    if resp.lost_focus() {
        if !ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            result.renamed = Some((state.idx, state.buffer.trim().to_string()));
        }
        *rename = None;
    }
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

/// Tab: transparent background, surface hover and accent underline when
/// active.
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

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        ctx: egui::Context,
        titles: Vec<String>,
        rename: Option<TabRename>,
        time: f64,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                ctx: egui::Context::default(),
                titles: vec!["SQL 1".into(), "SQL 2".into()],
                rename: None,
                time: 0.0,
            }
        }

        fn frame(&mut self, events: Vec<egui::Event>) -> TabBarResult {
            self.time += 0.02;
            let input = egui::RawInput {
                time: Some(self.time),
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 200.0),
                )),
                events,
                ..Default::default()
            };
            let mut result = TabBarResult::default();
            self.ctx
                .run_ui(input, |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        let view = TabBarView {
                            active: ActiveTab::Servidor,
                            server_name: "srv",
                            db_name: None,
                            table_name: None,
                            sql_titles: &self.titles,
                        };
                        result = show(ui, &view, &mut self.rename);
                    });
                })
                .drop_without_applying_deltas();
            result
        }

        fn click(&mut self, pos: egui::Pos2) {
            for pressed in [true, false] {
                let button = egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                };
                self.frame(vec![egui::Event::PointerMoved(pos), button]);
            }
        }

        fn press(&mut self, key: egui::Key) -> TabBarResult {
            self.frame(vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }])
        }

        /// Double click on the first SQL tab: the bar is scanned from left to
        /// right until it is found.
        fn start_rename(&mut self) {
            self.frame(Vec::new());
            for x in (0..900).step_by(6) {
                let pos = egui::pos2(x as f32, 25.0);
                self.click(pos);
                self.click(pos);
                if self.rename.is_some() {
                    // The field takes focus on its first frame
                    self.frame(Vec::new());
                    self.frame(Vec::new());
                    return;
                }
                // So the next attempt is not a triple click
                self.time += 1.0;
            }
            panic!("no double click started renaming");
        }
    }

    #[test]
    fn doble_clic_renombra_y_enter_confirma() {
        let mut f = Fixture::new();
        f.start_rename();
        assert_eq!(f.rename.as_ref().map(|r| r.idx), Some(0));
        assert_eq!(f.rename.as_ref().unwrap().buffer, "SQL 1");

        f.frame(vec![egui::Event::Text(" bis".into())]);
        let result = f.press(egui::Key::Enter);
        assert_eq!(result.renamed, Some((0, "SQL 1 bis".to_string())));
        assert!(f.rename.is_none());
    }

    #[test]
    fn escape_deja_el_nombre_como_estaba() {
        let mut f = Fixture::new();
        f.start_rename();
        f.frame(vec![egui::Event::Text("otro".into())]);
        let result = f.press(egui::Key::Escape);
        assert_eq!(result.renamed, None);
        assert!(f.rename.is_none());
    }

    #[test]
    fn si_la_pestana_desaparece_se_deja_de_renombrar() {
        let mut f = Fixture::new();
        f.rename = Some(TabRename::new(1, "SQL 2".into()));
        f.titles.pop();
        f.frame(Vec::new());
        assert!(f.rename.is_none());
    }
}

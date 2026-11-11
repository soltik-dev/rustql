use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use eframe::egui;

use crate::db::{CancelFlag, QUERY_CANCELLED, QueryResult};
use crate::settings::{CopyFormat, SavedSqlTab};
use crate::ui::results_grid::{self, GridAction, GridSelection};
use crate::ui::{CMD, sql_highlight, theme};

/// Alto mínimo del editor y del resultado al arrastrar el borde entre ambos.
const MIN_EDITOR_HEIGHT: f32 = 60.0;
const MIN_RESULT_HEIGHT: f32 = 120.0;
/// Alto de la franja que se arrastra para cambiar el alto del editor.
const HANDLE_HEIGHT: f32 = 9.0;

/// Una pestaña del editor SQL: la consulta, su contexto y el último
/// resultado.
pub struct SqlTab {
    /// Único en la aplicación: separa el estado de sus widgets (cursor del
    /// editor, deshacer, scroll) del de las demás pestañas.
    uid: u64,
    /// Nombre que le dio el usuario; sin él se llama «SQL n».
    pub name: Option<String>,
    pub db_context: String,
    pub sql_text: String,
    pub result: Option<Result<QueryResult, String>>,
    /// Ejecución en curso (su número), si la hay.
    pub running: Option<u64>,
    /// Señal para cancelar la ejecución en curso; activada, ya se ha pedido.
    pub cancel: Option<CancelFlag>,
    /// Ejecución que dio `result`: distingue un resultado del siguiente.
    pub result_run: u64,
    /// Lo que tardó la consulta de `result`.
    pub elapsed_ms: Option<u64>,
    /// Cursor y filas seleccionadas en el grid del resultado.
    pub selection: GridSelection,
    /// Texto seleccionado en el editor: se ejecuta eso en vez de la consulta
    /// entera. Solo cuenta mientras se ve, con el foco en el editor.
    pub selected_sql: Option<String>,
    /// Alto del editor elegido arrastrando su borde; `None`, el inicial.
    pub editor_height: Option<f32>,
}

impl Default for SqlTab {
    fn default() -> Self {
        static NEXT_UID: AtomicU64 = AtomicU64::new(1);
        Self {
            uid: NEXT_UID.fetch_add(1, Ordering::Relaxed),
            name: None,
            db_context: String::new(),
            sql_text: String::new(),
            result: None,
            running: None,
            cancel: None,
            result_run: 0,
            elapsed_ms: None,
            selection: GridSelection::default(),
            selected_sql: None,
            editor_height: None,
        }
    }
}

impl SqlTab {
    /// Pestaña nueva con esa consulta.
    pub fn new(db_context: String, sql_text: String) -> Self {
        Self {
            db_context,
            sql_text,
            ..Default::default()
        }
    }

    /// Pestaña guardada en los ajustes, sin resultado.
    pub fn restore(saved: &SavedSqlTab) -> Self {
        Self {
            name: saved.name.clone(),
            editor_height: saved.editor_height,
            ..Self::new(saved.db_context.clone(), saved.sql_text.clone())
        }
    }

    /// Lo que se guarda de la pestaña en los ajustes.
    pub fn saved(&self) -> SavedSqlTab {
        SavedSqlTab {
            name: self.name.clone(),
            db_context: self.db_context.clone(),
            sql_text: self.sql_text.clone(),
            editor_height: self.editor_height,
        }
    }

    /// Título de la pestaña que ocupa la posición `idx`.
    pub fn title(&self, idx: usize) -> String {
        self.name
            .clone()
            .unwrap_or_else(|| format!("SQL {}", idx + 1))
    }

    /// Lo que ejecutaría la pestaña ahora: la selección del editor si la hay
    /// y, si no, la consulta entera. `None` si no hay nada que ejecutar.
    pub fn sql_to_run(&self) -> Option<String> {
        let sql = self
            .selected_sql
            .as_deref()
            .map(str::trim)
            .filter(|sel| !sel.is_empty())
            .unwrap_or_else(|| self.sql_text.trim());
        (!sql.is_empty()).then(|| sql.to_string())
    }
}

#[derive(Debug, Default, PartialEq)]
pub enum SqlEditorAction {
    #[default]
    None,
    Execute {
        sql: String,
        db_context: String,
    },
    /// Borrar la consulta (la sesión pregunta antes).
    Clear,
    /// Cancelar la consulta que se está ejecutando.
    Cancel,
}

/// Panel "SQL": editor de consultas libre. Mientras la consulta anterior no
/// termina no se puede lanzar otra. `run_pressed`: el atajo de ejecutar
/// (Ctrl+Enter, o F9 como en HeidiSQL). `copy_format`: formato en el que
/// Ctrl+C copia las filas seleccionadas del resultado.
pub fn show(
    ui: &mut egui::Ui,
    tab: &mut SqlTab,
    run_pressed: bool,
    databases: &[String],
    copy_format: &CopyFormat,
) -> SqlEditorAction {
    ui.push_id(("sql_tab", tab.uid), |ui| {
        let (mut action, execute_held) = toolbar(ui, tab, databases);
        if run_pressed
            && tab.running.is_none()
            && let Some(sql) = tab.sql_to_run()
        {
            action = SqlEditorAction::Execute {
                sql,
                db_context: tab.db_context.clone(),
            };
        }
        ui.separator();
        editor(ui, tab, execute_held);
        result(ui, tab, copy_format);
        action
    })
    .inner
}

/// Barra de controles: BBDD de contexto, ejecutar, limpiar y cancelar.
/// Devuelve también si se está pulsando el botón de ejecutar.
fn toolbar(ui: &mut egui::Ui, tab: &mut SqlTab, databases: &[String]) -> (SqlEditorAction, bool) {
    let mut action = SqlEditorAction::None;
    let mut execute_held = false;
    let running = tab.running.is_some();
    let cancelling = tab
        .cancel
        .as_ref()
        .is_some_and(|c| c.load(Ordering::SeqCst));

    ui.horizontal(|ui| {
        ui.label("BD:");
        let db_context = &mut tab.db_context;
        egui::ComboBox::from_id_salt("sql_db_context")
            .selected_text(if db_context.is_empty() {
                "(ninguna)"
            } else {
                db_context.as_str()
            })
            .width(140.0)
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(db_context.is_empty(), "(ninguna)")
                    .clicked()
                {
                    db_context.clear();
                }
                for db in databases {
                    if ui.selectable_label(db_context == db, db).clicked() {
                        *db_context = db.clone();
                    }
                }
            });

        ui.separator();
        ui.label("SQL libre:");
        // Con texto seleccionado en el editor se ejecuta solo eso
        let has_selection = tab
            .selected_sql
            .as_deref()
            .is_some_and(|sel| !sel.trim().is_empty());
        let label = if has_selection {
            "Ejecutar selección"
        } else {
            "Ejecutar"
        };
        let execute = theme::primary_button_enabled(
            ui,
            !running,
            format!("{} {label}  {CMD}Enter", crate::ui::icons::CODE),
        );
        execute_held = execute.is_pointer_button_down_on();
        #[cfg(test)]
        ui.data_mut(|d| d.insert_temp(egui::Id::unique("sql_execute_rect"), execute.rect));
        if !running
            && execute.clicked()
            && let Some(sql) = tab.sql_to_run()
        {
            action = SqlEditorAction::Execute {
                sql,
                db_context: tab.db_context.clone(),
            };
        }
        let has_text = !tab.sql_text.trim().is_empty();
        if ui
            .add_enabled(has_text, egui::Button::new("Limpiar"))
            .clicked()
        {
            action = SqlEditorAction::Clear;
        }
        if running {
            ui.spinner();
            if cancelling {
                ui.label("Cancelando…");
            } else {
                ui.label("Ejecutando…");
                if ui
                    .button("Cancelar")
                    .on_hover_text("Interrumpe la consulta en el servidor")
                    .clicked()
                {
                    action = SqlEditorAction::Cancel;
                }
            }
        }
    });
    (action, execute_held)
}

/// Editor con resaltado de sintaxis y, debajo, la franja con la que se
/// cambia su alto. Si la consulta no cabe, el editor se desplaza.
fn editor(ui: &mut egui::Ui, tab: &mut SqlTab, execute_held: bool) {
    let p = theme::get(ui);
    let available = ui.available_height();
    let max_height = (available - MIN_RESULT_HEIGHT).max(MIN_EDITOR_HEIGHT);
    let height = tab
        .editor_height
        .unwrap_or((available * 0.35).clamp(80.0, 300.0))
        .clamp(MIN_EDITOR_HEIGHT, max_height);

    let dark = ui.visuals().dark_mode;
    let font_size = egui::TextStyle::Monospace.resolve(ui.style()).size;
    let mut layouter = move |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap_width: f32| {
        let mut job = sql_highlight::highlight(ui.ctx(), dark, font_size, buf.as_str());
        job.wrap.max_width = wrap_width;
        ui.fonts_mut(|f| f.layout_job(job))
    };
    let layouter: &mut dyn FnMut(&egui::Ui, &dyn egui::TextBuffer, f32) -> Arc<egui::Galley> =
        &mut layouter;

    egui::Frame::NONE
        .fill(p.bg_sunken)
        .stroke(egui::Stroke::new(1.0, p.border_subtle))
        .corner_radius(egui::CornerRadius::same(theme::RADIUS_MD))
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            // Barra a la vista (la de egui solo aparece al pasar el ratón):
            // dice que la consulta sigue más abajo
            ui.spacing_mut().scroll = egui::style::ScrollStyle::thin();
            egui::ScrollArea::vertical()
                .id_salt("sql_editor_scroll")
                .max_height(height)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let output = egui::TextEdit::multiline(&mut tab.sql_text)
                        .code_editor()
                        .layouter(layouter)
                        .frame(egui::Frame::NONE)
                        .hint_text("SELECT * FROM tabla LIMIT 100")
                        .desired_width(f32::INFINITY)
                        // Todo el recuadro es editor: un clic en su parte
                        // vacía también le da el foco
                        .min_size(egui::vec2(0.0, height))
                        .show(ui);
                    if output.response.has_focus() {
                        tab.selected_sql = output
                            .cursor_range
                            .filter(|range| !range.is_empty())
                            .map(|range| range.slice_str(&tab.sql_text).to_string());
                    } else if !execute_held {
                        // Sin el foco la selección no se ve: ejecutarla
                        // sería ejecutar a ciegas un trozo de la consulta.
                        // Pulsar «Ejecutar» quita el foco, pero ahí se
                        // conserva hasta que el clic termina
                        tab.selected_sql = None;
                    }
                });
        });

    // Franja para arrastrar el alto; doble clic vuelve al inicial
    let (rect, handle) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), HANDLE_HEIGHT),
        egui::Sense::click_and_drag(),
    );
    let handle = handle.on_hover_cursor(egui::CursorIcon::ResizeVertical);
    if handle.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
        let dragged = height + handle.drag_delta().y;
        tab.editor_height = Some(dragged.clamp(MIN_EDITOR_HEIGHT, max_height));
    }
    if handle.double_clicked() {
        tab.editor_height = None;
    }
    let active = handle.hovered() || handle.dragged();
    let color = if active { p.accent } else { p.border_subtle };
    let center = rect.center();
    ui.painter()
        .hline(rect.x_range(), center.y, egui::Stroke::new(1.0, color));
    // Agarre en el centro: indica que la línea se puede mover
    ui.painter().rect_filled(
        egui::Rect::from_center_size(center, egui::vec2(36.0, 3.0)),
        egui::CornerRadius::same(2),
        if active { p.accent } else { p.text_disabled },
    );
}

/// Resultado de la última ejecución.
fn result(ui: &mut egui::Ui, tab: &mut SqlTab, copy_format: &CopyFormat) {
    let p = theme::get(ui);
    let muted = |ui: &mut egui::Ui, text: String| {
        ui.label(egui::RichText::new(text).color(p.text_muted));
    };
    let took = tab
        .elapsed_ms
        .map_or_else(String::new, |ms| format!(" · {ms} ms"));
    match &tab.result {
        None => {
            ui.centered_and_justified(|ui| {
                muted(
                    ui,
                    format!("Escribe una consulta y pulsa {CMD}Enter o el botón Ejecutar"),
                );
            });
        }
        // Cancelarla fue cosa del usuario: no es un error
        Some(Err(e)) if e == QUERY_CANCELLED => muted(ui, format!("Consulta cancelada{took}")),
        Some(Err(e)) => theme::error_card(ui, &format!("Error: {e}")),
        // INSERT, UPDATE, DDL…: no hay nada que mostrar
        Some(Ok(data)) if data.columns.is_empty() => {
            muted(ui, format!("Ejecutada, sin resultado que mostrar{took}"));
        }
        Some(Ok(data)) => {
            ui.horizontal(|ui| {
                muted(
                    ui,
                    format!(
                        "{} filas, {} columnas{took}",
                        data.rows.len(),
                        data.columns.len()
                    ),
                );
                if data.truncated {
                    ui.label(
                        egui::RichText::new(format!(
                            "· solo las primeras {} (hay más; el tope se cambia en Ajustes)",
                            data.rows.len()
                        ))
                        .color(p.warning),
                    );
                }
            });
            ui.separator();

            // El mismo grid que el de las tablas, en solo lectura
            let grid_action =
                results_grid::show_read_only(ui, tab.result_run, data, &mut tab.selection);
            if let GridAction::CopyRows(fmt) = grid_action {
                let fmt = fmt.as_ref().unwrap_or(copy_format);
                results_grid::copy_rows(ui.ctx(), data, &tab.selection.rows, fmt, None);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn se_ejecuta_la_seleccion_o_la_consulta_entera() {
        let mut tab = SqlTab::new(String::new(), "  SELECT 1;\nSELECT 2  ".into());
        assert_eq!(tab.sql_to_run().as_deref(), Some("SELECT 1;\nSELECT 2"));
        tab.selected_sql = Some("\nSELECT 2 ".into());
        assert_eq!(tab.sql_to_run().as_deref(), Some("SELECT 2"));
        // Una selección de solo espacios no cuenta
        tab.selected_sql = Some("  \n".into());
        assert_eq!(tab.sql_to_run().as_deref(), Some("SELECT 1;\nSELECT 2"));
        tab.sql_text = "   ".into();
        assert_eq!(tab.sql_to_run(), None);
    }

    #[test]
    fn cada_pestana_tiene_su_identidad() {
        assert_ne!(SqlTab::default().uid, SqlTab::default().uid);
    }

    /// El editor dibujado en una ventana de 900×600, con eventos reales.
    struct Fixture {
        ctx: egui::Context,
        tab: SqlTab,
    }

    impl Fixture {
        fn new(sql: &str) -> Self {
            Self {
                ctx: egui::Context::default(),
                tab: SqlTab::new(String::new(), sql.into()),
            }
        }

        fn frame(&mut self, run_pressed: bool, events: Vec<egui::Event>) -> SqlEditorAction {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 600.0),
                )),
                events,
                ..Default::default()
            };
            let mut action = SqlEditorAction::None;
            self.ctx
                .run_ui(input, |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        action = show(ui, &mut self.tab, run_pressed, &[], &CopyFormat::Csv);
                    });
                })
                .drop_without_applying_deltas();
            action
        }

        fn pointer(&mut self, pos: egui::Pos2, pressed: bool) -> SqlEditorAction {
            let button = egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            self.frame(false, vec![egui::Event::PointerMoved(pos), button])
        }

        fn click(&mut self, pos: egui::Pos2) -> SqlEditorAction {
            self.pointer(pos, true);
            self.pointer(pos, false)
        }

        fn press(&mut self, modifiers: egui::Modifiers, key: egui::Key) {
            let event = egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            };
            self.frame(false, vec![egui::Event::ModifiersChanged(modifiers), event]);
            self.frame(
                false,
                vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)],
            );
        }

        /// Clic en el editor (lo enfoca) y selección de sus últimos `n`
        /// caracteres.
        fn select_last(&mut self, n: usize) {
            self.frame(false, Vec::new());
            self.click(egui::pos2(300.0, 100.0));
            let ctrl = egui::Modifiers::COMMAND | egui::Modifiers::CTRL;
            self.press(ctrl, egui::Key::End);
            for _ in 0..n {
                self.press(egui::Modifiers::SHIFT, egui::Key::ArrowLeft);
            }
        }

        fn execute_button(&self) -> egui::Pos2 {
            self.ctx
                .data(|d| d.get_temp::<egui::Rect>(egui::Id::unique("sql_execute_rect")))
                .expect("el botón Ejecutar")
                .center()
        }
    }

    fn executed(action: SqlEditorAction) -> Option<String> {
        match action {
            SqlEditorAction::Execute { sql, .. } => Some(sql),
            _ => None,
        }
    }

    #[test]
    fn el_atajo_ejecuta_solo_lo_seleccionado() {
        let mut f = Fixture::new("SELECT 1;\nSELECT 22");
        assert_eq!(
            executed(f.frame(true, Vec::new())).as_deref(),
            Some("SELECT 1;\nSELECT 22"),
            "sin selección, la consulta entera"
        );
        f.select_last(9);
        assert_eq!(f.tab.selected_sql.as_deref(), Some("SELECT 22"));
        assert_eq!(
            executed(f.frame(true, Vec::new())).as_deref(),
            Some("SELECT 22")
        );
    }

    #[test]
    fn el_boton_ejecuta_la_seleccion_aunque_el_clic_quite_el_foco() {
        let mut f = Fixture::new("SELECT 1;\nSELECT 22");
        f.select_last(9);
        let button = f.execute_button();
        f.pointer(button, true);
        assert_eq!(
            f.tab.selected_sql.as_deref(),
            Some("SELECT 22"),
            "se conserva mientras se pulsa el botón"
        );
        assert_eq!(
            executed(f.pointer(button, false)).as_deref(),
            Some("SELECT 22")
        );
    }

    #[test]
    fn sin_el_foco_en_el_editor_no_hay_seleccion_que_ejecutar() {
        let mut f = Fixture::new("SELECT 1;\nSELECT 22");
        f.select_last(9);
        // Clic fuera del editor (en la zona del resultado)
        f.click(egui::pos2(300.0, 550.0));
        assert_eq!(f.tab.selected_sql, None);
        assert_eq!(
            executed(f.frame(true, Vec::new())).as_deref(),
            Some("SELECT 1;\nSELECT 22")
        );
    }

    #[test]
    fn deshacer_no_trae_el_texto_de_otra_pestana() {
        // Las dos pestañas se dibujan en el mismo sitio: sin identidad
        // propia compartirían el historial de deshacer del editor
        let mut f = Fixture::new("primera");
        f.frame(false, Vec::new());
        f.click(egui::pos2(300.0, 100.0));
        f.frame(false, vec![egui::Event::Text(" consulta".into())]);
        assert_eq!(f.tab.sql_text, "primera consulta");

        f.tab = SqlTab::new(String::new(), "segunda".into());
        f.frame(false, Vec::new());
        f.click(egui::pos2(300.0, 100.0));
        let ctrl = egui::Modifiers::COMMAND | egui::Modifiers::CTRL;
        f.press(ctrl, egui::Key::Z);
        assert_eq!(f.tab.sql_text, "segunda");
    }
}

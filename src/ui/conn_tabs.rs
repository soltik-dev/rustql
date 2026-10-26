//! Barra superior de pestañas de conexión: una por sesión, con el color de su
//! conexión, y botón para abrir una nueva.

use eframe::egui;

use crate::ui::icons::PLUS;
use crate::ui::theme;

/// Lo que la barra necesita de cada sesión.
pub struct ConnTab<'a> {
    pub title: &'a str,
    /// «user@host:port»; vacío mientras la pestaña está en el formulario.
    pub tooltip: String,
    pub color: Option<egui::Color32>,
    /// Conectando o con una query en curso: spinner en lugar del punto.
    pub busy: bool,
    /// Cambios sin guardar: punto de aviso en el hueco del botón de cerrar.
    pub pending: bool,
}

pub enum ConnTabsAction {
    None,
    Select(usize),
    Close(usize),
    New,
}

const PAD: f32 = 10.0;
const DOT: f32 = 10.0;
const GAP: f32 = 7.0;
const CLOSE: f32 = 18.0;
const MAX_TITLE_W: f32 = 180.0;

pub fn show(ui: &mut egui::Ui, tabs: &[ConnTab], active: usize) -> ConnTabsAction {
    let mut action = ConnTabsAction::None;
    let height = ui.available_height();

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (i, tab) in tabs.iter().enumerate() {
            let (resp, close_resp) = tab_widget(ui, tab, i == active, height);
            if close_resp.clicked() || resp.middle_clicked() {
                action = ConnTabsAction::Close(i);
            } else if resp.clicked() {
                action = ConnTabsAction::Select(i);
            }
            if !tab.tooltip.is_empty() {
                resp.on_hover_text(&tab.tooltip);
            }
        }

        ui.add_space(4.0);
        if ui
            .add(egui::Button::new(PLUS).frame(false))
            .on_hover_text("Nueva conexión")
            .clicked()
        {
            action = ConnTabsAction::New;
        }
    });

    action
}

/// Pestaña dibujada a mano: [punto/spinner] título [×]. La activa se funde
/// con el contenido (fondo base) y lleva el tinte y la franja de su color.
fn tab_widget(
    ui: &mut egui::Ui,
    tab: &ConnTab,
    active: bool,
    height: f32,
) -> (egui::Response, egui::Response) {
    let p = theme::get(ui);
    let text_color = if active { p.text_primary } else { p.text_muted };

    let mut job = egui::text::LayoutJob::simple_singleline(
        tab.title.to_owned(),
        egui::TextStyle::Body.resolve(ui.style()),
        text_color,
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(MAX_TITLE_W);
    let galley = ui.painter().layout_job(job);

    let width = PAD + DOT + GAP + galley.size().x + GAP + CLOSE + 6.0;
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click());
    let close_rect = egui::Rect::from_center_size(
        egui::pos2(rect.right() - 6.0 - CLOSE / 2.0, rect.center().y),
        egui::vec2(CLOSE, CLOSE),
    );
    // Registrado después: tiene prioridad sobre la pestaña en su hueco
    let close_resp = ui.interact(close_rect, resp.id.with("close"), egui::Sense::click());
    let hovered = resp.hovered() || close_resp.hovered();

    let painter = ui.painter();
    let top_radius = egui::CornerRadius {
        nw: theme::RADIUS_MD,
        ne: theme::RADIUS_MD,
        sw: 0,
        se: 0,
    };

    // Fondo
    if active {
        painter.rect_filled(rect, top_radius, p.bg_base);
        if let Some(c) = tab.color {
            painter.rect_filled(rect, top_radius, c.gamma_multiply(0.18));
            painter.hline(
                rect.x_range().shrink(3.0),
                rect.top() + 1.0,
                egui::Stroke::new(2.0, c),
            );
        }
    } else if hovered {
        painter.rect_filled(rect, top_radius, p.bg_surface);
    }

    // Punto de color (anillo si no tiene) o spinner
    let dot_rect = egui::Rect::from_center_size(
        egui::pos2(rect.left() + PAD + DOT / 2.0, rect.center().y),
        egui::vec2(DOT, DOT),
    );
    if tab.busy {
        egui::Spinner::new()
            .color(tab.color.unwrap_or(p.text_muted))
            .paint_at(ui, dot_rect.expand(3.0));
    } else {
        match tab.color {
            Some(c) => {
                painter.circle_filled(dot_rect.center(), 4.0, c);
            }
            None => {
                painter.circle_stroke(
                    dot_rect.center(),
                    3.5,
                    egui::Stroke::new(1.0, p.text_disabled),
                );
            }
        }
    }

    // Título
    let text_pos = egui::pos2(
        dot_rect.right() + GAP,
        rect.center().y - galley.size().y / 2.0,
    );
    painter.galley(text_pos, galley, text_color);

    // Hueco de cerrar: × en la activa o al pasar el ratón; si hay cambios
    // pendientes, un punto de aviso que pasa a × con el ratón encima.
    if tab.pending && !hovered {
        painter.circle_filled(close_rect.center(), 3.5, p.warning);
    } else if active || hovered {
        if close_resp.hovered() {
            painter.rect_filled(close_rect, theme::RADIUS_SM, p.bg_raised);
        }
        let color = if close_resp.hovered() {
            p.text_primary
        } else {
            p.text_muted
        };
        painter.text(
            close_rect.center(),
            egui::Align2::CENTER_CENTER,
            "×",
            egui::TextStyle::Body.resolve(ui.style()),
            color,
        );
    }

    (resp, close_resp.on_hover_text("Cerrar conexión"))
}

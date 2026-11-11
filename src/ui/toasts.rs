//! Avisos no modales: mensajes breves en la esquina inferior derecha que
//! confirman lo que se acaba de hacer («cambios guardados», «3 filas
//! copiadas») o cuentan un fallo leve, sin interrumpir. Desaparecen solos;
//! un clic los quita antes. Lo que exige una decisión o leerse con calma
//! sigue yendo en un diálogo.

use std::collections::VecDeque;
use std::time::Duration;

use eframe::egui;

use crate::ui::theme;

/// Avisos a la vez en pantalla: con más, se van los más antiguos.
const MAX_VISIBLE: usize = 4;
const MAX_WIDTH: f32 = 380.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ToastKind {
    Success,
    Info,
    Error,
}

impl ToastKind {
    /// Segundos que el aviso está a la vista: un error se lee más despacio.
    fn seconds(self) -> f64 {
        match self {
            Self::Success | Self::Info => 4.0,
            Self::Error => 10.0,
        }
    }
}

#[derive(Debug)]
struct Toast {
    kind: ToastKind,
    text: String,
    /// Instante (del reloj de egui) en que desaparece. Se fija la primera
    /// vez que se dibuja: lo que tarde en llegar a pantalla no cuenta.
    expires_at: Option<f64>,
}

/// Cola de avisos de la aplicación, del más antiguo al más reciente.
#[derive(Debug, Default)]
pub struct Toasts {
    items: VecDeque<Toast>,
}

impl Toasts {
    pub fn success(&mut self, text: impl Into<String>) {
        self.push(ToastKind::Success, text.into());
    }

    pub fn info(&mut self, text: impl Into<String>) {
        self.push(ToastKind::Info, text.into());
    }

    pub fn error(&mut self, text: impl Into<String>) {
        self.push(ToastKind::Error, text.into());
    }

    fn push(&mut self, kind: ToastKind, text: String) {
        // Repetir la acción (Ctrl+C varias veces) renueva el aviso en vez
        // de apilar copias
        self.items.retain(|t| !(t.kind == kind && t.text == text));
        self.items.push_back(Toast {
            kind,
            text,
            expires_at: None,
        });
        while self.items.len() > MAX_VISIBLE {
            self.items.pop_front();
        }
    }

    /// Dibuja los avisos vigentes, el más reciente abajo, a `bottom_margin`
    /// del borde inferior de la ventana. Llamar una vez por frame, después
    /// de todo lo demás.
    pub fn show(&mut self, ctx: &egui::Context, bottom_margin: f32) {
        let now = ctx.input(|i| i.time);
        self.items
            .retain(|t| t.expires_at.is_none_or(|at| at > now));
        if self.items.is_empty() {
            return;
        }

        let mut dismissed = None;
        egui::Area::new(egui::Id::unique("rustql_toasts"))
            .order(egui::Order::Foreground)
            .anchor(
                egui::Align2::RIGHT_BOTTOM,
                egui::vec2(-12.0, -bottom_margin),
            )
            .show(ctx, |ui| {
                ui.with_layout(egui::Layout::top_down(egui::Align::RIGHT), |ui| {
                    for (idx, toast) in self.items.iter_mut().enumerate() {
                        let expires_at =
                            *toast.expires_at.get_or_insert(now + toast.kind.seconds());
                        let resp = draw(ui, toast);
                        if resp.clicked() {
                            dismissed = Some(idx);
                        } else if resp.hovered() {
                            // Mientras se lee no se va
                            toast.expires_at = Some(expires_at.max(now + 1.5));
                        }
                        ui.add_space(6.0);
                    }
                });
            });
        if let Some(idx) = dismissed {
            self.items.remove(idx);
        }

        // Sin entrada del usuario egui no repinta: despertar al caducar
        let next = self
            .items
            .iter()
            .filter_map(|t| t.expires_at)
            .fold(f64::INFINITY, f64::min);
        if next.is_finite() {
            ctx.request_repaint_after(Duration::from_secs_f64((next - now).max(0.0)));
        }
    }

    /// Textos de los avisos vigentes, del más antiguo al más reciente.
    #[cfg(test)]
    pub fn texts(&self) -> Vec<&str> {
        self.items.iter().map(|t| t.text.as_str()).collect()
    }

    #[cfg(test)]
    pub fn has_error(&self) -> bool {
        self.items.iter().any(|t| t.kind == ToastKind::Error)
    }
}

/// Un aviso: tarjeta con un punto del color de su tipo y el texto.
fn draw(ui: &mut egui::Ui, toast: &Toast) -> egui::Response {
    let p = theme::get(ui);
    let color = match toast.kind {
        ToastKind::Success => p.success,
        ToastKind::Info => p.accent,
        ToastKind::Error => p.danger,
    };
    // El punto va en el margen izquierdo, a la altura de la primera línea
    let dot_space = 16;
    let frame = egui::Frame::NONE
        .fill(p.bg_raised)
        .stroke(egui::Stroke::new(1.0, color.gamma_multiply(0.6)))
        .corner_radius(egui::CornerRadius::same(theme::RADIUS_MD))
        .inner_margin(egui::Margin {
            left: 10 + dot_space,
            right: 12,
            top: 8,
            bottom: 8,
        })
        .shadow(ui.visuals().popup_shadow)
        .show(ui, |ui| {
            // Texto maquetado a mano: la tarjeta se ciñe a él y, aunque el
            // aviso se alinee a la derecha de la ventana, se lee de izquierda
            // a derecha
            let font = egui::TextStyle::Body.resolve(ui.style());
            let galley = ui
                .painter()
                .layout(toast.text.clone(), font, p.text_primary, MAX_WIDTH);
            let (rect, _) = ui.allocate_exact_size(galley.size(), egui::Sense::hover());
            ui.painter().galley(rect.min, galley, p.text_primary);
            rect
        });
    let first_line = ui.text_style_height(&egui::TextStyle::Body);
    let text_rect = frame.inner;
    let dot = egui::pos2(
        text_rect.left() - f32::from(dot_space) / 2.0 - 2.0,
        text_rect.top() + first_line / 2.0,
    );
    ui.painter().circle_filled(dot, 4.0, color);
    frame
        .response
        .interact(egui::Sense::click())
        .on_hover_text("Clic para cerrar")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Un frame en el instante `time` (segundos).
    fn frame(toasts: &mut Toasts, ctx: &egui::Context, time: f64) {
        let input = egui::RawInput {
            time: Some(time),
            ..Default::default()
        };
        ctx.run_ui(input, |ui| toasts.show(ui.ctx(), 40.0))
            .drop_without_applying_deltas();
    }

    #[test]
    fn un_aviso_desaparece_solo_y_un_error_dura_mas() {
        let ctx = egui::Context::default();
        let mut toasts = Toasts::default();
        toasts.success("guardado");
        toasts.error("falló");
        frame(&mut toasts, &ctx, 100.0);
        assert_eq!(toasts.texts(), ["guardado", "falló"]);
        frame(&mut toasts, &ctx, 105.0);
        assert_eq!(toasts.texts(), ["falló"]);
        frame(&mut toasts, &ctx, 111.0);
        assert!(toasts.texts().is_empty());
    }

    #[test]
    fn el_tiempo_cuenta_desde_que_se_ve() {
        let ctx = egui::Context::default();
        let mut toasts = Toasts::default();
        toasts.info("hola");
        // Tarda en dibujarse (otra sesión, ventana minimizada…)
        frame(&mut toasts, &ctx, 50.0);
        frame(&mut toasts, &ctx, 53.0);
        assert_eq!(toasts.texts(), ["hola"]);
    }

    #[test]
    fn repetir_un_aviso_lo_renueva_sin_apilarlo() {
        let ctx = egui::Context::default();
        let mut toasts = Toasts::default();
        toasts.success("1 fila copiada");
        frame(&mut toasts, &ctx, 10.0);
        toasts.success("otro");
        toasts.success("1 fila copiada");
        assert_eq!(toasts.texts(), ["otro", "1 fila copiada"]);
        // El renovado vuelve a tener su tiempo entero
        frame(&mut toasts, &ctx, 13.0);
        frame(&mut toasts, &ctx, 16.0);
        assert_eq!(toasts.texts(), ["otro", "1 fila copiada"]);
    }

    #[test]
    fn con_demasiados_se_van_los_mas_antiguos() {
        let mut toasts = Toasts::default();
        for i in 0..6 {
            toasts.info(format!("aviso {i}"));
        }
        assert_eq!(toasts.texts(), ["aviso 2", "aviso 3", "aviso 4", "aviso 5"]);
    }
}

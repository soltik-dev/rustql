use std::sync::mpsc::{self, Receiver};

use eframe::egui;

use crate::profiles;
use crate::secrets;
use crate::session::{FrameKeys, Session, Shared};
use crate::settings;
use crate::table_state;
use crate::ui::conn_tabs::{self, ConnTab, ConnTabsAction};
use crate::ui::modals::ModalState;
use crate::ui::{log_panel, settings_panel, statusbar, theme};

/// Visibilidad de los paneles globales (comunes a todas las conexiones).
pub struct UiPanelState {
    pub show_sidebar: bool,
    pub show_log: bool,
    pub show_settings: bool,
    pub settings_dirty: bool,
}

// ── Estado principal ─────────────────────────────────────────────────────────

/// Shell de la aplicación: pestañas de conexión, paneles globales y el estado
/// que comparten. Cada pestaña es una `Session` independiente.
pub struct RustqlApp {
    /// Siempre hay al menos una.
    pub sessions: Vec<Session>,
    pub active: usize,
    next_session_id: u64,
    pub shared: Shared,
    pub ui: UiPanelState,
    /// Arranque del llavero: ids de los perfiles cuya contraseña en claro ya
    /// se ha pasado a él.
    keyring_migration: Option<Receiver<Vec<String>>>,
}

impl RustqlApp {
    pub fn new(ctx: &egui::Context) -> Self {
        let settings = settings::load();
        let ui = UiPanelState {
            show_sidebar: settings.show_sidebar,
            show_log: settings.show_log,
            show_settings: false,
            settings_dirty: false,
        };
        let profiles = profiles::load();
        let keyring_migration = Some(start_keyring(ctx, profiles::legacy_passwords(&profiles)));
        Self {
            sessions: vec![Session::new(0)],
            active: 0,
            next_session_id: 1,
            shared: Shared {
                settings,
                profiles,
                table_states: table_state::load(),
            },
            ui,
            keyring_migration,
        }
    }
}

/// Conecta con el llavero en segundo plano (el formulario consulta si está
/// disponible sin bloquear) y le pasa las contraseñas en claro heredadas.
fn start_keyring(
    ctx: &egui::Context,
    legacy: Vec<(String, String, String)>,
) -> Receiver<Vec<String>> {
    let (tx, rx) = mpsc::channel();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let mut migrated = Vec::new();
        if secrets::init() {
            for (id, label, password) in legacy {
                match secrets::set(&id, &label, &password) {
                    Ok(()) => migrated.push(id),
                    Err(e) => log::warn!("No se pasó al llavero la contraseña de «{label}»: {e}"),
                }
            }
        }
        let _ = tx.send(migrated);
        ctx.request_repaint();
    });
    rx
}

// ── Gestión de pestañas de conexión ──────────────────────────────────────────

impl RustqlApp {
    fn new_session(&mut self) {
        self.sessions.push(Session::new(self.next_session_id));
        self.next_session_id += 1;
        self.active = self.sessions.len() - 1;
    }

    /// Cierra una pestaña; si tiene cambios sin guardar, la activa y pide
    /// confirmación (el modal responde con SessionOutcome::close).
    fn request_close(&mut self, idx: usize) {
        let session = &mut self.sessions[idx];
        let pending_count = session.pending_count();
        if pending_count > 0 {
            session.modal = ModalState::ConfirmCloseSession {
                title: session.title.clone(),
                pending_count,
            };
            self.active = idx;
        } else {
            self.close_session(idx);
        }
    }

    /// Cierra sin preguntar. Activa la pestaña de la derecha (o la última) y,
    /// si no queda ninguna, abre una nueva con el formulario.
    fn close_session(&mut self, idx: usize) {
        self.sessions.remove(idx).shutdown();
        if self.sessions.is_empty() {
            self.new_session();
            return;
        }
        if idx < self.active {
            self.active -= 1;
        }
        self.active = self.active.min(self.sessions.len() - 1);
    }

    /// Quita del JSON las contraseñas en claro que ya están en el llavero.
    fn poll_keyring_migration(&mut self) {
        let Some(rx) = &self.keyring_migration else {
            return;
        };
        let migrated = match rx.try_recv() {
            Ok(ids) => ids,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Vec::new(),
        };
        self.keyring_migration = None;
        let mut changed = false;
        for p in &mut self.shared.profiles {
            // Si se volvió a guardar entretanto, ya no tiene la heredada.
            if p.password.is_some() && migrated.contains(&p.id) {
                p.password = None;
                p.password_in_keyring = true;
                changed = true;
            }
        }
        if changed {
            profiles::save(&self.shared.profiles);
        }
    }

    /// Tras borrar un perfil, corrige los índices de las demás sesiones.
    fn on_profile_removed(&mut self, removed: usize) {
        for session in &mut self.sessions {
            session.selected_profile_idx = match session.selected_profile_idx {
                Some(i) if i == removed => None,
                Some(i) if i > removed => Some(i - 1),
                other => other,
            };
        }
    }

    /// Barra superior a todo el ancho, con una línea inferior del color de la
    /// conexión activa. Devuelve también el rect de la barra.
    fn show_conn_tabs(&self, ui: &mut egui::Ui) -> (ConnTabsAction, egui::Rect) {
        let p = theme::get(ui);
        let tabs: Vec<ConnTab> = self
            .sessions
            .iter()
            .map(|s| ConnTab {
                title: s.tab_title(),
                tooltip: if s.is_connected() || s.is_busy() {
                    s.params.key()
                } else {
                    String::new()
                },
                color: s.color.map(|c| p.conn(c)),
                busy: s.is_busy(),
                pending: s.pending_count() > 0,
            })
            .collect();

        let frame = egui::Frame::NONE
            .fill(p.bg_sunken)
            .inner_margin(egui::Margin {
                left: 6,
                right: 6,
                top: 5,
                bottom: 0,
            });
        let panel = egui::Panel::top("conn_tabs")
            .exact_size(36.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(frame)
            .show(ui, |ui| conn_tabs::show(ui, &tabs, self.active));

        let rect = panel.response.rect;
        let stroke = match tabs[self.active].color {
            Some(c) => egui::Stroke::new(theme::CONN_FRAME_WIDTH, c),
            None => egui::Stroke::new(1.0, p.border_subtle),
        };
        ui.painter()
            .hline(rect.x_range(), rect.bottom() - stroke.width / 2.0, stroke);

        (panel.inner, rect)
    }
}

// ── Loop principal ────────────────────────────────────────────────────────────

impl eframe::App for RustqlApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = &ui.ctx().clone();
        let window_rect = ui.max_rect();
        // Aplicar ajustes si cambiaron
        if self.ui.settings_dirty {
            settings::setup_fonts(ctx, &self.shared.settings);
            settings::apply(ctx, &self.shared.settings);
            settings::save(&self.shared.settings);
            self.ui.settings_dirty = false;
        }

        let keys = FrameKeys::read(ctx);
        self.poll_keyring_migration();

        // ── Polling de hilos de fondo (todas las sesiones) ────────────────
        for session in &mut self.sessions {
            session.poll(ctx, &mut self.shared);
        }

        // Snapshot de visibilidad de paneles para detectar cambios
        let (prev_sidebar, prev_log) = (self.ui.show_sidebar, self.ui.show_log);
        let mut settings_changed = false;

        // ── Paneles globales (antes que los de la sesión) ─────────────────
        // Borde inferior del marco de color: encima del log, o de la barra
        // de estado si el log está oculto.
        let mut frame_bottom = {
            let (sb_host, sb_db) = self.sessions[self.active].status();
            let statusbar_frame =
                egui::Frame::side_top_panel(ui.style()).fill(theme::get(ui).bg_sunken);
            egui::Panel::bottom("statusbar")
                .exact_size(26.0)
                .frame(statusbar_frame)
                .show(ui, |ui| {
                    statusbar::show(
                        ui,
                        &mut self.ui.show_sidebar,
                        &mut self.ui.show_log,
                        &mut self.ui.show_settings,
                        sb_host.as_deref(),
                        sb_db.as_deref(),
                    );
                })
                .response
                .rect
                .top()
        };

        let (tabs_action, tabs_rect) = self.show_conn_tabs(ui);

        let session = &self.sessions[self.active];
        if session.is_connected() && self.ui.show_log {
            frame_bottom = egui::Panel::bottom("log_panel")
                .resizable(true)
                .min_size(80.0)
                .max_size(300.0)
                .show(ui, |ui| {
                    log_panel::show(ui, &session.query_log);
                })
                .response
                .rect
                .top();
        }

        if self.ui.show_settings {
            egui::Panel::right("settings_panel")
                .exact_size(240.0)
                .show(ui, |ui| {
                    settings_changed = settings_panel::show(ui, &mut self.shared.settings);
                });
        }

        // ── Sesión activa ─────────────────────────────────────────────────
        let outcome =
            self.sessions[self.active].show(ui, &mut self.shared, keys, self.ui.show_sidebar);

        // Marco con el color de la conexión activa alrededor del área de
        // trabajo (sin log ni barra de estado), por encima de sus paneles. Su
        // lado superior es la línea de las pestañas.
        if let Some(color) = self.sessions[self.active].color {
            let frame = egui::Rect::from_min_max(
                egui::pos2(
                    window_rect.left(),
                    tabs_rect.bottom() - theme::CONN_FRAME_WIDTH,
                ),
                egui::pos2(window_rect.right(), frame_bottom),
            );
            ui.painter().rect_stroke(
                frame,
                0.0,
                egui::Stroke::new(theme::CONN_FRAME_WIDTH, theme::get(ui).conn(color)),
                egui::StrokeKind::Inside,
            );
        }

        // ── Aplicar señales ───────────────────────────────────────────────

        if settings_changed {
            self.ui.settings_dirty = true;
        }

        // Persistir visibilidad de paneles cuando cambia
        if self.ui.show_sidebar != prev_sidebar || self.ui.show_log != prev_log {
            self.shared.settings.show_sidebar = self.ui.show_sidebar;
            self.shared.settings.show_log = self.ui.show_log;
            settings::save(&self.shared.settings);
        }

        if let Some(idx) = outcome.profile_removed {
            self.on_profile_removed(idx);
        }

        // Cerrar invalida los índices de la barra: ignorar su acción ese frame
        if outcome.close {
            self.close_session(self.active);
        } else {
            match tabs_action {
                ConnTabsAction::None => {}
                ConnTabsAction::Select(idx) => self.active = idx,
                ConnTabsAction::Close(idx) => self.request_close(idx),
                ConnTabsAction::New => self.new_session(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// App con `n` sesiones y sin tocar la configuración del disco.
    fn app_with(n: usize, active: usize) -> RustqlApp {
        RustqlApp {
            sessions: (0..n as u64).map(Session::new).collect(),
            active,
            next_session_id: n as u64,
            shared: Shared {
                settings: settings::AppSettings::default(),
                profiles: Vec::new(),
                table_states: Default::default(),
            },
            ui: UiPanelState {
                show_sidebar: true,
                show_log: true,
                show_settings: false,
                settings_dirty: false,
            },
            keyring_migration: None,
        }
    }

    fn ids(app: &RustqlApp) -> Vec<u64> {
        app.sessions.iter().map(|s| s.id).collect()
    }

    #[test]
    fn cerrar_activa_pasa_a_la_derecha() {
        let mut app = app_with(3, 1);
        app.close_session(1);
        assert_eq!(ids(&app), [0, 2]);
        assert_eq!(app.sessions[app.active].id, 2);
    }

    #[test]
    fn cerrar_ultima_activa_pasa_a_la_izquierda() {
        let mut app = app_with(3, 2);
        app.close_session(2);
        assert_eq!(app.sessions[app.active].id, 1);
    }

    #[test]
    fn cerrar_otra_conserva_la_activa() {
        let mut app = app_with(3, 2);
        app.close_session(0);
        assert_eq!(app.sessions[app.active].id, 2);
        let mut app = app_with(3, 0);
        app.close_session(2);
        assert_eq!(app.sessions[app.active].id, 0);
    }

    #[test]
    fn cerrar_la_unica_abre_una_nueva() {
        let mut app = app_with(1, 0);
        app.close_session(0);
        assert_eq!(app.sessions.len(), 1);
        assert_eq!(app.active, 0);
        assert_ne!(app.sessions[0].id, 0, "debe ser una sesión nueva");
    }

    #[test]
    fn cerrar_con_cambios_pide_confirmacion() {
        let mut app = app_with(2, 0);
        app.sessions[1].table_view.pending_deletes.insert(3);
        app.request_close(1);
        assert_eq!(app.sessions.len(), 2);
        assert_eq!(app.active, 1);
        assert!(matches!(
            app.sessions[1].modal,
            ModalState::ConfirmCloseSession {
                pending_count: 1,
                ..
            }
        ));
    }

    #[test]
    fn borrar_perfil_reajusta_indices() {
        let mut app = app_with(3, 0);
        app.sessions[0].selected_profile_idx = Some(1);
        app.sessions[1].selected_profile_idx = Some(2);
        app.sessions[2].selected_profile_idx = Some(0);
        app.on_profile_removed(1);
        let idx: Vec<_> = app
            .sessions
            .iter()
            .map(|s| s.selected_profile_idx)
            .collect();
        assert_eq!(idx, [None, Some(1), Some(0)]);
    }
}

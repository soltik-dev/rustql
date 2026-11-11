use std::sync::mpsc::{self, Receiver};

use eframe::egui;

use crate::profiles;
use crate::secrets;
use crate::session::{FrameKeys, Session, Shared};
use crate::settings;
use crate::table_state;
use crate::ui::conn_tabs::{self, ConnTab, ConnTabsAction};
use crate::ui::log_panel::LogAction;
use crate::ui::modals::ModalState;
use crate::ui::{log_panel, settings_panel, statusbar, theme};

const STATUSBAR_HEIGHT: f32 = 26.0;

/// Visibilidad de los paneles globales (comunes a todas las conexiones).
pub struct UiPanelState {
    pub show_sidebar: bool,
    pub show_log: bool,
    pub show_settings: bool,
    pub settings_dirty: bool,
    /// Fuente con la que se cargaron las fuentes por última vez.
    pub applied_font: String,
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
    window: WindowGeometry,
}

/// Tamaño de la ventana visto en el último frame, para guardarlo al salir.
#[derive(Default)]
struct WindowGeometry {
    /// Sin maximizar: es al que vuelve la ventana al restaurarla.
    size: Option<[f32; 2]>,
    maximized: bool,
}

impl WindowGeometry {
    fn track(&mut self, ctx: &egui::Context) {
        let (rect, maximized, fullscreen) = ctx.input(|i| {
            let v = i.viewport();
            (
                v.inner_rect,
                v.maximized.unwrap_or(false),
                v.fullscreen.unwrap_or(false),
            )
        });
        self.maximized = maximized;
        if !maximized
            && !fullscreen
            && let Some(rect) = rect
        {
            // El rect va en puntos de la interfaz, que dependen del zoom
            // (Ctrl + y Ctrl -); el tamaño inicial de la ventana no lo tiene
            let zoom = ctx.zoom_factor();
            self.size = Some([rect.width() * zoom, rect.height() * zoom]);
        }
    }
}

impl RustqlApp {
    pub fn new(ctx: &egui::Context, settings: settings::AppSettings) -> Self {
        let ui = UiPanelState {
            show_sidebar: settings.show_sidebar,
            show_log: settings.show_log,
            show_settings: false,
            settings_dirty: false,
            applied_font: settings.font_family.clone(),
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
                toasts: Default::default(),
            },
            ui,
            keyring_migration,
            window: WindowGeometry::default(),
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
        let session = self.sessions.remove(idx);
        session.save_tabs_state(&mut self.shared);
        session.shutdown();
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
    /// Al salir se guarda el texto de las pestañas SQL de cada conexión:
    /// hasta ahora solo se guardaba al ejecutar, y lo escrito sin ejecutar
    /// se perdía al cerrar la aplicación.
    fn on_exit(&mut self) {
        for session in &self.sessions {
            session.save_tabs_state(&mut self.shared);
        }
        // Y el tamaño de la ventana, para abrirla igual la próxima vez
        if self.window.size.is_some() {
            self.shared.settings.window_size = self.window.size;
        }
        self.shared.settings.window_maximized = self.window.maximized;
        settings::save(&self.shared.settings);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = &ui.ctx().clone();
        let window_rect = ui.max_rect();
        self.window.track(ctx);
        // Aplicar ajustes si cambiaron
        if self.ui.settings_dirty {
            // Recargar las fuentes rehace su atlas: solo si cambió la fuente
            if self.ui.applied_font != self.shared.settings.font_family {
                settings::setup_fonts(ctx, &self.shared.settings);
                self.ui.applied_font = self.shared.settings.font_family.clone();
            }
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
                .exact_size(STATUSBAR_HEIGHT)
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
        let mut log_action = LogAction::None;
        if session.is_connected() && self.ui.show_log {
            let panel = egui::Panel::bottom("log_panel")
                .resizable(true)
                .min_size(80.0)
                .max_size(300.0)
                .show(ui, |ui| log_panel::show(ui, &session.query_log));
            frame_bottom = panel.response.rect.top();
            log_action = panel.inner;
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
        if let LogAction::OpenInSql(sql) = log_action {
            self.sessions[self.active].open_sql_tab(&mut self.shared, None, sql);
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

        // Avisos: sobre todo lo demás, por encima de la barra de estado
        self.shared.toasts.show(ctx, STATUSBAR_HEIGHT + 12.0);
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
                toasts: Default::default(),
            },
            ui: UiPanelState {
                show_sidebar: true,
                show_log: true,
                show_settings: false,
                settings_dirty: false,
                applied_font: String::new(),
            },
            keyring_migration: None,
            window: WindowGeometry::default(),
        }
    }

    fn ids(app: &RustqlApp) -> Vec<u64> {
        app.sessions.iter().map(|s| s.id).collect()
    }

    /// Un frame con la ventana en ese estado.
    fn track(window: &mut WindowGeometry, size: [f32; 2], maximized: bool) {
        let ctx = egui::Context::default();
        let mut input = egui::RawInput::default();
        let info = input.viewports.entry(egui::ViewportId::ROOT).or_default();
        info.inner_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size.into()));
        info.maximized = Some(maximized);
        ctx.run_ui(input, |ui| window.track(ui.ctx()))
            .drop_without_applying_deltas();
    }

    #[test]
    fn se_recuerda_el_tamano_de_antes_de_maximizar() {
        let mut window = WindowGeometry::default();
        track(&mut window, [1200.0, 700.0], false);
        assert_eq!(window.size, Some([1200.0, 700.0]));
        assert!(!window.maximized);
        // Maximizada ocupa la pantalla: ese no es el tamaño al restaurarla
        track(&mut window, [1920.0, 1040.0], true);
        assert_eq!(window.size, Some([1200.0, 700.0]));
        assert!(window.maximized);
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

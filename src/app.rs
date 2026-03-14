use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use eframe::egui;

use crate::db::{ConnectionParams, ConnectEvent, DbBackend, QueryResult, connect_mysql_progress};
use crate::log::LogEntry;
use crate::profiles::{self, ConnectionProfile};
use crate::settings::{self, AppSettings};
use crate::ui::results_grid::{SortSpec, compute_sort_indices};
use crate::ui::{
    connection_panel, filter_bar, log_panel, results_grid, settings_panel, sidebar, statusbar,
};

pub enum AppState {
    Disconnected,
    /// Conexión en progreso en un hilo separado.
    Connecting {
        receiver: Receiver<ConnectEvent>,
        host: String,
        status: String,
        start: Instant,
    },
    Connected(Box<dyn DbBackend>),
}

pub struct RustqlApp {
    pub state: AppState,
    pub params: ConnectionParams,
    pub port_str: String,
    pub error_message: Option<String>,
    pub databases: Vec<String>,
    pub tables_cache: HashMap<String, Vec<String>>,
    pub table_search: String,
    pub selected_table: Option<(String, String)>,
    pub table_data: Option<QueryResult>,
    pub table_error: Option<String>,
    pub sort_specs: Vec<SortSpec>,
    pub sort_indices: Vec<usize>,
    pub filter_text: String,
    pub query_log: Vec<LogEntry>,
    pub profiles: Vec<ConnectionProfile>,
    pub selected_profile_idx: Option<usize>,
    pub profile_name: String,
    pub save_password: bool,
    pub show_sidebar: bool,
    pub show_log: bool,
    pub show_settings: bool,
    pub settings: AppSettings,
    pub settings_dirty: bool,
}

impl Default for RustqlApp {
    fn default() -> Self {
        let params = ConnectionParams::default();
        let port_str = params.port.to_string();
        Self {
            state: AppState::Disconnected,
            port_str,
            params,
            error_message: None,
            databases: Vec::new(),
            tables_cache: HashMap::new(),
            table_search: String::new(),
            selected_table: None,
            table_data: None,
            table_error: None,
            sort_specs: Vec::new(),
            sort_indices: Vec::new(),
            filter_text: String::new(),
            query_log: Vec::new(),
            profiles: profiles::load(),
            selected_profile_idx: None,
            profile_name: String::new(),
            save_password: false,
            show_sidebar: true,
            show_log: true,
            show_settings: false,
            settings: settings::load(),
            settings_dirty: true,
        }
    }
}

impl RustqlApp {
    fn run_query(&mut self, sql: &str) {
        let start = Instant::now();
        let result = match &mut self.state {
            AppState::Connected(b) => b.execute_query(sql),
            _ => return,
        };
        let duration_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(data) => {
                let rows = data.rows.len();
                self.push_log(LogEntry::ok(sql, duration_ms, rows));
                self.sort_specs.clear();
                self.sort_indices = (0..rows).collect();
                self.table_data = Some(data);
                self.table_error = None;
            }
            Err(e) => {
                self.push_log(LogEntry::err(sql, duration_ms, &e));
                self.table_data = None;
                self.table_error = Some(e);
                self.sort_specs.clear();
                self.sort_indices.clear();
            }
        }
    }

    fn push_log(&mut self, entry: LogEntry) {
        if self.query_log.len() >= 500 {
            self.query_log.remove(0);
        }
        self.query_log.push(entry);
    }

    fn build_sql(&self, db: &str, table: &str) -> String {
        let filter = self.filter_text.trim();
        if filter.is_empty() {
            format!("SELECT * FROM `{db}`.`{table}` LIMIT 500")
        } else {
            format!("SELECT * FROM `{db}`.`{table}` WHERE {filter} LIMIT 500")
        }
    }

    fn recompute_sort(&mut self) {
        if let Some(data) = &self.table_data {
            self.sort_indices = compute_sort_indices(&data.rows, &self.sort_specs);
        }
    }

    fn on_connected(&mut self, backend: Box<dyn DbBackend>) {
        self.state = AppState::Connected(backend);
        self.error_message = None;
        self.databases.clear();
        self.tables_cache.clear();
        self.table_search.clear();
        self.selected_table = None;
        self.table_data = None;
        self.table_error = None;
        self.sort_specs.clear();
        self.sort_indices.clear();
        self.filter_text.clear();
        self.query_log.clear();
    }
}

impl eframe::App for RustqlApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Aplicar ajustes si cambiaron
        if self.settings_dirty {
            settings::apply(ctx, &self.settings);
            settings::save(&self.settings);
            self.settings_dirty = false;
        }

        let f9 = ctx.input(|i| i.key_pressed(egui::Key::F9));

        // ── Polling del hilo de conexión ─────────────────────────────────
        let mut connect_result: Option<Result<Box<dyn DbBackend>, String>> = None;
        if let AppState::Connecting { receiver, status, .. } = &mut self.state {
            // Drain all pending status messages, keep last Done
            loop {
                match receiver.try_recv() {
                    Ok(ConnectEvent::Status(s)) => {
                        *status = s;
                        ctx.request_repaint_after(Duration::from_millis(50));
                    }
                    Ok(ConnectEvent::Done(result)) => {
                        connect_result = Some(result);
                        break;
                    }
                    Err(mpsc::TryRecvError::Empty) => {
                        ctx.request_repaint_after(Duration::from_millis(50));
                        break;
                    }
                    Err(mpsc::TryRecvError::Disconnected) => {
                        connect_result =
                            Some(Err("El hilo de conexión terminó inesperadamente".into()));
                        break;
                    }
                }
            }
        }
        if let Some(result) = connect_result {
            match result {
                Ok(backend) => self.on_connected(backend),
                Err(e) => {
                    self.state = AppState::Disconnected;
                    self.error_message = Some(e);
                }
            }
        }

        // ── Señales del frame ────────────────────────────────────────────
        let mut connect_clicked = false;
        let mut cancel_connect = false;
        let mut save_profile = false;
        let mut delete_profile = false;
        let mut profile_selected: Option<usize> = None;
        let mut table_clicked: Option<(String, String)> = None;
        let mut filter_action = filter_bar::FilterAction::None;
        let mut settings_changed = false;
        let mut sort_changed = false;

        // ── Paneles inferiores (primero siempre) ─────────────────────────
        if matches!(self.state, AppState::Connected(_)) {
            egui::TopBottomPanel::bottom("statusbar")
                .exact_height(26.0)
                .show(ctx, |ui| {
                    statusbar::show(
                        ui,
                        &mut self.show_sidebar,
                        &mut self.show_log,
                        &mut self.show_settings,
                    );
                });

            if self.show_log {
                egui::TopBottomPanel::bottom("log_panel")
                    .resizable(true)
                    .min_height(80.0)
                    .max_height(300.0)
                    .show(ctx, |ui| {
                        log_panel::show(ui, &self.query_log);
                    });
            }
        }

        // ── Panel de ajustes (derecha) ───────────────────────────────────
        if self.show_settings {
            egui::SidePanel::right("settings_panel")
                .exact_width(240.0)
                .show(ctx, |ui| {
                    settings_changed = settings_panel::show(ui, &mut self.settings);
                });
        }

        // ── Contenido principal según estado ─────────────────────────────
        match &mut self.state {
            AppState::Disconnected => {
                egui::SidePanel::left("conn_favorites")
                    .exact_width(200.0)
                    .show(ctx, |ui| {
                        profile_selected = connection_panel::show_favorites(
                            ui,
                            &self.profiles,
                            &self.selected_profile_idx,
                        );
                    });

                egui::CentralPanel::default().show(ctx, |ui| {
                    let result = connection_panel::show_form(
                        ui,
                        &mut self.params,
                        &mut self.port_str,
                        &self.error_message,
                        &mut self.profile_name,
                        &mut self.save_password,
                        &self.selected_profile_idx,
                    );
                    connect_clicked = result.connect;
                    save_profile = result.save_profile;
                    delete_profile = result.delete_profile;
                });
            }

            AppState::Connecting { host, status, start, .. } => {
                let host = host.clone();
                let status = status.clone();
                let elapsed = start.elapsed().as_secs();
                egui::SidePanel::left("conn_favorites")
                    .exact_width(200.0)
                    .show(ctx, |ui| {
                        // Favoritos visibles pero no interactivos durante la conexión
                        connection_panel::show_favorites(
                            ui,
                            &self.profiles,
                            &self.selected_profile_idx,
                        );
                    });

                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.add_space(ui.available_height() / 3.0);
                    ui.vertical_centered(|ui| {
                        ui.spinner();
                        ui.add_space(10.0);
                        ui.strong(format!("Conectando a {}...", host));
                        ui.add_space(6.0);
                        ui.label(&status);
                        ui.add_space(4.0);
                        ui.label(format!("{}s", elapsed));
                        ui.add_space(16.0);
                        if ui.button("Cancelar").clicked() {
                            cancel_connect = true;
                        }
                    });
                });
            }

            AppState::Connected(backend) => {
                if self.show_sidebar {
                    egui::SidePanel::left("sidebar")
                        .min_width(180.0)
                        .show(ctx, |ui| {
                            table_clicked = sidebar::show(
                                ui,
                                backend,
                                &mut self.databases,
                                &mut self.tables_cache,
                                &self.selected_table,
                                &mut self.table_search,
                            );
                        });
                }

                egui::CentralPanel::default().show(ctx, |ui| {
                    if self.selected_table.is_some() {
                        filter_action = filter_bar::show(ui, &mut self.filter_text, f9);
                        ui.separator();
                    }
                    sort_changed = results_grid::show(
                        ui,
                        &self.selected_table,
                        &self.table_data,
                        &self.table_error,
                        &mut self.sort_specs,
                        &self.sort_indices,
                    );
                });
            }
        }

        // ── Aplicar señales ──────────────────────────────────────────────

        if cancel_connect {
            self.state = AppState::Disconnected;
        }

        if settings_changed {
            self.settings_dirty = true;
        }

        if sort_changed {
            self.recompute_sort();
        }

        if let Some(idx) = profile_selected {
            if matches!(self.state, AppState::Disconnected) {
                let profile = &self.profiles[idx];
                self.params = profile.to_params();
                self.port_str = self.params.port.to_string();
                self.profile_name = profile.name.clone();
                self.save_password = profile.password.is_some();
                self.selected_profile_idx = Some(idx);
                self.error_message = None;
            }
        }

        if save_profile && !self.profile_name.trim().is_empty() {
            if let Ok(p) = self.port_str.parse::<u16>() {
                self.params.port = p;
            }
            let password = if self.save_password {
                Some(self.params.password.clone())
            } else {
                None
            };
            let profile = ConnectionProfile {
                name: self.profile_name.trim().to_string(),
                host: self.params.host.clone(),
                port: self.params.port,
                user: self.params.user.clone(),
                password,
            };
            if let Some(idx) = self.selected_profile_idx {
                self.profiles[idx] = profile;
            } else if let Some(pos) = self
                .profiles
                .iter()
                .position(|p| p.name == self.profile_name.trim())
            {
                self.profiles[pos] = profile;
                self.selected_profile_idx = Some(pos);
            } else {
                self.profiles.push(profile);
                self.selected_profile_idx = Some(self.profiles.len() - 1);
            }
            profiles::save(&self.profiles);
        }

        if delete_profile {
            if let Some(idx) = self.selected_profile_idx {
                self.profiles.remove(idx);
                self.selected_profile_idx = None;
                self.profile_name.clear();
                profiles::save(&self.profiles);
            }
        }

        // Iniciar conexión async
        if connect_clicked {
            if let Ok(p) = self.port_str.parse::<u16>() {
                self.params.port = p;
            }
            let params = self.params.clone();
            let host = self.params.host.clone();
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                connect_mysql_progress(&params, &tx);
            });
            self.state = AppState::Connecting {
                receiver: rx,
                host,
                status: "Iniciando...".to_string(),
                start: Instant::now(),
            };
            self.error_message = None;
        }

        if let Some((db, table)) = table_clicked {
            self.selected_table = Some((db.clone(), table.clone()));
            let sql = self.build_sql(&db, &table);
            self.run_query(&sql);
        }

        match filter_action {
            filter_bar::FilterAction::Apply | filter_bar::FilterAction::Clear => {
                if let Some((db, table)) = self.selected_table.clone() {
                    let sql = self.build_sql(&db, &table);
                    self.run_query(&sql);
                }
            }
            filter_bar::FilterAction::None => {}
        }
    }
}

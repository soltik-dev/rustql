use std::collections::HashMap;

use eframe::egui;

use crate::db::{ConnectionParams, DbBackend, QueryResult, connect_mysql};
use crate::ui::{connection_panel, results_grid, sidebar};

pub enum AppState {
    Disconnected,
    Connected(Box<dyn DbBackend>),
}

pub struct RustqlApp {
    pub state: AppState,
    pub params: ConnectionParams,
    pub port_str: String,
    pub databases: Vec<String>,
    pub tables_cache: HashMap<String, Vec<String>>,
    pub error_message: Option<String>,
    pub selected_table: Option<(String, String)>,
    pub table_data: Option<QueryResult>,
    pub table_error: Option<String>,
}

impl Default for RustqlApp {
    fn default() -> Self {
        let params = ConnectionParams::default();
        let port_str = params.port.to_string();
        Self {
            state: AppState::Disconnected,
            port_str,
            params,
            databases: Vec::new(),
            tables_cache: HashMap::new(),
            error_message: None,
            selected_table: None,
            table_data: None,
            table_error: None,
        }
    }
}

impl eframe::App for RustqlApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let mut connect_clicked = false;
        let mut table_clicked: Option<(String, String)> = None;

        match &mut self.state {
            AppState::Disconnected => {
                egui::CentralPanel::default().show(ctx, |ui| {
                    connect_clicked = connection_panel::show(
                        ui,
                        &mut self.params,
                        &mut self.port_str,
                        &self.error_message,
                    );
                });
            }
            AppState::Connected(backend) => {
                egui::SidePanel::left("sidebar")
                    .min_width(200.0)
                    .show(ctx, |ui| {
                        table_clicked = sidebar::show(
                            ui,
                            backend,
                            &mut self.databases,
                            &mut self.tables_cache,
                            &self.selected_table,
                        );
                    });
                egui::CentralPanel::default().show(ctx, |ui| {
                    results_grid::show(
                        ui,
                        &self.selected_table,
                        &self.table_data,
                        &self.table_error,
                    );
                });
            }
        }

        // Transición de conexión
        if connect_clicked {
            if let Ok(p) = self.port_str.parse::<u16>() {
                self.params.port = p;
            }
            match connect_mysql(&self.params) {
                Ok(backend) => {
                    self.state = AppState::Connected(backend);
                    self.error_message = None;
                    self.databases.clear();
                    self.tables_cache.clear();
                    self.selected_table = None;
                    self.table_data = None;
                    self.table_error = None;
                }
                Err(e) => {
                    self.error_message = Some(e);
                }
            }
        }

        // Cargar datos de tabla cuando el usuario hace click
        if let Some((db, table)) = table_clicked {
            self.selected_table = Some((db.clone(), table.clone()));
            if let AppState::Connected(backend) = &mut self.state {
                match backend.query_table(&db, &table, 500) {
                    Ok(data) => {
                        self.table_data = Some(data);
                        self.table_error = None;
                    }
                    Err(e) => {
                        self.table_data = None;
                        self.table_error = Some(e);
                    }
                }
            }
        }
    }
}

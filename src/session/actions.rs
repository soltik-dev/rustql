//! Lo que hace la sesión con las acciones del usuario: grid, sidebar,
//! pestañas SQL, diálogos y formulario de conexión.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Instant;

use eframe::egui;

use super::view::FrameSignals;
use super::writes::AfterCommit;
use super::{ConnState, SelectedNode, Session, Shared};
use crate::db::{connect_mysql_progress, escape_sql, qualified, quote_ident};
use crate::loadable::Loadable;
use crate::profiles::{self, ConnectionProfile};
use crate::secrets;
use crate::settings::{self, CopyFormat};
use crate::table_state;
use crate::ui::db_info::DbInfoAction;
use crate::ui::filter_bar::FilterAction;
use crate::ui::modals::{
    self, DropKind, ExportPhase, ExportRunning, ModalAction, ModalState, NewColumnDef, Reload,
    TruncateKind,
};
use crate::ui::results_grid::{self, ApplyBarAction, CellRef, GridAction, InsertValue, RowRef};
use crate::ui::sidebar::SidebarAction;
use crate::ui::sql_editor::{SqlEditorAction, SqlTab};
use crate::ui::tab_bar::{ActiveTab, TabBarResult};

impl Session {
    /// Aplica lo que el usuario hizo en el frame, en orden: lo de un
    /// manejador puede depender de lo que hizo el anterior.
    pub(super) fn apply_signals(
        &mut self,
        ctx: &egui::Context,
        sh: &mut Shared,
        signals: FrameSignals,
        f5: bool,
        ctrl_t: bool,
    ) {
        if signals.cancel_connect {
            self.state = ConnState::Disconnected;
        }
        self.handle_tab_bar(sh, signals.tab_bar, ctrl_t);
        self.handle_grid_action(ctx, sh, signals.grid);
        // Barra de guardar (solo visible con cambios pendientes)
        match signals.apply_bar {
            ApplyBarAction::Apply => self.commit_pending_changes(sh, AfterCommit::ReloadData),
            ApplyBarAction::Discard => self.discard_pending(),
            ApplyBarAction::None => {}
        }
        let disconnect = self.handle_sidebar_action(ctx, sh, signals.sidebar.action);
        // Datos que el sidebar necesita para dibujar el árbol
        for db in &signals.sidebar.load_tables {
            self.request_tables(db);
        }
        if signals.sidebar.load_all_tables {
            self.request_all_tables();
        }
        if disconnect {
            // Lo escrito y sin ejecutar no debe perderse al reconectar
            self.save_tabs_state(sh);
            self.disconnect();
        }
        if f5 {
            self.refresh_active_tab(sh);
        }
        self.handle_filter_action(sh, signals.filter);
        self.handle_db_info_action(sh, signals.db_info);
        self.handle_sql_action(sh, signals.sql);
    }

    /// F5: vuelve a leer lo que muestra la pestaña activa (antes recargaba
    /// siempre la tabla abierta, se estuviera viendo o no).
    fn refresh_active_tab(&mut self, sh: &mut Shared) {
        match self.active_tab {
            // Datos y estructura de la tabla abierta
            ActiveTab::Datos => self.request_reload(sh, Reload::Refresh),
            ActiveTab::Tabla => {
                if let Some(SelectedNode::Table { db, table }) = &self.schema.selected_node {
                    let key = (db.clone(), table.clone());
                    self.schema.columns_info.remove(&key);
                }
            }
            ActiveTab::Bbdd => {
                if let Some(db) = self.selected_db().map(str::to_string) {
                    self.schema.tables_info.remove(&db);
                }
            }
            ActiveTab::Servidor => self.schema.databases_info = Loadable::NotLoaded,
            // La consulta de la pestaña, como Ctrl+Enter
            ActiveTab::Sql(i) => {
                if let Some(tab) = self.sql_tabs.get(i)
                    && tab.running.is_none()
                    && !tab.sql_text.trim().is_empty()
                {
                    let action = SqlEditorAction::Execute {
                        sql: tab.sql_text.trim().to_string(),
                        db_context: tab.db_context.clone(),
                    };
                    self.handle_sql_action(sh, action);
                }
            }
        }
        // Lo invalidado se vuelve a pedir en el siguiente frame (request_tab_data)
    }

    fn handle_tab_bar(&mut self, sh: &mut Shared, tab_bar: TabBarResult, ctrl_t: bool) {
        if let Some(t) = tab_bar.tab_clicked {
            // Al salir de una pestaña SQL se guarda su texto
            if matches!(self.active_tab, ActiveTab::Sql(_)) && t != self.active_tab {
                self.save_tabs_state(sh);
            }
            self.active_tab = t;
        }

        if tab_bar.new_sql_tab || (ctrl_t && self.is_connected()) {
            let db_ctx = match &self.schema.selected_node {
                Some(SelectedNode::Database(d)) => d.clone(),
                Some(SelectedNode::Table { db, .. }) => db.clone(),
                _ => String::new(),
            };
            let new_idx = self.sql_tabs.len();
            self.sql_tabs.push(SqlTab {
                db_context: db_ctx,
                sql_text: String::new(),
                ..Default::default()
            });
            self.active_tab = ActiveTab::Sql(new_idx);
            self.save_tabs_state(sh);
        }

        if let Some(close_idx) = tab_bar.close_sql_tab
            && close_idx < self.sql_tabs.len()
        {
            self.sql_tabs.remove(close_idx);
            if self.sql_tabs.is_empty() {
                self.sql_tabs.push(SqlTab::default());
            }
            self.active_tab = match self.active_tab {
                ActiveTab::Sql(i) if i >= close_idx => {
                    let new_i = if i > 0 { i.saturating_sub(1) } else { 0 };
                    ActiveTab::Sql(new_i.min(self.sql_tabs.len().saturating_sub(1)))
                }
                other => other,
            };
            self.save_tabs_state(sh);
        }
    }

    fn handle_grid_action(&mut self, ctx: &egui::Context, sh: &mut Shared, action: GridAction) {
        match action {
            GridAction::SortChanged(sorts) => {
                // Re-ejecutar con ORDER BY para sort server-side (LIMIT correcto)
                self.request_reload(sh, Reload::Sort(sorts));
            }
            GridAction::CellEdited {
                row,
                col_idx,
                value,
            } => self.record_edit(row, col_idx, value),
            GridAction::CellSetDefault {
                insert_idx,
                col_idx,
            } => {
                if let Some(slot) = self
                    .table_view
                    .pending_inserts
                    .get_mut(insert_idx)
                    .and_then(|ins| ins.values.get_mut(col_idx))
                {
                    *slot = InsertValue::Default;
                }
            }
            GridAction::FilterBy { sql_fragment } => {
                self.request_reload(sh, Reload::Filter(sql_fragment));
            }
            GridAction::FilterByClipboard { col_name } => {
                match arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
                    Ok(text) => {
                        let esc_col = quote_ident(&col_name);
                        let esc_val = escape_sql(&text);
                        let filter = format!("{esc_col} = '{esc_val}'");
                        self.request_reload(sh, Reload::Filter(filter));
                    }
                    Err(e) => {
                        self.table_view.table_error =
                            Some(format!("No se pudo leer el portapapeles: {e}"));
                    }
                }
            }
            GridAction::FilterUserInput { col_name } => {
                self.modal = ModalState::FilterInput {
                    col_name,
                    value: String::new(),
                    operator: modals::FilterOperator::default(),
                };
            }
            GridAction::ToggleDeleteRows(rows) => {
                for row_idx in rows {
                    if !self.table_view.pending_deletes.remove(&row_idx) {
                        self.table_view.pending_deletes.insert(row_idx);
                    }
                }
            }
            GridAction::InsertRow => self.add_insert_row(ctx, None),
            GridAction::CloneRow { row_idx } => self.add_insert_row(ctx, Some(row_idx)),
            GridAction::RemoveInsertRow { insert_idx } => {
                if insert_idx < self.table_view.pending_inserts.len() {
                    self.table_view.pending_inserts.remove(insert_idx);
                }
            }
            GridAction::CopyRows(fmt) => {
                self.copy_selected_rows(ctx, fmt.as_ref().unwrap_or(&sh.settings.copy_format));
            }
            GridAction::None => {}
        }
    }

    /// Añade al grid una fila nueva (copia de `source_row` si se indica) y
    /// lleva a ella el cursor y el foco, lista para escribir.
    fn add_insert_row(&mut self, ctx: &egui::Context, source_row: Option<usize>) {
        let Some(row) = self.new_insert_row(source_row) else {
            return;
        };
        let col = row.auto_cols.iter().position(|auto| !auto).unwrap_or(0);
        let tv = &mut self.table_view;
        tv.pending_inserts.push(row);
        tv.selection.move_to(CellRef {
            row: RowRef::Insert(tv.pending_inserts.len() - 1),
            col,
        });
        if let Some(id) = tv.selection.focus_id {
            ctx.memory_mut(|m| m.request_focus(id));
        }
    }

    /// Copia las filas seleccionadas al portapapeles en el formato `fmt`.
    fn copy_selected_rows(&self, ctx: &egui::Context, fmt: &CopyFormat) {
        let Some(data) = &self.table_view.table_data else {
            return;
        };
        let table = match &self.schema.selected_node {
            Some(SelectedNode::Table { table, .. }) => Some(table.as_str()),
            _ => None,
        };
        let rows: Vec<&Vec<Option<String>>> = self
            .table_view
            .selection
            .rows
            .iter()
            .filter_map(|&i| data.rows.get(i))
            .collect();
        if !rows.is_empty() {
            ctx.copy_text(results_grid::format_rows_for_copy(
                &data.columns,
                &rows,
                fmt,
                table,
            ));
        }
    }

    /// Devuelve true si el usuario pidió desconectar.
    fn handle_sidebar_action(
        &mut self,
        ctx: &egui::Context,
        sh: &mut Shared,
        action: SidebarAction,
    ) -> bool {
        match action {
            SidebarAction::None => {}
            SidebarAction::SelectServer => {
                self.schema.selected_node = Some(SelectedNode::Server);
                self.active_tab = ActiveTab::Servidor;
            }
            SidebarAction::SelectDatabase(db) => {
                self.schema.selected_node = Some(SelectedNode::Database(db.clone()));
                self.active_tab = ActiveTab::Bbdd;
            }
            SidebarAction::SelectTable { db, table } => {
                self.switch_to_table(sh, db, table, ActiveTab::Datos);
            }
            SidebarAction::CreateDatabase => {
                self.modal = ModalState::CreateDatabase {
                    name: String::new(),
                    charset: "utf8mb4".into(),
                    error: None,
                };
            }
            SidebarAction::CreateTable { db } => {
                self.modal = ModalState::CreateTable {
                    db,
                    name: String::new(),
                    columns: vec![NewColumnDef::new()],
                    error: None,
                };
            }
            SidebarAction::DropDatabase { db } => {
                let target = db.clone();
                self.modal = ModalState::ConfirmDrop {
                    kind: DropKind::Database,
                    target,
                    db: Some(db),
                    table: None,
                };
            }
            SidebarAction::TruncateAllTables { db } => {
                let target = format!("todas las tablas de '{db}'");
                self.modal = ModalState::ConfirmTruncate {
                    kind: TruncateKind::AllInDatabase,
                    target,
                    db: Some(db),
                    table: None,
                };
            }
            SidebarAction::DropTable { db, table } => {
                let target = format!("{db}.{table}");
                self.modal = ModalState::ConfirmDrop {
                    kind: DropKind::Table,
                    target,
                    db: Some(db),
                    table: Some(table),
                };
            }
            SidebarAction::TruncateTable { db, table } => {
                let target = format!("{db}.{table}");
                self.modal = ModalState::ConfirmTruncate {
                    kind: TruncateKind::Table,
                    target,
                    db: Some(db),
                    table: Some(table),
                };
            }
            SidebarAction::CopyTableName(name) => {
                ctx.copy_text(name);
            }
            SidebarAction::Refresh => self.schema.clear(),
            SidebarAction::Disconnect => return true,
            SidebarAction::ToggleFav { db, table } => {
                let key = table_state::table_key(&db, &table);
                if sh.settings.fav_tables.contains(&key) {
                    sh.settings.fav_tables.remove(&key);
                } else {
                    sh.settings.fav_tables.insert(key);
                }
                settings::save(&sh.settings);
            }
            SidebarAction::OpenTableStructure { db, table } => {
                self.switch_to_table(sh, db, table, ActiveTab::Tabla);
            }
            SidebarAction::OpenInSql { db, table } => {
                let new_idx = self.sql_tabs.len();
                let sql_text = format!("SELECT * FROM {} WHERE ", qualified(&db, &table));
                self.sql_tabs.push(SqlTab {
                    db_context: db.clone(),
                    sql_text,
                    ..Default::default()
                });
                self.active_tab = ActiveTab::Sql(new_idx);
                self.save_tabs_state(sh);
            }
            SidebarAction::CloneTable { db, table } => {
                self.modal = ModalState::CloneTable {
                    db,
                    table,
                    new_name: String::new(),
                    copy_data: true,
                    error: None,
                };
            }
            SidebarAction::ExportDatabase(db) => self.open_export(sh, db, None),
            SidebarAction::ExportTable { db, table } => self.open_export(sh, db, Some(table)),
        }

        false
    }

    fn handle_filter_action(&mut self, sh: &mut Shared, action: FilterAction) {
        match action {
            FilterAction::Apply => {
                let filter = self.table_view.filter_text.trim().to_string();
                self.request_reload(sh, Reload::Filter(filter));
            }
            FilterAction::Clear => {
                self.request_reload(sh, Reload::Filter(String::new()));
            }
            FilterAction::None => {}
        }
    }

    /// Doble clic en una tabla de la pestaña BBDD: la abre.
    fn handle_db_info_action(&mut self, sh: &mut Shared, action: DbInfoAction) {
        if let DbInfoAction::OpenTable(table) = action {
            let db = match &self.schema.selected_node {
                Some(SelectedNode::Database(d)) => Some(d.clone()),
                Some(SelectedNode::Table { db, .. }) => Some(db.clone()),
                _ => None,
            };
            if let Some(db) = db {
                self.switch_to_table(sh, db, table, ActiveTab::Datos);
            }
        }
    }

    fn handle_sql_action(&mut self, sh: &mut Shared, action: SqlEditorAction) {
        // Persistir texto de pestañas SQL al ejecutar
        if let SqlEditorAction::Execute { sql, db_context } = action {
            self.save_tabs_state(sh);
            if let ActiveTab::Sql(tab_idx) = self.active_tab {
                self.run_editor(tab_idx, sql, db_context, sh.settings.sql_editor_max_rows);
            }
        }
    }

    /// Procesa la acción resultante del modal activo.
    pub(super) fn handle_modal_action(&mut self, modal_action: ModalAction, sh: &mut Shared) {
        match modal_action {
            ModalAction::None => {}
            ModalAction::Cancel | ModalAction::CloseSession => {
                self.modal = ModalState::None;
            }
            ModalAction::ApplyAndSwitch { new_db, new_table } => {
                self.modal = ModalState::None;
                let then = AfterCommit::Switch {
                    db: new_db,
                    table: new_table,
                };
                self.commit_pending_changes(sh, then);
            }
            ModalAction::DiscardAndSwitch { new_db, new_table } => {
                self.modal = ModalState::None;
                self.on_table_selected(sh, new_db, new_table);
                self.active_tab = ActiveTab::Datos;
            }
            ModalAction::ApplyAndReload(reload) => {
                self.modal = ModalState::None;
                self.commit_pending_changes(sh, AfterCommit::Reload(reload));
            }
            ModalAction::DiscardAndReload(reload) => {
                self.modal = ModalState::None;
                self.discard_pending();
                self.reload_now(sh, reload);
            }
            // Escrituras: en segundo plano. Los formularios siguen abiertos
            // hasta saber el resultado; las confirmaciones se cierran ya.
            ModalAction::CreateDatabase { name, charset } => self.create_database(name, charset),
            ModalAction::CreateTable { db, name, columns } => {
                let sql = modals::build_create_table_sql(&db, &name, &columns);
                self.create_table(db, sql);
            }
            ModalAction::DropDatabase(db) => {
                self.modal = ModalState::None;
                self.drop_database(db);
            }
            ModalAction::DropTable { db, table } => {
                self.modal = ModalState::None;
                self.drop_table(db, table);
            }
            ModalAction::TruncateTable { db, table } => {
                self.modal = ModalState::None;
                self.truncate(db, Some(vec![table]));
            }
            ModalAction::TruncateAllTables(db) => {
                self.modal = ModalState::None;
                self.truncate(db, None);
            }
            ModalAction::ApplyFilter { sql_fragment } => {
                self.modal = ModalState::None;
                self.request_reload(sh, Reload::Filter(sql_fragment));
            }
            ModalAction::CloneTable {
                db,
                src_table,
                new_name,
                copy_data,
            } => self.clone_table(db, src_table, new_name, copy_data),
            ModalAction::StartExport {
                source_db,
                tables,
                destination,
                options,
            } => {
                let cancel = Arc::new(AtomicBool::new(false));
                let rx = crate::db::export::spawn_export(
                    self.params.clone(),
                    source_db,
                    tables,
                    destination,
                    options,
                    Arc::clone(&cancel),
                );
                if let ModalState::ExportData { phase, .. } = &mut self.modal {
                    *phase = ExportPhase::Running(ExportRunning {
                        receiver: rx,
                        cancel_flag: cancel,
                        rows_done: 0,
                        done_count: 0,
                    });
                }
            }
            ModalAction::CancelExport => {
                if let ModalState::ExportData {
                    phase: ExportPhase::Running(running),
                    ..
                } = &mut self.modal
                {
                    running.cancel_flag.store(true, Ordering::Relaxed);
                }
            }
            ModalAction::CloseExport => {
                self.modal = ModalState::None;
            }
        }
    }

    /// Formulario de conexión: elegir, guardar o borrar un perfil. Devuelve
    /// el índice del perfil borrado (las demás sesiones deben reajustarse).
    pub(super) fn handle_profile_form(
        &mut self,
        ctx: &egui::Context,
        sh: &mut Shared,
        selected: Option<usize>,
        save: bool,
        delete: bool,
    ) -> Option<usize> {
        let mut removed = None;
        if let Some(idx) = selected
            && matches!(self.state, ConnState::Disconnected)
        {
            let profile = &sh.profiles[idx];
            self.params = profile.to_params();
            self.port_str = self.params.port.to_string();
            self.profile_name = profile.name.clone();
            self.save_password = profile.has_password();
            self.color = profile.color;
            self.selected_profile_idx = Some(idx);
            self.error_message = None;
        }

        if save && !self.profile_name.trim().is_empty() {
            if let Ok(p) = self.port_str.parse::<u16>() {
                self.params.port = p;
            }
            let name = self.profile_name.trim().to_string();
            let idx = self
                .selected_profile_idx
                .or_else(|| sh.profiles.iter().position(|p| p.name == name));
            let previous = idx.map(|i| &sh.profiles[i]);
            let had_keyring = previous.is_some_and(|p| p.password_in_keyring);
            let profile = ConnectionProfile {
                id: previous.map_or_else(profiles::new_id, |p| p.id.clone()),
                name,
                host: self.params.host.clone(),
                port: self.params.port,
                user: self.params.user.clone(),
                // Una contraseña nueva se marca cuando el llavero la confirma
                // (poll_keyring). Con el campo vacío se conserva la que había.
                password_in_keyring: had_keyring && self.save_password,
                password: None,
                color: self.color,
            };
            if self.save_password && !self.params.password.is_empty() {
                let id = profile.id.clone();
                let label = profile.keyring_label();
                let password = self.params.password.clone();
                let tx = self.keyring_tx.clone();
                let ctx = ctx.clone();
                std::thread::spawn(move || {
                    let result = secrets::set(&id, &label, &password);
                    let _ = tx.send((id, result));
                    ctx.request_repaint();
                });
            } else if had_keyring && !self.save_password {
                secrets::delete_in_background(profile.id.clone());
            }
            self.params.keyring_id = profile.password_in_keyring.then(|| profile.id.clone());
            match idx {
                Some(i) => sh.profiles[i] = profile,
                None => sh.profiles.push(profile),
            }
            self.selected_profile_idx = Some(idx.unwrap_or(sh.profiles.len() - 1));
            profiles::save(&sh.profiles);
        }

        if delete && let Some(idx) = self.selected_profile_idx {
            let profile = sh.profiles.remove(idx);
            if profile.password_in_keyring {
                secrets::delete_in_background(profile.id);
            }
            self.selected_profile_idx = None;
            self.profile_name.clear();
            profiles::save(&sh.profiles);
            removed = Some(idx);
        }

        removed
    }

    /// Lanza la conexión en segundo plano con los parámetros del formulario.
    pub(super) fn start_connect(&mut self, sh: &Shared) {
        if let Ok(p) = self.port_str.parse::<u16>() {
            self.params.port = p;
        }
        self.title = self
            .selected_profile_idx
            .and_then(|i| sh.profiles.get(i))
            .map(|p| p.name.clone())
            .unwrap_or_else(|| self.params.host.clone());
        let params = self.params.clone();
        let host = self.params.host.clone();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            connect_mysql_progress(&params, &tx);
        });
        self.state = ConnState::Connecting {
            receiver: rx,
            host,
            status: "Iniciando...".to_string(),
            start: Instant::now(),
        };
        self.error_message = None;
    }
}

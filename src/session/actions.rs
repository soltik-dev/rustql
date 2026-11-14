//! Lo que hace la sesión con las acciones del usuario: grid, sidebar,
//! pestañas SQL, diálogos y formulario de conexión.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Instant;

use eframe::egui;

use super::view::FrameSignals;
use super::writes::AfterCommit;
use super::{ConnState, FrameKeys, SelectedNode, Session, Shared};
use crate::db::QueryResult;
use crate::db::{Engine, connect_with_progress};
use crate::loadable::Loadable;
use crate::profiles::{self, ConnectionProfile};
use crate::secrets;
use crate::settings::{self, CopyFormat};
use crate::table_state;
use crate::ui::connection_panel::SaveProfile;
use crate::ui::db_info::DbInfoAction;
use crate::ui::filter_bar::FilterAction;
use crate::ui::modals::{
    self, DbObject, ExportPhase, ExportRunning, ModalAction, ModalState, NewColumnDef, Reload,
};
use crate::ui::results_grid::{self, ApplyBarAction, CellRef, GridAction, InsertValue, RowRef};
use crate::ui::sidebar::SidebarAction;
use crate::ui::sql_editor::{SqlEditorAction, SqlTab};
use crate::ui::tab_bar::{ActiveTab, TabBarResult};
use std::collections::BTreeSet;

impl Session {
    /// Aplica lo que el usuario hizo en el frame, en orden: lo de un
    /// manejador puede depender de lo que hizo el anterior.
    pub(super) fn apply_signals(
        &mut self,
        ctx: &egui::Context,
        sh: &mut Shared,
        signals: FrameSignals,
        keys: FrameKeys,
    ) {
        if signals.cancel_connect {
            self.state = ConnState::Disconnected;
        }
        if signals.load_databases {
            self.load_database_list(ctx);
        }
        self.handle_tab_bar(sh, signals.tab_bar, keys);
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
        if keys.f5 {
            self.refresh_active_tab(ctx, sh);
        }
        self.handle_filter_action(sh, signals.filter);
        self.handle_db_info_action(sh, signals.db_info);
        self.handle_sql_action(ctx, sh, signals.sql);
    }

    /// F5: vuelve a leer lo que muestra la pestaña activa (antes recargaba
    /// siempre la tabla abierta, se estuviera viendo o no).
    fn refresh_active_tab(&mut self, ctx: &egui::Context, sh: &mut Shared) {
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
                    && let Some(sql) = tab.sql_to_run()
                {
                    let action = SqlEditorAction::Execute {
                        sql,
                        db_context: tab.db_context.clone(),
                    };
                    self.handle_sql_action(ctx, sh, action);
                }
            }
        }
        // Lo invalidado se vuelve a pedir en el siguiente frame (request_tab_data)
    }

    /// Pestañas de la barra en su orden, sin las que están desactivadas por
    /// no haber BBDD o tabla seleccionada.
    fn tab_order(&self) -> Vec<ActiveTab> {
        let mut tabs = vec![ActiveTab::Servidor];
        if self.schema.selected_db().is_some() {
            tabs.push(ActiveTab::Bbdd);
        }
        if self.schema.selected_table().is_some() {
            tabs.extend([ActiveTab::Tabla, ActiveTab::Datos]);
        }
        tabs.extend((0..self.sql_tabs.len()).map(ActiveTab::Sql));
        tabs
    }

    /// Pestaña siguiente (o anterior) a la activa, dando la vuelta.
    fn neighbour_tab(&self, forward: bool) -> ActiveTab {
        let tabs = self.tab_order();
        let current = tabs.iter().position(|t| *t == self.active_tab).unwrap_or(0);
        let step = if forward { 1 } else { tabs.len() - 1 };
        tabs[(current + step) % tabs.len()]
    }

    fn handle_tab_bar(&mut self, sh: &mut Shared, mut tab_bar: TabBarResult, keys: FrameKeys) {
        // El teclado hace lo mismo que el ratón sobre la barra
        if self.is_connected() {
            if let Some(forward) = keys.cycle_tab {
                tab_bar.tab_clicked = Some(self.neighbour_tab(forward));
            }
            if keys.ctrl_w
                && let ActiveTab::Sql(idx) = self.active_tab
            {
                tab_bar.close_sql_tab = Some(idx);
            }
        }

        if let Some(t) = tab_bar.tab_clicked {
            // Al salir de una pestaña SQL se guarda su texto
            if matches!(self.active_tab, ActiveTab::Sql(_)) && t != self.active_tab {
                self.save_tabs_state(sh);
            }
            self.active_tab = t;
        }

        if let Some((idx, name)) = tab_bar.renamed
            && let Some(tab) = self.sql_tabs.get_mut(idx)
        {
            tab.name = (!name.is_empty()).then_some(name);
            self.save_tabs_state(sh);
        }

        if tab_bar.new_sql_tab || (keys.ctrl_t && self.is_connected()) {
            self.open_sql_tab(sh, None, String::new());
        }

        if let Some(idx) = tab_bar.close_sql_tab
            && let Some(tab) = self.sql_tabs.get(idx)
        {
            // Cerrar borra su consulta para siempre: con texto, se pregunta
            if tab.sql_text.trim().is_empty() {
                self.close_sql_tab(sh, idx);
            } else {
                self.modal = ModalState::ConfirmCloseSqlTab {
                    idx,
                    title: tab.title(idx),
                    preview: crate::ui::ellipsize(tab.sql_text.trim(), 120),
                };
            }
        }
    }

    /// Abre una pestaña SQL nueva con `sql_text` y pasa a ella. Su contexto
    /// es `db` o, sin él, la BBDD seleccionada en el árbol.
    pub fn open_sql_tab(&mut self, sh: &mut Shared, db: Option<String>, sql_text: String) {
        if !self.is_connected() {
            return;
        }
        let db_context = db.unwrap_or_else(|| self.selected_db().unwrap_or_default().to_string());
        self.sql_tabs.push(SqlTab::new(db_context, sql_text));
        self.active_tab = ActiveTab::Sql(self.sql_tabs.len() - 1);
        self.save_tabs_state(sh);
    }

    /// Cierra la pestaña SQL `idx` y descarta su texto. Siempre queda al
    /// menos una pestaña.
    pub(super) fn close_sql_tab(&mut self, sh: &mut Shared, idx: usize) {
        if idx >= self.sql_tabs.len() {
            return;
        }
        // Su consulta, si aún se está ejecutando, ya no le interesa a nadie
        self.cancel_editor(idx);
        // Los índices cambian: el nombre a medio escribir no se aplica
        self.sql_tab_rename = None;
        self.sql_tabs.remove(idx);
        if self.sql_tabs.is_empty() {
            self.sql_tabs.push(SqlTab::default());
        }
        self.active_tab = match self.active_tab {
            ActiveTab::Sql(i) if i >= idx => {
                ActiveTab::Sql(i.saturating_sub(1).min(self.sql_tabs.len() - 1))
            }
            other => other,
        };
        self.save_tabs_state(sh);
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
                        let engine = self.engine();
                        let filter = format!(
                            "{} = {}",
                            engine.quote_ident(&col_name),
                            engine.literal(&text)
                        );
                        self.request_reload(sh, Reload::Filter(filter));
                    }
                    // En un aviso: como error de la tabla tapaba el grid
                    // hasta la siguiente recarga
                    Err(e) => {
                        sh.toasts
                            .error(format!("No se pudo leer el portapapeles: {e}"));
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
                let fmt = fmt.unwrap_or_else(|| sh.settings.copy_format.clone());
                if let Some(data) = &self.table_view.table_data {
                    let table = self.schema.selected_table().map(|(_, table)| table);
                    let rows = &self.table_view.selection.rows;
                    copy_rows(ctx, sh, data, rows, &fmt, table, self.engine());
                }
            }
            GridAction::PageChanged(page) => self.request_reload(sh, Reload::Page(page)),
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
                self.modal = ModalState::ConfirmDrop {
                    target: DbObject::Database(db),
                    typed: String::new(),
                };
            }
            SidebarAction::TruncateAllTables { db } => {
                self.modal = ModalState::ConfirmTruncate {
                    target: DbObject::Database(db),
                };
            }
            SidebarAction::DropTable { db, table } => {
                self.modal = ModalState::ConfirmDrop {
                    target: DbObject::Table { db, table },
                    typed: String::new(),
                };
            }
            SidebarAction::TruncateTable { db, table } => {
                self.modal = ModalState::ConfirmTruncate {
                    target: DbObject::Table { db, table },
                };
            }
            SidebarAction::CopyTableName(name) => {
                sh.toasts.info(format!("«{name}» copiado al portapapeles"));
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
                let sql_text = format!(
                    "SELECT * FROM {} WHERE ",
                    self.engine().qualified(&db, &table)
                );
                self.open_sql_tab(sh, Some(db), sql_text);
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
        if let DbInfoAction::OpenTable(table) = action
            && let Some(db) = self.selected_db().map(str::to_string)
        {
            self.switch_to_table(sh, db, table, ActiveTab::Datos);
        }
    }

    fn handle_sql_action(&mut self, ctx: &egui::Context, sh: &mut Shared, action: SqlEditorAction) {
        let ActiveTab::Sql(tab_idx) = self.active_tab else {
            return;
        };
        match action {
            SqlEditorAction::None => {}
            SqlEditorAction::Execute { sql, db_context } => {
                // Persistir texto de pestañas SQL al ejecutar
                self.save_tabs_state(sh);
                self.run_editor(tab_idx, sql, db_context, sh.settings.sql_editor_max_rows);
            }
            SqlEditorAction::Cancel => self.cancel_editor(tab_idx),
            SqlEditorAction::CopyRows(fmt) => {
                let fmt = fmt.unwrap_or_else(|| sh.settings.copy_format.clone());
                if let Some(tab) = self.sql_tabs.get(tab_idx)
                    && let Some(Ok(data)) = &tab.result
                {
                    copy_rows(
                        ctx,
                        sh,
                        data,
                        &tab.selection.rows,
                        &fmt,
                        None,
                        self.engine(),
                    );
                }
            }
            // Limpiar no se puede deshacer: se pregunta antes
            SqlEditorAction::Clear => {
                if let Some(tab) = self.sql_tabs.get(tab_idx) {
                    self.modal = ModalState::ConfirmClearSql {
                        idx: tab_idx,
                        title: tab.title(tab_idx),
                        preview: crate::ui::ellipsize(tab.sql_text.trim(), 120),
                    };
                }
            }
        }
    }

    /// Procesa la acción resultante del modal activo.
    pub(super) fn handle_modal_action(&mut self, modal_action: ModalAction, sh: &mut Shared) {
        match modal_action {
            ModalAction::None => {}
            ModalAction::Cancel | ModalAction::CloseSession | ModalAction::DeleteProfile => {
                self.modal = ModalState::None;
            }
            ModalAction::CloseSqlTab(idx) => {
                self.modal = ModalState::None;
                self.close_sql_tab(sh, idx);
            }
            ModalAction::ClearSql(idx) => {
                self.modal = ModalState::None;
                if let Some(tab) = self.sql_tabs.get_mut(idx) {
                    tab.sql_text.clear();
                }
                self.save_tabs_state(sh);
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
                let sql = modals::build_create_table_sql(self.engine(), &db, &name, &columns);
                self.create_table(db, name, sql);
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
        save: Option<SaveProfile>,
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
            // La lista era la del servidor del perfil anterior
            self.db_picker = Default::default();
        }

        if let Some(mode) = save {
            self.save_profile(ctx, sh, mode);
        }

        if delete && let Some(idx) = self.selected_profile_idx {
            let profile = sh.profiles.remove(idx);
            if profile.password_in_keyring {
                secrets::delete_in_background(profile.id);
            }
            self.selected_profile_idx = None;
            self.profile_name.clear();
            profiles::save(&sh.profiles);
            sh.toasts
                .info(format!("Favorito «{}» eliminado", profile.name));
            removed = Some(idx);
        }

        removed
    }

    /// Guarda el formulario como favorito: sobre el seleccionado (o el que
    /// se llame igual) o, con `SaveProfile::AsNew`, como otro distinto.
    fn save_profile(&mut self, ctx: &egui::Context, sh: &mut Shared, mode: SaveProfile) {
        let name = self.profile_name.trim().to_string();
        if name.is_empty() {
            self.error_message = Some("Pon un nombre al favorito para guardarlo.".into());
            return;
        }
        if !self.read_port() {
            return;
        }
        let same_name = sh.profiles.iter().position(|p| p.name == name);
        let idx = match mode {
            SaveProfile::Update => self.selected_profile_idx.or(same_name),
            SaveProfile::AsNew if same_name.is_some() => {
                self.error_message = Some(format!(
                    "Ya hay un favorito llamado «{name}»: cambia el nombre para guardarlo \
                     como otro."
                ));
                return;
            }
            SaveProfile::AsNew => None,
        };
        self.error_message = None;
        let previous = idx.map(|i| &sh.profiles[i]);
        let had_keyring = previous.is_some_and(|p| p.password_in_keyring);
        let profile = ConnectionProfile {
            id: previous.map_or_else(profiles::new_id, |p| p.id.clone()),
            name,
            engine: self.params.engine,
            host: self.params.host.clone(),
            port: self.params.port,
            user: self.params.user.clone(),
            database: self.params.database.trim().to_string(),
            tls: self.params.tls,
            tls_ca: self.params.tls_ca.trim().to_string(),
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
        sh.toasts
            .success(format!("Favorito «{}» guardado", self.profile_name.trim()));
    }

    /// Pasa el puerto del formulario a los parámetros. Si no es un puerto
    /// válido lo dice en el formulario y devuelve false: antes se ignoraba y
    /// se usaba el puerto anterior sin avisar.
    fn read_port(&mut self) -> bool {
        match self.port_str.trim().parse::<u16>() {
            Ok(port) if port > 0 => {
                self.params.port = port;
                true
            }
            _ => {
                self.error_message = Some(format!(
                    "Puerto no válido: «{}». Debe ser un número entre 1 y 65535.",
                    self.port_str.trim()
                ));
                false
            }
        }
    }

    /// Lee en segundo plano las BBDD del servidor del formulario, con sus
    /// credenciales, para elegir una. Cada petición sustituye a la anterior.
    pub(super) fn load_database_list(&mut self, ctx: &egui::Context) {
        if self.params.host.trim().is_empty() {
            self.db_picker.list = Loadable::Failed("Indica el host del servidor.".into());
            return;
        }
        if !self.read_port() {
            self.db_picker.list = Loadable::Failed(self.error_message.take().unwrap_or_default());
            return;
        }
        let params = self.params.clone();
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(crate::db::list_server_databases(&params));
            ctx.request_repaint();
        });
        self.db_picker.list = Loadable::Loading(0);
        self.db_picker.receiver = Some(rx);
    }

    /// Lanza la conexión en segundo plano con los parámetros del formulario.
    pub(super) fn start_connect(&mut self, sh: &Shared) {
        if self.params.host.trim().is_empty() {
            self.error_message = Some("Indica el host del servidor.".into());
            return;
        }
        if !self.read_port() {
            return;
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
            connect_with_progress(&params, &tx);
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

/// Copia al portapapeles las filas `rows` de `data` en el formato `fmt` y lo
/// confirma con un aviso. `table` es el nombre que llevan los INSERT.
fn copy_rows(
    ctx: &egui::Context,
    sh: &mut Shared,
    data: &QueryResult,
    rows: &BTreeSet<usize>,
    fmt: &CopyFormat,
    table: Option<&str>,
    engine: Engine,
) {
    let copied = results_grid::copy_rows(ctx, data, rows, fmt, table, engine);
    let format = match fmt {
        CopyFormat::Csv => "CSV",
        CopyFormat::Tsv => "TSV",
        CopyFormat::SqlInserts => "INSERT",
    };
    match copied {
        0 => {}
        1 => sh.toasts.info(format!("1 fila copiada ({format})")),
        n => sh.toasts.info(format!("{n} filas copiadas ({format})")),
    }
}

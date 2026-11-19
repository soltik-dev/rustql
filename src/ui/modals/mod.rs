//! Modal dialogs of the session: forms, confirmations and errors.

mod export;

use eframe::egui;

use self::export::show_export_data;
pub use self::export::{ExportPhase, ExportRunning, TableExportEntry, TableExportState};

use crate::db::export::{ExportDestination, ExportOptions};
use crate::db::{Engine, TableKind};
use crate::profiles::ConnectionProfile;
use crate::ui::icons::*;
use crate::ui::results_grid::SortSpec;
use crate::ui::theme;

// ── Data types for forms ────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct NewColumnDef {
    pub name: String,
    pub col_type: String,
    pub length: String,
    pub not_null: bool,
    pub primary_key: bool,
}

impl NewColumnDef {
    pub fn new() -> Self {
        Self {
            col_type: "VARCHAR".into(),
            length: "255".into(),
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub enum FilterOperator {
    #[default]
    Equals,
    NotEquals,
    Contains,
    StartsWith,
    EndsWith,
    GreaterThan,
    LessThan,
    GreaterOrEqual,
    LessOrEqual,
}

impl FilterOperator {
    fn label(&self) -> &'static str {
        match self {
            Self::Equals => "= igual a",
            Self::NotEquals => "≠ distinto de",
            Self::Contains => "LIKE '%…%' contiene",
            Self::StartsWith => "LIKE '…%' empieza por",
            Self::EndsWith => "LIKE '%…' termina en",
            Self::GreaterThan => "> mayor que",
            Self::LessThan => "< menor que",
            Self::GreaterOrEqual => ">= mayor o igual",
            Self::LessOrEqual => "<= menor o igual",
        }
    }
}

/// What gets dropped or emptied: a whole database or a table.
#[derive(Debug, Clone, PartialEq)]
pub enum DbObject {
    Database(String),
    /// A table or a view.
    Table {
        db: String,
        table: String,
        kind: TableKind,
    },
}

/// Reload of the open table. With pending changes it waits for the user
/// to decide what to do with them: they refer to rows by position and
/// with the new data they would point to others.
#[derive(Debug, Clone)]
pub enum Reload {
    /// New sort order; the grid keeps the previous one until confirmed.
    Sort(Vec<SortSpec>),
    /// New WHERE filter (empty = no filter).
    Filter(String),
    /// F5: read the data and the structure again.
    Refresh,
    /// Another page of the table (0 = the first).
    Page(usize),
}

// ── Modal state ──────────────────────────────────────────────────────────────

/// There is one per session: the export taking more space than the other
/// variants is not worth the indirection of a Box.
#[derive(Debug, Default)]
#[allow(clippy::large_enum_variant)]
pub enum ModalState {
    #[default]
    None,
    CreateDatabase {
        name: String,
        charset: String,
        error: Option<String>,
    },
    CreateTable {
        db: String,
        name: String,
        columns: Vec<NewColumnDef>,
        error: Option<String>,
    },
    ConfirmDrop {
        target: DbObject,
        /// Name the user types to confirm (only for a database).
        typed: String,
    },
    /// Empty a table or, with a database, all of its tables.
    ConfirmTruncate {
        target: DbObject,
    },
    FilterInput {
        col_name: String,
        value: String,
        operator: FilterOperator,
    },
    /// There are unsaved changes in the current table and the user tries to switch to another.
    ConfirmSwitchTable {
        new_db: String,
        new_table: String,
        pending_count: usize,
        current_table: String,
    },
    /// There are unsaved changes and the user sorts, filters or reloads the table.
    ConfirmReload {
        reload: Reload,
        pending_count: usize,
        current_table: String,
    },
    /// There are unsaved changes and the user closes the connection tab.
    ConfirmCloseSession {
        title: String,
        pending_count: usize,
    },
    /// The user deletes the favourite selected in the form.
    ConfirmDeleteProfile {
        name: String,
    },
    /// The user closes a SQL tab that has text.
    ConfirmCloseSqlTab {
        idx: usize,
        /// Title of the tab.
        title: String,
        /// Start of the query, to recognise it.
        preview: String,
    },
    /// The user presses "Clear" in a SQL tab that has text.
    ConfirmClearSql {
        idx: usize,
        title: String,
        preview: String,
    },
    CloneTable {
        db: String,
        table: String,
        new_name: String,
        copy_data: bool,
        error: Option<String>,
    },
    Error {
        title: String,
        message: String,
    },
    ExportData {
        source_db: String,
        source_table: Option<String>,
        tables: Vec<TableExportEntry>,
        databases: Vec<String>,
        profiles: Vec<ConnectionProfile>,
        destination: ExportDestination,
        target_db_name: String,
        profile_idx: usize,
        options: ExportOptions,
        phase: ExportPhase,
    },
}

// ── Actions returned to the caller ────────────────────────────────────────────

pub enum ModalAction {
    None,
    CreateDatabase {
        name: String,
        charset: String,
    },
    CreateTable {
        db: String,
        name: String,
        columns: Vec<NewColumnDef>,
    },
    DropDatabase(String),
    DropTable {
        db: String,
        table: String,
    },
    TruncateTable {
        db: String,
        table: String,
    },
    TruncateAllTables(String),
    ApplyFilter {
        sql_fragment: String,
    },
    Cancel,
    /// Save changes and then switch to the new table.
    ApplyAndSwitch {
        new_db: String,
        new_table: String,
    },
    /// Discard changes and switch to the new table.
    DiscardAndSwitch {
        new_db: String,
        new_table: String,
    },
    /// Save changes and then reload the table.
    ApplyAndReload(Reload),
    /// Discard changes and reload the table.
    DiscardAndReload(Reload),
    /// Close the connection tab discarding the pending changes.
    CloseSession,
    /// Delete the favourite selected in the form.
    DeleteProfile,
    /// Close the SQL tab and discard its query.
    CloseSqlTab(usize),
    /// Clear the SQL tab's query.
    ClearSql(usize),
    CloneTable {
        db: String,
        src_table: String,
        new_name: String,
        copy_data: bool,
    },
    /// Starts the export: app.rs creates the connection and the thread.
    StartExport {
        source_db: String,
        tables: Vec<String>,
        destination: ExportDestination,
        options: ExportOptions,
    },
    CancelExport,
    CloseExport,
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn red_btn(ui: &mut egui::Ui, text: &str) -> bool {
    theme::danger_button(ui, text).clicked()
}

// ── Main function ─────────────────────────────────────────────────────────────

/// Modal dialog: dims and blocks the rest of the interface until it is
/// closed. With `open`, Escape or the × set it to false (same as Cancel);
/// without it (an export running) it only closes with its buttons. A
/// click outside does not close it: it would lose what was typed in a form.
fn dialog(
    ctx: &egui::Context,
    title: &str,
    min_width: f32,
    open: Option<&mut bool>,
    content: impl FnOnce(&mut egui::Ui),
) {
    let closable = open.is_some();
    let mut close_clicked = false;
    let response = egui::Modal::new(egui::Id::unique(("rustql_dialog", title))).show(ctx, |ui| {
        ui.set_min_width(min_width);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(title).strong());
            if closable {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    close_clicked = ui
                        .add(egui::Button::new("×").frame(false))
                        .on_hover_text("Cerrar  Esc")
                        .clicked();
                });
            }
        });
        ui.separator();
        ui.add_space(4.0);
        content(ui);
    });
    let escape = response.is_top_modal
        && !response.any_popup_open
        && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    if let Some(open) = open
        && (close_clicked || escape)
    {
        *open = false;
    }
}

/// Button row of a form. With `busy` (a write in progress) it is disabled
/// and shows a spinner: that way nothing is sent twice.
fn form_buttons(ui: &mut egui::Ui, busy: bool, add_buttons: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.add_enabled_ui(!busy, add_buttons);
        if busy {
            ui.spinner();
            ui.label("Ejecutando…");
        }
    });
}

/// Renders the active modal on top of everything else.
/// Must be called from `update()` after every panel, passing `ctx`.
/// `busy`: a write is in progress (the forms wait for its result).
pub fn show(
    ctx: &egui::Context,
    modal: &mut ModalState,
    engine: Engine,
    busy: bool,
) -> ModalAction {
    match modal {
        ModalState::None => ModalAction::None,
        ModalState::CreateDatabase {
            name,
            charset,
            error,
        } => show_create_database(ctx, engine, name, charset, error, busy),
        ModalState::CreateTable {
            db,
            name,
            columns,
            error,
        } => show_create_table(ctx, engine, db, name, columns, error, busy),
        ModalState::ConfirmDrop { target, typed } => show_confirm_drop(ctx, target, typed),
        ModalState::ConfirmTruncate { target } => show_confirm_truncate(ctx, target),
        ModalState::FilterInput {
            col_name,
            value,
            operator,
        } => show_filter_input(ctx, engine, col_name, value, operator),
        ModalState::ConfirmSwitchTable {
            new_db,
            new_table,
            pending_count,
            current_table,
        } => show_confirm_switch_table(ctx, new_db, new_table, *pending_count, current_table),
        ModalState::ConfirmReload {
            reload,
            pending_count,
            current_table,
        } => show_confirm_reload(ctx, reload, *pending_count, current_table),
        ModalState::ConfirmCloseSession {
            title,
            pending_count,
        } => show_confirm_close_session(ctx, title, *pending_count),
        ModalState::ConfirmDeleteProfile { name } => show_confirm_delete_profile(ctx, name),
        ModalState::ConfirmCloseSqlTab {
            idx,
            title,
            preview,
        } => show_confirm_discard_sql(
            ctx,
            &format!("Cerrar «{title}»"),
            "Cerrar pestaña",
            preview,
            ModalAction::CloseSqlTab(*idx),
        ),
        ModalState::ConfirmClearSql {
            idx,
            title,
            preview,
        } => show_confirm_discard_sql(
            ctx,
            &format!("Limpiar «{title}»"),
            "Limpiar",
            preview,
            ModalAction::ClearSql(*idx),
        ),
        ModalState::CloneTable {
            db,
            table,
            new_name,
            copy_data,
            error,
        } => show_clone_table(ctx, db, table, new_name, copy_data, error, busy),
        ModalState::Error { title, message } => show_error(ctx, title, message),
        ModalState::ExportData {
            source_db,
            source_table,
            tables,
            databases,
            profiles,
            destination,
            target_db_name,
            profile_idx,
            options,
            phase,
        } => show_export_data(
            ctx,
            engine,
            source_db,
            source_table,
            tables,
            databases,
            profiles,
            destination,
            target_db_name,
            profile_idx,
            options,
            phase,
        ),
    }
}

// ── Form: create database ─────────────────────────────────────────────────────

fn show_create_database(
    ctx: &egui::Context,
    engine: Engine,
    name: &mut String,
    charset: &mut String,
    error: &mut Option<String>,
    busy: bool,
) -> ModalAction {
    let mut action = ModalAction::None;
    let mut open = true;

    dialog(ctx, "Crear base de datos", 280.0, Some(&mut open), |ui| {
        egui::Grid::new("create_db_grid")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .show(ui, |ui| {
                ui.label("Nombre:");
                ui.add(
                    egui::TextEdit::singleline(name)
                        .desired_width(200.0)
                        .hint_text("mi_base_de_datos"),
                );
                ui.end_row();

                // Only for engines with a per-database charset
                let charsets = engine.database_charsets();
                if !charsets.is_empty() {
                    ui.label("Charset:");
                    egui::ComboBox::from_id_salt("charset_combo")
                        .selected_text(charset.as_str())
                        .show_ui(ui, |ui| {
                            for &cs in charsets {
                                ui.selectable_value(charset, cs.to_string(), cs);
                            }
                        });
                    ui.end_row();
                }
            });

        if let Some(e) = error {
            ui.separator();
            theme::error_card(ui, &format!("Error: {e}"));
        }

        ui.separator();
        form_buttons(ui, busy, |ui| {
            let create_clicked = theme::primary_button(ui, "Crear").clicked();
            let enter_pressed = ui.is_enabled() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if (create_clicked || enter_pressed) && !name.trim().is_empty() {
                action = ModalAction::CreateDatabase {
                    name: name.trim().to_string(),
                    charset: charset.clone(),
                };
            }
            if ui.button("Cancelar").clicked() {
                action = ModalAction::Cancel;
            }
        });
    });

    if !open {
        action = ModalAction::Cancel;
    }

    action
}

// ── Form: create table ────────────────────────────────────────────────────────

fn show_create_table(
    ctx: &egui::Context,
    engine: Engine,
    db: &str,
    name: &mut String,
    columns: &mut Vec<NewColumnDef>,
    error: &mut Option<String>,
    busy: bool,
) -> ModalAction {
    let mut action = ModalAction::None;
    let mut open = true;

    // There is always at least one column row to fill in
    if columns.is_empty() {
        columns.push(NewColumnDef::new());
    }

    dialog(ctx, "Crear tabla", 600.0, Some(&mut open), |ui| {
        ui.horizontal(|ui| {
            ui.label("Base de datos:");
            ui.strong(db);
            ui.separator();
            ui.label("Nombre de tabla:");
            ui.add(
                egui::TextEdit::singleline(name)
                    .desired_width(180.0)
                    .hint_text("mi_tabla"),
            );
        });
        ui.separator();

        ui.strong("Columnas:");
        ui.add_space(4.0);

        use egui_extras::{Column, TableBuilder};

        let mut remove_idx: Option<usize> = None;

        TableBuilder::new(ui)
            .resizable(false)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::exact(160.0)) // name
            .column(Column::exact(110.0)) // type
            .column(Column::exact(70.0)) // length
            .column(Column::exact(60.0)) // not null
            .column(Column::exact(40.0)) // PK
            .column(Column::exact(30.0)) // delete
            .header(20.0, |mut header| {
                for h in &["Nombre", "Tipo", "Longitud", "Not Null", "PK", ""] {
                    header.col(|ui| {
                        ui.strong(*h);
                    });
                }
            })
            .body(|body| {
                let n = columns.len();
                body.rows(24.0, n, |mut row| {
                    let i = row.index();
                    let col = &mut columns[i];
                    row.col(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut col.name)
                                .desired_width(f32::INFINITY)
                                .hint_text("columna"),
                        );
                    });
                    row.col(|ui| {
                        egui::ComboBox::from_id_salt(format!("type_{i}"))
                            .selected_text(&col.col_type)
                            .width(100.0)
                            .show_ui(ui, |ui| {
                                for &(t, _) in engine.column_types() {
                                    ui.selectable_value(&mut col.col_type, t.to_string(), t);
                                }
                            });
                    });
                    row.col(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut col.length)
                                .desired_width(f32::INFINITY),
                        );
                    });
                    row.col(|ui| {
                        ui.centered_and_justified(|ui| {
                            ui.checkbox(&mut col.not_null, "");
                        });
                    });
                    row.col(|ui| {
                        ui.centered_and_justified(|ui| {
                            ui.checkbox(&mut col.primary_key, "");
                        });
                    });
                    row.col(|ui| {
                        if ui.small_button("✖").clicked() {
                            remove_idx = Some(i);
                        }
                    });
                });
            });

        if let Some(idx) = remove_idx {
            columns.remove(idx);
        }

        if ui.button(format!("{PLUS} Añadir columna")).clicked() {
            columns.push(NewColumnDef::new());
        }

        if let Some(e) = error {
            ui.separator();
            theme::error_card(ui, &format!("Error: {e}"));
        }

        ui.separator();
        form_buttons(ui, busy, |ui| {
            let create_clicked = theme::primary_button(ui, "Crear tabla").clicked();
            let enter_pressed = ui.is_enabled() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if (create_clicked || enter_pressed) && !name.trim().is_empty() && !columns.is_empty() {
                action = ModalAction::CreateTable {
                    db: db.to_string(),
                    name: name.trim().to_string(),
                    columns: columns.clone(),
                };
            }
            if ui.button("Cancelar").clicked() {
                action = ModalAction::Cancel;
            }
        });
    });

    if !open {
        action = ModalAction::Cancel;
    }

    action
}

// ── Confirmation: drop ────────────────────────────────────────────────────────

fn show_confirm_drop(ctx: &egui::Context, target: &DbObject, typed: &mut String) -> ModalAction {
    let mut action = ModalAction::None;
    let mut open = true;

    let (title, what, name) = match target {
        DbObject::Database(db) => ("Eliminar base de datos", "la base de datos", db.clone()),
        DbObject::Table { db, table, kind } => {
            let title = if kind.is_view() {
                "Eliminar vista"
            } else {
                "Eliminar tabla"
            };
            (title, kind.label(), format!("{db}.{table}"))
        }
    };

    dialog(ctx, title, 300.0, Some(&mut open), |ui| {
        ui.label(format!("¿Seguro que quieres eliminar {what}:"));
        ui.add_space(4.0);
        ui.strong(&name);
        ui.add_space(4.0);
        ui.colored_label(
            theme::get(ui).warning,
            "⚠ Esta acción no se puede deshacer.",
        );
        // A whole database is not dropped with a single click
        let confirmed = match target {
            DbObject::Database(db) => {
                ui.add_space(6.0);
                ui.label("Escribe su nombre para confirmar:");
                ui.add(egui::TextEdit::singleline(typed).desired_width(f32::INFINITY));
                typed.trim() == db
            }
            DbObject::Table { .. } => true,
        };
        ui.separator();
        ui.horizontal(|ui| {
            let delete = ui
                .add_enabled_ui(confirmed, |ui| red_btn(ui, "Eliminar"))
                .inner;
            if delete {
                action = match target.clone() {
                    DbObject::Database(db) => ModalAction::DropDatabase(db),
                    DbObject::Table { db, table, .. } => ModalAction::DropTable { db, table },
                };
            }
            if ui.button("Cancelar").clicked() {
                action = ModalAction::Cancel;
            }
        });
    });

    if !open {
        action = ModalAction::Cancel;
    }

    action
}

// ── Confirmation: empty ───────────────────────────────────────────────────────

fn show_confirm_truncate(ctx: &egui::Context, target: &DbObject) -> ModalAction {
    let mut action = ModalAction::None;
    let mut open = true;

    let (what, name) = match target {
        DbObject::Database(db) => ("todas las tablas de", db.clone()),
        DbObject::Table { db, table, .. } => ("la tabla", format!("{db}.{table}")),
    };

    dialog(ctx, "Vaciar datos", 280.0, Some(&mut open), |ui| {
        ui.label(format!("¿Seguro que quieres vaciar {what}:"));
        ui.add_space(4.0);
        ui.strong(&name);
        ui.add_space(4.0);
        ui.colored_label(
            theme::get(ui).warning,
            "⚠ Se eliminarán todos los datos. No se puede deshacer.",
        );
        ui.separator();
        ui.horizontal(|ui| {
            if red_btn(ui, "Vaciar") {
                action = match target.clone() {
                    DbObject::Database(db) => ModalAction::TruncateAllTables(db),
                    DbObject::Table { db, table, .. } => ModalAction::TruncateTable { db, table },
                };
            }
            if ui.button("Cancelar").clicked() {
                action = ModalAction::Cancel;
            }
        });
    });

    if !open {
        action = ModalAction::Cancel;
    }

    action
}

// ── Text filter ───────────────────────────────────────────────────────────────

const FILTER_OPERATORS: &[FilterOperator] = &[
    FilterOperator::Equals,
    FilterOperator::NotEquals,
    FilterOperator::Contains,
    FilterOperator::StartsWith,
    FilterOperator::EndsWith,
    FilterOperator::GreaterThan,
    FilterOperator::LessThan,
    FilterOperator::GreaterOrEqual,
    FilterOperator::LessOrEqual,
];

fn show_filter_input(
    ctx: &egui::Context,
    engine: Engine,
    col_name: &mut String,
    value: &mut String,
    operator: &mut FilterOperator,
) -> ModalAction {
    let mut action = ModalAction::None;
    let mut open = true;

    dialog(
        ctx,
        &format!("Filtrar por {col_name}"),
        320.0,
        Some(&mut open),
        |ui| {
            egui::Grid::new("filter_input_grid")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label("Operador:");
                    egui::ComboBox::from_id_salt("filter_op_combo")
                        .selected_text(operator.label())
                        .width(200.0)
                        .show_ui(ui, |ui| {
                            for op in FILTER_OPERATORS {
                                ui.selectable_value(operator, op.clone(), op.label());
                            }
                        });
                    ui.end_row();

                    ui.label("Valor:");
                    let resp = ui.add(
                        egui::TextEdit::singleline(value)
                            .desired_width(200.0)
                            .hint_text("valor a filtrar"),
                    );
                    // Auto-focus the text field
                    resp.request_focus();
                    ui.end_row();
                });

            ui.separator();
            ui.horizontal(|ui| {
                let apply_clicked = theme::primary_button(ui, "Aplicar").clicked();
                let enter_pressed = ui.input(|i| i.key_pressed(egui::Key::Enter));
                if apply_clicked || enter_pressed {
                    let frag = build_filter_fragment(engine, col_name, operator, value);
                    action = ModalAction::ApplyFilter { sql_fragment: frag };
                }
                if ui.button("Cancelar").clicked() {
                    action = ModalAction::Cancel;
                }
            });
        },
    );

    if !open {
        action = ModalAction::Cancel;
    }

    action
}

fn build_filter_fragment(
    engine: Engine,
    col_name: &str,
    operator: &FilterOperator,
    value: &str,
) -> String {
    let compare = |op: &str| {
        format!(
            "{} {op} {}",
            engine.quote_ident(col_name),
            engine.literal(value)
        )
    };
    match operator {
        FilterOperator::Equals => compare("="),
        FilterOperator::NotEquals => compare("!="),
        FilterOperator::Contains => engine.like(col_name, &format!("%{value}%")),
        FilterOperator::StartsWith => engine.like(col_name, &format!("{value}%")),
        FilterOperator::EndsWith => engine.like(col_name, &format!("%{value}")),
        FilterOperator::GreaterThan => compare(">"),
        FilterOperator::LessThan => compare("<"),
        FilterOperator::GreaterOrEqual => compare(">="),
        FilterOperator::LessOrEqual => compare("<="),
    }
}

// ── Helpers to build CREATE TABLE SQL ─────────────────────────────────────────

pub fn build_create_table_sql(
    engine: Engine,
    db: &str,
    name: &str,
    columns: &[NewColumnDef],
) -> String {
    let mut parts = Vec::new();
    let mut pks = Vec::new();

    for col in columns {
        if col.name.trim().is_empty() {
            continue;
        }
        // A type not in the list (should not happen) is left as it comes
        let takes_length = engine
            .column_types()
            .iter()
            .find(|(name, _)| *name == col.col_type)
            .is_none_or(|(_, takes_length)| *takes_length);
        let type_part = if col.length.trim().is_empty() || !takes_length {
            col.col_type.clone()
        } else {
            format!("{}({})", col.col_type, col.length.trim())
        };

        let null_part = if col.not_null { " NOT NULL" } else { "" };
        parts.push(format!(
            "  {} {}{}",
            engine.quote_ident(col.name.trim()),
            type_part,
            null_part
        ));

        if col.primary_key {
            pks.push(engine.quote_ident(col.name.trim()));
        }
    }

    if !pks.is_empty() {
        parts.push(format!("  PRIMARY KEY ({})", pks.join(", ")));
    }

    format!(
        "CREATE TABLE {} (\n{}\n)",
        engine.qualified(db, name),
        parts.join(",\n")
    )
}

// ── Confirmation: switch table with pending changes ───────────────────────────

fn show_confirm_switch_table(
    ctx: &egui::Context,
    new_db: &str,
    new_table: &str,
    pending_count: usize,
    current_table: &str,
) -> ModalAction {
    let mut action = ModalAction::None;
    let mut open = true;

    dialog(ctx, "Cambios sin guardar", 280.0, Some(&mut open), |ui| {
        ui.label(format!(
            "Tienes {} cambio(s) sin guardar en «{}».",
            pending_count, current_table
        ));
        ui.add_space(4.0);
        ui.label(format!(
            "¿Qué quieres hacer antes de abrir «{}.{}»?",
            new_db, new_table
        ));
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if theme::primary_button(ui, format!("{CLONE} Guardar y cambiar")).clicked() {
                action = ModalAction::ApplyAndSwitch {
                    new_db: new_db.to_string(),
                    new_table: new_table.to_string(),
                };
            }
            if theme::warning_button(ui, format!("{TRASH}  Descartar y cambiar")).clicked() {
                action = ModalAction::DiscardAndSwitch {
                    new_db: new_db.to_string(),
                    new_table: new_table.to_string(),
                };
            }
            if ui.button("Cancelar").clicked() {
                action = ModalAction::Cancel;
            }
        });
    });

    if !open {
        action = ModalAction::Cancel;
    }
    action
}

fn show_confirm_reload(
    ctx: &egui::Context,
    reload: &Reload,
    pending_count: usize,
    current_table: &str,
) -> ModalAction {
    let mut action = ModalAction::None;
    let mut open = true;

    // Changes go by row position: with other data they are not valid
    let (lost, save, discard) = match reload {
        Reload::Page(_) => (
            "Al cambiar de página se perderían.",
            "Guardar y cambiar",
            "Descartar y cambiar",
        ),
        _ => (
            "Al recargar los datos se perderían.",
            "Guardar y recargar",
            "Descartar y recargar",
        ),
    };
    dialog(ctx, "Cambios sin guardar", 280.0, Some(&mut open), |ui| {
        ui.label(format!(
            "Tienes {pending_count} cambio(s) sin guardar en «{current_table}»."
        ));
        ui.add_space(4.0);
        ui.label(lost);
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if theme::primary_button(ui, save).clicked() {
                action = ModalAction::ApplyAndReload(reload.clone());
            }
            if theme::warning_button(ui, format!("{TRASH}  {discard}")).clicked() {
                action = ModalAction::DiscardAndReload(reload.clone());
            }
            if ui.button("Cancelar").clicked() {
                action = ModalAction::Cancel;
            }
        });
    });

    if !open {
        action = ModalAction::Cancel;
    }
    action
}

fn show_confirm_close_session(
    ctx: &egui::Context,
    title: &str,
    pending_count: usize,
) -> ModalAction {
    let mut action = ModalAction::None;
    let mut open = true;

    dialog(ctx, "Cerrar conexión", 280.0, Some(&mut open), |ui| {
        ui.label(format!(
            "Tienes {pending_count} cambio(s) sin guardar en «{title}»."
        ));
        ui.add_space(4.0);
        ui.label("Si cierras la pestaña se perderán.");
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if theme::warning_button(ui, format!("{TRASH}  Descartar y cerrar")).clicked() {
                action = ModalAction::CloseSession;
            }
            if ui.button("Cancelar").clicked() {
                action = ModalAction::Cancel;
            }
        });
    });

    if !open {
        action = ModalAction::Cancel;
    }
    action
}

fn show_confirm_delete_profile(ctx: &egui::Context, name: &str) -> ModalAction {
    let mut action = ModalAction::None;
    let mut open = true;

    dialog(ctx, "Eliminar favorito", 280.0, Some(&mut open), |ui| {
        ui.label("¿Seguro que quieres eliminar el favorito:");
        ui.add_space(4.0);
        ui.strong(name);
        ui.add_space(4.0);
        ui.label("Se borrará también su contraseña del llavero.");
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if red_btn(ui, "Eliminar") {
                action = ModalAction::DeleteProfile;
            }
            if ui.button("Cancelar").clicked() {
                action = ModalAction::Cancel;
            }
        });
    });

    if !open {
        action = ModalAction::Cancel;
    }
    action
}

/// Close or clear a SQL tab with text: in both cases the query is lost.
/// `confirm` is the button text and `confirmed` what it returns.
fn show_confirm_discard_sql(
    ctx: &egui::Context,
    title: &str,
    confirm: &str,
    preview: &str,
    confirmed: ModalAction,
) -> ModalAction {
    let mut action = ModalAction::None;
    let mut open = true;

    dialog(ctx, title, 320.0, Some(&mut open), |ui| {
        ui.set_max_width(420.0);
        ui.label("La consulta de esta pestaña se perderá:");
        ui.add_space(4.0);
        ui.label(egui::RichText::new(preview).monospace().weak());
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if theme::warning_button(ui, format!("{TRASH}  {confirm}")).clicked() {
                action = confirmed;
            }
            if ui.button("Cancelar").clicked() {
                action = ModalAction::Cancel;
            }
        });
    });

    if !open {
        action = ModalAction::Cancel;
    }
    action
}

// ── Clone table ──────────────────────────────────────────────────────────────

fn show_clone_table(
    ctx: &egui::Context,
    db: &str,
    table: &str,
    new_name: &mut String,
    copy_data: &mut bool,
    error: &mut Option<String>,
    busy: bool,
) -> ModalAction {
    let mut action = ModalAction::None;
    let mut open = true;

    dialog(
        ctx,
        &format!("Clonar tabla — {db}.{table}"),
        280.0,
        Some(&mut open),
        |ui| {
            egui::Grid::new("clone_table_grid")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label("Nuevo nombre:");
                    let resp = ui.add(
                        egui::TextEdit::singleline(new_name)
                            .desired_width(220.0)
                            .hint_text(format!("{table}_copia")),
                    );
                    ui.end_row();
                    ui.label("Copiar datos:");
                    ui.checkbox(copy_data, "INSERT INTO … SELECT *");
                    ui.end_row();
                    let _ = resp;
                });

            if let Some(err) = error {
                theme::error_card(ui, &format!("Error: {err}"));
            }
            ui.separator();
            form_buttons(ui, busy, |ui| {
                let ok = theme::primary_button_enabled(
                    ui,
                    !new_name.is_empty(),
                    format!("{CLONE}  Clonar"),
                )
                .clicked();
                let enter_pressed =
                    ui.is_enabled() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if ok || (enter_pressed && !new_name.is_empty()) {
                    action = ModalAction::CloneTable {
                        db: db.to_string(),
                        src_table: table.to_string(),
                        new_name: new_name.clone(),
                        copy_data: *copy_data,
                    };
                }
                if ui.button("Cancelar").clicked() {
                    action = ModalAction::Cancel;
                }
            });
        },
    );

    if !open {
        action = ModalAction::Cancel;
    }
    action
}

// ── Generic error ─────────────────────────────────────────────────────────────

fn show_error(ctx: &egui::Context, title: &str, message: &str) -> ModalAction {
    let mut open = true;
    let mut close = false;

    dialog(ctx, title, 280.0, Some(&mut open), |ui| {
        theme::error_card(ui, message);
        ui.separator();
        if ui.button("Cerrar").clicked() {
            close = true;
        }
    });

    if !open || close {
        ModalAction::Cancel
    } else {
        ModalAction::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filtros_del_dialogo_en_cada_dialecto() {
        let fragment = |engine, op| build_filter_fragment(engine, "no`m\"bre", &op, "O'Brien");
        assert_eq!(
            fragment(Engine::Mysql, FilterOperator::Equals),
            "`no``m\"bre` = 'O''Brien'"
        );
        assert_eq!(
            fragment(Engine::Mysql, FilterOperator::Contains),
            "`no``m\"bre` LIKE '%O''Brien%'"
        );
        assert_eq!(
            fragment(Engine::Postgres, FilterOperator::GreaterOrEqual),
            "\"no`m\"\"bre\" >= 'O''Brien'"
        );
        // In PostgreSQL, case-insensitive and over any type
        assert_eq!(
            fragment(Engine::Postgres, FilterOperator::StartsWith),
            "\"no`m\"\"bre\"::text ILIKE 'O''Brien%'"
        );
    }

    #[test]
    fn crear_tabla_en_cada_dialecto() {
        let column = |name: &str, col_type: &str, length: &str, key: bool| NewColumnDef {
            name: name.into(),
            col_type: col_type.into(),
            length: length.into(),
            not_null: key,
            primary_key: key,
        };
        let columns = [
            column("id", "INT", "", true),
            column("nombre", "VARCHAR", "50", false),
        ];
        assert_eq!(
            build_create_table_sql(Engine::Mysql, "bd", "t", &columns),
            "CREATE TABLE `bd`.`t` (\n  `id` INT NOT NULL,\n  `nombre` VARCHAR(50),\n  \
             PRIMARY KEY (`id`)\n)"
        );
        // Types without a length ignore it even if the field has one
        let columns = [
            column("id", "SERIAL", "11", true),
            column("nombre", "VARCHAR", "50", false),
        ];
        assert_eq!(
            build_create_table_sql(Engine::Postgres, "bd", "ventas.t", &columns),
            "CREATE TABLE \"ventas\".\"t\" (\n  \"id\" SERIAL NOT NULL,\n  \"nombre\" VARCHAR(50),\n  \
             PRIMARY KEY (\"id\")\n)"
        );
    }
}

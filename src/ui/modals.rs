use eframe::egui;

// ── Tipos de datos para formularios ─────────────────────────────────────────

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

#[derive(Debug, Clone)]
pub enum DropKind {
    Database,
    Table,
}

#[derive(Debug, Clone)]
pub enum TruncateKind {
    Table,
    AllInDatabase,
}

// ── Estado de modal ──────────────────────────────────────────────────────────

#[derive(Debug, Default)]
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
        kind: DropKind,
        target: String,
        db: Option<String>,
        table: Option<String>,
    },
    ConfirmTruncate {
        kind: TruncateKind,
        target: String,
        db: Option<String>,
        table: Option<String>,
    },
}

// ── Acciones devueltas al caller ──────────────────────────────────────────────

pub enum ModalAction {
    None,
    CreateDatabase { name: String, charset: String },
    CreateTable { db: String, name: String, columns: Vec<NewColumnDef> },
    DropDatabase(String),
    DropTable { db: String, table: String },
    TruncateTable { db: String, table: String },
    TruncateAllTables(String),
    Cancel,
}

// ── Helpers ──────────────────────────────────────────────────────────────────

const CHARSETS: &[&str] = &["utf8mb4", "utf8", "latin1", "ascii", "binary"];
const COL_TYPES: &[&str] = &[
    "INT", "BIGINT", "SMALLINT", "TINYINT",
    "VARCHAR", "TEXT", "MEDIUMTEXT", "LONGTEXT", "CHAR",
    "DECIMAL", "FLOAT", "DOUBLE",
    "DATE", "DATETIME", "TIMESTAMP", "TIME",
    "BOOLEAN", "JSON", "BLOB",
];

fn red_btn(ui: &mut egui::Ui, text: &str) -> bool {
    ui.add(
        egui::Button::new(text)
            .fill(egui::Color32::from_rgb(180, 40, 40)),
    )
    .clicked()
}

// ── Función principal ─────────────────────────────────────────────────────────

/// Renderiza el modal activo sobre todo lo demás.
/// Debe llamarse desde `update()` después de todos los paneles, pasando `ctx`.
pub fn show(ctx: &egui::Context, modal: &mut ModalState) -> ModalAction {
    match modal {
        ModalState::None => ModalAction::None,
        ModalState::CreateDatabase { name, charset, error } => {
            show_create_database(ctx, name, charset, error)
        }
        ModalState::CreateTable { db, name, columns, error } => {
            show_create_table(ctx, db, name, columns, error)
        }
        ModalState::ConfirmDrop { kind, target, db, table } => {
            show_confirm_drop(ctx, kind, target, db, table)
        }
        ModalState::ConfirmTruncate { kind, target, db, table } => {
            show_confirm_truncate(ctx, kind, target, db, table)
        }
    }
}

// ── Formulario: crear BBDD ────────────────────────────────────────────────────

fn show_create_database(
    ctx: &egui::Context,
    name: &mut String,
    charset: &mut String,
    error: &mut Option<String>,
) -> ModalAction {
    let mut action = ModalAction::None;
    let mut open = true;

    egui::Window::new("Crear base de datos")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .open(&mut open)
        .show(ctx, |ui| {
            egui::Grid::new("create_db_grid")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label("Nombre:");
                    let resp = ui.add(
                        egui::TextEdit::singleline(name)
                            .desired_width(200.0)
                            .hint_text("mi_base_de_datos"),
                    );
                    if resp.lost_focus()
                        && ui.input(|i| i.key_pressed(egui::Key::Enter))
                    {
                        // handled below
                    }
                    ui.end_row();

                    ui.label("Charset:");
                    egui::ComboBox::from_id_salt("charset_combo")
                        .selected_text(charset.as_str())
                        .show_ui(ui, |ui| {
                            for &cs in CHARSETS {
                                ui.selectable_value(charset, cs.to_string(), cs);
                            }
                        });
                    ui.end_row();
                });

            if let Some(e) = error {
                ui.separator();
                ui.colored_label(egui::Color32::RED, format!("Error: {e}"));
            }

            ui.separator();
            ui.horizontal(|ui| {
                let create_clicked = ui.button("Crear").clicked();
                let enter_pressed = ui.input(|i| i.key_pressed(egui::Key::Enter));
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

// ── Formulario: crear tabla ───────────────────────────────────────────────────

fn show_create_table(
    ctx: &egui::Context,
    db: &mut String,
    name: &mut String,
    columns: &mut Vec<NewColumnDef>,
    error: &mut Option<String>,
) -> ModalAction {
    let mut action = ModalAction::None;
    let mut open = true;

    // Ensure at least one column row
    if columns.is_empty() {
        columns.push(NewColumnDef::new());
    }

    egui::Window::new("Crear tabla")
        .collapsible(false)
        .resizable(true)
        .min_width(600.0)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .open(&mut open)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Base de datos:");
                ui.strong(db.as_str());
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
                .column(Column::exact(160.0)) // nombre
                .column(Column::exact(110.0)) // tipo
                .column(Column::exact(70.0))  // longitud
                .column(Column::exact(60.0))  // not null
                .column(Column::exact(40.0))  // PK
                .column(Column::exact(30.0))  // borrar
                .header(20.0, |mut header| {
                    for h in &["Nombre", "Tipo", "Longitud", "Not Null", "PK", ""] {
                        header.col(|ui| { ui.strong(*h); });
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
                                    for &t in COL_TYPES {
                                        ui.selectable_value(
                                            &mut col.col_type,
                                            t.to_string(),
                                            t,
                                        );
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

            if ui.button("+ Añadir columna").clicked() {
                columns.push(NewColumnDef::new());
            }

            if let Some(e) = error {
                ui.separator();
                ui.colored_label(egui::Color32::RED, format!("Error: {e}"));
            }

            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Crear tabla").clicked()
                    && !name.trim().is_empty()
                    && !columns.is_empty()
                {
                    action = ModalAction::CreateTable {
                        db: db.clone(),
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

// ── Confirmación: eliminar ────────────────────────────────────────────────────

fn show_confirm_drop(
    ctx: &egui::Context,
    kind: &DropKind,
    target: &str,
    db: &mut Option<String>,
    table: &mut Option<String>,
) -> ModalAction {
    let mut action = ModalAction::None;
    let mut open = true;

    let kind_label = match kind {
        DropKind::Database => "base de datos",
        DropKind::Table => "tabla",
    };

    egui::Window::new(format!("Eliminar {kind_label}"))
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .open(&mut open)
        .show(ctx, |ui| {
            ui.label(format!("¿Seguro que quieres eliminar la {kind_label}:"));
            ui.add_space(4.0);
            ui.strong(target);
            ui.add_space(4.0);
            ui.colored_label(
                egui::Color32::from_rgb(220, 100, 50),
                "⚠ Esta acción no se puede deshacer.",
            );
            ui.separator();
            ui.horizontal(|ui| {
                if red_btn(ui, "Eliminar") {
                    action = match kind {
                        DropKind::Database => {
                            ModalAction::DropDatabase(db.clone().unwrap_or_default())
                        }
                        DropKind::Table => ModalAction::DropTable {
                            db: db.clone().unwrap_or_default(),
                            table: table.clone().unwrap_or_default(),
                        },
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

// ── Confirmación: truncar ─────────────────────────────────────────────────────

fn show_confirm_truncate(
    ctx: &egui::Context,
    kind: &TruncateKind,
    target: &str,
    db: &mut Option<String>,
    table: &mut Option<String>,
) -> ModalAction {
    let mut action = ModalAction::None;
    let mut open = true;

    let kind_label = match kind {
        TruncateKind::Table => "tabla",
        TruncateKind::AllInDatabase => "todas las tablas de",
    };

    egui::Window::new("Vaciar datos")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .open(&mut open)
        .show(ctx, |ui| {
            ui.label(format!("¿Seguro que quieres vaciar {kind_label}:"));
            ui.add_space(4.0);
            ui.strong(target);
            ui.add_space(4.0);
            ui.colored_label(
                egui::Color32::from_rgb(220, 100, 50),
                "⚠ Se eliminarán todos los datos. No se puede deshacer.",
            );
            ui.separator();
            ui.horizontal(|ui| {
                if red_btn(ui, "Vaciar") {
                    action = match kind {
                        TruncateKind::Table => ModalAction::TruncateTable {
                            db: db.clone().unwrap_or_default(),
                            table: table.clone().unwrap_or_default(),
                        },
                        TruncateKind::AllInDatabase => {
                            ModalAction::TruncateAllTables(db.clone().unwrap_or_default())
                        }
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

// ── Helpers para construir CREATE TABLE SQL ───────────────────────────────────

pub fn build_create_table_sql(db: &str, name: &str, columns: &[NewColumnDef]) -> String {
    let mut parts = Vec::new();
    let mut pks = Vec::new();

    for col in columns {
        if col.name.trim().is_empty() {
            continue;
        }
        let type_part = if col.length.trim().is_empty()
            || matches!(col.col_type.as_str(), "TEXT" | "MEDIUMTEXT" | "LONGTEXT" | "BLOB" | "JSON" | "BOOLEAN" | "DATE" | "DATETIME" | "TIMESTAMP" | "TIME")
        {
            col.col_type.clone()
        } else {
            format!("{}({})", col.col_type, col.length.trim())
        };

        let null_part = if col.not_null { " NOT NULL" } else { "" };
        parts.push(format!(
            "  `{}` {}{}",
            col.name.trim().replace('`', "``"),
            type_part,
            null_part
        ));

        if col.primary_key {
            pks.push(format!("`{}`", col.name.trim().replace('`', "``")));
        }
    }

    if !pks.is_empty() {
        parts.push(format!("  PRIMARY KEY ({})", pks.join(", ")));
    }

    format!(
        "CREATE TABLE `{}`.`{}` (\n{}\n)",
        db.replace('`', "``"),
        name.replace('`', "``"),
        parts.join(",\n")
    )
}

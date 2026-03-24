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
            Self::Equals       => "= igual a",
            Self::NotEquals    => "≠ distinto de",
            Self::Contains     => "LIKE '%…%' contiene",
            Self::StartsWith   => "LIKE '…%' empieza por",
            Self::EndsWith     => "LIKE '%…' termina en",
            Self::GreaterThan  => "> mayor que",
            Self::LessThan     => "< menor que",
            Self::GreaterOrEqual => ">= mayor o igual",
            Self::LessOrEqual  => "<= menor o igual",
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
#[allow(dead_code)]
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
    FilterInput {
        col_name: String,
        value: String,
        operator: FilterOperator,
    },
    /// Hay cambios sin guardar en la tabla actual y el usuario intenta cambiar a otra.
    ConfirmSwitchTable {
        new_db: String,
        new_table: String,
        pending_count: usize,
        current_table: String,
    },
    ConfirmDeleteRow {
        db: String,
        table: String,
        row_idx: usize,
        /// Descripción corta de la fila (valores PK) para mostrar al usuario.
        pk_desc: String,
    },
    CloneTable {
        db: String,
        table: String,
        new_name: String,
        copy_data: bool,
        error: Option<String>,
    },
    InsertRow {
        db: String,
        table: String,
        col_names: Vec<String>,
        values: Vec<String>,
        nulls: Vec<bool>,
        error: Option<String>,
    },
    Error {
        title: String,
        message: String,
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
    ApplyFilter { sql_fragment: String },
    Cancel,
    /// Guardar cambios y luego cambiar a la nueva tabla.
    ApplyAndSwitch { new_db: String, new_table: String },
    /// Descartar cambios y cambiar a la nueva tabla.
    DiscardAndSwitch { new_db: String, new_table: String },
    DeleteRow { db: String, table: String, row_idx: usize },
    InsertRow { db: String, table: String, col_names: Vec<String>, values: Vec<String>, nulls: Vec<bool> },
    CloneTable { db: String, src_table: String, new_name: String, copy_data: bool },
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
        ModalState::FilterInput { col_name, value, operator } => {
            show_filter_input(ctx, col_name, value, operator)
        }
        ModalState::ConfirmSwitchTable { new_db, new_table, pending_count, current_table } => {
            show_confirm_switch_table(ctx, new_db, new_table, *pending_count, current_table)
        }
        ModalState::ConfirmDeleteRow { db, table, row_idx, pk_desc } => {
            show_confirm_delete_row(ctx, db, table, *row_idx, pk_desc)
        }
        ModalState::InsertRow { db, table, col_names, values, nulls, error } => {
            show_insert_row(ctx, db, table, col_names, values, nulls, error)
        }
        ModalState::CloneTable { db, table, new_name, copy_data, error } => {
            show_clone_table(ctx, db, table, new_name, copy_data, error)
        }
        ModalState::Error { title, message } => {
            show_error(ctx, title, message)
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

// ── Filtro por texto ──────────────────────────────────────────────────────────

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
    col_name: &mut String,
    value: &mut String,
    operator: &mut FilterOperator,
) -> ModalAction {
    let mut action = ModalAction::None;
    let mut open = true;

    egui::Window::new(format!("Filtrar por {col_name}"))
        .collapsible(false)
        .resizable(false)
        .min_width(320.0)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .open(&mut open)
        .show(ctx, |ui| {
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
                    // Auto-foco en el campo de texto
                    resp.request_focus();
                    ui.end_row();
                });

            ui.separator();
            ui.horizontal(|ui| {
                let apply_clicked = ui.button("Aplicar").clicked();
                let enter_pressed = ui.input(|i| i.key_pressed(egui::Key::Enter));
                if apply_clicked || enter_pressed {
                    let frag = build_filter_fragment(col_name, operator, value);
                    action = ModalAction::ApplyFilter { sql_fragment: frag };
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

fn build_filter_fragment(col_name: &str, operator: &FilterOperator, value: &str) -> String {
    let esc_col = format!("`{}`", col_name.replace('`', "``"));
    let esc_val = value.replace('\\', "\\\\").replace('\'', "\\'");
    match operator {
        FilterOperator::Equals       => format!("{esc_col} = '{esc_val}'"),
        FilterOperator::NotEquals    => format!("{esc_col} != '{esc_val}'"),
        FilterOperator::Contains     => format!("{esc_col} LIKE '%{esc_val}%'"),
        FilterOperator::StartsWith   => format!("{esc_col} LIKE '{esc_val}%'"),
        FilterOperator::EndsWith     => format!("{esc_col} LIKE '%{esc_val}'"),
        FilterOperator::GreaterThan  => format!("{esc_col} > '{esc_val}'"),
        FilterOperator::LessThan     => format!("{esc_col} < '{esc_val}'"),
        FilterOperator::GreaterOrEqual => format!("{esc_col} >= '{esc_val}'"),
        FilterOperator::LessOrEqual  => format!("{esc_col} <= '{esc_val}'"),
    }
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

// ── Confirmación: cambiar tabla con cambios pendientes ────────────────────────

fn show_confirm_switch_table(
    ctx: &egui::Context,
    new_db: &str,
    new_table: &str,
    pending_count: usize,
    current_table: &str,
) -> ModalAction {
    let mut action = ModalAction::None;

    egui::Window::new("Cambios sin guardar")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.label(format!(
                "Tienes {} cambio(s) sin guardar en «{}».",
                pending_count, current_table
            ));
            ui.add_space(4.0);
            ui.label(format!("¿Qué quieres hacer antes de abrir «{}.{}»?", new_db, new_table));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button("💾  Guardar y cambiar").clicked() {
                    action = ModalAction::ApplyAndSwitch {
                        new_db: new_db.to_string(),
                        new_table: new_table.to_string(),
                    };
                }
                if ui
                    .add(egui::Button::new("🗑  Descartar y cambiar")
                        .fill(egui::Color32::from_rgb(180, 120, 40)))
                    .clicked()
                {
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

    action
}

// ── Confirmar eliminación de fila ─────────────────────────────────────────────

fn show_confirm_delete_row(
    ctx: &egui::Context,
    db: &str,
    table: &str,
    row_idx: usize,
    pk_desc: &str,
) -> ModalAction {
    let mut action = ModalAction::None;
    let mut open = true;

    egui::Window::new("Eliminar fila")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .open(&mut open)
        .show(ctx, |ui| {
            ui.label(format!("¿Eliminar esta fila de {db}.{table}?"));
            if !pk_desc.is_empty() {
                ui.monospace(pk_desc);
            }
            ui.label("Esta acción no se puede deshacer.");
            ui.separator();
            ui.horizontal(|ui| {
                if red_btn(ui, "🗑  Eliminar") {
                    action = ModalAction::DeleteRow {
                        db: db.to_string(),
                        table: table.to_string(),
                        row_idx,
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

// ── Insertar fila ─────────────────────────────────────────────────────────────

fn show_insert_row(
    ctx: &egui::Context,
    db: &str,
    table: &str,
    col_names: &[String],
    values: &mut Vec<String>,
    nulls: &mut Vec<bool>,
    error: &mut Option<String>,
) -> ModalAction {
    while values.len() < col_names.len() { values.push(String::new()); }
    while nulls.len() < col_names.len() { nulls.push(false); }

    let mut action = ModalAction::None;
    let mut open = true;

    egui::Window::new(format!("Insertar fila — {db}.{table}"))
        .collapsible(false)
        .resizable(true)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .open(&mut open)
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().max_height(400.0).show(ui, |ui| {
                egui::Grid::new("insert_row_grid")
                    .num_columns(3)
                    .spacing([8.0, 6.0])
                    .show(ui, |ui| {
                        ui.strong("Columna");
                        ui.strong("Valor");
                        ui.strong("NULL");
                        ui.end_row();

                        for i in 0..col_names.len() {
                            ui.label(&col_names[i]);
                            ui.add_enabled(
                                !nulls[i],
                                egui::TextEdit::singleline(&mut values[i]).desired_width(220.0),
                            );
                            ui.checkbox(&mut nulls[i], "");
                            if nulls[i] {
                                values[i].clear();
                            }
                            ui.end_row();
                        }
                    });
            });

            if let Some(err) = error {
                ui.colored_label(egui::Color32::RED, format!("Error: {err}"));
            }
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("➕  Insertar").clicked() {
                    action = ModalAction::InsertRow {
                        db: db.to_string(),
                        table: table.to_string(),
                        col_names: col_names.to_vec(),
                        values: values.clone(),
                        nulls: nulls.clone(),
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


// ── Clonar tabla ─────────────────────────────────────────────────────────────

fn show_clone_table(
    ctx: &egui::Context,
    db: &str,
    table: &str,
    new_name: &mut String,
    copy_data: &mut bool,
    error: &mut Option<String>,
) -> ModalAction {
    let mut action = ModalAction::None;
    let mut open = true;

    egui::Window::new(format!("Clonar tabla — {db}.{table}"))
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .open(&mut open)
        .show(ctx, |ui| {
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
                ui.colored_label(egui::Color32::RED, format!("Error: {err}"));
            }
            ui.separator();
            ui.horizontal(|ui| {
                let ok = ui.add_enabled(!new_name.is_empty(), egui::Button::new("🗒  Clonar")).clicked();
                if ok {
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
        });

    if !open {
        action = ModalAction::Cancel;
    }
    action
}

// ── Error genérico ────────────────────────────────────────────────────────────

fn show_error(ctx: &egui::Context, title: &str, message: &str) -> ModalAction {
    let mut open = true;
    let mut close = false;

    egui::Window::new(title)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .open(&mut open)
        .show(ctx, |ui| {
            ui.colored_label(egui::Color32::RED, message);
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

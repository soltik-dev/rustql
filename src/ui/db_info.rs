use eframe::egui;
use egui_extras::{Column, TableBuilder};

use crate::db::TableInfo;
use crate::ui::{format_bytes, theme};

/// Returned by db_info::show when the user clicks a table.
#[derive(Default)]
pub enum DbInfoAction {
    #[default]
    None,
    OpenTable(String),
}

/// "Database" panel: list of tables with statistics of the selected database.
pub fn show(ui: &mut egui::Ui, db: &str, tables: &[TableInfo]) -> DbInfoAction {
    let mut action = DbInfoAction::None;

    ui.horizontal(|ui| {
        ui.strong(format!("Base de datos: {db}"));
        ui.label(
            egui::RichText::new(format!("— {} tablas", tables.len()))
                .color(theme::get(ui).text_muted),
        );
    });
    ui.separator();

    if tables.is_empty() {
        // Already loaded and with nothing to list (while loading it never gets here)
        ui.label(
            egui::RichText::new("Esta base de datos no tiene tablas.")
                .color(theme::get(ui).text_muted),
        );
        return action;
    }

    let mut clicked_table: Option<String> = None;

    // Engine and collation only if the server has them per table (MySQL)
    let has_engine = tables.iter().any(|t| !t.engine.is_empty());
    let has_collation = tables.iter().any(|t| !t.collation.is_empty());
    let mut headers = vec!["Tabla"];
    let mut builder = TableBuilder::new(ui)
        .striped(true)
        .resizable(true)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(Column::auto().at_least(150.0).clip(true));
    if has_engine {
        headers.push("Motor");
        builder = builder.column(Column::auto().at_least(70.0));
    }
    headers.extend(["Filas", "Tamaño"]);
    builder = builder
        .column(Column::auto().at_least(80.0))
        .column(Column::auto().at_least(80.0));
    if has_collation {
        headers.push("Collation");
        builder = builder.column(Column::remainder().at_least(140.0).clip(true));
    }

    builder
        .header(24.0, |mut header| {
            for name in headers {
                header.col(|ui| {
                    ui.label(egui::RichText::new(name).color(theme::get(ui).text_muted));
                });
            }
        })
        .body(|body| {
            body.rows(20.0, tables.len(), |mut row| {
                let t = &tables[row.index()];
                row.col(|ui| {
                    let resp = ui.add(
                        egui::Label::new(&t.name)
                            .truncate()
                            .sense(egui::Sense::click()),
                    );
                    if resp.double_clicked() {
                        clicked_table = Some(t.name.clone());
                    }
                    resp.on_hover_text("Doble click para abrir tabla");
                });
                if has_engine {
                    row.col(|ui| {
                        ui.label(&t.engine);
                    });
                }
                row.col(|ui| {
                    ui.label(t.row_count.to_string());
                });
                row.col(|ui| {
                    ui.label(format_bytes(t.size_bytes));
                });
                if has_collation {
                    row.col(|ui| {
                        ui.add(egui::Label::new(&t.collation).truncate());
                    });
                }
            });
        });

    if let Some(table) = clicked_table {
        action = DbInfoAction::OpenTable(table);
    }

    action
}

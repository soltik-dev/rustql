use eframe::egui;
use egui_extras::{Column, TableBuilder};

use crate::db::TableInfo;
use crate::ui::theme;

fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / 1024.0 / 1024.0)
    } else {
        format!("{:.1} GB", bytes as f64 / 1024.0 / 1024.0 / 1024.0)
    }
}

/// Retornado por db_info::show cuando el usuario hace click en una tabla.
pub enum DbInfoAction {
    None,
    OpenTable(String),
}

/// Panel "BBDD": lista de tablas con estadísticas de la base de datos seleccionada.
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
        ui.centered_and_justified(|ui| {
            ui.spinner();
        });
        return action;
    }

    let mut clicked_table: Option<String> = None;

    TableBuilder::new(ui)
        .striped(true)
        .resizable(true)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(Column::auto().at_least(150.0).clip(true)) // tabla
        .column(Column::auto().at_least(70.0)) // motor
        .column(Column::auto().at_least(80.0)) // filas
        .column(Column::auto().at_least(80.0)) // tamaño
        .column(Column::remainder().at_least(140.0).clip(true)) // collation
        .header(24.0, |mut header| {
            for name in &["Tabla", "Motor", "Filas", "Tamaño", "Collation"] {
                header.col(|ui| {
                    ui.label(egui::RichText::new(*name).color(theme::get(ui).text_muted));
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
                row.col(|ui| {
                    ui.label(&t.engine);
                });
                row.col(|ui| {
                    ui.label(t.row_count.to_string());
                });
                row.col(|ui| {
                    ui.label(format_bytes(t.size_bytes));
                });
                row.col(|ui| {
                    ui.add(egui::Label::new(&t.collation).truncate());
                });
            });
        });

    if let Some(table) = clicked_table {
        action = DbInfoAction::OpenTable(table);
    }

    action
}

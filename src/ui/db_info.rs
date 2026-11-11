use eframe::egui;
use egui_extras::{Column, TableBuilder};

use crate::db::TableInfo;
use crate::ui::{format_bytes, theme};

/// Retornado por db_info::show cuando el usuario hace click en una tabla.
#[derive(Default)]
pub enum DbInfoAction {
    #[default]
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
        // Ya cargado y sin nada que listar (mientras carga no se llega aquí)
        ui.label(
            egui::RichText::new("Esta base de datos no tiene tablas.")
                .color(theme::get(ui).text_muted),
        );
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

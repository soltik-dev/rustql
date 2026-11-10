use eframe::egui;
use egui_extras::{Column, TableBuilder};

use crate::db::DatabaseInfo;
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

/// Panel "Servidor": lista de bases de datos con estadísticas.
pub fn show(ui: &mut egui::Ui, host: &str, databases: &[DatabaseInfo]) {
    ui.horizontal(|ui| {
        ui.strong(format!("Servidor: {host}"));
        ui.label(
            egui::RichText::new(format!("— {} bases de datos", databases.len()))
                .color(theme::get(ui).text_muted),
        );
    });
    ui.separator();

    if databases.is_empty() {
        // Ya cargado y sin nada que listar (mientras carga no se llega aquí)
        ui.label(egui::RichText::new("(sin bases de datos)").color(theme::get(ui).text_muted));
        return;
    }

    TableBuilder::new(ui)
        .striped(true)
        .resizable(true)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(Column::auto().at_least(150.0).clip(true)) // nombre
        .column(Column::auto().at_least(90.0)) // charset
        .column(Column::auto().at_least(160.0)) // collation
        .column(Column::auto().at_least(60.0)) // tablas
        .column(Column::auto().at_least(80.0)) // tamaño
        .header(24.0, |mut header| {
            for name in &["Base de datos", "Charset", "Collation", "Tablas", "Tamaño"] {
                header.col(|ui| {
                    ui.label(egui::RichText::new(*name).color(theme::get(ui).text_muted));
                });
            }
        })
        .body(|body| {
            body.rows(20.0, databases.len(), |mut row| {
                let db = &databases[row.index()];
                row.col(|ui| {
                    ui.add(egui::Label::new(&db.name).truncate());
                });
                row.col(|ui| {
                    ui.label(&db.charset);
                });
                row.col(|ui| {
                    ui.add(egui::Label::new(&db.collation).truncate());
                });
                row.col(|ui| {
                    ui.label(db.table_count.to_string());
                });
                row.col(|ui| {
                    ui.label(format_bytes(db.size_bytes));
                });
            });
        });
}

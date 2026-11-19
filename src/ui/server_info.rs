use eframe::egui;
use egui_extras::{Column, TableBuilder};

use crate::db::DatabaseInfo;
use crate::ui::{format_bytes, theme};

/// "Server" panel: list of databases with statistics. `known_tables` says
/// how many tables a database has if they have already been read (when
/// expanding it in the tree): it completes the count the server could
/// not give.
pub fn show(
    ui: &mut egui::Ui,
    host: &str,
    databases: &[DatabaseInfo],
    known_tables: impl Fn(&str) -> Option<usize>,
) {
    ui.horizontal(|ui| {
        ui.strong(format!("Servidor: {host}"));
        ui.label(
            egui::RichText::new(match databases.len() {
                1 => "— 1 base de datos".to_string(),
                n => format!("— {n} bases de datos"),
            })
            .color(theme::get(ui).text_muted),
        );
    });
    ui.separator();

    if databases.is_empty() {
        // Already loaded and with nothing to list (while loading it never gets here)
        ui.label(egui::RichText::new("(sin bases de datos)").color(theme::get(ui).text_muted));
        return;
    }

    TableBuilder::new(ui)
        .striped(true)
        .resizable(true)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(Column::auto().at_least(150.0).clip(true)) // name
        .column(Column::auto().at_least(90.0)) // charset
        .column(Column::auto().at_least(160.0)) // collation
        .column(Column::auto().at_least(60.0)) // tables
        .column(Column::auto().at_least(80.0)) // size
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
                    // Some engines cannot count them without entering the database
                    match db.table_count.or_else(|| known_tables(&db.name)) {
                        Some(count) => ui.label(count.to_string()),
                        None => ui.label("—"),
                    };
                });
                row.col(|ui| {
                    ui.label(format_bytes(db.size_bytes));
                });
            });
        });
}

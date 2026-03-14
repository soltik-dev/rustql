use eframe::egui;
use egui_extras::{Column, TableBuilder};

use crate::db::ColumnInfo;

/// Panel "Tabla": lista de columnas con información de estructura.
pub fn show(ui: &mut egui::Ui, db: &str, table: &str, columns: &[ColumnInfo]) {
    ui.horizontal(|ui| {
        ui.strong(format!("{db}.{table}"));
        ui.label(format!("— {} columnas", columns.len()));
    });
    ui.separator();

    if columns.is_empty() {
        ui.centered_and_justified(|ui| {
            ui.spinner();
        });
        return;
    }

    TableBuilder::new(ui)
        .striped(true)
        .resizable(true)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(Column::auto().at_least(120.0).clip(true)) // nombre
        .column(Column::auto().at_least(120.0).clip(true)) // tipo
        .column(Column::auto().at_least(60.0))  // nullable
        .column(Column::auto().at_least(100.0).clip(true)) // default
        .column(Column::auto().at_least(50.0))  // clave
        .column(Column::remainder().at_least(100.0).clip(true)) // extra
        .header(24.0, |mut header| {
            for name in &["Columna", "Tipo", "Nullable", "Default", "Clave", "Extra"] {
                header.col(|ui| {
                    ui.strong(*name);
                });
            }
        })
        .body(|body| {
            body.rows(20.0, columns.len(), |mut row| {
                let c = &columns[row.index()];

                row.col(|ui| {
                    let text = if c.column_key == "PRI" {
                        egui::RichText::new(&c.name).strong()
                    } else {
                        egui::RichText::new(&c.name)
                    };
                    ui.add(egui::Label::new(text).truncate());
                });

                row.col(|ui| {
                    ui.add(egui::Label::new(&c.col_type).truncate());
                });

                row.col(|ui| {
                    if c.is_nullable {
                        ui.label("YES");
                    } else {
                        ui.colored_label(egui::Color32::from_rgb(180, 80, 80), "NO");
                    }
                });

                row.col(|ui| {
                    let default_text = c
                        .default_val
                        .as_deref()
                        .unwrap_or("NULL");
                    ui.add(egui::Label::new(default_text).truncate());
                });

                row.col(|ui| {
                    let (text, color) = match c.column_key.as_str() {
                        "PRI" => ("PK", egui::Color32::from_rgb(100, 150, 220)),
                        "UNI" => ("UNI", egui::Color32::from_rgb(100, 200, 100)),
                        "MUL" => ("IDX", egui::Color32::from_rgb(180, 140, 80)),
                        _ => ("", egui::Color32::TRANSPARENT),
                    };
                    if !text.is_empty() {
                        ui.colored_label(color, text);
                    }
                });

                row.col(|ui| {
                    ui.add(egui::Label::new(&c.extra).truncate());
                });
            });
        });
}

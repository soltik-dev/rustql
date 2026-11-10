use eframe::egui;
use egui_extras::{Column, TableBuilder};

use crate::db::ColumnInfo;
use crate::ui::theme;

/// Panel "Tabla": lista de columnas con información de estructura.
pub fn show(ui: &mut egui::Ui, db: &str, table: &str, columns: &[ColumnInfo]) {
    ui.horizontal(|ui| {
        ui.strong(format!("{db}.{table}"));
        ui.label(
            egui::RichText::new(format!("— {} columnas", columns.len()))
                .color(theme::get(ui).text_muted),
        );
    });
    ui.separator();

    if columns.is_empty() {
        // Ya cargado y sin nada que listar (mientras carga no se llega aquí)
        ui.label(egui::RichText::new("(sin columnas)").color(theme::get(ui).text_muted));
        return;
    }

    TableBuilder::new(ui)
        .striped(true)
        .resizable(true)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(Column::auto().at_least(120.0).clip(true)) // nombre
        .column(Column::auto().at_least(120.0).clip(true)) // tipo
        .column(Column::auto().at_least(60.0)) // nullable
        .column(Column::auto().at_least(100.0).clip(true)) // default
        .column(Column::auto().at_least(50.0)) // clave
        .column(Column::remainder().at_least(100.0).clip(true)) // extra
        .header(24.0, |mut header| {
            for name in &["Columna", "Tipo", "Nullable", "Default", "Clave", "Extra"] {
                header.col(|ui| {
                    ui.label(egui::RichText::new(*name).color(theme::get(ui).text_muted));
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
                        ui.colored_label(theme::get(ui).danger, "NO");
                    }
                });

                row.col(|ui| {
                    let default_text = c.default_val.as_deref().unwrap_or("NULL");
                    ui.add(egui::Label::new(default_text).truncate());
                });

                row.col(|ui| {
                    let p = theme::get(ui);
                    let (text, color) = match c.column_key.as_str() {
                        "PRI" => ("PK", p.badge_pk),
                        "UNI" => ("UNI", p.badge_uni),
                        "MUL" => ("IDX", p.badge_idx),
                        _ => ("", egui::Color32::TRANSPARENT),
                    };
                    if !text.is_empty() {
                        theme::badge(ui, text, color);
                    }
                });

                row.col(|ui| {
                    ui.add(egui::Label::new(&c.extra).truncate());
                });
            });
        });
}

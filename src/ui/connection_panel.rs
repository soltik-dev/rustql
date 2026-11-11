use eframe::egui;

use crate::db::ConnectionParams;
use crate::loadable::Loadable;
use crate::profiles::{ConnColor, ConnectionProfile};
use crate::secrets;
use crate::ui::icons::DATABASE;
use crate::ui::theme;

/// Cómo guardar el formulario como favorito.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SaveProfile {
    /// Sobre el favorito seleccionado (o uno nuevo si no hay ninguno).
    Update,
    /// Como otro favorito, dejando intacto el seleccionado.
    AsNew,
}

/// Resultado de la interacción con el formulario de conexión.
#[derive(Default)]
pub struct FormResult {
    pub connect: bool,
    pub save_profile: Option<SaveProfile>,
    pub delete_profile: bool,
    /// Se abrió la lista de BBDD: hay que leerla del servidor.
    pub load_databases: bool,
}

/// Panel izquierdo: lista de favoritos guardados.
/// Retorna (índice del perfil seleccionado, true si fue doble-click para conectar).
pub fn show_favorites(
    ui: &mut egui::Ui,
    profiles: &[ConnectionProfile],
    selected_idx: &Option<usize>,
) -> (Option<usize>, bool) {
    let mut clicked = None;
    let mut connect_now = false;

    ui.add_space(8.0);
    theme::section_label(ui, "Favoritos");
    ui.separator();

    if profiles.is_empty() {
        ui.add_space(8.0);
        ui.label("Sin conexiones guardadas.");
        ui.add_space(4.0);
        ui.label("Rellena el formulario\ny pulsa Guardar.");
    } else {
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (i, profile) in profiles.iter().enumerate() {
                let is_selected = selected_idx.is_some_and(|s| s == i);
                let resp = ui
                    .horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        color_dot(ui, profile.color);
                        ui.selectable_label(is_selected, &profile.name)
                    })
                    .inner;
                if resp.clicked() {
                    clicked = Some(i);
                }
                if resp.double_clicked() {
                    clicked = Some(i);
                    connect_now = true;
                }
            }
        });
    }

    (clicked, connect_now)
}

/// Punto con el color del perfil; anillo apagado si no tiene color.
fn color_dot(ui: &mut egui::Ui, color: Option<ConnColor>) {
    let p = theme::get(ui);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
    match color {
        Some(c) => {
            ui.painter().circle_filled(rect.center(), 4.0, p.conn(c));
        }
        None => {
            ui.painter()
                .circle_stroke(rect.center(), 3.5, egui::Stroke::new(1.0, p.text_disabled));
        }
    }
}

/// Lo que enseña y edita el formulario de conexión.
pub struct FormView<'a> {
    pub params: &'a mut ConnectionParams,
    pub port_str: &'a mut String,
    pub error_message: &'a Option<String>,
    pub profile_name: &'a mut String,
    pub save_password: &'a mut bool,
    pub color: &'a mut Option<ConnColor>,
    pub selected_idx: Option<usize>,
    /// BBDD del servidor, leídas al pulsar el botón de elegir.
    pub databases: &'a Loadable<Vec<String>>,
}

/// Panel derecho: formulario de conexión + botones de gestión.
pub fn show_form(ui: &mut egui::Ui, view: FormView<'_>) -> FormResult {
    let mut result = FormResult::default();
    let p = theme::get(ui);

    ui.add_space(32.0);
    ui.vertical_centered(|ui| {
        ui.heading("RustQL");
        ui.label(egui::RichText::new("Conectar a base de datos").color(p.text_muted));
    });
    ui.add_space(20.0);

    ui.vertical_centered(|ui| {
        ui.set_max_width(420.0);
        egui::Frame::NONE
            .fill(p.bg_raised)
            .stroke(egui::Stroke::new(1.0, p.border_subtle))
            .corner_radius(egui::CornerRadius::same(theme::RADIUS_LG))
            .inner_margin(egui::Margin::same(20))
            .show(ui, |ui| show_form_fields(ui, view, &mut result));
    });

    result
}

fn show_form_fields(ui: &mut egui::Ui, view: FormView<'_>, result: &mut FormResult) {
    let FormView {
        params,
        port_str,
        error_message,
        profile_name,
        save_password,
        color,
        selected_idx,
        databases,
    } = view;
    let p = theme::get(ui);

    egui::Grid::new("connection_form")
        .num_columns(2)
        .spacing([16.0, 8.0])
        .min_col_width(70.0)
        .show(ui, |ui| {
            // Enter en cualquiera de los campos de la conexión conecta
            let mut connect = false;
            let mut field_width = FIELD_WIDTH;
            let mut field = |ui: &mut egui::Ui, edit: egui::TextEdit<'_>| {
                let resp = ui.add(edit.min_size(egui::vec2(FIELD_WIDTH, 0.0)));
                field_width = resp.rect.width();
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    connect = true;
                }
            };

            ui.label("Host:");
            field(ui, egui::TextEdit::singleline(&mut params.host));
            ui.end_row();

            ui.label("Puerto:");
            field(ui, egui::TextEdit::singleline(port_str));
            ui.end_row();

            ui.label("Usuario:");
            field(ui, egui::TextEdit::singleline(&mut params.user));
            ui.end_row();

            ui.label("Contraseña:");
            let hint = if params.keyring_id.is_some() {
                "Guardada en el llavero"
            } else {
                ""
            };
            field(
                ui,
                egui::TextEdit::singleline(&mut params.password)
                    .password(true)
                    .hint_text(hint),
            );
            ui.end_row();

            // El campo y su botón de elegir ocupan lo que los demás campos
            ui.label("Base de datos:");
            let row = egui::vec2(field_width, ui.spacing().interact_size.y);
            let layout = egui::Layout::right_to_left(egui::Align::Center);
            ui.allocate_ui_with_layout(row, layout, |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                result.load_databases = database_picker(ui, &mut params.database, databases);
                let edit = egui::TextEdit::singleline(&mut params.database)
                    .hint_text("Todas")
                    .desired_width(ui.available_width());
                let resp = ui.add(edit);
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    connect = true;
                }
            });
            ui.end_row();
            result.connect |= connect;

            ui.label("");
            ui.vertical(|ui| {
                if secrets::available() == Some(false) {
                    *save_password = false;
                    ui.add_enabled(
                        false,
                        egui::Checkbox::new(save_password, "Guardar contraseña"),
                    );
                    ui.colored_label(p.warning, "⚠ Sin llavero del sistema: no se guardará");
                } else {
                    ui.checkbox(save_password, "Guardar contraseña");
                    if *save_password {
                        ui.label(
                            egui::RichText::new("Se guarda cifrada en el llavero del sistema")
                                .color(p.text_muted)
                                .small(),
                        );
                    }
                }
            });
            ui.end_row();
        });

    ui.add_space(16.0);
    ui.separator();
    ui.add_space(8.0);

    // Sección guardar/eliminar favorito
    egui::Grid::new("profile_form")
        .num_columns(2)
        .spacing([16.0, 8.0])
        .min_col_width(70.0)
        .show(ui, |ui| {
            ui.label("Nombre:");
            ui.add(
                egui::TextEdit::singleline(profile_name)
                    .hint_text("Mi servidor...")
                    .min_size(egui::vec2(220.0, 0.0)),
            );
            ui.end_row();

            ui.label("Color:");
            color_swatches(ui, color);
            ui.end_row();
        });

    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if ui.button("Guardar").clicked() {
            result.save_profile = Some(SaveProfile::Update);
        }
        if selected_idx.is_some() {
            if ui
                .button("Guardar como nuevo")
                .on_hover_text("Crea otro favorito con estos datos, sin tocar el seleccionado")
                .clicked()
            {
                result.save_profile = Some(SaveProfile::AsNew);
            }
            if ui.button("Eliminar").clicked() {
                result.delete_profile = true;
            }
        }
    });

    ui.add_space(16.0);
    ui.separator();
    ui.add_space(12.0);

    if let Some(err) = error_message {
        theme::error_card(ui, err);
        ui.add_space(8.0);
    }

    ui.vertical_centered(|ui| {
        ui.spacing_mut().button_padding = egui::vec2(48.0, 8.0);
        if theme::primary_button(ui, "Conectar").clicked() {
            result.connect = true;
        }
    });
}

/// Ancho mínimo de los campos del formulario.
const FIELD_WIDTH: f32 = 220.0;

/// Botón que despliega las BBDD del servidor para elegir una. Devuelve true
/// al abrirse: hay que leer la lista (con las credenciales del formulario).
fn database_picker(
    ui: &mut egui::Ui,
    database: &mut String,
    databases: &Loadable<Vec<String>>,
) -> bool {
    let p = theme::get(ui);
    let resp = ui
        .button(DATABASE)
        .on_hover_text("Elegir entre las bases de datos del servidor");
    let popup = egui::Popup::menu(&resp)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .width(FIELD_WIDTH);
    // El clic aún no se ha aplicado: si estaba cerrado, es que se abre
    let opened = resp.clicked() && !popup.is_open();
    // El desplegable recuerda su tamaño mientras está abierto: al cambiar lo
    // que enseña (del «leyendo…» a la lista) hay que volver a medirlo, o la
    // lista se queda con la altura de una línea
    let stage = std::mem::discriminant(databases);
    let stage_id = resp.id.with("stage");
    let changed = ui.data_mut(|d| {
        let previous = d.get_temp(stage_id);
        d.insert_temp(stage_id, stage);
        previous != Some(stage)
    });
    popup.sizing_pass(changed && !opened).show(|ui| {
        ui.set_min_width(FIELD_WIDTH);
        match databases {
            // Recién abierto: la lista se pide en este mismo frame
            Loadable::NotLoaded | Loadable::Loading(_) => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Leyendo las bases de datos…");
                });
            }
            Loadable::Failed(e) => {
                ui.set_max_width(FIELD_WIDTH + 80.0);
                ui.add(egui::Label::new(egui::RichText::new(e).color(p.danger)).wrap());
            }
            Loadable::Ready(list) => {
                if ui
                    .selectable_label(database.trim().is_empty(), "Todas")
                    .clicked()
                {
                    database.clear();
                    ui.close();
                }
                ui.separator();
                egui::ScrollArea::vertical()
                    .max_height(260.0)
                    .show(ui, |ui| {
                        for name in list {
                            if ui.selectable_label(database.trim() == name, name).clicked() {
                                *database = name.clone();
                                ui.close();
                            }
                        }
                    });
            }
        }
    });
    opened
}

/// Muestras para elegir el color de la conexión: «sin color», los presets de
/// la paleta y uno personalizado que abre el selector libre.
fn color_swatches(ui: &mut egui::Ui, color: &mut Option<ConnColor>) {
    let p = theme::get(ui);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;

        if swatch(ui, None, color.is_none(), None)
            .on_hover_text("Sin color")
            .clicked()
        {
            *color = None;
        }
        for preset in ConnColor::PRESETS {
            if swatch(ui, Some(p.conn(preset)), *color == Some(preset), None)
                .on_hover_text(preset.label())
                .clicked()
            {
                *color = Some(preset);
            }
        }

        let custom = color.filter(|c| matches!(c, ConnColor::Custom(_)));
        let glyph = if custom.is_some() { None } else { Some("+") };
        let resp = swatch(ui, custom.map(|c| p.conn(c)), custom.is_some(), glyph)
            .on_hover_text("Personalizado…");
        egui::Popup::menu(&resp)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                // Parte del color actual para que el selector no arranque en negro
                let mut c = p.conn(color.unwrap_or(ConnColor::Blue));
                if egui::color_picker::color_picker_color32(
                    ui,
                    &mut c,
                    egui::color_picker::Alpha::Opaque,
                ) {
                    *color = Some(ConnColor::Custom([c.r(), c.g(), c.b()]));
                }
            });
    });
}

/// Muestra cuadrada clicable. Sin `fill` se dibuja hueca: con `glyph` encima
/// (personalizado) o con una diagonal («sin color»).
fn swatch(
    ui: &mut egui::Ui,
    fill: Option<egui::Color32>,
    selected: bool,
    glyph: Option<&str>,
) -> egui::Response {
    let p = theme::get(ui);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::click());
    let inner = rect.shrink(4.0);
    let painter = ui.painter();

    match fill {
        Some(c) => {
            painter.rect_filled(inner, theme::RADIUS_SM, c);
        }
        None => {
            painter.rect_stroke(
                inner,
                theme::RADIUS_SM,
                egui::Stroke::new(1.0, p.border_strong),
                egui::StrokeKind::Inside,
            );
            match glyph {
                Some(g) => {
                    painter.text(
                        inner.center(),
                        egui::Align2::CENTER_CENTER,
                        g,
                        egui::FontId::proportional(12.0),
                        p.text_muted,
                    );
                }
                None => {
                    painter.line_segment(
                        [inner.left_bottom(), inner.right_top()],
                        egui::Stroke::new(1.0, p.text_disabled),
                    );
                }
            }
        }
    }

    let ring = if selected {
        Some(egui::Stroke::new(1.5, p.text_primary))
    } else if resp.hovered() {
        Some(egui::Stroke::new(1.0, p.border_strong))
    } else {
        None
    };
    if let Some(stroke) = ring {
        painter.rect_stroke(
            rect.shrink(1.0),
            theme::RADIUS_MD,
            stroke,
            egui::StrokeKind::Inside,
        );
    }
    resp
}

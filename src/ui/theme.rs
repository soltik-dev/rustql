//! Tema visual central: paleta semántica con variantes clara/oscura,
//! construcción del Style de egui y helpers de widgets compartidos.

use eframe::egui::{self, Color32, CornerRadius, Shadow, Stroke};

use crate::profiles::ConnColor;

// Radios de esquina (egui 0.35 usa u8)
pub const RADIUS_SM: u8 = 4;
pub const RADIUS_MD: u8 = 6;
pub const RADIUS_LG: u8 = 8;

/// Familia registrada en setup_fonts para etiquetas de sección (Inter Medium).
pub const MEDIUM_FAMILY: &str = "InterMedium";

/// Paleta semántica: cada campo describe un uso, no un color concreto.
pub struct Palette {
    // Fondos (niveles de superficie, de más hundido a más elevado)
    pub bg_sunken: Color32,
    pub bg_base: Color32,
    pub bg_surface: Color32,
    pub bg_raised: Color32,
    // Bordes
    pub border_subtle: Color32,
    pub border_strong: Color32,
    // Texto
    pub text_primary: Color32,
    pub text_muted: Color32,
    pub text_disabled: Color32,
    // Acento (azul)
    pub accent: Color32,
    pub accent_hover: Color32,
    pub accent_bg: Color32,
    pub on_accent: Color32,
    // Semánticos
    pub success: Color32,
    pub warning: Color32,
    pub danger: Color32,
    pub danger_hover: Color32,
    // Grid de resultados (fondos opacos: se pintan sobre stripes)
    pub grid_stripe: Color32,
    pub grid_header_bg: Color32,
    pub grid_sel_row_bg: Color32,
    pub grid_edit_bg: Color32,
    pub grid_edit_fg: Color32,
    pub grid_insert_bg: Color32,
    pub grid_insert_fg: Color32,
    pub grid_delete_bg: Color32,
    pub grid_delete_fg: Color32,
    pub null_text: Color32,
    // Tokens SQL
    pub sql_keyword: Color32,
    pub sql_string: Color32,
    pub sql_number: Color32,
    pub sql_comment: Color32,
    pub sql_operator: Color32,
    // Varios
    pub fav_star: Color32,
    pub badge_pk: Color32,
    pub badge_uni: Color32,
    pub badge_idx: Color32,
    // Colores de conexión, en el orden de ConnColor::PRESETS
    pub conn_presets: [Color32; 8],
}

pub const DARK: Palette = Palette {
    bg_sunken: Color32::from_rgb(13, 17, 26),
    bg_base: Color32::from_rgb(19, 24, 36),
    bg_surface: Color32::from_rgb(27, 34, 49),
    bg_raised: Color32::from_rgb(31, 39, 56),
    border_subtle: Color32::from_rgb(42, 51, 72),
    border_strong: Color32::from_rgb(61, 74, 102),
    text_primary: Color32::from_rgb(221, 227, 238),
    text_muted: Color32::from_rgb(138, 148, 171),
    text_disabled: Color32::from_rgb(90, 100, 120),
    accent: Color32::from_rgb(76, 141, 255),
    accent_hover: Color32::from_rgb(107, 161, 255),
    accent_bg: Color32::from_rgba_premultiplied(38, 70, 128, 128),
    on_accent: Color32::WHITE,
    success: Color32::from_rgb(63, 185, 104),
    warning: Color32::from_rgb(229, 165, 58),
    danger: Color32::from_rgb(229, 83, 75),
    danger_hover: Color32::from_rgb(244, 110, 102),
    grid_stripe: Color32::from_rgb(22, 28, 41),
    grid_header_bg: Color32::from_rgb(27, 34, 49),
    grid_sel_row_bg: Color32::from_rgb(33, 51, 82),
    grid_edit_bg: Color32::from_rgb(66, 53, 24),
    grid_edit_fg: Color32::from_rgb(240, 200, 110),
    grid_insert_bg: Color32::from_rgb(24, 54, 36),
    grid_insert_fg: Color32::from_rgb(126, 226, 168),
    grid_delete_bg: Color32::from_rgb(62, 28, 30),
    grid_delete_fg: Color32::from_rgb(200, 120, 116),
    null_text: Color32::from_rgb(95, 107, 130),
    sql_keyword: Color32::from_rgb(86, 156, 214),
    sql_string: Color32::from_rgb(206, 145, 120),
    sql_number: Color32::from_rgb(181, 206, 168),
    sql_comment: Color32::from_rgb(106, 153, 85),
    sql_operator: Color32::from_rgb(190, 200, 216),
    fav_star: Color32::from_rgb(222, 179, 62),
    badge_pk: Color32::from_rgb(100, 150, 220),
    badge_uni: Color32::from_rgb(100, 200, 100),
    badge_idx: Color32::from_rgb(180, 140, 80),
    conn_presets: [
        Color32::from_rgb(76, 141, 255),  // azul
        Color32::from_rgb(45, 186, 186),  // turquesa
        Color32::from_rgb(63, 185, 104),  // verde
        Color32::from_rgb(222, 190, 60),  // amarillo
        Color32::from_rgb(240, 136, 62),  // naranja
        Color32::from_rgb(229, 83, 75),   // rojo
        Color32::from_rgb(232, 106, 168), // rosa
        Color32::from_rgb(163, 113, 247), // morado
    ],
};

pub const LIGHT: Palette = Palette {
    bg_sunken: Color32::WHITE,
    bg_base: Color32::from_rgb(244, 246, 249),
    bg_surface: Color32::from_rgb(233, 237, 243),
    bg_raised: Color32::WHITE,
    border_subtle: Color32::from_rgb(213, 219, 229),
    border_strong: Color32::from_rgb(179, 188, 204),
    text_primary: Color32::from_rgb(32, 38, 52),
    text_muted: Color32::from_rgb(102, 112, 132),
    text_disabled: Color32::from_rgb(154, 164, 181),
    accent: Color32::from_rgb(31, 111, 235),
    accent_hover: Color32::from_rgb(24, 90, 188),
    accent_bg: Color32::from_rgba_premultiplied(200, 217, 245, 245),
    on_accent: Color32::WHITE,
    success: Color32::from_rgb(26, 127, 55),
    warning: Color32::from_rgb(154, 103, 0),
    danger: Color32::from_rgb(207, 34, 46),
    danger_hover: Color32::from_rgb(164, 22, 35),
    grid_stripe: Color32::from_rgb(238, 241, 246),
    grid_header_bg: Color32::from_rgb(233, 237, 243),
    grid_sel_row_bg: Color32::from_rgb(208, 225, 250),
    grid_edit_bg: Color32::from_rgb(255, 240, 200),
    grid_edit_fg: Color32::from_rgb(120, 84, 0),
    grid_insert_bg: Color32::from_rgb(214, 242, 224),
    grid_insert_fg: Color32::from_rgb(15, 90, 40),
    grid_delete_bg: Color32::from_rgb(250, 222, 222),
    grid_delete_fg: Color32::from_rgb(160, 50, 50),
    null_text: Color32::from_rgb(160, 168, 182),
    sql_keyword: Color32::from_rgb(7, 87, 186),
    sql_string: Color32::from_rgb(154, 54, 23),
    sql_number: Color32::from_rgb(15, 123, 77),
    sql_comment: Color32::from_rgb(122, 130, 142),
    sql_operator: Color32::from_rgb(86, 96, 112),
    fav_star: Color32::from_rgb(150, 100, 10),
    badge_pk: Color32::from_rgb(30, 90, 190),
    badge_uni: Color32::from_rgb(26, 127, 55),
    badge_idx: Color32::from_rgb(140, 100, 30),
    conn_presets: [
        Color32::from_rgb(31, 111, 235), // azul
        Color32::from_rgb(0, 128, 128),  // turquesa
        Color32::from_rgb(26, 127, 55),  // verde
        Color32::from_rgb(176, 136, 0),  // amarillo
        Color32::from_rgb(196, 90, 10),  // naranja
        Color32::from_rgb(207, 34, 46),  // rojo
        Color32::from_rgb(191, 57, 137), // rosa
        Color32::from_rgb(130, 80, 223), // morado
    ],
};

impl Palette {
    /// Color de una conexión resuelto para este tema.
    pub fn conn(&self, color: ConnColor) -> Color32 {
        let idx = match color {
            ConnColor::Blue => 0,
            ConnColor::Teal => 1,
            ConnColor::Green => 2,
            ConnColor::Yellow => 3,
            ConnColor::Orange => 4,
            ConnColor::Red => 5,
            ConnColor::Pink => 6,
            ConnColor::Purple => 7,
            ConnColor::Custom([r, g, b]) => return Color32::from_rgb(r, g, b),
        };
        self.conn_presets[idx]
    }
}

pub fn of(dark: bool) -> &'static Palette {
    if dark { &DARK } else { &LIGHT }
}

pub fn get(ui: &egui::Ui) -> &'static Palette {
    of(ui.visuals().dark_mode)
}

/// Construye el Style completo (visuals + spacing + tamaños de texto) de un tema.
pub fn style(dark: bool, font_size: f32) -> egui::Style {
    let mut style = egui::Style {
        visuals: visuals(dark),
        ..Default::default()
    };

    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(12.0, 5.0);
    style.spacing.menu_margin = egui::Margin::same(6);
    style.spacing.window_margin = egui::Margin::same(14);
    style.spacing.indent = 14.0;
    style.spacing.interact_size.y = 26.0;

    use egui::TextStyle::*;
    for (text_style, font_id) in style.text_styles.iter_mut() {
        font_id.size = match text_style {
            Heading => font_size + 3.0,
            Small => (font_size - 2.0).max(9.0),
            _ => font_size,
        };
    }

    style
}

fn visuals(dark: bool) -> egui::Visuals {
    let p = of(dark);
    let mut v = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };

    v.panel_fill = p.bg_base;
    v.window_fill = p.bg_raised;
    v.faint_bg_color = p.grid_stripe; // rayado de TableBuilder
    v.extreme_bg_color = p.bg_sunken;
    v.text_edit_bg_color = Some(p.bg_sunken);
    v.code_bg_color = p.bg_surface;

    v.window_stroke = Stroke::new(1.0, p.border_subtle);
    v.window_corner_radius = CornerRadius::same(RADIUS_LG);
    v.menu_corner_radius = CornerRadius::same(RADIUS_MD);
    v.window_shadow = Shadow {
        offset: [0, 8],
        blur: 24,
        spread: 0,
        color: Color32::from_black_alpha(if dark { 96 } else { 40 }),
    };
    v.popup_shadow = Shadow {
        offset: [0, 4],
        blur: 12,
        spread: 0,
        color: Color32::from_black_alpha(if dark { 80 } else { 32 }),
    };

    v.selection.bg_fill = p.accent_bg;
    v.selection.stroke = Stroke::new(1.0, p.accent); // anillo de foco de TextEdit
    v.hyperlink_color = p.accent;
    v.warn_fg_color = p.warning;
    v.error_fg_color = p.danger;

    // Los botones leen weak_bg_fill; checkboxes/sliders leen bg_fill: fijar ambos.
    v.widgets.noninteractive.bg_fill = p.bg_surface;
    v.widgets.noninteractive.weak_bg_fill = p.bg_surface;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.border_subtle);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.text_primary);

    v.widgets.inactive.bg_fill = p.bg_surface;
    v.widgets.inactive.weak_bg_fill = p.bg_surface;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, p.border_subtle);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, p.text_primary);

    v.widgets.hovered.bg_fill = p.bg_raised;
    v.widgets.hovered.weak_bg_fill = p.bg_raised;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, p.border_strong);
    v.widgets.hovered.fg_stroke = Stroke::new(1.5, p.text_primary);

    v.widgets.active.bg_fill = p.accent_bg;
    v.widgets.active.weak_bg_fill = p.accent_bg;
    v.widgets.active.bg_stroke = Stroke::new(1.0, p.accent);
    v.widgets.active.fg_stroke = Stroke::new(1.5, p.text_primary);

    v.widgets.open.bg_fill = p.bg_raised;
    v.widgets.open.weak_bg_fill = p.bg_raised;
    v.widgets.open.bg_stroke = Stroke::new(1.0, p.border_subtle);
    v.widgets.open.fg_stroke = Stroke::new(1.0, p.text_primary);

    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.corner_radius = CornerRadius::same(RADIUS_SM);
    }

    v
}

/// Etiqueta de sección: mayúsculas, apagada, con tracking (estilo "TABLAS").
pub fn section_label(ui: &mut egui::Ui, text: &str) {
    let p = get(ui);
    let size = egui::TextStyle::Body.resolve(ui.style()).size * 0.85;
    let mut job = egui::text::LayoutJob::default();
    job.append(
        &text.to_uppercase(),
        0.0,
        egui::TextFormat {
            font_id: egui::FontId::new(size, egui::FontFamily::Name(MEDIUM_FAMILY.into())),
            color: p.text_muted,
            extra_letter_spacing: 0.8,
            ..Default::default()
        },
    );
    ui.label(job);
}

fn tinted_button(
    ui: &mut egui::Ui,
    text: impl Into<egui::WidgetText>,
    enabled: bool,
    fill: Color32,
    fill_hover: Color32,
    fg: Color32,
) -> egui::Response {
    ui.scope(|ui| {
        let w = &mut ui.style_mut().visuals.widgets;
        w.inactive.weak_bg_fill = fill;
        w.hovered.weak_bg_fill = fill_hover;
        w.active.weak_bg_fill = fill_hover;
        for state in [&mut w.inactive, &mut w.hovered, &mut w.active] {
            state.bg_stroke = Stroke::NONE;
            state.fg_stroke = Stroke::new(1.0, fg);
        }
        ui.add_enabled(enabled, egui::Button::new(text))
    })
    .inner
}

/// Botón de acción primaria (fondo de acento).
pub fn primary_button(ui: &mut egui::Ui, text: impl Into<egui::WidgetText>) -> egui::Response {
    let p = get(ui);
    tinted_button(ui, text, true, p.accent, p.accent_hover, p.on_accent)
}

/// Botón primario con estado enabled explícito (equivale a add_enabled).
pub fn primary_button_enabled(
    ui: &mut egui::Ui,
    enabled: bool,
    text: impl Into<egui::WidgetText>,
) -> egui::Response {
    let p = get(ui);
    tinted_button(ui, text, enabled, p.accent, p.accent_hover, p.on_accent)
}

/// Botón de acción destructiva (fondo rojo).
pub fn danger_button(ui: &mut egui::Ui, text: impl Into<egui::WidgetText>) -> egui::Response {
    let p = get(ui);
    tinted_button(ui, text, true, p.danger, p.danger_hover, p.on_accent)
}

/// Botón de advertencia (fondo ámbar): acciones que descartan trabajo.
pub fn warning_button(ui: &mut egui::Ui, text: impl Into<egui::WidgetText>) -> egui::Response {
    let p = get(ui);
    tinted_button(
        ui,
        text,
        true,
        p.warning,
        p.warning.gamma_multiply(0.85),
        p.on_accent,
    )
}

/// Pill compacta con tinte translúcido del color dado (PK/UNI/IDX, contadores…).
pub fn badge(ui: &mut egui::Ui, text: &str, color: Color32) {
    egui::Frame::NONE
        .fill(color.gamma_multiply(0.18))
        .corner_radius(CornerRadius::same(RADIUS_LG))
        .inner_margin(egui::Margin::symmetric(6, 1))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(text)
                    .color(color)
                    .text_style(egui::TextStyle::Small),
            );
        });
}

/// Card de error: fondo rojizo translúcido con borde y texto en danger.
pub fn error_card(ui: &mut egui::Ui, text: &str) {
    let p = get(ui);
    egui::Frame::NONE
        .fill(p.danger.gamma_multiply(0.12))
        .stroke(Stroke::new(1.0, p.danger.gamma_multiply(0.5)))
        .corner_radius(CornerRadius::same(RADIUS_MD))
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(egui::RichText::new(text).color(p.danger));
        });
}

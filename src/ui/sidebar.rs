use std::collections::{HashMap, HashSet};

use eframe::egui;

use crate::loadable::Loadable;
use crate::table_state::table_key;
use crate::ui::icons::*;
use crate::ui::{CMD, ellipsize, theme};

/// Si `db` tiene alguna tabla favorita. Los favoritos son claves «bd/tabla»:
/// no hace falta haber cargado las tablas de la BBDD para saberlo.
fn has_favorite(fav_tables: &HashSet<String>, db: &str) -> bool {
    fav_tables.iter().any(|key| {
        key.strip_prefix(db)
            .is_some_and(|rest| rest.starts_with('/'))
    })
}

#[derive(Debug, Clone, Default)]
pub enum SidebarAction {
    #[default]
    None,
    SelectServer,
    SelectDatabase(String),
    SelectTable {
        db: String,
        table: String,
    },
    CreateDatabase,
    CreateTable {
        db: String,
    },
    DropDatabase {
        db: String,
    },
    TruncateAllTables {
        db: String,
    },
    DropTable {
        db: String,
        table: String,
    },
    TruncateTable {
        db: String,
        table: String,
    },
    CopyTableName(String),
    Refresh,
    Disconnect,
    ToggleFav {
        db: String,
        table: String,
    },
    OpenTableStructure {
        db: String,
        table: String,
    },
    OpenInSql {
        db: String,
        table: String,
    },
    CloneTable {
        db: String,
        table: String,
    },
    ExportDatabase(String),
    ExportTable {
        db: String,
        table: String,
    },
}

/// Lo que el sidebar pide a la sesión: la acción del usuario y los datos
/// que le faltan para dibujar el árbol (los carga la sesión en segundo plano).
#[derive(Default)]
pub struct SidebarOutput {
    pub action: SidebarAction,
    /// BBDD desplegadas cuyas tablas aún no se han pedido.
    pub load_tables: Vec<String>,
    /// El buscador necesita las tablas de todas las BBDD.
    pub load_all_tables: bool,
}

fn loading_label(ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.spinner();
        ui.label("Cargando…");
    });
}

/// Lo que el sidebar enseña: el esquema cargado hasta ahora y qué está
/// seleccionado.
pub struct SidebarView<'a> {
    pub databases: &'a Loadable<Vec<String>>,
    /// Tablas de cada BBDD, según se van pidiendo.
    pub tables: &'a HashMap<String, Loadable<Vec<String>>>,
    pub selected_db: Option<&'a str>,
    pub selected_table: Option<&'a str>,
    /// Claves «bd/tabla» de las tablas favoritas.
    pub favorites: &'a HashSet<String>,
    pub server_name: &'a str,
    /// Color de la conexión: tiñe la raíz del árbol y la selección.
    pub conn_color: Option<egui::Color32>,
    /// La conexión se limita a una BBDD: sale desplegada, y ni se crean
    /// otras ni se elimina esa (el árbol no podría enseñar el resultado).
    pub single_database: bool,
}

/// Muestra el panel izquierdo con buscador y árbol servidor/databases/tablas.
/// El texto del buscador y el filtro de favoritos son estado suyo, que
/// cambia él mismo; lo demás lo pide con `SidebarOutput`.
pub fn show(
    ui: &mut egui::Ui,
    view: &SidebarView,
    table_search: &mut String,
    show_only_fav: &mut bool,
) -> SidebarOutput {
    let mut out = SidebarOutput::default();
    let p = theme::get(ui);

    ui.add_space(4.0);
    theme::section_label(ui, "Esquema");
    ui.add_space(2.0);
    toolbar(ui, table_search, show_only_fav, &mut out);
    ui.separator();

    let dbs = match view.databases {
        Loadable::Ready(dbs) => dbs,
        Loadable::Failed(e) => {
            ui.colored_label(p.danger, format!("Error: {e}"));
            return out;
        }
        _ => {
            loading_label(ui);
            return out;
        }
    };

    let tree = Tree {
        view,
        filter: table_search.to_lowercase(),
        only_fav: *show_only_fav,
    };
    ui.spacing_mut().indent = 10.0;
    egui::ScrollArea::vertical()
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
            tree.server(ui, dbs, &mut out);
        });
    out
}

/// Filtro de favoritos, recargar y buscador.
fn toolbar(
    ui: &mut egui::Ui,
    table_search: &mut String,
    show_only_fav: &mut bool,
    out: &mut SidebarOutput,
) {
    let p = theme::get(ui);
    let ctrl_f = ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::F));

    ui.horizontal(|ui| {
        let fav_btn = ui.add(
            egui::Button::new(STAR_FILLED)
                .selected(*show_only_fav)
                .small(),
        );
        if fav_btn.clicked() {
            *show_only_fav = !*show_only_fav;
        }
        fav_btn.on_hover_text(if *show_only_fav {
            "Mostrando solo favoritos"
        } else {
            "Filtrar favoritos"
        });
        let refresh_btn = ui.add(egui::Button::new(REFRESH).small());
        if refresh_btn.clicked() {
            out.action = SidebarAction::Refresh;
        }
        refresh_btn.on_hover_text("Recargar esquema");

        // Buscador tipo pill
        egui::Frame::NONE
            .fill(p.bg_surface)
            .stroke(egui::Stroke::new(1.0, p.border_subtle))
            .corner_radius(egui::CornerRadius::same(theme::RADIUS_LG))
            .inner_margin(egui::Margin::symmetric(8, 3))
            .show(ui, |ui| {
                let search_resp = ui.add(
                    egui::TextEdit::singleline(table_search)
                        .id_salt("sidebar_table_search")
                        .hint_text(format!("Buscar…  {CMD}F"))
                        .frame(egui::Frame::NONE)
                        .desired_width(f32::INFINITY),
                );
                if ctrl_f {
                    search_resp.request_focus();
                }
            });
    });
}

/// Opción de un menú contextual: al elegirla, el menú se cierra.
fn menu_item(ui: &mut egui::Ui, label: String) -> bool {
    let clicked = ui.button(label).clicked();
    if clicked {
        ui.close();
    }
    clicked
}

/// Como `menu_item`, para lo que destruye algo.
fn danger_item(ui: &mut egui::Ui, label: String) -> bool {
    let clicked = theme::danger_button(ui, label).clicked();
    if clicked {
        ui.close();
    }
    clicked
}

/// Lo que el árbol enseña bajo una BBDD.
#[derive(Debug, PartialEq)]
enum Listing<'a> {
    /// La BBDD no pasa los filtros: no sale en el árbol.
    Hidden,
    Tables(Vec<&'a String>),
    /// Sus tablas aún no se han pedido al servidor.
    NotRequested,
    Loading,
    Failed(&'a str),
}

/// El árbol con sus filtros: el texto buscado (en minúsculas) y si solo se
/// ven los favoritos.
struct Tree<'a> {
    view: &'a SidebarView<'a>,
    filter: String,
    only_fav: bool,
}

impl Tree<'_> {
    fn filtering(&self) -> bool {
        !self.filter.is_empty()
    }

    fn is_fav(&self, db: &str, table: &str) -> bool {
        self.view.favorites.contains(&table_key(db, table))
    }

    /// Raíz del árbol: el servidor, con sus BBDD dentro.
    fn server(&self, ui: &mut egui::Ui, dbs: &[String], out: &mut SidebarOutput) {
        let label = format!("{} {}", PLUG, ellipsize(self.view.server_name, 28));
        // Hueco para el fondo de la raíz: se rellena al conocer su rect.
        let background = ui.painter().add(egui::Shape::Noop);
        let header = egui::CollapsingHeader::new(&label)
            .id_salt("server_root")
            .default_open(true)
            .show(ui, |ui| {
                for db in dbs {
                    self.database(ui, db, out);
                }
            })
            .header_response;

        if let Some(color) = self.view.conn_color {
            let rect = egui::Rect::from_x_y_ranges(ui.max_rect().x_range(), header.rect.y_range());
            ui.painter().set(
                background,
                egui::epaint::RectShape::filled(rect, theme::RADIUS_SM, color.gamma_multiply(0.22)),
            );
        }
        if header.clicked() {
            out.action = SidebarAction::SelectServer;
        }
        header.context_menu(|ui| {
            if !self.view.single_database {
                if menu_item(ui, format!("{PLUS} Crear base de datos")) {
                    out.action = SidebarAction::CreateDatabase;
                }
                ui.separator();
            }
            if menu_item(ui, format!("{REFRESH} Refrescar")) {
                out.action = SidebarAction::Refresh;
            }
            if menu_item(ui, format!("{PLUG} Desconectar")) {
                out.action = SidebarAction::Disconnect;
            }
        });
    }

    /// Lo que hay bajo `db` con los filtros puestos. Al buscar solo salen
    /// las BBDD con alguna tabla que coincida; con el filtro de favoritos,
    /// las que tienen alguno.
    fn listing(&self, db: &str) -> Listing<'_> {
        let view = self.view;
        if self.only_fav && !has_favorite(view.favorites, db) {
            return Listing::Hidden;
        }
        let cached = view.tables.get(db);
        let mut tables: Vec<&String> = if self.filtering() {
            let matching: Vec<&String> = cached
                .and_then(Loadable::ready)
                .into_iter()
                .flatten()
                .filter(|table| table.to_lowercase().contains(&self.filter))
                .collect();
            if matching.is_empty() {
                return Listing::Hidden;
            }
            matching
        } else {
            match cached {
                Some(Loadable::Ready(tables)) => tables.iter().collect(),
                Some(Loadable::Failed(e)) => return Listing::Failed(e),
                Some(Loadable::Loading(_)) => return Listing::Loading,
                None | Some(Loadable::NotLoaded) => return Listing::NotRequested,
            }
        };
        if self.only_fav {
            tables.retain(|table| self.is_fav(db, table));
        }
        Listing::Tables(tables)
    }

    /// Una BBDD con sus tablas dentro.
    fn database(&self, ui: &mut egui::Ui, db: &String, out: &mut SidebarOutput) {
        let p = theme::get(ui);
        // El buscador mira en las tablas de todas las BBDD
        let cached = self.view.tables.get(db);
        if self.filtering() && cached.is_none_or(Loadable::needs_load) {
            out.load_all_tables = true;
        }
        let listing = self.listing(db);
        if listing == Listing::Hidden {
            return;
        }

        let header = egui::CollapsingHeader::new(egui::RichText::new(ellipsize(db, 24)).strong())
            // Al buscar, las BBDD con coincidencias salen desplegadas: lo
            // que se busca son tablas. Con otro id, para que al borrar la
            // búsqueda el árbol vuelva a estar como estaba.
            .id_salt((db, self.filtering()))
            .default_open(self.filtering() || self.view.single_database)
            .show(ui, |ui| match listing {
                Listing::Hidden => {}
                Listing::Failed(e) => {
                    ui.colored_label(p.danger, format!("Error: {e}"));
                }
                Listing::Loading => loading_label(ui),
                // Se piden al desplegarla por primera vez
                Listing::NotRequested => {
                    out.load_tables.push(db.clone());
                    loading_label(ui);
                }
                Listing::Tables(tables) if tables.is_empty() => {
                    ui.label(if self.filtering() {
                        "(sin coincidencias)"
                    } else {
                        "(sin tablas)"
                    });
                }
                Listing::Tables(tables) => {
                    for table in tables {
                        self.table_row(ui, db, table, out);
                    }
                }
            })
            .header_response;

        if header.clicked() {
            out.action = SidebarAction::SelectDatabase(db.clone());
        }
        header.context_menu(|ui| {
            let db = || db.clone();
            if menu_item(ui, format!("{PLUS} Crear tabla")) {
                out.action = SidebarAction::CreateTable { db: db() };
            }
            if menu_item(ui, format!("{EXPORT} Exportar base de datos")) {
                out.action = SidebarAction::ExportDatabase(db());
            }
            ui.separator();
            if menu_item(ui, format!("{TRASH} Vaciar todas las tablas")) {
                out.action = SidebarAction::TruncateAllTables { db: db() };
            }
            if !self.view.single_database
                && danger_item(ui, format!("{TRASH} Eliminar base de datos"))
            {
                out.action = SidebarAction::DropDatabase { db: db() };
            }
        });
    }

    /// Fila de una tabla: la estrella de favorito a la izquierda y el nombre,
    /// recortado al ancho del panel.
    fn table_row(&self, ui: &mut egui::Ui, db: &str, table: &str, out: &mut SidebarOutput) {
        let p = theme::get(ui);
        let view = self.view;
        let is_selected = view.selected_db == Some(db) && view.selected_table == Some(table);
        let is_fav = self.is_fav(db, table);
        // Las acciones se llevan sus nombres
        let names = || (db.to_string(), table.to_string());

        let row_height = ui.spacing().interact_size.y;
        let icon_w = 18.0;
        let (row_rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), row_height),
            egui::Sense::hover(),
        );
        let fav_rect = egui::Rect::from_min_size(row_rect.min, egui::vec2(icon_w, row_height));
        let label_rect = egui::Rect::from_min_max(
            egui::pos2(row_rect.min.x + icon_w, row_rect.min.y),
            row_rect.max,
        );
        let fav_resp = ui.interact(
            fav_rect,
            ui.scope_id().with(("fav", db, table)),
            egui::Sense::click(),
        );
        let resp = ui.interact(
            label_rect,
            ui.scope_id().with(("row", db, table)),
            egui::Sense::click(),
        );

        // Fondo hover/selected
        let bg = if is_selected {
            view.conn_color
                .map_or(p.grid_sel_row_bg, |c| c.gamma_multiply(0.3))
        } else if resp.hovered() || fav_resp.hovered() {
            p.bg_surface
        } else {
            egui::Color32::TRANSPARENT
        };
        ui.painter()
            .rect_filled(row_rect, f32::from(theme::RADIUS_SM), bg);

        let star_color = if is_fav {
            p.fav_star
        } else if fav_resp.hovered() {
            ui.visuals().strong_text_color()
        } else {
            ui.visuals().weak_text_color()
        };
        ui.painter().text(
            fav_rect.center(),
            egui::Align2::CENTER_CENTER,
            if is_fav { STAR_FILLED } else { STAR },
            egui::TextStyle::Body.resolve(ui.style()),
            star_color,
        );

        let text_color = if is_selected {
            p.text_primary
        } else if is_fav {
            p.fav_star
        } else {
            ui.visuals().text_color()
        };
        // Recortado al ancho del panel, no a un número fijo de caracteres
        let mut job = egui::text::LayoutJob::simple_singleline(
            table.to_string(),
            egui::TextStyle::Body.resolve(ui.style()),
            text_color,
        );
        job.wrap = egui::text::TextWrapping::truncate_at_width(label_rect.width() - 4.0);
        let galley = ui.painter().layout_job(job);
        let name_cut = galley.elided;
        ui.painter().galley(
            egui::pos2(
                label_rect.left() + 2.0,
                label_rect.center().y - galley.size().y / 2.0,
            ),
            galley,
            text_color,
        );

        if fav_resp.clicked() {
            let (db, table) = names();
            out.action = SidebarAction::ToggleFav { db, table };
        }
        if resp.clicked() {
            let (db, table) = names();
            out.action = SidebarAction::SelectTable { db, table };
        }
        // Nombre completo si no cabe
        let resp = if name_cut {
            resp.on_hover_text(table)
        } else {
            resp
        };

        resp.context_menu(|ui| {
            let mut item =
                |ui: &mut egui::Ui, label: String, action: fn(String, String) -> SidebarAction| {
                    if menu_item(ui, label) {
                        let (db, table) = names();
                        out.action = action(db, table);
                    }
                };
            let fav_label = if is_fav {
                format!("{STAR} Quitar de favoritos")
            } else {
                format!("{STAR_FILLED} Marcar favorito")
            };
            item(ui, fav_label, |db, table| SidebarAction::ToggleFav {
                db,
                table,
            });
            item(ui, format!("{EDIT} Ver estructura"), |db, table| {
                SidebarAction::OpenTableStructure { db, table }
            });
            item(ui, format!("{CODE} Abrir en SQL"), |db, table| {
                SidebarAction::OpenInSql { db, table }
            });
            item(ui, format!("{COPY} Copiar nombre"), |_, table| {
                SidebarAction::CopyTableName(table)
            });
            item(ui, format!("{CLONE} Clonar tabla"), |db, table| {
                SidebarAction::CloneTable { db, table }
            });
            item(ui, format!("{EXPORT} Exportar tabla"), |db, table| {
                SidebarAction::ExportTable { db, table }
            });
            ui.separator();
            item(ui, format!("{TRASH} Vaciar tabla"), |db, table| {
                SidebarAction::TruncateTable { db, table }
            });
            if danger_item(ui, format!("{TRASH} Eliminar tabla")) {
                let (db, table) = names();
                out.action = SidebarAction::DropTable { db, table };
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn favoritos_sin_cargar_las_tablas() {
        let favs = ["tienda/clientes".to_string()].into_iter().collect();
        assert!(has_favorite(&favs, "tienda"));
        assert!(!has_favorite(&favs, "tien"), "no basta con el prefijo");
        assert!(!has_favorite(&favs, "otra"));
    }

    /// Esquema de prueba y el sidebar dibujado con eventos reales.
    struct Fixture {
        ctx: egui::Context,
        databases: Loadable<Vec<String>>,
        tables: HashMap<String, Loadable<Vec<String>>>,
        favorites: HashSet<String>,
        search: String,
        only_fav: bool,
    }

    impl Fixture {
        /// BBDD `tienda` (tablas cargadas) y `otra` (sin pedir).
        fn new() -> Self {
            let tables = vec!["clientes".to_string(), "pedidos".to_string()];
            Self {
                ctx: egui::Context::default(),
                databases: Loadable::Ready(vec!["tienda".into(), "otra".into()]),
                tables: [("tienda".to_string(), Loadable::Ready(tables))].into(),
                favorites: HashSet::new(),
                search: String::new(),
                only_fav: false,
            }
        }

        fn frame(&mut self, events: Vec<egui::Event>) -> SidebarOutput {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(240.0, 400.0),
                )),
                events,
                ..Default::default()
            };
            let mut out = SidebarOutput::default();
            self.ctx
                .run_ui(input, |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        let view = SidebarView {
                            databases: &self.databases,
                            tables: &self.tables,
                            selected_db: None,
                            selected_table: None,
                            favorites: &self.favorites,
                            server_name: "srv",
                            conn_color: None,
                            single_database: false,
                        };
                        out = show(ui, &view, &mut self.search, &mut self.only_fav);
                    });
                })
                .drop_without_applying_deltas();
            out
        }

        /// Acciones que salen al hacer clic a la altura `x` en cada fila del
        /// panel, de arriba abajo, sin repetir las consecutivas. Los nombres
        /// de las BBDD quedan a la izquierda; las filas de las tablas ocupan
        /// todo el ancho.
        fn click_down_the_tree(&mut self, x: f32) -> Vec<String> {
            let mut seen: Vec<String> = Vec::new();
            for y in (60..400).step_by(6) {
                let pos = egui::pos2(x, y as f32);
                let button = |pressed| egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                };
                self.frame(vec![egui::Event::PointerMoved(pos), button(true)]);
                let out = self.frame(vec![button(false)]);
                let action = format!("{:?}", out.action);
                if !matches!(out.action, SidebarAction::None) && seen.last() != Some(&action) {
                    seen.push(action);
                }
            }
            seen
        }
    }

    #[test]
    fn buscar_pide_las_tablas_que_faltan_y_despliega_las_coincidencias() {
        let mut f = Fixture::new();
        f.frame(Vec::new());
        f.search = "CLI".into();
        let out = f.frame(Vec::new());
        assert!(out.load_all_tables, "«otra» aún no tiene sus tablas");

        // Solo queda la BBDD con coincidencias, ya desplegada
        f.tables
            .insert("otra".into(), Loadable::Ready(vec!["facturas".into()]));
        assert!(!f.frame(Vec::new()).load_all_tables);
        let clicks = f.click_down_the_tree(120.0);
        assert!(
            clicks.contains(&r#"SelectTable { db: "tienda", table: "clientes" }"#.to_string()),
            "{clicks:?}"
        );
        assert!(!clicks.iter().any(|a| a.contains("pedidos")), "{clicks:?}");
        assert!(!clicks.iter().any(|a| a.contains("otra")), "{clicks:?}");
    }

    #[test]
    fn lo_que_sale_bajo_cada_bbdd() {
        let mut f = Fixture::new();
        f.tables
            .insert("rota".into(), Loadable::Failed("sin permiso".into()));
        f.favorites.insert(table_key("tienda", "pedidos"));
        let tree = |f: &Fixture, filter: &str, only_fav| {
            let view = SidebarView {
                databases: &f.databases,
                tables: &f.tables,
                selected_db: None,
                selected_table: None,
                favorites: &f.favorites,
                server_name: "srv",
                conn_color: None,
                single_database: false,
            };
            let tree = Tree {
                view: &view,
                filter: filter.into(),
                only_fav,
            };
            // Como texto: el resultado toma prestado de `view`
            ["tienda", "otra", "rota"].map(|db| format!("{:?}", tree.listing(db)))
        };

        assert_eq!(
            tree(&f, "", false),
            [
                r#"Tables(["clientes", "pedidos"])"#,
                "NotRequested",
                r#"Failed("sin permiso")"#
            ]
        );
        // Buscando: solo las BBDD con alguna tabla que coincida
        assert_eq!(
            tree(&f, "cli", false),
            [r#"Tables(["clientes"])"#, "Hidden", "Hidden"]
        );
        // Solo favoritos: fuera las BBDD sin ninguno y las tablas que no lo son
        assert_eq!(
            tree(&f, "", true),
            [r#"Tables(["pedidos"])"#, "Hidden", "Hidden"]
        );
        assert_eq!(tree(&f, "cli", true), [r#"Tables([])"#, "Hidden", "Hidden"]);
    }

    #[test]
    fn el_filtro_de_favoritos_esconde_las_bbdd_sin_ninguno() {
        let mut f = Fixture::new();
        f.favorites.insert(table_key("tienda", "pedidos"));
        f.only_fav = true;
        f.search = "e".into();
        f.tables
            .insert("otra".into(), Loadable::Ready(vec!["facturas_e".into()]));
        f.frame(Vec::new());
        let clicks = f.click_down_the_tree(120.0);
        assert!(
            clicks.contains(&r#"SelectTable { db: "tienda", table: "pedidos" }"#.to_string()),
            "{clicks:?}"
        );
        assert!(!clicks.iter().any(|a| a.contains("clientes")), "{clicks:?}");
        assert!(!clicks.iter().any(|a| a.contains("otra")), "{clicks:?}");
    }
}

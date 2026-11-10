//! Pruebas del grid con un contexto egui real: se le pasan teclas como
//! eventos y se mira qué queda en el estado y qué acción devuelve.

use eframe::egui;

use super::*;

const COLS: usize = 30;

struct Fixture {
    ctx: egui::Context,
    view: TableViewState,
    column_info: Vec<ColumnInfo>,
    /// Cargando o guardando: el grid no deja editar.
    busy: bool,
}

fn column(name: &str, extra: &str, nullable: bool) -> ColumnInfo {
    ColumnInfo {
        name: name.into(),
        col_type: "varchar(20)".into(),
        is_nullable: nullable,
        default_val: None,
        column_key: String::new(),
        extra: extra.into(),
    }
}

impl Fixture {
    /// Tabla de `rows` filas × 30 columnas con «r{fila}c{columna}» en cada
    /// celda. La columna 0 no admite NULL; las demás sí.
    fn new(rows: usize) -> Self {
        let columns: Vec<String> = (0..COLS).map(|c| format!("col{c}")).collect();
        let column_info = columns
            .iter()
            .enumerate()
            .map(|(c, name)| column(name, "", c > 0))
            .collect();
        let data = QueryResult {
            columns,
            rows: (0..rows)
                .map(|r| (0..COLS).map(|c| Some(format!("r{r}c{c}"))).collect())
                .collect(),
            truncated: false,
        };
        let view = TableViewState {
            table_data: Some(data),
            ..Default::default()
        };
        Self {
            ctx: egui::Context::default(),
            view,
            column_info,
            busy: false,
        }
    }

    /// Un frame de 400×300 con los eventos indicados.
    fn frame(&mut self, modifiers: egui::Modifiers, mut events: Vec<egui::Event>) -> GridAction {
        // Los modificadores llegan como evento y se quedan hasta el siguiente
        events.insert(0, egui::Event::ModifiersChanged(modifiers));
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(400.0, 300.0),
            )),
            events,
            ..Default::default()
        };
        let mut action = GridAction::None;
        self.ctx
            .run_ui(input, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    action = show(
                        ui,
                        Some(("db", "t")),
                        &mut self.view,
                        &self.column_info,
                        self.busy,
                    );
                });
            })
            .drop_without_applying_deltas();
        action
    }

    fn idle(&mut self) -> GridAction {
        self.frame(egui::Modifiers::NONE, Vec::new())
    }

    /// Da el foco del teclado al grid, como al pulsar dentro de él.
    fn focus(&mut self) {
        self.idle();
        let id = self.view.selection.focus_id.expect("el grid publica su id");
        self.ctx.memory_mut(|m| m.request_focus(id));
        self.idle();
    }

    fn press_with(&mut self, modifiers: egui::Modifiers, key: egui::Key) -> GridAction {
        let event = egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        self.frame(modifiers, vec![event])
    }

    fn press(&mut self, key: egui::Key) -> GridAction {
        self.press_with(egui::Modifiers::NONE, key)
    }

    fn type_text(&mut self, text: &str) -> GridAction {
        self.frame(egui::Modifiers::NONE, vec![egui::Event::Text(text.into())])
    }

    fn cursor(&self) -> Option<(RowRef, usize)> {
        self.view.selection.cell.map(|c| (c.row, c.col))
    }

    fn put_cursor(&mut self, row: RowRef, col: usize) {
        self.view.selection.select_cell(CellRef { row, col });
    }

    fn add_insert_row(&mut self) {
        let mut auto_cols = vec![false; COLS];
        auto_cols[0] = true;
        self.view.pending_inserts.push(PendingInsert {
            values: vec![InsertValue::Default; COLS],
            auto_cols,
        });
    }

    /// Lo que se ve del grid: (primera posición, tamaño) en cada eje.
    fn visible(&self) -> (egui::Vec2, egui::Vec2) {
        let layout = self.view.selection.layout.as_ref().unwrap();
        (layout.offset, layout.size)
    }
}

#[test]
fn sin_foco_las_teclas_no_son_del_grid() {
    let mut f = Fixture::new(5);
    f.idle();
    f.put_cursor(RowRef::Existing(0), 0);
    f.press(egui::Key::ArrowDown);
    assert_eq!(f.cursor(), Some((RowRef::Existing(0), 0)));
    assert!(matches!(f.press(egui::Key::Delete), GridAction::None));
}

#[test]
fn la_primera_flecha_pone_el_cursor_en_la_primera_celda() {
    let mut f = Fixture::new(5);
    f.focus();
    f.press(egui::Key::ArrowDown);
    assert_eq!(f.cursor(), Some((RowRef::Existing(0), 0)));
    f.press(egui::Key::ArrowDown);
    f.press(egui::Key::ArrowRight);
    assert_eq!(f.cursor(), Some((RowRef::Existing(1), 1)));
}

#[test]
fn el_scroll_vertical_sigue_al_cursor() {
    let mut f = Fixture::new(200);
    f.focus();
    for _ in 0..=60 {
        f.press(egui::Key::ArrowDown);
    }
    f.idle();
    assert_eq!(f.cursor(), Some((RowRef::Existing(60), 0)));
    let (offset, size) = f.visible();
    assert!(offset.y > 0.0, "el grid no se ha desplazado");
    let pitch = table::ROW_H + f.ctx.global_style().spacing.item_spacing.y;
    let top = 60.0 * pitch;
    assert!(
        top >= offset.y && top + table::ROW_H <= offset.y + size.y + 0.5,
        "la fila 60 ({top}) queda fuera de {}..{}",
        offset.y,
        offset.y + size.y
    );

    // Y de vuelta arriba
    f.press_with(egui::Modifiers::COMMAND, egui::Key::Home);
    f.idle();
    assert_eq!(f.cursor(), Some((RowRef::Existing(0), 0)));
    assert_eq!(f.visible().0.y, 0.0);
}

#[test]
fn el_scroll_horizontal_sigue_al_cursor() {
    let mut f = Fixture::new(5);
    f.focus();
    f.press(egui::Key::ArrowDown);
    f.press(egui::Key::End);
    f.idle();
    assert_eq!(f.cursor(), Some((RowRef::Existing(0), COLS - 1)));
    let (offset, size) = f.visible();
    assert!(offset.x > 0.0, "el grid no se ha desplazado");
    let layout = f.view.selection.layout.clone().unwrap();
    let gap = f.ctx.global_style().spacing.item_spacing.x;
    let left: f32 = layout.col_widths[..COLS - 1].iter().map(|w| w + gap).sum();
    let right = left + layout.col_widths[COLS - 1];
    assert!(
        left >= offset.x && right <= offset.x + size.x + 0.5,
        "la última columna ({left}..{right}) queda fuera de {}..{}",
        offset.x,
        offset.x + size.x
    );

    f.press(egui::Key::Home);
    f.idle();
    assert_eq!(f.visible().0.x, 0.0);
}

#[test]
fn el_cursor_baja_a_las_filas_nuevas() {
    let mut f = Fixture::new(2);
    f.add_insert_row();
    f.focus();
    for _ in 0..5 {
        f.press(egui::Key::ArrowDown);
    }
    assert_eq!(f.cursor(), Some((RowRef::Insert(0), 0)));
}

#[test]
fn enter_edita_y_enter_confirma() {
    let mut f = Fixture::new(3);
    f.focus();
    f.put_cursor(RowRef::Existing(1), 2);
    f.press(egui::Key::Enter);
    let editing = f.view.editing_cell.as_ref().expect("Enter abre el editor");
    assert_eq!(editing.buffer, "r1c2");
    f.idle();
    f.type_text("x");
    let action = f.press(egui::Key::Enter);
    assert!(
        matches!(
            &action,
            GridAction::CellEdited { row: RowRef::Existing(1), col_idx: 2, value: Some(v) } if v == "r1c2x"
        ),
        "{action:?}"
    );
    assert!(f.view.editing_cell.is_none());
    // El foco vuelve al grid: las flechas siguen moviendo el cursor
    f.press(egui::Key::ArrowDown);
    assert_eq!(f.cursor(), Some((RowRef::Existing(2), 2)));
}

#[test]
fn escape_cancela_la_edicion() {
    let mut f = Fixture::new(3);
    f.focus();
    f.put_cursor(RowRef::Existing(0), 1);
    f.press(egui::Key::F2);
    f.idle();
    f.type_text("x");
    let action = f.press(egui::Key::Escape);
    assert!(matches!(action, GridAction::None), "{action:?}");
    assert!(f.view.editing_cell.is_none());
}

#[test]
fn escribir_sobre_una_celda_sustituye_su_valor() {
    let mut f = Fixture::new(3);
    f.focus();
    f.put_cursor(RowRef::Existing(0), 1);
    f.type_text("7");
    assert_eq!(f.view.editing_cell.as_ref().unwrap().buffer, "7");
    f.idle();
    f.type_text("5");
    let action = f.press(egui::Key::Enter);
    assert!(
        matches!(&action, GridAction::CellEdited { value: Some(v), .. } if v == "75"),
        "{action:?}"
    );
}

#[test]
fn tab_confirma_y_edita_la_celda_siguiente() {
    let mut f = Fixture::new(3);
    f.focus();
    f.put_cursor(RowRef::Existing(0), COLS - 1);
    f.press(egui::Key::F2);
    f.idle();
    f.type_text("x");
    let action = f.press(egui::Key::Tab);
    assert!(
        matches!(&action, GridAction::CellEdited { row: RowRef::Existing(0), value: Some(v), .. } if v.ends_with('x')),
        "{action:?}"
    );
    // Última columna: pasa a la primera de la fila siguiente
    let editing = f.view.editing_cell.as_ref().expect("sigue editando");
    assert_eq!(
        editing.cell,
        CellRef {
            row: RowRef::Existing(1),
            col: 0
        }
    );
    assert_eq!(editing.buffer, "r1c0");
    assert_eq!(f.cursor(), Some((RowRef::Existing(1), 0)));

    // Y Mayús+Tab vuelve
    f.idle();
    f.press_with(egui::Modifiers::SHIFT, egui::Key::Tab);
    assert_eq!(
        f.view.editing_cell.as_ref().unwrap().cell,
        CellRef {
            row: RowRef::Existing(0),
            col: COLS - 1
        }
    );
}

#[test]
fn tab_salta_las_columnas_auto_de_una_fila_nueva() {
    let mut f = Fixture::new(1);
    f.add_insert_row();
    f.focus();
    f.put_cursor(RowRef::Existing(0), COLS - 1);
    f.press(egui::Key::F2);
    f.idle();
    f.press(egui::Key::Tab);
    // La columna 0 de la fila nueva es AUTO_INCREMENT
    assert_eq!(
        f.view.editing_cell.as_ref().unwrap().cell,
        CellRef {
            row: RowRef::Insert(0),
            col: 1
        }
    );
}

#[test]
fn supr_vacia_la_celda_segun_la_columna() {
    let mut f = Fixture::new(2);
    f.add_insert_row();
    f.focus();

    f.put_cursor(RowRef::Existing(0), 1);
    let action = f.press(egui::Key::Delete);
    assert!(
        matches!(action, GridAction::CellEdited { value: None, .. }),
        "admite NULL: {action:?}"
    );

    f.put_cursor(RowRef::Existing(0), 0);
    let action = f.press(egui::Key::Delete);
    assert!(
        matches!(&action, GridAction::CellEdited { value: Some(v), .. } if v.is_empty()),
        "NOT NULL: {action:?}"
    );

    f.put_cursor(RowRef::Insert(0), 1);
    let action = f.press(egui::Key::Delete);
    assert!(
        matches!(
            action,
            GridAction::CellSetDefault {
                insert_idx: 0,
                col_idx: 1
            }
        ),
        "fila nueva: {action:?}"
    );
}

#[test]
fn ctrl_supr_elimina_filas_o_quita_la_nueva() {
    let mut f = Fixture::new(3);
    f.add_insert_row();
    f.focus();

    f.put_cursor(RowRef::Existing(1), 0);
    let action = f.press_with(egui::Modifiers::COMMAND, egui::Key::Delete);
    assert!(
        matches!(&action, GridAction::ToggleDeleteRows(rows) if rows == &[1]),
        "{action:?}"
    );

    f.view.selection.select_row(0, 0, false, false);
    f.view.selection.select_row(2, 0, false, true);
    let action = f.press_with(egui::Modifiers::COMMAND, egui::Key::Delete);
    assert!(
        matches!(&action, GridAction::ToggleDeleteRows(rows) if rows == &[0, 2]),
        "{action:?}"
    );

    f.put_cursor(RowRef::Insert(0), 1);
    let action = f.press_with(egui::Modifiers::COMMAND, egui::Key::Delete);
    assert!(
        matches!(action, GridAction::RemoveInsertRow { insert_idx: 0 }),
        "{action:?}"
    );
}

#[test]
fn ctrl_ins_inserta_sin_cursor_y_en_una_tabla_sin_filas() {
    let mut f = Fixture::new(0);
    f.focus();
    let action = f.press_with(egui::Modifiers::COMMAND, egui::Key::Insert);
    assert!(matches!(action, GridAction::InsertRow), "{action:?}");
}

#[test]
fn las_filas_eliminadas_y_las_columnas_auto_no_se_editan() {
    let mut f = Fixture::new(2);
    f.add_insert_row();
    f.view.pending_deletes.insert(0);
    f.focus();

    f.put_cursor(RowRef::Existing(0), 1);
    f.press(egui::Key::Enter);
    assert!(f.view.editing_cell.is_none(), "fila marcada para eliminar");

    f.put_cursor(RowRef::Insert(0), 0);
    f.type_text("9");
    assert!(f.view.editing_cell.is_none(), "columna AUTO_INCREMENT");
}

#[test]
fn mientras_carga_no_se_edita() {
    let mut f = Fixture::new(2);
    f.focus();
    f.put_cursor(RowRef::Existing(0), 1);
    f.busy = true;
    f.press(egui::Key::Enter);
    assert!(f.view.editing_cell.is_none());
    assert!(matches!(f.press(egui::Key::Delete), GridAction::None));
}

#[test]
fn la_celda_muestra_el_cambio_pendiente() {
    let mut f = Fixture::new(2);
    f.view.pending_edits.push(PendingEdit {
        row_idx: 0,
        col_idx: 1,
        new_value: "nuevo".into(),
        is_null: false,
    });
    f.view.pending_edits.push(PendingEdit {
        row_idx: 1,
        col_idx: 1,
        new_value: String::new(),
        is_null: true,
    });
    let data = f.view.table_data.as_ref().unwrap();
    let grid = Grid {
        id: egui::Id::unique("test"),
        data,
        column_info: &f.column_info,
        sorts: &[],
        pending_edits: &f.view.pending_edits,
        pending_deletes: &f.view.pending_deletes,
        pending_inserts: &f.view.pending_inserts,
        editable: true,
        queryable: true,
    };
    let cell = |row, col| CellRef {
        row: RowRef::Existing(row),
        col,
    };
    assert_eq!(grid.shown(cell(0, 1)), Shown::Value("nuevo"));
    assert_eq!(grid.shown(cell(1, 1)), Shown::Null);
    assert_eq!(grid.shown(cell(0, 2)), Shown::Value("r0c2"));
    assert_eq!(grid.stored(0, 1), Some("r0c1"), "el filtro usa lo guardado");
    assert_eq!(grid.edit_text(cell(1, 1)), "", "NULL se edita como vacío");
}

impl Fixture {
    /// Clic con el botón principal en `pos`.
    fn click(&mut self, pos: egui::Pos2) {
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        self.frame(
            egui::Modifiers::NONE,
            vec![egui::Event::PointerMoved(pos), button(true)],
        );
        self.frame(egui::Modifiers::NONE, vec![button(false)]);
        self.idle();
    }

    fn has_focus(&self) -> bool {
        let id = self.view.selection.focus_id.unwrap();
        self.ctx.memory(|m| m.has_focus(id))
    }
}

#[test]
fn un_clic_en_una_celda_pone_el_cursor_y_da_el_foco() {
    let mut f = Fixture::new(20);
    f.idle();
    assert!(!f.has_focus());

    f.click(egui::pos2(60.0, 150.0));
    let (row, col) = f.cursor().expect("el clic pone el cursor");
    let RowRef::Existing(row) = row else {
        panic!("{row:?}");
    };
    assert!(row > 0);
    assert!(f.has_focus(), "el grid recibe el teclado");
    f.press(egui::Key::ArrowUp);
    assert_eq!(f.cursor(), Some((RowRef::Existing(row - 1), col)));

    // Un clic fuera (la barra con el nombre de la tabla) se lo quita
    f.click(egui::pos2(200.0, 12.0));
    assert!(!f.has_focus());
    let before = f.cursor();
    f.press(egui::Key::ArrowDown);
    assert_eq!(f.cursor(), before, "sin foco, las flechas no son suyas");
}

#[test]
fn pasar_por_una_celda_sin_cambiarla_no_es_una_edicion() {
    let mut f = Fixture::new(2);
    f.view.table_data.as_mut().unwrap().rows[0][1] = None;
    f.add_insert_row();
    f.focus();

    // Enter y Enter sobre un NULL: antes quedaba un cambio pendiente a ''
    f.put_cursor(RowRef::Existing(0), 1);
    f.press(egui::Key::Enter);
    f.idle();
    let action = f.press(egui::Key::Enter);
    assert!(matches!(action, GridAction::None), "{action:?}");

    // Tab por las celdas de una fila nueva las deja en DEFAULT
    f.put_cursor(RowRef::Insert(0), 1);
    f.press(egui::Key::F2);
    f.idle();
    let action = f.press(egui::Key::Tab);
    assert!(matches!(action, GridAction::None), "{action:?}");
    assert_eq!(
        f.view.editing_cell.as_ref().unwrap().cell,
        CellRef {
            row: RowRef::Insert(0),
            col: 2
        },
        "Tab sigue avanzando"
    );
}

#[test]
fn teclear_el_mismo_valor_tampoco_es_una_edicion() {
    let mut f = Fixture::new(2);
    f.view.table_data.as_mut().unwrap().rows[0][1] = Some("7".into());
    f.focus();
    f.put_cursor(RowRef::Existing(0), 1);
    f.type_text("7");
    f.idle();
    let action = f.press(egui::Key::Enter);
    assert!(matches!(action, GridAction::None), "{action:?}");
}

#[test]
fn una_tabla_sin_filas_muestra_sus_columnas() {
    let mut f = Fixture::new(0);
    f.idle();
    f.idle();
    // Con cabecera hay tabla: antes solo salía el texto «(tabla vacía)»
    let layout = f
        .view
        .selection
        .layout
        .as_ref()
        .expect("se dibuja la tabla");
    assert_eq!(layout.col_widths.len(), COLS);
}

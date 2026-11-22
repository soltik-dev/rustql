//! Drawing of the table: header, cells, cell editor and scrolling.

use eframe::egui;
use egui_extras::{Column, TableBuilder};

use super::state::GridLayout;
use super::{CellRef, EditingCell, Grid, GridAction, GridSelection, RowRef, Shown, SortDir, menu};
use crate::db::QueryResult;
use crate::i18n::t;
use crate::ui::icons::KEY;
use crate::ui::theme;

/// Row height: that of the cell text plus a margin, the same for every row
/// (the cell does not change size when editing starts). It follows the
/// font size from the settings; with the default one it is 22 points.
fn row_height(ui: &egui::Ui) -> f32 {
    (ui.text_style_height(&egui::TextStyle::Monospace) + 6.0)
        .round()
        .max(22.0)
}

/// What happened in the table this frame.
#[derive(Default)]
pub(super) struct TableOut {
    pub action: GridAction,
    /// Header clicked (to sort by it).
    pub clicked_col: Option<usize>,
}

pub(super) fn show(
    ui: &mut egui::Ui,
    g: &Grid,
    sel: &mut GridSelection,
    editing: &mut Option<EditingCell>,
    focused: bool,
) -> TableOut {
    let mut out = TableOut::default();
    // Row under the pointer while dragging to select a range
    let mut drag_row: Option<usize> = None;

    let initial_widths = initial_col_widths(ui, g.data);
    let total_width: f32 = initial_widths.iter().sum::<f32>() + 24.0;
    let spacing = ui.spacing().item_spacing;
    let row_h = row_height(ui);
    let header_h = row_h + 2.0;
    let revealing = sel.reveal;
    let (scroll_x, scroll_y) = pending_scroll(ui, g, sel);
    let highlight = g.highlighted_value(sel.cell);

    let mut col_widths = Vec::new();
    // First and last rows drawn: (position in the grid, and on screen)
    let mut drawn_rows: Option<((usize, f32), (usize, f32))> = None;
    // Scroll bars always visible: egui's floating ones only show on hover,
    // and in a data grid you need to see where you are
    ui.spacing_mut().scroll = egui::style::ScrollStyle::thin();
    let mut hscroll = egui::ScrollArea::horizontal()
        .id_salt((g.id, "hscroll"))
        .auto_shrink([false, false]);
    if let Some(x) = scroll_x {
        hscroll = hscroll.horizontal_scroll_offset(x);
    }
    let outer = hscroll.show(ui, |ui| {
        ui.set_min_width(total_width);

        // Header background band (painted before building the table)
        let header_band = egui::Rect::from_min_size(
            ui.cursor().min,
            egui::vec2(ui.available_width().max(total_width), header_h),
        );
        ui.painter()
            .rect_filled(header_band, 0.0, theme::get(ui).grid_header_bg);

        let mut builder = TableBuilder::new(ui)
            .id_salt((g.id, "table"))
            .striped(true)
            .resizable(true)
            // By default the body never gets shorter than 200 even with less
            // room (small window, big log) and the last rows end up out of
            // view
            .min_scrolled_height(0.0)
            // The table's vertical bar sits on the right edge of the content,
            // out of view until scrolled all the way to the right: it is
            // hidden and our own is drawn (vertical_bar)
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center));
        if let Some(y) = scroll_y {
            builder = builder.vertical_scroll_offset(y);
        }
        for &w in &initial_widths {
            builder = builder.column(Column::initial(w).resizable(true).clip(true).at_least(40.0));
        }
        builder
            .header(header_h, |mut header| {
                for col in 0..g.cols() {
                    header.col(|ui| {
                        if header_cell(ui, g, col) {
                            out.clicked_col = Some(col);
                        }
                    });
                }
            })
            .body(|body| {
                col_widths = body.widths().to_vec();
                body.rows(row_h, g.total_rows(), |mut row| {
                    let display = row.index();
                    let row_ref = RowRef::from_display(display, g.existing());
                    for col in 0..g.cols() {
                        let cell = CellRef { row: row_ref, col };
                        row.col(|ui| {
                            if col == 0 {
                                let here = (display, ui.max_rect().top());
                                drawn_rows =
                                    Some((drawn_rows.map_or(here, |(first, _)| first), here));
                            }
                            show_cell(
                                ui,
                                g,
                                sel,
                                editing,
                                &mut out,
                                &mut drag_row,
                                focused,
                                highlight,
                                cell,
                            );
                        });
                    }
                });
            })
    });

    let body = &outer.inner;
    // Where row 0 falls and how far it is from one row to the next, from
    // what was just drawn (with no rows drawn, the nominal values)
    let nominal_pitch = row_h + spacing.y;
    let content_y =
        |screen_y: f32| screen_y - body.inner_rect.top() + body.state.unclamped_offset().y;
    let (row_pitch, row_origin) = match drawn_rows {
        Some(((i0, y0), (i1, y1))) if i1 > i0 => {
            let pitch = (y1 - y0) / (i1 - i0) as f32;
            (pitch, content_y(y0) - i0 as f32 * pitch)
        }
        Some(((i0, y0), _)) => (nominal_pitch, content_y(y0) - i0 as f32 * nominal_pitch),
        None => (nominal_pitch, 0.0),
    };
    // What is really visible of the body: its area may stick out below
    // that of the horizontal scroll containing it
    let visible_body = body.inner_rect.intersect(outer.inner_rect);
    let layout = GridLayout {
        offset: egui::vec2(
            outer.state.clamped_offset().x,
            body.state.clamped_offset().y,
        ),
        size: egui::vec2(outer.inner_rect.width(), visible_body.height().max(0.0)),
        max_offset: egui::vec2(
            (outer.content_size.x - outer.inner_rect.width()).max(0.0),
            (body.content_size.y - body.inner_rect.height()).max(0.0),
        ),
        row_origin,
        row_pitch,
        row_h,
        col_widths,
        col_gap: spacing.x,
    };
    // The scroll to the cursor was computed with the previous frame's
    // geometry. If it changed in this one (a new row makes the content
    // longer and brings up the save bar, which shrinks the room) it may
    // fall short: it is checked with the current one and repeated. With a
    // limit, in case the target cannot be reached.
    if revealing
        && !sel.reveal
        && sel.reveal_tries < 3
        && let Some(cell) = sel.cell
    {
        let (x, y) = layout.reveal(cell.row.display(g.existing()), cell.col);
        if x.is_some() || y.is_some() {
            sel.reveal = true;
            sel.reveal_tries += 1;
            ui.ctx().request_repaint();
        }
    }
    vertical_bar(ui, g, visible_body, &layout, sel);
    sel.layout = Some(layout);

    // The whole grid is a focusable widget: with focus it receives the keys.
    // egui takes it away on every click that does not land on it (and those
    // inside land on the cells), so it is given back if the click was inside.
    let focus_id = g.focus_id();
    ui.interact(
        outer.inner_rect,
        focus_id,
        egui::Sense::focusable_noninteractive(),
    );
    let pointer_inside = ui.rect_contains_pointer(outer.inner_rect)
        && ui.input(|i| i.pointer.any_pressed() || i.pointer.any_click());
    // While editing, the focus belongs to the cell editor
    if pointer_inside && editing.is_none() {
        ui.memory_mut(|m| m.request_focus(focus_id));
    }

    // Dragging selects the range of rows between the start and the pointer
    if ui.input(|i| i.pointer.is_decidedly_dragging()) {
        if let (Some(start), Some(current)) = (sel.drag_from, drag_row) {
            sel.rows = (start.min(current)..=start.max(current)).collect();
        }
    } else if sel.drag_from.is_some() && !ui.input(|i| i.pointer.any_down()) {
        sel.drag_from = None;
    }

    out
}

/// Scroll to apply this frame, if any: to the top (new data) or to the
/// cursor's cell if it moved with the keyboard.
fn pending_scroll(ui: &egui::Ui, g: &Grid, sel: &mut GridSelection) -> (Option<f32>, Option<f32>) {
    // Going back to the top wins over whatever was asked with the bar
    let bar = sel.scroll_to_y.take();
    let to_top = std::mem::take(&mut sel.scroll_top).then_some(0.0).or(bar);
    if !sel.reveal {
        return (None, to_top);
    }
    let Some(cell) = sel.cell else {
        sel.reveal = false;
        return (None, to_top);
    };
    // The previous frame's geometry is needed; on the first one it waits
    let Some(layout) = &sel.layout else {
        ui.ctx().request_repaint();
        return (None, to_top);
    };
    let (x, y) = layout.reveal(cell.row.display(g.existing()), cell.col);
    sel.reveal = false;
    (x, y.or(to_top))
}

/// Vertical scroll bar on the right edge of the visible rows area.
/// Dragging it (or clicking its track) requests the scroll for the next
/// frame.
fn vertical_bar(
    ui: &mut egui::Ui,
    g: &Grid,
    body: egui::Rect,
    layout: &GridLayout,
    sel: &mut GridSelection,
) {
    const WIDTH: f32 = 10.0;
    const MIN_HANDLE: f32 = 24.0;
    let max = layout.max_offset.y;
    if max <= 0.0 || body.height() <= MIN_HANDLE {
        return;
    }
    let track = egui::Rect::from_min_max(egui::pos2(body.right() - WIDTH, body.top()), body.max);
    let handle_h = (track.height() * body.height() / (max + body.height())).max(MIN_HANDLE);
    let travel = track.height() - handle_h;
    let handle_top = track.top() + travel * (layout.offset.y / max).clamp(0.0, 1.0);
    let handle = egui::Rect::from_min_size(
        egui::pos2(track.left() + 2.0, handle_top),
        egui::vec2(WIDTH - 4.0, handle_h),
    );

    let resp = ui.interact(
        track,
        g.id.with("vbar"),
        egui::Sense::CLICK | egui::Sense::DRAG,
    );
    if (resp.dragged() || resp.clicked())
        && let Some(pos) = resp.interact_pointer_pos()
        && travel > 0.0
    {
        // The centre of the thumb follows the pointer
        let t = ((pos.y - handle_h / 2.0 - track.top()) / travel).clamp(0.0, 1.0);
        sel.scroll_to_y = Some(t * max);
        ui.ctx().request_repaint();
    }

    let p = theme::get(ui);
    let active = resp.hovered() || resp.dragged();
    if active {
        ui.painter().rect_filled(track, 0.0, p.bg_sunken);
    }
    let color = if active {
        p.text_muted
    } else {
        p.border_strong
    };
    ui.painter().rect_filled(handle, 3.0, color);
}

/// Initial width of each column according to its header and a sample of
/// the content.
fn initial_col_widths(ui: &egui::Ui, data: &QueryResult) -> Vec<f32> {
    // Cells are painted in monospace (JetBrains Mono, advance ~0.6 em)
    let font_size = egui::TextStyle::Monospace.resolve(ui.style()).size;
    let char_w = font_size * 0.62;
    let sort_reserve = font_size * 1.8;
    let padding = 8.0;

    let sample = data.rows.len().min(200);

    data.columns
        .iter()
        .enumerate()
        .map(|(col_i, col_name)| {
            let header_w = col_name.chars().count() as f32 * char_w + sort_reserve + padding;

            let max_content = data.rows[..sample]
                .iter()
                .map(|row| {
                    row.get(col_i)
                        .and_then(|v| v.as_deref())
                        .map_or(0, |s| s.chars().count())
                })
                .max()
                .unwrap_or(0);
            let content_w = max_content as f32 * char_w + 8.0;

            header_w.max(content_w).clamp(50.0, 400.0)
        })
        .collect()
}

/// Header of a column: key icon, name and position in the sort order.
/// Returns true if it was clicked to sort by it.
fn header_cell(ui: &mut egui::Ui, g: &Grid, col: usize) -> bool {
    let sort_pos = g.sorts.iter().position(|s| s.col_idx == col);
    let icon = match g.column_info.get(col).map(|c| c.column_key.as_str()) {
        Some("PRI") => Some(KEY),
        Some("UNI") => Some("U"),
        Some("MUL") => Some("I"),
        _ => None,
    };

    let name = &g.data.columns[col];
    let mut label = match icon {
        Some(icon) => format!("{icon} {name}"),
        None => name.clone(),
    };
    if let Some(pos) = sort_pos {
        label.push_str(if g.sorts[pos].dir == SortDir::Asc {
            " ↑"
        } else {
            " ↓"
        });
        if g.sorts.len() > 1 {
            label.push_str(&(pos + 1).to_string());
        }
    }

    let p = theme::get(ui);
    let color = if sort_pos.is_some() {
        p.accent
    } else {
        p.text_muted
    };
    let text = egui::RichText::new(label).color(color);
    if !g.queryable {
        ui.label(text);
        return false;
    }

    let hint = match sort_pos {
        None => t!("grid-sort-asc-hint"),
        Some(i) if g.sorts[i].dir == SortDir::Asc => t!("grid-sort-desc-hint"),
        _ => t!("grid-sort-off-hint"),
    };
    ui.add(egui::Button::new(text).frame(false))
        .on_hover_text(hint)
        .clicked()
}

/// A cell: background according to its row's state, text, cursor and the
/// mouse interaction. `highlight` is the value of the cursor's cell: the
/// other cells showing it are highlighted.
#[allow(clippy::too_many_arguments)]
fn show_cell(
    ui: &mut egui::Ui,
    g: &Grid,
    sel: &mut GridSelection,
    editing: &mut Option<EditingCell>,
    out: &mut TableOut,
    drag_row: &mut Option<usize>,
    focused: bool,
    highlight: Option<&str>,
    cell: CellRef,
) {
    let rect = ui.available_rect_before_wrap();
    let p = theme::get(ui);
    let deleted = g.is_deleted(cell.row);
    let inserted = matches!(cell.row, RowRef::Insert(_));
    let row_selected = matches!(cell.row, RowRef::Existing(r) if sel.rows.contains(&r)) && !deleted;
    let edit = match cell.row {
        RowRef::Existing(r) if !deleted => g.pending_edit(r, cell.col),
        _ => None,
    };
    let is_cursor = sel.cell == Some(cell);
    let shown = g.shown(cell);
    // Deleted rows are not highlighted: their red background is the warning
    let same_value = !is_cursor && !deleted && highlight.is_some_and(|v| shown == Shown::Value(v));

    // Row background (opaque: it does not blend with the stripes)
    let row_bg = if deleted {
        Some(p.grid_delete_bg)
    } else if inserted {
        Some(p.grid_insert_bg)
    } else if row_selected {
        Some(p.grid_sel_row_bg)
    } else {
        None
    };
    if let Some(bg) = row_bg {
        ui.painter().rect_filled(rect, 0.0, bg);
    }

    if editing.as_ref().is_some_and(|e| e.cell == cell) {
        show_editor(ui, g, sel, editing, out);
        return;
    }

    if edit.is_some() {
        // Cell with a pending change: background and marked corner (this one,
        // above the highlight)
        ui.painter().rect_filled(rect, 0.0, p.grid_edit_bg);
    } else if let RowRef::Existing(r) = cell.row
        && !deleted
    {
        // Faint background if the row has changes in other cells
        if g.pending_edits.iter().any(|e| e.row_idx == r) {
            ui.painter()
                .rect_filled(rect, 0.0, p.grid_edit_bg.gamma_multiply(0.5));
        }
        if is_cursor && !row_selected {
            ui.painter()
                .rect_filled(rect, 0.0, ui.visuals().selection.bg_fill);
        }
    }
    if same_value {
        ui.painter().rect_filled(rect, 0.0, p.grid_match_bg);
    }
    if edit.is_some() {
        let corner = vec![
            rect.right_top(),
            rect.right_top() - egui::vec2(8.0, 0.0),
            rect.right_top() + egui::vec2(0.0, 8.0),
        ];
        ui.painter().add(egui::Shape::convex_polygon(
            corner,
            p.warning,
            egui::Stroke::NONE,
        ));
    }
    if is_cursor {
        // The border shows on any background; dimmed if the grid does not
        // have keyboard focus
        let color = if focused { p.accent } else { p.border_strong };
        ui.painter().rect_stroke(
            rect,
            0.0,
            egui::Stroke::new(1.5, color),
            egui::StrokeKind::Inside,
        );
    }

    let normal_fg = if deleted {
        p.grid_delete_fg
    } else if edit.is_some() {
        p.grid_edit_fg
    } else {
        p.null_text
    };
    let (text, color) = match shown {
        Shown::Value(v) if deleted => (v, p.grid_delete_fg),
        Shown::Value(v) if edit.is_some() => (v, p.grid_edit_fg),
        Shown::Value(v) if inserted => (v, p.grid_insert_fg),
        Shown::Value(v) => (v, ui.visuals().text_color()),
        Shown::Null => ("NULL", normal_fg),
        Shown::Default => ("(DEFAULT)", p.null_text),
        Shown::Auto => ("(AUTO)", p.null_text),
    };

    // No FOCUSABLE: keyboard focus belongs to the grid, not to each cell
    let resp = ui.allocate_rect(rect, egui::Sense::CLICK | egui::Sense::DRAG);
    paint_cell_text(ui, rect, text, color);

    if let RowRef::Existing(r) = cell.row {
        if resp.drag_started() {
            sel.select_row(r, cell.col, false, false);
            sel.drag_from = Some(r);
            *drag_row = Some(r);
        }
        // While dragging, the row comes from the pointer position
        if ui.input(|i| i.pointer.is_decidedly_dragging())
            && let Some(pos) = ui.input(|i| i.pointer.hover_pos())
            && rect.y_range().contains(pos.y)
        {
            *drag_row = Some(r);
        }
    }

    if resp.clicked() {
        let (shift, ctrl) = ui.input(|i| (i.modifiers.shift, i.modifiers.command));
        match cell.row {
            RowRef::Existing(r) if shift || ctrl => sel.select_row(r, cell.col, shift, ctrl),
            _ => sel.select_cell(cell),
        }
    }
    // The menu acts on the clicked cell, except inside a row selection,
    // which is kept
    if resp.secondary_clicked() && !row_selected {
        sel.select_cell(cell);
    }
    if resp.double_clicked() && g.can_edit(cell) {
        *editing = Some(EditingCell::new(cell, g.edit_text(cell)));
    }

    resp.context_menu(|ui| {
        if let Some(action) = menu::cell(ui, g, sel, editing, cell) {
            out.action = action;
            ui.memory_mut(|m| m.request_focus(g.focus_id()));
        }
    });

    // × to remove the new row, in its last column
    if let RowRef::Insert(insert_idx) = cell.row
        && cell.col + 1 == g.cols()
        && g.editable
        && remove_button(ui, rect)
    {
        out.action = GridAction::RemoveInsertRow { insert_idx };
    }
}

/// Editor of the cell being edited. Enter, Tab or losing focus confirm
/// (if the text changed); Escape cancels. Tab moves on to edit the next editable cell (Shift+Tab,
/// the previous one).
fn show_editor(
    ui: &mut egui::Ui,
    g: &Grid,
    sel: &mut GridSelection,
    editing: &mut Option<EditingCell>,
    out: &mut TableOut,
) {
    let Some(edit) = editing.as_mut() else {
        return;
    };
    let cell = edit.cell;
    // lock_focus: with Tab, egui does not move the focus to another widget
    let resp = ui.add(
        egui::TextEdit::singleline(&mut edit.buffer)
            // The same font as the cells: the text does not change when editing
            .font(egui::TextStyle::Monospace)
            .desired_width(f32::INFINITY)
            .lock_focus(true),
    );
    if edit.needs_focus {
        // First frame: this frame's keys (the Enter that opened the editor)
        // are not meant for it
        resp.request_focus();
        edit.needs_focus = false;
        return;
    }

    let tab = if resp.has_focus() {
        ui.input_mut(|i| {
            if i.consume_key(egui::Modifiers::SHIFT, egui::Key::Tab) {
                Some(false)
            } else if i.consume_key(egui::Modifiers::NONE, egui::Key::Tab) {
                Some(true)
            } else {
                None
            }
        })
    } else {
        None
    };
    let (escape, enter) = ui.input(|i| {
        (
            i.key_pressed(egui::Key::Escape),
            i.key_pressed(egui::Key::Enter),
        )
    });
    if tab.is_none() && !escape && !resp.lost_focus() {
        return;
    }

    let value = std::mem::take(&mut edit.buffer);
    let changed = value != edit.original;
    *editing = None;
    if changed && !escape {
        out.action = GridAction::CellEdited {
            row: cell.row,
            col_idx: cell.col,
            value: Some(value),
        };
    }

    let next = tab.and_then(|forward| g.next_editable(cell, forward));
    if let Some(next) = next {
        sel.move_to(next);
        *editing = Some(EditingCell::new(next, g.edit_text(next)));
    } else if tab.is_some() || escape || enter {
        // It ended with the keyboard: focus goes back to the grid. If it was
        // by clicking elsewhere, the focus belongs to whatever was clicked.
        sel.move_to(cell);
        ui.memory_mut(|m| m.request_focus(g.focus_id()));
    }
}

/// Paints a cell's text in monospace, truncated with "…" if it does not fit.
fn paint_cell_text(ui: &egui::Ui, rect: egui::Rect, text: &str, color: egui::Color32) {
    if !ui.is_rect_visible(rect) {
        return;
    }
    let clip = rect.shrink2(egui::vec2(4.0, 0.0));
    let font_id = egui::TextStyle::Monospace.resolve(ui.style());
    let mut job = egui::text::LayoutJob {
        wrap: egui::text::TextWrapping {
            max_width: clip.width(),
            max_rows: 1,
            break_anywhere: false,
            overflow_character: Some('…'),
        },
        ..Default::default()
    };
    job.append(
        text,
        0.0,
        egui::text::TextFormat {
            font_id,
            color,
            ..Default::default()
        },
    );
    let painter = ui.painter().with_clip_rect(clip);
    let galley = painter.layout_job(job);
    let pos = egui::pos2(clip.min.x, clip.center().y - galley.size().y * 0.5);
    painter.galley(pos, galley, color);
}

/// × button at the end of the cell. Returns true if it was clicked.
fn remove_button(ui: &mut egui::Ui, cell_rect: egui::Rect) -> bool {
    let p = theme::get(ui);
    let size = egui::vec2(16.0, 16.0);
    let rect = egui::Rect::from_min_size(
        egui::pos2(
            cell_rect.max.x - size.x - 2.0,
            cell_rect.center().y - size.y * 0.5,
        ),
        size,
    );
    let resp = ui
        .allocate_rect(rect, egui::Sense::CLICK)
        .on_hover_text(t!("grid-remove-new-row"));
    if resp.hovered() {
        ui.painter().rect_filled(rect, 3.0, p.danger);
    }
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        "×",
        egui::FontId::proportional(12.0),
        if resp.hovered() {
            p.on_accent
        } else {
            p.danger
        },
    );
    resp.clicked()
}

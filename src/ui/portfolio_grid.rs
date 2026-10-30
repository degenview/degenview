//! Painter-drawn table primitives for the portfolio tabs: proportional column widths, the
//! rounded container, a sortable header cell and aligned cell text. Rows are painted at
//! measured positions, so columns line up with their headers whatever the content.

use crate::ui::accessibility;
use crate::ui::icons::{self, Icon};
use crate::ui::kit::{self, REGULAR, SEMIBOLD};
use crate::ui::theme;
use eframe::egui::{
    self, Align2, Color32, CursorIcon, Id, Key, Modifiers, Pos2, Rect, Response, Sense, Shape,
    Stroke, StrokeKind, Ui, Vec2, pos2, vec2,
};

/// Side margin of a table inside its tab.
pub const INSET: f32 = 16.0;
pub const HEADER_HEIGHT: f32 = 30.0;
/// Rows and their highlights sit this far in from the container's edge, so a highlight's
/// rounded corners never poke past the container's own.
pub const ROW_INSET: f32 = 6.0;
/// Space between a cell's edge and its text.
pub const CELL_PAD: f32 = 10.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Col {
    /// The narrowest the column gets.
    pub min: f32,
    /// Its share of the width: columns grow in proportion to this.
    pub ideal: f32,
    pub right: bool,
    /// Extra space kept clear at the cell's trailing edge (for a row's action button).
    pub trailing: f32,
    /// Whether clicking the header sorts by this column.
    pub sortable: bool,
}

impl Col {
    pub const fn new(min: f32, ideal: f32, right: bool) -> Self {
        Self {
            min,
            ideal,
            right,
            trailing: 0.0,
            sortable: true,
        }
    }

    pub const fn unsortable(mut self) -> Self {
        self.sortable = false;
        self
    }

    pub const fn trailing(mut self, trailing: f32) -> Self {
        self.trailing = trailing;
        self
    }
}

/// Column widths for `available` points: each column takes its share of `ideal`, none goes
/// under its `min`, and the others give up the difference. Narrower than every `min` and the
/// columns keep their minimums (the overflow is clipped).
pub fn widths(columns: &[Col], available: f32) -> Vec<f32> {
    let mut fixed = vec![false; columns.len()];
    let mut result = vec![0.0; columns.len()];
    loop {
        let pinned: f32 = columns
            .iter()
            .zip(&fixed)
            .filter(|(_, fixed)| **fixed)
            .map(|(column, _)| column.min)
            .sum();
        let share: f32 = columns
            .iter()
            .zip(&fixed)
            .filter(|(_, fixed)| !**fixed)
            .map(|(column, _)| column.ideal)
            .sum();
        if share <= 0.0 {
            break;
        }
        let scale = ((available - pinned) / share).max(0.0);
        let mut changed = false;
        for (index, column) in columns.iter().enumerate() {
            if fixed[index] {
                result[index] = column.min;
                continue;
            }
            let wanted = column.ideal * scale;
            if wanted < column.min {
                fixed[index] = true;
                result[index] = column.min;
                changed = true;
            } else {
                result[index] = wanted;
            }
        }
        if !changed {
            break;
        }
    }
    result
}

/// The columns' rects side by side from `left`, `height` tall starting at `top`.
pub fn column_rects(
    columns: &[Col],
    left: f32,
    top: f32,
    height: f32,
    available: f32,
) -> Vec<Rect> {
    let mut x = left;
    widths(columns, available)
        .into_iter()
        .map(|width| {
            let rect = Rect::from_min_size(pos2(x, top), vec2(width, height));
            x += width;
            rect
        })
        .collect()
}

/// The rounded, hairline-bordered area a table lives in.
pub fn paint_frame(ui: &Ui, rect: Rect) {
    ui.painter().rect_filled(rect, 8.0, theme::surface());
    ui.painter().rect_stroke(
        rect,
        8.0,
        Stroke::new(1.0, theme::separator()),
        StrokeKind::Inside,
    );
}

/// A page-level table's card: rounded at the top, and open at the bottom where it meets the
/// window's edge, so it fills the tab down to the bottom.
pub fn paint_page_frame(ui: &Ui, rect: Rect) {
    const RADIUS: f32 = 8.0;
    ui.painter().rect_filled(
        rect,
        egui::CornerRadius {
            nw: RADIUS as u8,
            ne: RADIUS as u8,
            sw: 0,
            se: 0,
        },
        theme::surface(),
    );
    // The outline: up the left side, round the top corners, down the right side.
    let edge = rect.shrink(0.5);
    let arc = |center: Pos2, from: f32| -> Vec<Pos2> {
        (0..=8)
            .map(|step| {
                let angle = from + std::f32::consts::FRAC_PI_2 * step as f32 / 8.0;
                center + Vec2::angled(angle) * (RADIUS - 0.5)
            })
            .collect()
    };
    let mut outline = vec![pos2(edge.left(), edge.bottom())];
    outline.extend(arc(
        pos2(edge.left() + RADIUS - 0.5, edge.top() + RADIUS - 0.5),
        std::f32::consts::PI,
    ));
    outline.extend(arc(
        pos2(edge.right() - RADIUS + 0.5, edge.top() + RADIUS - 0.5),
        std::f32::consts::PI * 1.5,
    ));
    outline.push(pos2(edge.right(), edge.bottom()));
    ui.painter()
        .add(Shape::line(outline, Stroke::new(1.0, theme::separator())));
}

/// A small filled triangle for a sort direction.
pub fn paint_sort_arrow(ui: &Ui, center: Pos2, ascending: bool, color: Color32) {
    let (half_width, half_height) = (3.5, 2.5);
    let points = if ascending {
        vec![
            pos2(center.x - half_width, center.y + half_height),
            pos2(center.x + half_width, center.y + half_height),
            pos2(center.x, center.y - half_height),
        ]
    } else {
        vec![
            pos2(center.x - half_width, center.y - half_height),
            pos2(center.x + half_width, center.y - half_height),
            pos2(center.x, center.y + half_height),
        ]
    };
    ui.painter()
        .add(Shape::convex_polygon(points, color, Stroke::NONE));
}

/// A header cell in `rect`: the title (primary and arrowed while sorted by it), a wash on
/// hover. Returns whether it was clicked.
pub fn header_cell(
    ui: &mut Ui,
    id: Id,
    rect: Rect,
    column: &Col,
    title: &str,
    sorted: Option<bool>,
) -> bool {
    let response = ui.interact(
        rect,
        id,
        if column.sortable {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let response = if column.sortable {
        response.on_hover_cursor(CursorIcon::PointingHand)
    } else {
        response
    };
    if column.sortable && response.hovered() {
        ui.painter()
            .rect_filled(rect.shrink2(vec2(2.0, 3.0)), 5.0, theme::ink(10));
    }
    let color = if sorted.is_some() {
        theme::text_primary()
    } else {
        theme::text_secondary()
    };
    let text_rect = text_span(rect, column);
    let galley = kit::galley(
        ui,
        title,
        kit::size::SUBHEADLINE,
        SEMIBOLD,
        color,
        text_rect.width(),
    );
    let x = if column.right {
        text_rect.right() - galley.size().x
    } else {
        text_rect.left()
    };
    let y = rect.center().y - galley.size().y / 2.0;
    let galley_width = galley.size().x;
    ui.painter()
        .galley(pos2(x, y), galley, Color32::PLACEHOLDER);
    if let Some(ascending) = sorted {
        // On the side facing away from the cell's edge, so titles keep lining up with figures.
        let arrow_x = if column.right {
            x - 9.0
        } else {
            x + galley_width + 9.0
        };
        paint_sort_arrow(ui, pos2(arrow_x, rect.center().y), ascending, color);
    }
    column.sortable && response.clicked()
}

/// Where a cell's text may go: the cell less its padding and trailing reserve.
pub fn text_span(cell: Rect, column: &Col) -> Rect {
    Rect::from_min_max(
        pos2(cell.left() + CELL_PAD, cell.top()),
        pos2(
            (cell.right() - CELL_PAD - column.trailing).max(cell.left() + CELL_PAD + 1.0),
            cell.bottom(),
        ),
    )
}

/// One line of text in `cell`, aligned as its column is and cut with an ellipsis if it does
/// not fit. Returns the rect it covers.
pub fn cell_text(
    ui: &Ui,
    cell: Rect,
    column: &Col,
    text: &str,
    size: f32,
    weight: f32,
    color: Color32,
) -> Rect {
    let span = text_span(cell, column);
    let galley = kit::galley(ui, text, size, weight, color, span.width());
    let anchor = if column.right {
        Align2::RIGHT_CENTER
    } else {
        Align2::LEFT_CENTER
    };
    let at = if column.right {
        pos2(span.right(), cell.center().y)
    } else {
        pos2(span.left(), cell.center().y)
    };
    let rect = anchor.anchor_size(at, galley.size());
    ui.painter().galley(rect.min, galley, Color32::PLACEHOLDER);
    rect
}

/// A figure in a table cell: regular weight, 13 pt.
pub fn figure(ui: &Ui, cell: Rect, column: &Col, text: &str, color: Color32) {
    cell_text(ui, cell, column, text, kit::size::BODY, REGULAR, color);
}

// MARK: Keyboard and scrolling

/// Keyboard focus and scroll bookkeeping for one table.
#[derive(Debug, Default)]
pub struct Navigation {
    /// Whether the arrow keys, Return and Delete act on the table: set by a click on a row,
    /// cleared while a text field has the keyboard.
    pub focused: bool,
    scroll_y: f32,
    viewport_height: f32,
    scroll_to: Option<f32>,
}

impl Navigation {
    /// Scrolls just far enough that row `index` is fully visible.
    fn reveal(&mut self, index: usize, row_height: f32) {
        let top = index as f32 * row_height;
        if top < self.scroll_y {
            self.scroll_to = Some(top);
        } else if top + row_height > self.scroll_y + self.viewport_height {
            self.scroll_to = Some(top + row_height - self.viewport_height);
        }
    }
}

/// What happened in the table this frame.
#[derive(Debug, Default, PartialEq)]
pub struct TableOutput {
    /// A sortable header was clicked.
    pub header_clicked: Option<usize>,
    /// A row was clicked (either button).
    pub row_clicked: Option<usize>,
    /// The arrow keys moved the selection here.
    pub moved_to: Option<usize>,
    /// Return was pressed on the table.
    pub activated: bool,
    /// Delete or Backspace was pressed on the table.
    pub deleted: bool,
}

pub struct TableSpec<'a> {
    pub id: &'a str,
    pub columns: &'a [Col],
    pub headers: &'a [String],
    pub row_height: f32,
    pub count: usize,
    /// The sorted column and its direction.
    pub sorted: Option<(usize, bool)>,
    /// The highlighted row.
    pub selected: Option<usize>,
    /// Whether the card ends in a footer strip the rows must stop above.
    pub has_footer: bool,
}

/// One row, handed to the painter closure.
pub struct RowContext<'a> {
    pub index: usize,
    pub cells: &'a [Rect],
    pub response: &'a Response,
}

/// Draws the table inside `outer` (already framed by the caller): a sticky header, then
/// `count` rows in a scroll area. `sorted` is the sorted column and its direction, `selected`
/// the highlighted row. `paint_row` draws a row's cells and menus.
pub fn show_table(
    ui: &mut Ui,
    outer: Rect,
    spec: &TableSpec,
    navigation: &mut Navigation,
    mut paint_row: impl FnMut(&mut Ui, &RowContext),
) -> TableOutput {
    let (count, sorted, selected) = (spec.count, spec.sorted, spec.selected);
    let mut output = TableOutput::default();
    let inner_left = outer.left() + ROW_INSET;
    let inner_width = outer.width() - 2.0 * ROW_INSET;

    let header_rects = column_rects(
        spec.columns,
        inner_left,
        outer.top(),
        HEADER_HEIGHT,
        inner_width,
    );
    for (index, (column, rect)) in spec.columns.iter().zip(&header_rects).enumerate() {
        let direction =
            sorted.and_then(|(column, ascending)| (column == index).then_some(ascending));
        if header_cell(
            ui,
            Id::new((spec.id, "header", index)),
            *rect,
            column,
            &spec.headers[index],
            direction,
        ) {
            output.header_clicked = Some(index);
        }
    }
    ui.painter().hline(
        outer.x_range().shrink(1.0),
        outer.top() + HEADER_HEIGHT,
        Stroke::new(1.0, theme::separator()),
    );

    // Keyboard: arrows move the selection, Return activates, Delete removes.
    if navigation.focused && ui.memory(|memory| memory.focused().is_none()) && count > 0 {
        let step = ui.input_mut(|input| {
            i32::from(input.consume_key(Modifiers::NONE, Key::ArrowDown))
                - i32::from(input.consume_key(Modifiers::NONE, Key::ArrowUp))
        });
        if step != 0 {
            let next = match selected {
                Some(index) => (index as i32 + step).clamp(0, count as i32 - 1) as usize,
                None if step > 0 => 0,
                None => count - 1,
            };
            output.moved_to = Some(next);
            navigation.reveal(next, spec.row_height);
        }
        output.activated = ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter));
        output.deleted = ui.input_mut(|input| {
            input.consume_key(Modifiers::NONE, Key::Delete)
                | input.consume_key(Modifiers::NONE, Key::Backspace)
        });
    }

    let body = Rect::from_min_max(
        pos2(outer.left(), outer.top() + HEADER_HEIGHT + BODY_PADDING),
        pos2(
            outer.right(),
            outer.bottom() - BODY_PADDING - if spec.has_footer { FOOTER_STRIP } else { 0.0 },
        ),
    );
    let mut body_ui = ui.new_child(egui::UiBuilder::new().max_rect(body));
    body_ui.set_clip_rect(body.intersect(ui.clip_rect()));
    body_ui.spacing_mut().item_spacing = Vec2::ZERO;
    let mut area = egui::ScrollArea::vertical()
        .id_salt((spec.id, "rows"))
        .auto_shrink([false, false])
        .max_height(body.height());
    if let Some(target) = navigation.scroll_to.take() {
        area = area.vertical_scroll_offset(target.max(0.0));
    }
    let scrolled = area.show_rows(&mut body_ui, spec.row_height, count, |ui, range| {
        for index in range {
            let (slot, response) =
                ui.allocate_exact_size(vec2(ui.available_width(), spec.row_height), Sense::click());
            let row = Rect::from_min_max(
                pos2(inner_left, slot.top() + 1.0),
                pos2(inner_left + inner_width, slot.bottom() - 1.0),
            );
            if selected == Some(index) {
                ui.painter()
                    .rect_filled(row, 7.0, theme::selected_tint(false));
            } else if response.hovered() {
                ui.painter().rect_filled(row, 7.0, theme::ink(12));
            } else if index % 2 == 1 {
                ui.painter().rect_filled(row, 7.0, theme::ink(6));
            }
            let cells = column_rects(
                spec.columns,
                row.left(),
                row.top(),
                row.height(),
                row.width(),
            );
            if response.clicked() || response.secondary_clicked() {
                output.row_clicked = Some(index);
                navigation.focused = true;
            }
            paint_row(
                ui,
                &RowContext {
                    index,
                    cells: &cells,
                    response: &response,
                },
            );
        }
    });
    navigation.scroll_y = scrolled.state.offset.y;
    navigation.viewport_height = scrolled.inner_rect.height();
    output
}

/// Space between the header rule and the first row, and under the last.
const BODY_PADDING: f32 = 4.0;

// MARK: Row furniture

/// The always-visible `⋯` button at the end of a row's last column, and the menu it opens.
/// Returns the button's response; `menu` fills the popup.
pub fn row_menu_button(
    ui: &mut Ui,
    last_cell: Rect,
    id: Id,
    label: &str,
    menu: impl FnOnce(&mut Ui),
) -> Response {
    let button = Rect::from_center_size(
        pos2(
            last_cell.right() - 8.0 - MENU_BUTTON / 2.0,
            last_cell.center().y,
        ),
        Vec2::splat(MENU_BUTTON),
    );
    let response = ui
        .interact(button, id, Sense::click())
        .on_hover_cursor(CursorIcon::PointingHand);
    let open = egui::Popup::is_id_open(ui.ctx(), id.with("popup"));
    let lit = response.hovered() || open;
    if lit {
        ui.painter().rect_filled(button, 6.0, theme::ink(18));
    }
    icons::paint(
        ui.painter(),
        Rect::from_center_size(button.center(), Vec2::splat(15.0)),
        Icon::Ellipsis,
        if lit {
            theme::text_primary()
        } else {
            theme::text_secondary()
        },
    );
    let response = accessibility::label(response, label.to_owned());
    egui::Popup::menu(&response)
        .id(id.with("popup"))
        .frame(kit::menu_frame())
        .gap(4.0)
        .show(menu);
    response
}

/// Width reserved at the end of a column for `row_menu_button`.
pub const MENU_BUTTON: f32 = 24.0;
pub const MENU_RESERVE: f32 = MENU_BUTTON + 6.0;

/// A bordered drop-down button; `items` fills its menu. Highlighted while a filter is set.
pub fn dropdown(ui: &mut Ui, id: Id, label: &str, active: bool, items: impl FnOnce(&mut Ui)) {
    let color = if active {
        theme::text_primary()
    } else {
        theme::text_secondary()
    };
    let text = kit::galley(ui, label, kit::size::BODY, REGULAR, color, 220.0);
    let size = vec2(text.size().x + 8.0 + 14.0 + 22.0, 26.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let response = response.on_hover_cursor(CursorIcon::PointingHand);
    let open = egui::Popup::is_id_open(ui.ctx(), id);
    let fill = if open {
        theme::quaternary(1.0)
    } else if response.hovered() {
        theme::quaternary(0.75)
    } else {
        theme::quaternary(0.5)
    };
    ui.painter().rect_filled(rect, 7.0, fill);
    ui.painter().rect_stroke(
        rect,
        7.0,
        Stroke::new(1.0, theme::separator().gamma_multiply(0.6)),
        StrokeKind::Inside,
    );
    ui.painter().galley(
        pos2(rect.left() + 11.0, rect.center().y - text.size().y / 2.0),
        text,
        Color32::PLACEHOLDER,
    );
    icons::paint(
        ui.painter(),
        Rect::from_center_size(
            pos2(rect.right() - 12.0, rect.center().y),
            Vec2::splat(10.0),
        ),
        Icon::ChevronUpDown,
        theme::text_secondary(),
    );
    egui::Popup::menu(&response)
        .id(id)
        .frame(kit::menu_frame())
        .gap(4.0)
        .show(|ui| {
            ui.set_min_width(170.0);
            ui.spacing_mut().item_spacing.y = 0.0;
            items(ui);
        });
}

/// A text link in the accent colour; returns whether it was clicked.
pub fn link(ui: &mut Ui, text: &str) -> bool {
    ui.add(
        egui::Label::new(kit::text(
            text,
            kit::size::BODY,
            REGULAR,
            theme::SYSTEM_ACCENT,
        ))
        .sense(Sense::click()),
    )
    .on_hover_cursor(CursorIcon::PointingHand)
    .clicked()
}

/// The strip inside the bottom of a table card that holds its count and note.
pub const FOOTER_STRIP: f32 = 32.0;

/// The part of a table card above its footer strip: where rows, and the loading and empty
/// messages, live.
pub fn body_rect(outer: Rect) -> Rect {
    Rect::from_min_max(
        outer.min,
        pos2(outer.right(), outer.bottom() - FOOTER_STRIP),
    )
}

/// The count ("8 assets") at the card's left and `note` opposite, in a strip inside the card's
/// bottom edge under a hairline, so the card itself reaches the bottom of the window.
pub fn footer(ui: &mut Ui, outer: Rect, count: &str, note: &str) {
    let strip = Rect::from_min_max(pos2(outer.left(), outer.bottom() - FOOTER_STRIP), outer.max);
    ui.painter().hline(
        strip.x_range().shrink(1.0),
        strip.top(),
        Stroke::new(1.0, theme::separator()),
    );
    let middle = strip.center().y;
    let secondary = theme::text_secondary();
    let left = kit::galley(ui, count, kit::size::CAPTION, REGULAR, secondary, 400.0);
    ui.painter().galley(
        pos2(strip.left() + 14.0, middle - left.size().y / 2.0),
        left,
        Color32::PLACEHOLDER,
    );
    let right = kit::galley(ui, note, kit::size::CAPTION, REGULAR, secondary, 600.0);
    ui.painter().galley(
        pos2(
            strip.right() - 14.0 - right.size().x,
            middle - right.size().y / 2.0,
        ),
        right,
        Color32::PLACEHOLDER,
    );
}

/// The table's outer rect: inset from the sides and filling the height down to the bottom of
/// the tab. Advances the cursor past it.
pub fn allocate_table(ui: &mut Ui) -> Rect {
    let height = ui.available_height().max(140.0);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    Rect::from_min_max(
        pos2(rect.left() + INSET, rect.top()),
        pos2(rect.right() - INSET, rect.bottom()),
    )
}

/// A spinner and `message` in the middle of `area`.
pub fn loading_message(ui: &mut Ui, area: Rect, message: &str) {
    let label = kit::galley(
        ui,
        message,
        12.0,
        REGULAR,
        theme::text_secondary(),
        area.width(),
    );
    let width = 16.0 + 8.0 + label.size().x;
    let left = area.center().x - width / 2.0;
    ui.put(
        Rect::from_center_size(pos2(left + 8.0, area.center().y), Vec2::splat(16.0)),
        egui::Spinner::new().size(16.0),
    );
    ui.painter().galley(
        pos2(left + 24.0, area.center().y - label.size().y / 2.0),
        label,
        Color32::PLACEHOLDER,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    const COLUMNS: [Col; 3] = [
        Col::new(100.0, 200.0, false),
        Col::new(50.0, 100.0, true),
        Col::new(50.0, 100.0, true),
    ];

    #[test]
    fn widths_follow_the_ideal_shares_and_fill_the_space() {
        let widths = widths(&COLUMNS, 800.0);
        assert_eq!(widths, [400.0, 200.0, 200.0]);
        assert!((widths.iter().sum::<f32>() - 800.0).abs() < 1e-3);
    }

    #[test]
    fn a_column_never_goes_under_its_minimum() {
        // 240 shared 2:1:1 gives A 120, B 60, C 60: all fit.
        for (found, wanted) in widths(&COLUMNS, 240.0).into_iter().zip([120.0, 60.0, 60.0]) {
            assert!((found - wanted).abs() < 1e-3);
        }
        // 200 would give A 100, B 50, C 50: exactly the minimums.
        for (found, wanted) in widths(&COLUMNS, 200.0).into_iter().zip([100.0, 50.0, 50.0]) {
            assert!((found - wanted).abs() < 1e-3);
        }
        // Squeezed past the minimums they hold, and the overflow is clipped by the caller.
        assert_eq!(widths(&COLUMNS, 120.0), [100.0, 50.0, 50.0]);
    }

    #[test]
    fn a_pinned_column_hands_its_excess_to_the_others() {
        let columns = [Col::new(10.0, 100.0, false), Col::new(80.0, 10.0, true)];
        let widths = widths(&columns, 200.0);
        assert_eq!(widths[1], 80.0, "pinned at its minimum");
        assert!(
            (widths[0] - 120.0).abs() < 1e-3,
            "the rest goes to the other"
        );
    }

    #[test]
    fn column_rects_sit_side_by_side() {
        let rects = column_rects(&COLUMNS, 10.0, 20.0, 30.0, 400.0);
        assert_eq!(rects[0].left(), 10.0);
        assert_eq!(rects[1].left(), rects[0].right());
        assert_eq!(rects[2].left(), rects[1].right());
        assert_eq!((rects[0].top(), rects[0].height()), (20.0, 30.0));
    }

    #[test]
    fn trailing_space_is_kept_clear_of_text() {
        let column = Col::new(100.0, 100.0, true).trailing(30.0);
        let cell = Rect::from_min_size(pos2(0.0, 0.0), vec2(120.0, 40.0));
        let span = text_span(cell, &column);
        assert_eq!(
            (span.left(), span.right()),
            (CELL_PAD, 120.0 - CELL_PAD - 30.0)
        );
    }

    #[test]
    fn keyboard_moves_scroll_just_far_enough_to_show_the_row() {
        let mut navigation = Navigation {
            scroll_y: 100.0,
            viewport_height: 200.0,
            ..Navigation::default()
        };
        // Row 3 at 40 pt spans 120..160: already visible.
        navigation.reveal(3, 40.0);
        assert_eq!(navigation.scroll_to, None);
        // Row 1 spans 40..80: above the viewport, so its top becomes the offset.
        navigation.reveal(1, 40.0);
        assert_eq!(navigation.scroll_to, Some(40.0));
        // Row 9 spans 360..400: below, so its bottom meets the viewport's.
        navigation.reveal(9, 40.0);
        assert_eq!(navigation.scroll_to, Some(200.0));
    }
}

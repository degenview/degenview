//! The ladder: a row for each price near the market, with the resting size on each side, what was
//! stacked or pulled there, and what traded. Bars behind the figures are scaled to the largest
//! row. Scrolling moves the rows; the toolbar brings them back.

use super::{BookViewState, size_text, usd};
use crate::i18n::{tr, tr_c};
use crate::model::depth::{DepthSnapshot, Ladder, Price, Row, Scale};
use crate::model::taker_flow::Taker;
use crate::ui::accessibility;
use crate::ui::kit::{self, size};
use crate::ui::theme;
use eframe::egui::{
    self, Align2, Color32, FontId, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2,
};

const ROW_HEIGHT: f32 = 20.0;
const HEADER_HEIGHT: f32 = 22.0;
const PAD: f32 = 10.0;
const FONT: f32 = 12.0;
/// A pulled or stacked figure is coloured once it is this share of what rests at the price.
const NOTABLE_SHARE: f64 = 0.25;
const BAR_ALPHA: f32 = 0.22;

/// The x ranges of the five columns, left to right.
struct Columns {
    stack: (f32, f32),
    bid: (f32, f32),
    price: (f32, f32),
    ask: (f32, f32),
    traded: (f32, f32),
}

impl Columns {
    fn new(rect: Rect, price_width: f32) -> Self {
        let inner = rect.shrink2(vec2(PAD, 0.0));
        let rest = (inner.width() - price_width).max(0.0);
        let (stack, side) = (rest * 0.2, rest * 0.27);
        let mut x = inner.left();
        let mut next = |width: f32| {
            let range = (x, x + width);
            x += width;
            range
        };
        Self {
            stack: next(stack),
            bid: next(side),
            price: next(price_width),
            ask: next(side),
            traded: next(rest - stack - 2.0 * side),
        }
    }
}

/// What a line of the ladder is.
#[derive(Clone, Copy)]
pub(super) enum Line {
    Row(usize),
    Spread,
}

pub(super) fn show(ui: &mut Ui, state: &mut BookViewState, snapshot: &DepthSnapshot, height: f32) {
    let ladder = &snapshot.ladder;
    let (area, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let painter = ui.painter_at(area);
    let decimals = ladder.group.decimals();
    let widest = ladder.rows.first().map_or(0.0, |row| {
        kit::galley(
            ui,
            &row.price.format(decimals),
            FONT,
            kit::REGULAR,
            Color32::WHITE,
            f32::INFINITY,
        )
        .size()
        .x
    });
    let drawing = Drawing {
        painter: &painter,
        columns: Columns::new(area, widest + 16.0),
        scale: ladder.scale(),
        decimals,
    };
    header(ui, &painter, area, &drawing.columns);

    let body = Rect::from_min_max(pos2(area.left(), area.top() + HEADER_HEIGHT), area.max);
    let capacity = (body.height() / ROW_HEIGHT) as usize;
    note_rows(ui, state, capacity);
    let last_trade = ladder.last_trade_row().zip(snapshot.tape.last_trade());
    for (index, line) in lines(ladder, capacity).into_iter().enumerate() {
        let rect = Rect::from_min_size(
            pos2(body.left(), body.top() + index as f32 * ROW_HEIGHT),
            vec2(body.width(), ROW_HEIGHT),
        );
        match line {
            Line::Row(row) => {
                let marked = last_trade
                    .filter(|(at, _)| *at == row)
                    .map(|(_, trade)| taker_color(trade.taker));
                draw_row(ui, &drawing, rect, &ladder.rows[row], row, marked);
            }
            Line::Spread => draw_spread(&painter, rect, ladder, snapshot.tick),
        }
    }
    scroll(ui, state, ladder, area);
}

/// Remembers how many price rows `capacity` lines hold, for the next frame to ask the host for.
fn note_rows(ui: &Ui, state: &mut BookViewState, capacity: usize) {
    let rows = rows_in(capacity);
    if state.rows != rows {
        state.rows = rows;
        ui.ctx().request_repaint();
    }
}

/// How many price rows to ask for to fill `capacity` lines: all of them, the spread line taking
/// the place of the last row when it is in view; never none.
pub(super) fn rows_in(capacity: usize) -> usize {
    capacity.max(1)
}

/// The lines that fit `capacity`: the rows from the top, with the spread above the best bid's
/// row unless that is the first line, the last row falling off for it.
pub(super) fn lines(ladder: &Ladder, capacity: usize) -> Vec<Line> {
    let spread_above = ladder.best_bid_row().filter(|row| *row > 0);
    let mut lines = Vec::with_capacity(ladder.rows.len() + 1);
    for index in 0..ladder.rows.len() {
        if spread_above == Some(index) {
            lines.push(Line::Spread);
        }
        lines.push(Line::Row(index));
    }
    lines.truncate(capacity);
    lines
}

fn header(ui: &Ui, painter: &egui::Painter, area: Rect, columns: &Columns) {
    let color = theme::text_tertiary();
    let y = area.top() + HEADER_HEIGHT / 2.0;
    // A heading shrinks to the room of its column rather than run into the next one.
    let put = |text: &str, x: f32, align: Align2, room: f32| {
        let galley = kit::galley_fitted(ui, text, size::CAPTION, kit::REGULAR, color, room);
        let width = galley.size().x;
        let left = match align {
            Align2::RIGHT_CENTER => x - width,
            Align2::CENTER_CENTER => x - width / 2.0,
            _ => x,
        };
        painter.galley(
            pos2(left, y - galley.size().y / 2.0),
            galley,
            Color32::PLACEHOLDER,
        );
    };
    let room = |range: (f32, f32)| range.1 - range.0 - 6.0;
    put(
        tr!("Stack"),
        columns.stack.1 - 6.0,
        Align2::RIGHT_CENTER,
        room(columns.stack) + PAD,
    );
    put(
        tr!("Bid"),
        columns.bid.1 - 6.0,
        Align2::RIGHT_CENTER,
        room(columns.bid),
    );
    put(
        tr!("Price"),
        (columns.price.0 + columns.price.1) / 2.0,
        Align2::CENTER_CENTER,
        columns.price.1 - columns.price.0,
    );
    put(
        tr_c!("order book", "Ask"),
        columns.ask.0 + 6.0,
        Align2::LEFT_CENTER,
        room(columns.ask),
    );
    put(
        tr!("Traded"),
        columns.traded.1,
        Align2::RIGHT_CENTER,
        columns.traded.1 - columns.traded.0,
    );
    let tip = Rect::from_x_y_ranges(
        columns.stack.0 - PAD..=columns.stack.1,
        area.top()..=area.top() + HEADER_HEIGHT,
    );
    ui.interact(tip, ui.id().with("stack-header"), Sense::hover())
        .on_hover_text(tr!(
            "Pull/Stack: size added (+) or taken away (−) at the price over the last 10 seconds, not counting what traded"
        ));
    painter.hline(
        area.x_range(),
        area.top() + HEADER_HEIGHT - 0.5,
        Stroke::new(1.0, theme::separator()),
    );
}

/// What every row of the ladder is drawn with.
struct Drawing<'a> {
    painter: &'a egui::Painter,
    columns: Columns,
    scale: Scale,
    /// How many places a row's price is written with.
    decimals: u8,
}

/// Draws row `index` into `rect`; `last_trade` is the colour of its frame when the last trade
/// belongs to it.
fn draw_row(
    ui: &Ui,
    drawing: &Drawing<'_>,
    rect: Rect,
    row: &Row,
    index: usize,
    last_trade: Option<Color32>,
) {
    let Drawing {
        painter,
        columns,
        scale,
        decimals,
    } = drawing;
    let decimals = *decimals;
    let response = ui.interact(rect, ui.id().with(("ladder-row", index)), Sense::hover());
    if response.hovered() {
        painter.rect_filled(rect, 0.0, kit::primary(0.06));
    }
    let (top, bottom) = (rect.top() + 1.0, rect.bottom() - 1.0);
    let bar = |range: (f32, f32), toward_left: bool, share: f64, color: Color32| {
        let length = (range.1 - range.0) * share.clamp(0.0, 1.0) as f32;
        let (from, to) = if toward_left {
            (range.1 - length, range.1)
        } else {
            (range.0, range.0 + length)
        };
        painter.rect_filled(
            Rect::from_x_y_ranges(from..=to, top..=bottom),
            2.0,
            theme::with_alpha(color, BAR_ALPHA),
        );
    };
    let share = |value: f64, of: f64| if of > 0.0 { value / of } else { 0.0 };
    bar(columns.bid, true, share(row.bid, scale.size), theme::UP);
    bar(columns.ask, false, share(row.ask, scale.size), theme::DOWN);
    let traded = row.bought + row.sold;
    if traded > 0.0 {
        let total = share(traded, scale.traded);
        let width = (columns.traded.1 - columns.traded.0) * total as f32;
        let split = columns.traded.0 + width * share(row.bought, traded) as f32;
        for (from, to, color) in [
            (columns.traded.0, split, theme::UP),
            (split, columns.traded.0 + width, theme::DOWN),
        ] {
            painter.rect_filled(
                Rect::from_x_y_ranges(from..=to, top..=bottom),
                0.0,
                theme::with_alpha(color, BAR_ALPHA * 1.5),
            );
        }
    }

    let y = rect.center().y;
    let text = |content: String, x: f32, align: Align2, color: Color32| {
        painter.text(
            pos2(x, y),
            align,
            content,
            FontId::proportional(FONT),
            color,
        );
    };
    if row.bid > 0.0 {
        text(
            size_text(row.bid),
            columns.bid.1 - 6.0,
            Align2::RIGHT_CENTER,
            theme::text_primary(),
        );
    }
    if row.ask > 0.0 {
        text(
            size_text(row.ask),
            columns.ask.0 + 6.0,
            Align2::LEFT_CENTER,
            theme::text_primary(),
        );
    }
    if traded > 0.0 {
        text(
            size_text(traded),
            columns.traded.1,
            Align2::RIGHT_CENTER,
            theme::text_secondary(),
        );
    }
    if let Some((figure, color)) = stack_figure(row) {
        text(figure, columns.stack.1 - 6.0, Align2::RIGHT_CENTER, color);
    }
    let resting = row.bid > 0.0 || row.ask > 0.0;
    let price_color = if resting {
        theme::text_primary()
    } else {
        theme::text_tertiary()
    };
    text(
        row.price.format(decimals),
        (columns.price.0 + columns.price.1) / 2.0,
        Align2::CENTER_CENTER,
        price_color,
    );
    if let Some(color) = last_trade {
        painter.rect_stroke(
            Rect::from_x_y_ranges(columns.price.0 + 2.0..=columns.price.1 - 2.0, top..=bottom),
            3.0,
            Stroke::new(1.0, color),
            StrokeKind::Inside,
        );
    }
    let description = describe(row, decimals);
    accessibility::label_as(response, egui::WidgetType::Label, description.clone())
        .on_hover_text(description);
}

/// `+12` stacked or `−3.4` pulled, in colour once it is a notable share of what rests there.
pub(super) fn stack_figure(row: &Row) -> Option<(String, Color32)> {
    let change = row.pull_stack;
    if change.abs() < 5e-4 {
        return None;
    }
    let sign = if change > 0.0 { '+' } else { '−' };
    let notable = change.abs() >= NOTABLE_SHARE * row.bid.max(row.ask);
    let color = match (notable, change > 0.0) {
        (false, _) => theme::text_tertiary(),
        (true, true) => theme::UP,
        (true, false) => theme::DOWN,
    };
    Some((format!("{sign}{}", size_text(change.abs())), color))
}

pub(super) fn taker_color(taker: Taker) -> Color32 {
    match taker {
        Taker::Buy => theme::UP,
        Taker::Sell => theme::DOWN,
    }
}

/// What a row says, for a hover and for a screen reader: sizes in the base coin, with what they
/// are worth. One whole message for each fact, a line each, so a language orders and punctuates
/// every one as it needs and none is a clause of another.
fn describe(row: &Row, decimals: u8) -> String {
    let worth = |size: f64| usd(size * row.price.as_f64());
    let mut lines = vec![tr!("Price {price}", price = row.price.format(decimals))];
    if row.bid > 0.0 {
        lines.push(tr!(
            "Bid {size} ({worth})",
            size = size_text(row.bid),
            worth = worth(row.bid)
        ));
    }
    if row.ask > 0.0 {
        lines.push(tr!(
            "Ask {size} ({worth})",
            size = size_text(row.ask),
            worth = worth(row.ask)
        ));
    }
    if row.bought > 0.0 || row.sold > 0.0 {
        lines.push(tr!(
            "Bought {bought}, sold {sold}",
            bought = size_text(row.bought),
            sold = size_text(row.sold)
        ));
    }
    if stack_figure(row).is_some() {
        let size = size_text(row.pull_stack.abs());
        lines.push(if row.pull_stack > 0.0 {
            tr!("Stacked {size} in the last 10 s", size = size)
        } else {
            tr!("Pulled {size} in the last 10 s", size = size)
        });
    }
    lines.join("\n")
}

fn draw_spread(painter: &egui::Painter, rect: Rect, ladder: &Ladder, tick: Price) {
    let Some(spread) = ladder.spread() else {
        return;
    };
    painter.rect_filled(rect, 0.0, theme::quaternary(0.28));
    let text = tr!("Spread {spread}", spread = spread.format(tick.decimals()));
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        FontId::proportional(size::CAPTION),
        theme::text_secondary(),
    );
}

/// Scrolling over the ladder moves its rows a row for each row's height; it stays where it was
/// put until the toolbar centres it again.
fn scroll(ui: &Ui, state: &mut BookViewState, ladder: &Ladder, area: Rect) {
    if !ui.rect_contains_pointer(area) {
        state.scrolled = 0.0;
        return;
    }
    state.scrolled += ui.input(|input| input.smooth_scroll_delta.y);
    let rows = (state.scrolled / ROW_HEIGHT).trunc();
    if rows == 0.0 {
        return;
    }
    state.scrolled -= rows * ROW_HEIGHT;
    if let Some(centre) = state.centre.or_else(|| ladder.centre()) {
        // Scrolling up shows higher prices.
        state.centre = Some(centre + ladder.group.times(rows as i64));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{Locale, with_locale};

    fn row(bid: f64, ask: f64, traded: (f64, f64), pull_stack: f64) -> Row {
        Row {
            price: Price::parse("60110").unwrap(),
            bid,
            ask,
            bought: traded.0,
            sold: traded.1,
            pull_stack,
        }
    }

    #[test]
    fn a_row_is_described_one_whole_message_a_line() {
        assert_eq!(describe(&row(0.0, 0.0, (0.0, 0.0), 0.0), 0), "Price 60,110");
        let busy = describe(&row(0.5, 0.0, (0.1, 0.2), -0.25), 0);
        let lines: Vec<&str> = busy.lines().collect();
        assert_eq!(lines.len(), 4, "{busy}");
        assert_eq!(lines[0], "Price 60,110");
        assert!(lines[1].starts_with("Bid 0.500 ("), "{}", lines[1]);
        assert_eq!(lines[2], "Bought 0.100, sold 0.200");
        assert_eq!(lines[3], "Pulled 0.250 in the last 10 s");
        let stacked = describe(&row(0.0, 2.0, (0.0, 0.0), 0.5), 0);
        assert!(stacked.contains("Ask 2.00 ("), "{stacked}");
        assert!(
            stacked.ends_with("Stacked 0.500 in the last 10 s"),
            "{stacked}"
        );
    }

    #[test]
    fn every_line_of_a_row_follows_the_language() {
        let text = with_locale(Locale::PSEUDO, || {
            describe(&row(0.5, 2.0, (0.1, 0.2), 0.5), 0)
        });
        assert_eq!(text.lines().count(), 5, "{text}");
        for line in text.lines() {
            assert!(line.starts_with('[') && line.ends_with(']'), "{line}");
        }
    }
}

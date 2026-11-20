//! The ladder: a row for each price near the market, with the resting size on each side, what was
//! stacked or pulled there, and what traded. Bars behind the figures are scaled to the largest in
//! view. Scrolling moves the window; the toolbar brings it back.

use super::{BookViewState, size_text};
use crate::model::depth::{CENTRE_ROW, Ladder, Price, Row, WINDOW_ROWS};
use crate::model::taker_flow::Taker;
use crate::ui::accessibility;
use crate::ui::kit::{self, size};
use crate::ui::market_flow::usd;
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
const HEADER_TIP: &str = "Pull/Stack: size added (+) or taken away (−) at the price over the last 10 seconds, not counting what traded";

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

pub(super) fn show(ui: &mut Ui, state: &mut BookViewState, ladder: &Ladder, height: f32) {
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
    let columns = Columns::new(area, widest + 16.0);
    header(ui, &painter, area, &columns);

    let body = Rect::from_min_max(pos2(area.left(), area.top() + HEADER_HEIGHT), area.max);
    let lines = lines(ladder, (body.height() / ROW_HEIGHT) as usize);
    for (index, line) in lines.into_iter().enumerate() {
        let rect = Rect::from_min_size(
            pos2(body.left(), body.top() + index as f32 * ROW_HEIGHT),
            vec2(body.width(), ROW_HEIGHT),
        );
        match line {
            Line::Row(row) => draw_row(ui, &painter, rect, &columns, ladder, row),
            Line::Spread => draw_spread(&painter, rect, ladder),
        }
    }
    scroll(ui, state, ladder, area);
}

/// The lines that fit `capacity`, the rows around the middle of the window with the spread above
/// the best bid.
pub(super) fn lines(ladder: &Ladder, capacity: usize) -> Vec<Line> {
    let start = CENTRE_ROW.saturating_sub(capacity / 2);
    let end = (start + capacity).min(WINDOW_ROWS);
    let bid_row = ladder
        .best_bid
        .map(|bid| bid.floor_to(ladder.group))
        .and_then(|price| ladder.rows.iter().position(|row| row.price == price));
    let mut lines = Vec::with_capacity(capacity);
    for index in start..end {
        if bid_row == Some(index) && index > start {
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
    let put = |text: &str, x: f32, align: Align2| {
        painter.text(
            pos2(x, y),
            align,
            text,
            FontId::proportional(size::CAPTION),
            color,
        );
    };
    put("Stack", columns.stack.1 - 6.0, Align2::RIGHT_CENTER);
    put("Bid", columns.bid.1 - 6.0, Align2::RIGHT_CENTER);
    put(
        "Price",
        (columns.price.0 + columns.price.1) / 2.0,
        Align2::CENTER_CENTER,
    );
    put("Ask", columns.ask.0 + 6.0, Align2::LEFT_CENTER);
    put("Traded", columns.traded.1, Align2::RIGHT_CENTER);
    let tip = Rect::from_x_y_ranges(
        columns.stack.0 - PAD..=columns.stack.1,
        area.top()..=area.top() + HEADER_HEIGHT,
    );
    ui.interact(tip, ui.id().with("stack-header"), Sense::hover())
        .on_hover_text(HEADER_TIP);
    painter.hline(
        area.x_range(),
        area.top() + HEADER_HEIGHT - 0.5,
        Stroke::new(1.0, theme::separator()),
    );
}

fn draw_row(
    ui: &Ui,
    painter: &egui::Painter,
    rect: Rect,
    columns: &Columns,
    ladder: &Ladder,
    index: usize,
) {
    let row = &ladder.rows[index];
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
    bar(
        columns.bid,
        true,
        share(row.bid, ladder.largest_size),
        theme::UP,
    );
    bar(
        columns.ask,
        false,
        share(row.ask, ladder.largest_size),
        theme::DOWN,
    );
    let traded = row.bought + row.sold;
    if traded > 0.0 {
        let total = share(traded, ladder.largest_traded);
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
    let decimals = ladder.group.decimals();
    text(
        row.price.format(decimals),
        (columns.price.0 + columns.price.1) / 2.0,
        Align2::CENTER_CENTER,
        price_color,
    );
    if last_trade_row(ladder) == Some(row.price) {
        let color = ladder
            .last_trade
            .map_or(theme::text_secondary(), |trade| taker_color(trade.taker));
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

/// The row the last trade belongs to: a buyer lifted an ask, a seller hit a bid.
fn last_trade_row(ladder: &Ladder) -> Option<Price> {
    let trade = ladder.last_trade?;
    Some(match trade.taker {
        Taker::Buy => trade.price.ceil_to(ladder.group),
        Taker::Sell => trade.price.floor_to(ladder.group),
    })
}

pub(super) fn taker_color(taker: Taker) -> Color32 {
    match taker {
        Taker::Buy => theme::UP,
        Taker::Sell => theme::DOWN,
    }
}

/// What a row says, for a hover and for a screen reader: sizes in the base coin, with what they
/// are worth.
fn describe(row: &Row, decimals: u8) -> String {
    let worth = |size: f64| usd(size * row.price.as_f64());
    let mut parts = vec![format!("Price {}", row.price.format(decimals))];
    if row.bid > 0.0 {
        parts.push(format!("bid {} ({})", size_text(row.bid), worth(row.bid)));
    }
    if row.ask > 0.0 {
        parts.push(format!("ask {} ({})", size_text(row.ask), worth(row.ask)));
    }
    if row.bought > 0.0 || row.sold > 0.0 {
        parts.push(format!(
            "bought {}, sold {}",
            size_text(row.bought),
            size_text(row.sold)
        ));
    }
    if let Some((figure, _)) = stack_figure(row) {
        parts.push(format!("stack {figure} in 10 s"));
    }
    parts.join(", ")
}

fn draw_spread(painter: &egui::Painter, rect: Rect, ladder: &Ladder) {
    let Some(spread) = ladder.spread() else {
        return;
    };
    painter.rect_filled(rect, 0.0, theme::quaternary(0.28));
    let text = format!("Spread {}", spread.format(ladder.tick.decimals()));
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        FontId::proportional(size::CAPTION),
        theme::text_secondary(),
    );
}

/// Scrolling over the ladder moves its window a row for each row's height; it stays where it was
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

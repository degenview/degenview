//! The market-flow panel, in three modes: the Liquidations (what the last minute, five minutes and
//! hour of forced liquidations added up to, and a tape of the latest ones), the Taker flow of the
//! focused chart's asset (who crossed the spread, by venue), and the Order book of the focused
//! chart's market (a ladder of resting size and a trades tape). Drawn from snapshots; nothing here
//! opens a socket.

mod book;
mod flow;
#[cfg(test)]
mod tests;

use crate::i18n::tr;
use crate::model::liquidation::{Liquidation, MinSize, Side, Snapshot, Totals};
use crate::model::taker_flow;
use crate::ui::icons::{self, Icon};
use crate::ui::kit::{self, MEDIUM, REGULAR, SEMIBOLD, size};
use crate::ui::theme;
pub use book::{BookBody, BookFeed, BookViewState};
use eframe::egui::{self, Align, Color32, Layout, Rect, Sense, Ui, pos2, vec2};

const HEADER_HEIGHT: f32 = 44.0;
const MARGIN: f32 = 12.0;
const ROW_HEIGHT: f32 = 26.0;
/// The least room each of the tape's columns keeps; [`TapeColumns`] widens them to their words.
const TIME_WIDTH: f32 = 64.0;
const SIDE_WIDTH: f32 = 44.0;
const SIZE_WIDTH: f32 = 68.0;
const FILTER_WIDTH: f32 = 116.0;
const BAR_HEIGHT: f32 = 4.0;
/// From this size a liquidation's figure is set heavier: a big one is what the eye is after.
const BIG: f64 = 100_000.0;

/// What the panel shows.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Mode {
    #[default]
    Liquidations,
    TakerFlow,
    Book,
}

/// What the panel remembers between frames.
#[derive(Default)]
pub struct MarketFlowState {
    pub mode: Mode,
    pub minimum: MinSize,
    pub book: BookViewState,
}

/// What the taker-flow mode draws: the asset it follows (none while the focused chart has no crypto
/// market) and what its venues have traded.
#[derive(Clone, Copy)]
pub struct FlowBody<'a> {
    pub asset: Option<&'a str>,
    pub snapshot: Option<&'a taker_flow::Snapshot>,
}

/// The data for the mode on show.
pub enum Body<'a> {
    Liquidations(&'a Snapshot),
    TakerFlow(FlowBody<'a>),
    Book(BookBody<'a>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Action {
    Close,
}

/// Longs liquidated are forced sales and read red; shorts, forced buys, green. Colour is data.
fn side_color(side: Side) -> Color32 {
    match side {
        Side::Long => theme::DOWN,
        Side::Short => theme::UP,
    }
}

fn side_name(side: Side) -> &'static str {
    match side {
        Side::Long => tr!("Long"),
        Side::Short => tr!("Short"),
    }
}

/// `$1.2M`, `$340K`, `$850`: a liquidation's worth in as few characters as keep it exact enough.
pub fn usd(value: f64) -> String {
    let value = value.abs();
    if value >= 1_000_000_000.0 {
        format!("${:.2}B", value / 1_000_000_000.0)
    } else if value >= 1_000_000.0 {
        format!("${:.2}M", value / 1_000_000.0)
    } else if value >= 10_000.0 {
        format!("${:.0}K", value / 1_000.0)
    } else if value >= 1_000.0 {
        format!("${:.1}K", value / 1_000.0)
    } else {
        format!("${value:.0}")
    }
}

pub fn show(ui: &mut Ui, state: &mut MarketFlowState, body: &Body<'_>) -> Option<Action> {
    let mut action = None;
    header(ui, state, &mut action);
    kit::divider(ui);
    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
    match body {
        Body::Liquidations(snapshot) => liquidations(ui, state, snapshot),
        Body::TakerFlow(body) => flow::show(ui, body),
        Body::Book(body) => book::show(ui, &mut state.book, body),
    }
    action
}

fn liquidations(ui: &mut Ui, state: &mut MarketFlowState, snapshot: &Snapshot) {
    totals(ui, snapshot);
    kit::divider(ui);
    let footnote_height = footnote_height(ui, liquidations_footnote());
    let list_height = (ui.available_height() - footnote_height).max(60.0);
    let (list, _) = ui.allocate_exact_size(vec2(ui.available_width(), list_height), Sense::hover());
    let mut list_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(list)
            .layout(Layout::top_down(Align::Min)),
    );
    tape(&mut list_ui, state, snapshot);
    footnote(ui, liquidations_footnote());
}

/// The room the close button takes at the right of the header, with the space before it.
const CLOSE_ROOM: f32 = 26.0 + MARGIN - 2.0 + 8.0;
/// The row the close button has to itself when the mode tabs do not fit beside it.
const CLOSE_ROW_HEIGHT: f32 = 32.0;

/// Whether the mode tabs and the close button share a row. Measured from the tabs' words, so a
/// language whose words are longer than English drops the close button to a row of its own
/// instead of laying it over the last tab.
fn tabs_fit_beside_close(tabs: f32, width: f32) -> bool {
    MARGIN + tabs + CLOSE_ROOM <= width
}

fn header(ui: &mut Ui, state: &mut MarketFlowState, action: &mut Option<Action>) {
    let width = ui.available_width();
    let modes = [
        kit::Segment::new(Mode::Liquidations, tr!("Liquidations")),
        kit::Segment::new(Mode::TakerFlow, tr!("Taker flow")),
        kit::Segment::new(Mode::Book, tr!("Book")),
    ];
    let id = egui::Id::new("market-flow-mode");
    if tabs_fit_beside_close(kit::segments_width(ui, &modes), width) {
        ui.allocate_ui_with_layout(
            vec2(width, HEADER_HEIGHT),
            Layout::left_to_right(Align::Center),
            |ui| {
                ui.spacing_mut().item_spacing = vec2(8.0, 0.0);
                ui.add_space(MARGIN);
                mode_tabs(ui, id, &modes, state);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    close_button(ui, action)
                });
            },
        );
    } else {
        ui.allocate_ui_with_layout(
            vec2(width, CLOSE_ROW_HEIGHT),
            Layout::right_to_left(Align::Center),
            |ui| close_button(ui, action),
        );
        ui.allocate_ui_with_layout(
            vec2(width, HEADER_HEIGHT - 8.0),
            Layout::left_to_right(Align::Center),
            |ui| {
                ui.add_space(MARGIN);
                mode_tabs(ui, id, &modes, state);
            },
        );
    }
}

fn mode_tabs(
    ui: &mut Ui,
    id: egui::Id,
    modes: &[kit::Segment<'_, Mode>],
    state: &mut MarketFlowState,
) {
    if let Some(mode) = kit::segments(ui, id, modes, state.mode, true) {
        state.mode = mode;
    }
}

fn close_button(ui: &mut Ui, action: &mut Option<Action>) {
    ui.add_space(MARGIN - 2.0);
    if icons::button(
        ui,
        Icon::Xmark,
        26.0,
        theme::text_secondary(),
        tr!("Close market flow"),
    )
    .clicked()
    {
        *action = Some(Action::Close);
    }
}

/// The last minute, five minutes and hour side by side, each with its total and how it splits
/// between longs and shorts.
fn totals(ui: &mut Ui, snapshot: &Snapshot) {
    let periods = [
        (tr!("Last minute"), snapshot.minute),
        (tr!("5 minutes"), snapshot.five_minutes),
        (tr!("Hour"), snapshot.hour),
    ];
    let width = ui.available_width() - 2.0 * MARGIN;
    let gap = 12.0;
    let column = (width - 2.0 * gap) / 3.0;
    ui.add_space(12.0);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        ui.add_space(MARGIN);
        for (index, (name, totals)) in periods.into_iter().enumerate() {
            if index > 0 {
                ui.add_space(gap);
            }
            ui.allocate_ui_with_layout(vec2(column, 0.0), Layout::top_down(Align::Min), |ui| {
                period(ui, name, totals, column)
            });
        }
    });
    ui.add_space(8.0);
    legend(ui);
    ui.add_space(12.0);
}

fn period(ui: &mut Ui, name: &str, totals: Totals, width: f32) {
    ui.spacing_mut().item_spacing.y = 3.0;
    ui.label(kit::text(
        name,
        size::CAPTION,
        REGULAR,
        theme::text_secondary(),
    ));
    ui.label(kit::text(
        usd(totals.total()),
        size::HEADLINE,
        SEMIBOLD,
        theme::text_primary(),
    ));
    let (rect, response) = ui.allocate_exact_size(vec2(width, BAR_HEIGHT), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, BAR_HEIGHT / 2.0, theme::quaternary(0.5));
    if totals.total() > 0.0 {
        let long = (totals.long / totals.total()) as f32;
        let split = rect.left() + rect.width() * long;
        let longs = Rect::from_min_max(rect.min, pos2(split, rect.max.y));
        let shorts = Rect::from_min_max(pos2(split, rect.min.y), rect.max);
        let round = BAR_HEIGHT / 2.0;
        painter.rect_filled(
            longs,
            egui::CornerRadius {
                nw: round as u8,
                sw: round as u8,
                ..Default::default()
            },
            theme::DOWN,
        );
        painter.rect_filled(
            shorts,
            egui::CornerRadius {
                ne: round as u8,
                se: round as u8,
                ..Default::default()
            },
            theme::UP,
        );
    }
    response.on_hover_text(tr!(
        "Longs {long} · shorts {short}",
        long = usd(totals.long),
        short = usd(totals.short)
    ));
}

/// What the two colours are.
fn legend(ui: &mut Ui) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 14.0;
        ui.add_space(MARGIN);
        for (side, text) in [
            (Side::Long, tr!("Longs liquidated")),
            (Side::Short, tr!("Shorts liquidated")),
        ] {
            let (dot, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
            ui.painter()
                .circle_filled(dot.center(), 3.0, side_color(side));
            ui.add_space(-8.0);
            ui.label(kit::text(
                text,
                size::CAPTION,
                REGULAR,
                theme::text_secondary(),
            ));
        }
    });
}

/// The row above the tape: what it lists, and the smallest size it lists.
fn filter_row(ui: &mut Ui, state: &mut MarketFlowState) {
    let width = ui.available_width();
    ui.allocate_ui_with_layout(
        vec2(width, 36.0),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.add_space(MARGIN);
            ui.label(kit::text(
                tr!("Latest"),
                size::CAPTION,
                MEDIUM,
                theme::text_secondary(),
            ));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(MARGIN);
                let labels: Vec<String> = MinSize::ALL
                    .iter()
                    .map(|size| size.label().to_owned())
                    .collect();
                let selected = MinSize::ALL
                    .iter()
                    .position(|size| *size == state.minimum)
                    .unwrap_or(0);
                if let Some(chosen) =
                    kit::menu_picker_fitted(ui, FILTER_WIDTH, &labels, selected, true)
                {
                    state.minimum = MinSize::ALL[chosen];
                }
            });
        },
    );
}

fn tape(ui: &mut Ui, state: &mut MarketFlowState, snapshot: &Snapshot) {
    filter_row(ui, state);
    if snapshot.latest.is_empty() {
        let (title, detail) = if snapshot.count == 0 {
            (
                tr!("Waiting for liquidations"),
                tr!("Binance and Bybit announce them as they happen."),
            )
        } else {
            (
                tr!("Nothing this size yet"),
                tr!("Lower the size to see the smaller ones."),
            )
        };
        kit::empty_state(
            ui,
            ui.available_rect_before_wrap(),
            Icon::Flame,
            title,
            detail,
        );
        return;
    }
    let columns = TapeColumns::measure(ui);
    columns_header(ui, &columns);
    egui::ScrollArea::vertical()
        .id_salt("liquidations-tape")
        .auto_shrink([false, false])
        .show_rows(ui, ROW_HEIGHT, snapshot.latest.len(), |ui, range| {
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            for index in range {
                row(ui, &snapshot.latest[index], index, &columns);
            }
        });
}

/// The room the tape's time, side and size columns keep: [`TIME_WIDTH`], [`SIDE_WIDTH`] and
/// [`SIZE_WIDTH`], or what their words need when a language writes them longer.
struct TapeColumns {
    time: f32,
    side: f32,
    size: f32,
}

impl TapeColumns {
    fn measure(ui: &Ui) -> Self {
        let width = |text: &str, font: f32, weight| {
            kit::galley(ui, text, font, weight, Color32::WHITE, f32::INFINITY)
                .size()
                .x
                .ceil()
        };
        let head = |text: &str| width(text, size::CAPTION, REGULAR);
        let side = [Side::Long, Side::Short]
            .into_iter()
            .map(|side| width(side_name(side), 12.0, MEDIUM))
            .fold(head(tr!("Side")), f32::max);
        Self {
            time: TIME_WIDTH.max(head(tr!("Time")) + 8.0),
            side: SIDE_WIDTH.max(side),
            size: SIZE_WIDTH.max(head(tr!("Size"))),
        }
    }

    /// Where the side column's right edge is: left of the size column.
    fn side_right(&self, rect: Rect) -> f32 {
        rect.right() - MARGIN - self.size - 8.0
    }
}

fn columns_header(ui: &mut Ui, columns: &TapeColumns) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
    let painter = ui.painter();
    let color = theme::text_tertiary();
    let put = |text: &str, x: f32, align: egui::Align2| {
        painter.text(
            pos2(x, rect.center().y),
            align,
            text,
            egui::FontId::proportional(size::CAPTION),
            color,
        );
    };
    put(tr!("Time"), rect.left() + MARGIN, egui::Align2::LEFT_CENTER);
    put(
        tr!("Market"),
        rect.left() + MARGIN + columns.time,
        egui::Align2::LEFT_CENTER,
    );
    put(
        tr!("Side"),
        columns.side_right(rect),
        egui::Align2::RIGHT_CENTER,
    );
    put(
        tr!("Size"),
        rect.right() - MARGIN,
        egui::Align2::RIGHT_CENTER,
    );
    painter.hline(
        rect.x_range(),
        rect.bottom() - 0.5,
        egui::Stroke::new(1.0, theme::separator()),
    );
}

fn row(ui: &mut Ui, liquidation: &Liquidation, index: usize, columns: &TapeColumns) {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    if response.hovered() {
        ui.painter().rect_filled(rect, 0.0, kit::primary(0.06));
    } else if index % 2 == 1 {
        ui.painter().rect_filled(rect, 0.0, theme::quaternary(0.28));
    }
    let mid = rect.center().y;
    let notional = liquidation.notional();
    let text = |content: String, x: f32, align: egui::Align2, color: Color32, weight| {
        let galley = kit::galley(ui, &content, 12.0, weight, color, f32::INFINITY);
        let origin = match align {
            egui::Align2::RIGHT_CENTER => pos2(x - galley.size().x, mid - galley.size().y / 2.0),
            _ => pos2(x, mid - galley.size().y / 2.0),
        };
        (origin, galley)
    };
    let time = text(
        kit::time_text::clock_time_seconds(Some(liquidation.time_ms)),
        rect.left() + MARGIN,
        egui::Align2::LEFT_CENTER,
        theme::text_tertiary(),
        REGULAR,
    );
    let market_room =
        rect.width() - 2.0 * MARGIN - columns.time - columns.side - columns.size - 8.0;
    let market = {
        let galley = kit::galley(
            ui,
            &liquidation.symbol,
            12.0,
            MEDIUM,
            theme::text_primary(),
            market_room.max(20.0),
        );
        (
            pos2(
                rect.left() + MARGIN + columns.time,
                mid - galley.size().y / 2.0,
            ),
            galley,
        )
    };
    let side = text(
        side_name(liquidation.side).to_owned(),
        columns.side_right(rect),
        egui::Align2::RIGHT_CENTER,
        side_color(liquidation.side),
        MEDIUM,
    );
    let figure = text(
        usd(notional),
        rect.right() - MARGIN,
        egui::Align2::RIGHT_CENTER,
        theme::text_primary(),
        if notional >= BIG { SEMIBOLD } else { REGULAR },
    );
    let painter = ui.painter();
    for (origin, galley) in [time, market, side, figure] {
        painter.galley(origin, galley, Color32::PLACEHOLDER);
    }
    let exchange = liquidation.exchange.name();
    let symbol = &liquidation.symbol;
    let quantity = crate::ui::pine::report::PineReportFormat::price(liquidation.quantity);
    let price = crate::ui::pine::report::PineReportFormat::price(liquidation.price);
    let worth = usd(notional);
    response.on_hover_text(match liquidation.side {
        Side::Long => tr!(
            "{exchange} {symbol}: long liquidated, {quantity} at {price}, {worth}",
            exchange = exchange,
            symbol = symbol,
            quantity = quantity,
            price = price,
            worth = worth
        ),
        Side::Short => tr!(
            "{exchange} {symbol}: short liquidated, {quantity} at {price}, {worth}",
            exchange = exchange,
            symbol = symbol,
            quantity = quantity,
            price = price,
            worth = worth
        ),
    });
}

fn liquidations_footnote() -> &'static str {
    tr!(
        "Binance sends at most the latest liquidation of each market a second, and Bybit's 60 busiest markets are followed, so a busy moment adds up to less than it was."
    )
}

fn footnote_height(ui: &Ui, text: &str) -> f32 {
    // Every row of it: a language that writes the note longer gets the room, not an ellipsis.
    let galley = kit::paragraph(
        ui,
        text,
        size::CAPTION,
        REGULAR,
        theme::text_tertiary(),
        ui.available_width() - 2.0 * MARGIN,
    );
    galley.size().y + 20.0
}

fn footnote(ui: &mut Ui, text: &str) {
    kit::divider(ui);
    ui.add_space(8.0);
    // The width `footnote_height` measured the note at, so the room it was given is the room it
    // fills and the note keeps the margin on both sides.
    let width = ui.available_width() - 2.0 * MARGIN;
    ui.horizontal(|ui| {
        ui.add_space(MARGIN);
        ui.scope(|ui| {
            ui.set_max_width(width);
            ui.add(
                egui::Label::new(kit::text(
                    text,
                    size::CAPTION,
                    REGULAR,
                    theme::text_tertiary(),
                ))
                .wrap(),
            );
        });
    });
}

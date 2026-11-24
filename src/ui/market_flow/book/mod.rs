//! The order-book mode of the panel: the focused chart's market as a ladder of prices with the
//! resting size on each side, what traded at each price and what was stacked or pulled, and a tape
//! of the latest trades. Drawn from a [`DepthSnapshot`] value; nothing here knows a book, a socket or how
//! an exchange keeps its book right.

mod ladder;
mod state;
mod tape;
#[cfg(test)]
mod tests;
mod toolbar;

#[cfg(test)]
use state::BigTrade;
pub use state::BookViewState;

use super::{MARGIN, footnote, footnote_height, usd};
use crate::i18n::tr;
use crate::model::depth::{DepthSnapshot, Link};
use crate::ui::icons::Icon;
use crate::ui::kit;
use crate::ui::kit::time_text::clock_time_seconds;
use eframe::egui::Ui;

/// The share of the free height the ladder takes; the tape has the rest.
const LADDER_SHARE: f32 = 0.6;
/// Neither part gets less than this, whatever the panel's height.
const MIN_PART: f32 = 120.0;

/// The market the panel follows, and what its exchange has said of it.
#[derive(Clone, Copy)]
pub struct BookFeed<'a> {
    pub exchange: &'a str,
    pub symbol: &'a str,
    pub snapshot: &'a DepthSnapshot,
}

/// What the mode draws: the focused chart's feed, none while its market has no book to follow.
#[derive(Clone, Copy)]
pub struct BookBody<'a> {
    pub feed: Option<BookFeed<'a>>,
}

pub(super) fn show(ui: &mut Ui, state: &mut BookViewState, body: &BookBody<'_>) {
    let area = ui.available_rect_before_wrap();
    let Some(feed) = body.feed else {
        kit::empty_state(
            ui,
            area,
            Icon::List,
            tr!("No order book"),
            tr!("The order book follows the focused chart when it is on Binance or Bybit."),
        );
        return;
    };
    let waiting = match feed.snapshot.link {
        Link::Live => None,
        Link::Connecting => Some((
            tr!("Connecting to {exchange}", exchange = feed.exchange),
            tr!("Waiting for the {symbol} book.", symbol = feed.symbol),
        )),
        Link::Reconnecting => Some((
            tr!("Reconnecting to {exchange}", exchange = feed.exchange),
            tr!("The book is shown again as soon as it is back.").to_owned(),
        )),
        Link::Unlisted => Some((
            tr!("No order book").to_owned(),
            tr!(
                "{exchange} does not list {symbol}.",
                exchange = feed.exchange,
                symbol = feed.symbol
            ),
        )),
    };
    if let Some((title, detail)) = waiting {
        kit::empty_state(ui, area, Icon::List, &title, &detail);
        return;
    }
    live(ui, state, feed);
}

fn live(ui: &mut Ui, state: &mut BookViewState, feed: BookFeed<'_>) {
    let note = footnote_text(feed);
    let reserved = footnote_height(ui, &note);
    toolbar::show(ui, state, &feed.snapshot.ladder);
    kit::divider(ui);
    let free = (ui.available_height() - reserved).max(2.0 * MIN_PART);
    let ladder_height = (free * LADDER_SHARE).max(MIN_PART);
    ladder::show(ui, state, feed.snapshot, ladder_height);
    kit::divider(ui);
    tape::show(ui, state, feed.snapshot, free - ladder_height);
    footnote(ui, &note);
}

fn footnote_text(feed: BookFeed<'_>) -> String {
    let (symbol, exchange) = (feed.symbol, feed.exchange);
    match feed.snapshot.tape.since_ms {
        None => tr!(
            "{symbol} spot, {exchange}. Sizes are in the base coin. No trades yet. Stack is size added or taken away over the last 10 seconds.",
            symbol = symbol,
            exchange = exchange
        ),
        Some(since) => tr!(
            "{symbol} spot, {exchange}. Sizes are in the base coin. Traded since {time}. Stack is size added or taken away over the last 10 seconds.",
            symbol = symbol,
            exchange = exchange,
            time = clock_time_seconds(Some(since))
        ),
    }
}

/// A size in the base coin with as few characters as keep it readable: `1.2K`, `340`, `12.3`,
/// `0.283`.
pub(super) fn size_text(size: f64) -> String {
    let size = size.abs();
    if size >= 10_000.0 {
        format!("{:.0}K", size / 1_000.0)
    } else if size >= 1_000.0 {
        format!("{:.1}K", size / 1_000.0)
    } else if size >= 100.0 {
        format!("{size:.0}")
    } else if size >= 10.0 {
        format!("{size:.1}")
    } else if size >= 1.0 {
        format!("{size:.2}")
    } else if size >= 0.001 {
        format!("{size:.3}")
    } else if size > 0.0 {
        "<0.001".to_owned()
    } else {
        "0".to_owned()
    }
}

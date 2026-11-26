//! What the TradingView import's sheets say. Pure, so the words are tested without drawing.

use crate::i18n::{tr, tr_n};
use crate::model::tradingview::library::{Layout, TvWatchlist};
use crate::model::tradingview::plan::{LeftOut, Unmatched, Usage};

/// The things of a list as one line: `BTCUSDT · 5m · 12 drawings`.
fn dotted(parts: &[String]) -> String {
    parts.join(" \u{00B7} ")
}

pub fn drawings(count: usize) -> String {
    tr_n!(count, "{count} drawing", "{count} drawings")
}

pub fn charts(count: usize) -> String {
    tr_n!(count, "{count} chart", "{count} charts")
}

pub fn markets(count: usize) -> String {
    tr_n!(count, "{count} market", "{count} markets")
}

pub fn layouts(count: usize) -> String {
    tr_n!(count, "{count} layout", "{count} layouts")
}

pub fn watchlists(count: usize) -> String {
    tr_n!(count, "{count} watchlist", "{count} watchlists")
}

pub fn scripts(count: usize) -> String {
    tr_n!(count, "{count} script", "{count} scripts")
}

/// What a layout holds, under its name.
pub fn layout_detail(layout: &Layout) -> String {
    let mut symbols: Vec<&str> = Vec::new();
    for chart in layout.shown_charts() {
        if !symbols.contains(&chart.symbol.ticker()) {
            symbols.push(chart.symbol.ticker());
        }
    }
    let mut parts = vec![
        symbols.into_iter().take(3).collect::<Vec<_>>().join(", "),
        charts(layout.shown_charts().count()),
        drawings(layout.drawings.len()),
    ];
    if layout.is_rebuilt() {
        parts.push(tr!("settings not found: rebuilt from its drawings").to_owned());
    }
    dotted(&parts)
}

pub fn watchlist_detail(list: &TvWatchlist) -> String {
    markets(list.symbols().count())
}

/// What uses a symbol.
pub fn usage(exchange: &str, usage: Usage) -> String {
    let mut parts = vec![exchange.to_owned()];
    if usage.charts > 0 {
        parts.push(charts(usage.charts));
    }
    if usage.drawings > 0 {
        parts.push(drawings(usage.drawings));
    }
    // The row has room for what the symbol is on in a layout, or else which lists hold it.
    if parts.len() == 1 && usage.watchlists > 0 {
        parts.push(watchlists(usage.watchlists));
    }
    dotted(&parts)
}

pub fn unmatched(why: Unmatched) -> &'static str {
    match why {
        Unmatched::Derivative => tr!("Perpetuals and futures aren\u{2019}t available"),
        Unmatched::Venue => tr!("No exchange here lists it"),
        Unmatched::NotFound => tr!("Not found on any exchange"),
        Unmatched::Expression => tr!("A formula, not a single market"),
        Unmatched::NeedsKey => tr!("Stocks need an Alpaca key (Settings)"),
        Unmatched::Unreachable => tr!("Couldn\u{2019}t reach the exchanges"),
    }
}

/// Something left out or changed, as a sentence of the preview.
pub fn left_out(entry: &LeftOut) -> String {
    match entry {
        LeftOut::NoChart => {
            tr!("None of its charts has a market, so it isn\u{2019}t imported").to_owned()
        }
        LeftOut::Chart(symbol) => tr!(
            "The {symbol} chart: its symbol has no market",
            symbol = symbol.as_str()
        ),
        LeftOut::Comparison(symbol) => tr!(
            "The comparison with {symbol}: it has no market",
            symbol = symbol.as_str()
        ),
        LeftOut::DrawingsOfMarket { symbol, count } => tr_n!(
            *count,
            "{count} drawing on {symbol}, which has no market",
            "{count} drawings on {symbol}, which has no market",
            symbol = symbol.as_str()
        ),
        LeftOut::DrawingsOfTool { class, count } => tr_n!(
            *count,
            "{count} {tool} drawing can\u{2019}t be brought over",
            "{count} {tool} drawings can\u{2019}t be brought over",
            tool = class.as_str()
        ),
        LeftOut::Pictures(count) => tr_n!(
            *count,
            "{count} picture: TradingView keeps it online",
            "{count} pictures: TradingView keeps them online"
        ),
        LeftOut::PositionsWithoutPriceStep(count) => tr_n!(
            *count,
            "{count} position whose price step is unknown",
            "{count} positions whose price step is unknown"
        ),
        LeftOut::BrokenDrawings(count) => tr_n!(
            *count,
            "{count} drawing that couldn\u{2019}t be read",
            "{count} drawings that couldn\u{2019}t be read"
        ),
        LeftOut::CutText(count) => tr_n!(
            *count,
            "Text of {count} drawing is shortened",
            "Text of {count} drawings is shortened"
        ),
        LeftOut::Indicator(name) => tr!("The {name} indicator", name = name.as_str()),
        LeftOut::IndicatorSettings(name) => tr!(
            "{name} is added with its default settings",
            name = name.as_str()
        ),
        LeftOut::HiddenCharts(count) => tr_n!(
            *count,
            "{count} chart TradingView keeps but doesn\u{2019}t show",
            "{count} charts TradingView keeps but doesn\u{2019}t show"
        ),
        LeftOut::ChartStyle(_) => {
            tr!("A chart type DegenView doesn\u{2019}t draw: your usual one is used").to_owned()
        }
        LeftOut::Interval(interval) => tr!(
            "The {interval} interval isn\u{2019}t a timeframe here",
            interval = interval.as_str()
        ),
        LeftOut::SameMarket(count) => tr_n!(
            *count,
            "{count} symbol that is the same market as another",
            "{count} symbols that are the same market as others"
        ),
        LeftOut::ListFull(count) => tr_n!(
            *count,
            "{count} symbol that doesn\u{2019}t fit in one watchlist",
            "{count} symbols that don\u{2019}t fit in one watchlist"
        ),
        LeftOut::OtherMarket {
            source,
            symbol,
            count,
        } => tr_n!(
            *count,
            "{count} drawing is on {symbol} ({exchange}): switch the chart to that market to see it",
            "{count} drawings are on {symbol} ({exchange}): switch the chart to that market to see them",
            symbol = symbol.as_str(),
            exchange = source.display_name()
        ),
        LeftOut::Pinned(count) => tr_n!(
            *count,
            "{count} drawing pinned to the screen can\u{2019}t be brought over",
            "{count} drawings pinned to the screen can\u{2019}t be brought over"
        ),
        LeftOut::PostsEstimated(count) => tr_n!(
            *count,
            "{count} post is placed at an estimated height",
            "{count} posts are placed at an estimated height"
        ),
        LeftOut::PostsUnplaced(count) => tr_n!(
            *count,
            "{count} post whose place on the chart couldn\u{2019}t be worked out",
            "{count} posts whose place on the chart couldn\u{2019}t be worked out"
        ),
        LeftOut::PercentBands(count) => tr_n!(
            *count,
            "{count} anchored VWAP loses its percentage bands",
            "{count} anchored VWAPs lose their percentage bands"
        ),
        LeftOut::DefaultLevels(count) => tr_n!(
            *count,
            "{count} Fibonacci or Gann drawing uses DegenView\u{2019}s levels",
            "{count} Fibonacci or Gann drawings use DegenView\u{2019}s levels"
        ),
        LeftOut::Markets(count) => tr_n!(
            *count,
            "{count} symbol without a market",
            "{count} symbols without a market"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::tradingview::TvSymbol;

    #[test]
    fn left_out_things_read_as_sentences() {
        let symbol = TvSymbol::read("A:B").unwrap();
        assert_eq!(
            left_out(&LeftOut::DrawingsOfMarket { symbol, count: 2 }),
            "2 drawings on A:B, which has no market"
        );
        assert_eq!(
            left_out(&LeftOut::DrawingsOfTool {
                class: "Emoji".into(),
                count: 1
            }),
            "1 Emoji drawing can\u{2019}t be brought over"
        );
    }

    #[test]
    fn usage_lists_only_what_there_is() {
        let used = Usage {
            charts: 0,
            drawings: 3,
            watchlists: 1,
        };
        assert_eq!(usage("OKX", used), "OKX \u{00B7} 3 drawings");
        let listed = Usage {
            drawings: 0,
            ..used
        };
        assert_eq!(usage("OKX", listed), "OKX \u{00B7} 1 watchlist");
    }
}

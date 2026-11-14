//! What a chart tab says about itself: the symbol, the live price and its change, and the name
//! of the layout it has open. Pure: it turns a tab's identity and the quote the app already has
//! into the pieces the strip paints, so the rules can be tested without a window.
//!
//! The price and change come from the shared quote book (the provider's own reference: a
//! rolling 24 hours or the previous close), never from the chart's candles, and a missing value
//! stays missing: the price block is left out rather than showing a made-up `+0.00%`.

use serde::{Deserialize, Serialize};

use crate::model::DataSource;
use crate::model::watchlist;
use crate::model::watchlist::quote::{Direction, Freshness, WatchlistQuote};

/// How long a tab title the user typed may be.
pub const MAX_CUSTOM_TITLE_CHARS: usize = 60;

/// How a chart tab names itself (Settings ▸ Appearance ▸ Tabs). TradingView documents a
/// layout name as an alternative to the symbol, set in its Settings, with "reset to default";
/// the price block and the choices around it are this app's own.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TabTitleStyle {
    /// Symbol, direction, price and change, then the layout name.
    #[default]
    Full,
    SymbolOnly,
    LayoutOnly,
}

impl TabTitleStyle {
    /// The `setting` row the choice is stored under. Local to the Mac (see `sync::record::policy`).
    pub const KEY: &'static str = "tab.title_style";

    pub const ALL: [Self; 3] = [Self::Full, Self::SymbolOnly, Self::LayoutOnly];

    pub fn label(self) -> &'static str {
        match self {
            Self::Full => "Symbol, price and layout",
            Self::SymbolOnly => "Symbol only",
            Self::LayoutOnly => "Layout name only",
        }
    }
}

/// What a chart tab knows about itself. The instrument and the layout are the tab's identity;
/// the price is not stored here, it is read from the quote book each frame.
#[derive(Clone, Debug, PartialEq)]
pub struct TabChart {
    pub source: DataSource,
    /// The provider's symbol: the key for the logo and the quote.
    pub symbol: String,
    /// The name shown for the market ("BTC/USDT", "AAPL").
    pub title: String,
    /// The saved layout this tab has open; `None` while it is unnamed.
    pub layout: Option<String>,
    /// A title the user typed for this tab. It stands in for the whole text, and changing the
    /// symbol or the layout does not touch it.
    pub custom_title: Option<String>,
}

impl TabChart {
    /// Whether a quote is worth asking for. Prediction markets poll a day of candles per
    /// market and CoinMarketCap's charts are indices without quotes.
    pub fn wants_quote(&self) -> bool {
        wants_quote(self.source)
    }
}

pub fn wants_quote(source: DataSource) -> bool {
    !source.is_prediction_market() && source != DataSource::CoinMarketCap
}

/// How much of the tab fits. Chosen from the tab's width alone (never from the text, which
/// changes with every quote), so a price tick cannot move anything.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Detail {
    /// Just the logo: a tab squeezed to its narrowest. The tooltip names it.
    Logo,
    /// Logo and symbol.
    Symbol,
    /// Plus the price.
    Price,
    /// Plus the change.
    Change,
    /// Plus the layout name, cut off (and faded) where the tab ends.
    Layout,
}

impl Detail {
    pub const LAYOUT_WIDTH: f32 = 260.0;
    pub const CHANGE_WIDTH: f32 = 230.0;
    pub const PRICE_WIDTH: f32 = 150.0;
    pub const SYMBOL_WIDTH: f32 = 96.0;

    pub fn for_width(width: f32) -> Self {
        if width >= Self::LAYOUT_WIDTH {
            Self::Layout
        } else if width >= Self::CHANGE_WIDTH {
            Self::Change
        } else if width >= Self::PRICE_WIDTH {
            Self::Price
        } else if width >= Self::SYMBOL_WIDTH {
            Self::Symbol
        } else {
            Self::Logo
        }
    }
}

/// The price block: `▲ 142.92 +0.96%`.
#[derive(Clone, Debug, PartialEq)]
pub struct PriceBlock {
    pub direction: Direction,
    pub price: String,
    /// Left out when the provider gave no reference to measure the change from.
    pub change: Option<String>,
    /// A price older than the provider's limit: drawn dimmed.
    pub stale: bool,
}

/// The pieces of one tab's title.
#[derive(Clone, Debug, PartialEq)]
pub struct Segments {
    /// The symbol, the layout name (style `LayoutOnly`) or the custom title.
    pub text: String,
    pub price: Option<PriceBlock>,
    /// After a `/`.
    pub layout: Option<String>,
}

/// The price block for a quote, or `None` when there is nothing true to show.
pub fn price_block(source: DataSource, quote: Option<&WatchlistQuote>) -> Option<PriceBlock> {
    if !wants_quote(source) {
        return None;
    }
    let quote = quote?;
    quote.last.filter(|last| last.is_finite())?;
    if matches!(
        quote.freshness,
        Freshness::Unavailable | Freshness::Unsupported
    ) {
        return None;
    }
    let direction = watchlist::quote::direction(Some(quote));
    let change = quote
        .change_percent
        .filter(|percent| percent.is_finite())
        .map(|_| {
            if direction == Direction::Flat {
                "0.00%".to_owned()
            } else {
                watchlist::quote::percent_text(Some(quote), Some(source))
            }
        });
    Some(PriceBlock {
        direction,
        price: watchlist::quote::last_text(Some(quote), Some(source)),
        change,
        stale: quote.freshness == Freshness::Stale,
    })
}

/// The tab's text for a style, at a level of detail, with the quote the app holds for it.
pub fn compose(
    style: TabTitleStyle,
    chart: &TabChart,
    quote: Option<&WatchlistQuote>,
    detail: Detail,
) -> Segments {
    let layout = chart
        .layout
        .as_deref()
        .filter(|name| !name.trim().is_empty());
    if let Some(custom) = chart
        .custom_title
        .as_deref()
        .map(str::trim)
        .filter(|title| !title.is_empty())
    {
        return Segments {
            text: custom.to_owned(),
            price: None,
            layout: None,
        };
    }
    match style {
        TabTitleStyle::SymbolOnly => Segments {
            text: chart.title.clone(),
            price: None,
            layout: None,
        },
        // Without a layout name the tab falls back to its symbol rather than going blank.
        TabTitleStyle::LayoutOnly => Segments {
            text: layout.map_or_else(|| chart.title.clone(), str::to_owned),
            price: None,
            layout: None,
        },
        TabTitleStyle::Full => Segments {
            text: chart.title.clone(),
            price: (detail >= Detail::Price)
                .then(|| price_block(chart.source, quote))
                .flatten()
                .map(|mut block| {
                    if detail < Detail::Change {
                        block.change = None;
                    }
                    block
                }),
            layout: (detail >= Detail::Layout)
                .then(|| layout.map(str::to_owned))
                .flatten(),
        },
    }
}

/// The text a hover shows: everything the tab knows, whatever the strip had room for.
pub fn tooltip(chart: &TabChart, quote: Option<&WatchlistQuote>, timeframe: &str) -> String {
    let mut lines = vec![format!(
        "{} · {} · {timeframe}",
        chart.title,
        chart.source.display_name()
    )];
    if let Some(block) = price_block(chart.source, quote) {
        let mut line = block.price;
        if let Some(change) = block.change {
            line.push_str(&format!("  {change}"));
        }
        if let Some(basis) = quote.and_then(|quote| quote.change_basis) {
            line.push_str(&format!("  ({})", basis.label().to_lowercase()));
        }
        if block.stale {
            line.push_str("  · delayed");
        }
        lines.push(line);
    }
    if let Some(layout) = chart.layout.as_deref() {
        lines.push(format!("Layout: {layout}"));
    }
    if let Some(custom) = chart.custom_title.as_deref() {
        lines.push(format!("Tab title: {custom}"));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::watchlist::quote::{ChangeBasis, SourceQuote};

    fn chart() -> TabChart {
        TabChart {
            source: DataSource::Alpaca,
            symbol: "AAPL".into(),
            title: "AAPL".into(),
            layout: Some("Layout 1".into()),
            custom_title: None,
        }
    }

    fn quote(price: f64, previous: Option<f64>) -> WatchlistQuote {
        WatchlistQuote::from_source(
            DataSource::Alpaca,
            &SourceQuote {
                price,
                previous_day_price: previous,
                ..SourceQuote::default()
            },
            0,
        )
    }

    #[test]
    fn up_down_and_flat_read_from_the_quote() {
        let up = price_block(DataSource::Alpaca, Some(&quote(142.92, Some(141.56)))).unwrap();
        assert_eq!(up.direction, Direction::Up);
        assert_eq!(up.price, "142.92");
        assert_eq!(up.change.as_deref(), Some("+0.96%"));

        let down = price_block(DataSource::Alpaca, Some(&quote(141.2, Some(142.89)))).unwrap();
        assert_eq!(down.direction, Direction::Down);
        assert_eq!(down.change.as_deref(), Some("-1.18%"));

        let flat = price_block(DataSource::Alpaca, Some(&quote(100.0, Some(100.0)))).unwrap();
        assert_eq!(flat.direction, Direction::Flat);
        assert_eq!(flat.change.as_deref(), Some("0.00%"));
    }

    #[test]
    fn a_missing_reference_shows_the_price_without_a_change() {
        let block = price_block(DataSource::Alpaca, Some(&quote(142.92, None))).unwrap();
        assert_eq!(block.price, "142.92");
        assert_eq!(block.change, None);
        assert_eq!(block.direction, Direction::Flat);
        // A zero or invalid reference is no reference.
        let zero = price_block(DataSource::Alpaca, Some(&quote(142.92, Some(0.0)))).unwrap();
        assert_eq!(zero.change, None);
        let nan = price_block(DataSource::Alpaca, Some(&quote(142.92, Some(f64::NAN)))).unwrap();
        assert_eq!(nan.change, None);
    }

    #[test]
    fn no_quote_no_price_no_invention() {
        assert_eq!(price_block(DataSource::Alpaca, None), None);
        assert_eq!(
            price_block(DataSource::Alpaca, Some(&WatchlistQuote::default())),
            None
        );
        let mut no_last = quote(1.0, Some(1.0));
        no_last.last = None;
        assert_eq!(price_block(DataSource::Alpaca, Some(&no_last)), None);
        let mut infinite = quote(1.0, Some(1.0));
        infinite.last = Some(f64::INFINITY);
        assert_eq!(price_block(DataSource::Alpaca, Some(&infinite)), None);
        let unsupported = WatchlistQuote::unsupported(None);
        assert_eq!(price_block(DataSource::Alpaca, Some(&unsupported)), None);
    }

    #[test]
    fn stale_dims_and_closed_market_does_not() {
        let live = quote(10.0, Some(9.0));
        let stale = live.with_freshness(Freshness::Stale);
        assert!(price_block(DataSource::Alpaca, Some(&stale)).unwrap().stale);
        let closed = live.with_freshness(Freshness::MarketClosed);
        assert!(
            !price_block(DataSource::Alpaca, Some(&closed))
                .unwrap()
                .stale
        );
    }

    #[test]
    fn markets_without_quotes_never_get_a_price() {
        let live = quote(0.66, Some(0.6));
        assert_eq!(price_block(DataSource::Polymarket, Some(&live)), None);
        assert_eq!(price_block(DataSource::CoinMarketCap, Some(&live)), None);
        assert!(!wants_quote(DataSource::Kalshi));
        assert!(wants_quote(DataSource::Binance));
    }

    #[test]
    fn price_precision_follows_the_shared_format() {
        let btc = WatchlistQuote::from_source(
            DataSource::Binance,
            &SourceQuote {
                price: 68421.5,
                previous_day_price: Some(67000.0),
                ..SourceQuote::default()
            },
            0,
        );
        let block = price_block(DataSource::Binance, Some(&btc)).unwrap();
        assert_eq!(block.price, watchlist::quote::last_text(Some(&btc), None));
        // Large prices lose their decimals under the shared rule; the tab says what it says.
        assert!(block.price.starts_with("68,42"), "{}", block.price);
        assert_eq!(btc.change_basis, Some(ChangeBasis::Rolling24h));
    }

    #[test]
    fn detail_follows_width_only() {
        assert_eq!(Detail::for_width(300.0), Detail::Layout);
        assert_eq!(Detail::for_width(Detail::LAYOUT_WIDTH), Detail::Layout);
        assert_eq!(
            Detail::for_width(Detail::LAYOUT_WIDTH - 0.1),
            Detail::Change
        );
        assert_eq!(Detail::for_width(160.0), Detail::Price);
        assert_eq!(Detail::for_width(Detail::SYMBOL_WIDTH), Detail::Symbol);
        assert_eq!(Detail::for_width(52.0), Detail::Logo);
        assert!(Detail::Layout > Detail::Change && Detail::Change > Detail::Price);
        assert!(Detail::Symbol > Detail::Logo);
    }

    #[test]
    fn full_style_drops_parts_as_room_runs_out() {
        let q = quote(142.92, Some(141.56));
        let full = compose(TabTitleStyle::Full, &chart(), Some(&q), Detail::Layout);
        assert_eq!(full.text, "AAPL");
        assert_eq!(full.layout.as_deref(), Some("Layout 1"));
        assert!(full.price.as_ref().unwrap().change.is_some());

        let change = compose(TabTitleStyle::Full, &chart(), Some(&q), Detail::Change);
        assert_eq!(change.layout, None);
        assert!(change.price.as_ref().unwrap().change.is_some());

        let price = compose(TabTitleStyle::Full, &chart(), Some(&q), Detail::Price);
        assert_eq!(price.price.as_ref().unwrap().change, None);

        let symbol = compose(TabTitleStyle::Full, &chart(), Some(&q), Detail::Symbol);
        assert_eq!(symbol.price, None);
        assert_eq!(symbol.text, "AAPL");
    }

    #[test]
    fn an_unnamed_layout_has_no_separator_segment() {
        let mut unnamed = chart();
        unnamed.layout = None;
        let out = compose(TabTitleStyle::Full, &unnamed, None, Detail::Layout);
        assert_eq!(out.layout, None);
        unnamed.layout = Some("  ".into());
        let out = compose(TabTitleStyle::Full, &unnamed, None, Detail::Layout);
        assert_eq!(out.layout, None);
    }

    #[test]
    fn styles_choose_the_text() {
        let q = quote(1.0, Some(1.0));
        let symbol = compose(
            TabTitleStyle::SymbolOnly,
            &chart(),
            Some(&q),
            Detail::Layout,
        );
        assert_eq!(
            (symbol.text.as_str(), symbol.price, symbol.layout),
            ("AAPL", None, None)
        );
        let layout = compose(
            TabTitleStyle::LayoutOnly,
            &chart(),
            Some(&q),
            Detail::Layout,
        );
        assert_eq!(layout.text, "Layout 1");
        assert_eq!(layout.price, None);
        // No layout name: the symbol, not an empty tab.
        let mut unnamed = chart();
        unnamed.layout = None;
        let layout = compose(TabTitleStyle::LayoutOnly, &unnamed, None, Detail::Layout);
        assert_eq!(layout.text, "AAPL");
    }

    #[test]
    fn a_custom_title_stands_in_for_everything_and_survives_a_symbol_change() {
        let mut tab = chart();
        tab.custom_title = Some("  Earnings  ".into());
        let q = quote(1.0, Some(1.0));
        for style in TabTitleStyle::ALL {
            let out = compose(style, &tab, Some(&q), Detail::Layout);
            assert_eq!(out.text, "Earnings");
            assert_eq!((out.price, out.layout), (None, None));
        }
        // The symbol changes, the title does not.
        tab.symbol = "MSFT".into();
        tab.title = "MSFT".into();
        assert_eq!(
            compose(TabTitleStyle::Full, &tab, None, Detail::Layout).text,
            "Earnings"
        );
        // A blank title is no title.
        tab.custom_title = Some("   ".into());
        assert_eq!(
            compose(TabTitleStyle::Full, &tab, None, Detail::Layout).text,
            "MSFT"
        );
    }

    #[test]
    fn the_tooltip_keeps_what_the_strip_dropped() {
        let q = quote(142.92, Some(141.56));
        let text = tooltip(&chart(), Some(&q), "1D");
        assert!(text.contains("AAPL"));
        assert!(text.contains("1D"));
        assert!(text.contains("142.92"));
        assert!(text.contains("+0.96%"));
        assert!(text.contains("Layout: Layout 1"));
        assert!(text.contains("previous close"));
    }

    #[test]
    fn the_style_is_stored_under_its_key_and_defaults_to_full() {
        assert_eq!(TabTitleStyle::default(), TabTitleStyle::Full);
        assert_eq!(TabTitleStyle::KEY, "tab.title_style");
        let json = serde_json::to_string(&TabTitleStyle::LayoutOnly).unwrap();
        assert_eq!(json, "\"layoutOnly\"");
        assert_eq!(
            serde_json::from_str::<TabTitleStyle>("\"symbolOnly\"").unwrap(),
            TabTitleStyle::SymbolOnly
        );
    }
}

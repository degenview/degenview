//! Portfolio cards: grid cards that show a portfolio instead of a market. They share the chart
//! grid with market charts but have no candles, indicators, alerts, drawings or scripts, so
//! nothing here knows about `DataSource`. Pure values and arithmetic; the data feeds and the
//! painters live in `service/` and `ui/portfolio_cards/`.
//!
//! Not to be confused with `chart_type::ChartKind`, which tells a Pine script what chart it runs on.

use crate::model::portfolio::format::PortfolioValueFormatter;
use crate::model::portfolio::statistics::{
    PortfolioHistoryRange, PortfolioPeriodChange, PortfolioValueChange,
};
use crate::model::portfolio::{Portfolio, PortfolioHolding, PortfolioSnapshot};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// What a portfolio card shows.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CardKind {
    /// Total value over time.
    ValueChart,
    /// The current total, big.
    Value,
    /// How the portfolio splits across assets.
    Allocation,
    /// The holdings that did best over a range.
    TopPerformers,
}

impl CardKind {
    pub const ALL: [Self; 4] = [
        Self::ValueChart,
        Self::Value,
        Self::Allocation,
        Self::TopPerformers,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::ValueChart => "Portfolio Value Chart",
            Self::Value => "Portfolio Value",
            Self::Allocation => "Allocation",
            Self::TopPerformers => "Top Performers",
        }
    }

    /// One line for the Add Chart sheet.
    pub fn summary(self) -> &'static str {
        match self {
            Self::ValueChart => "Value over time, from one day to all time",
            Self::Value => "The current total as one big number",
            Self::Allocation => "How the portfolio splits across assets",
            Self::TopPerformers => "The holdings that gained the most over a range",
        }
    }

    /// Whether the card has a range of its own.
    pub fn has_range(self) -> bool {
        matches!(self, Self::ValueChart | Self::TopPerformers)
    }

    fn slug(self) -> &'static str {
        match self {
            Self::ValueChart => "value-chart",
            Self::Value => "value",
            Self::Allocation => "allocation",
            Self::TopPerformers => "top-performers",
        }
    }
}

/// One portfolio card: what it shows, for which portfolio, over which range.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct PortfolioCard {
    pub kind: CardKind,
    /// `None` covers every active portfolio.
    #[serde(default)]
    pub portfolio_id: Option<u64>,
    #[serde(default)]
    pub range: PortfolioHistoryRange,
}

impl PortfolioCard {
    pub fn new(kind: CardKind, portfolio_id: Option<u64>) -> Self {
        Self {
            kind,
            portfolio_id,
            range: PortfolioHistoryRange::default(),
        }
    }

    /// Identity for a watchlist entry and for "is this card already on the grid". The range is
    /// view state, not identity: the same card at another range is the same card.
    pub fn key(&self) -> String {
        match self.portfolio_id {
            Some(id) => format!("portfolio:{}:{id}", self.kind.slug()),
            None => format!("portfolio:{}:all", self.kind.slug()),
        }
    }

    /// The identity a card has as a watchlist row. A card is not a market, so it takes the
    /// provider the watchlist already treats as having no quote (CoinMarketCap's indices) with
    /// the card's key as the symbol; the row's `card` field, not the provider, says what it is.
    pub fn instrument_id(&self) -> crate::model::InstrumentId {
        crate::model::InstrumentId::new(crate::model::DataSource::CoinMarketCap, self.key())
    }

    pub fn same_card(&self, other: &Self) -> bool {
        self.kind == other.kind && self.portfolio_id == other.portfolio_id
    }

    /// "All Portfolios" or the portfolio's name; a deleted portfolio reads as all.
    pub fn scope_name(&self, portfolios: &[Portfolio]) -> String {
        self.portfolio_id
            .and_then(|id| portfolios.iter().find(|portfolio| portfolio.id == id))
            .map_or_else(|| "All Portfolios".to_owned(), |p| p.name.clone())
    }
}

/// Everything a card paints, gathered by `portfolio_card_host` so the painters stay pure.
#[derive(Clone, Debug, PartialEq)]
pub struct CardData {
    /// False when there is no portfolio to show (none exist, or all are archived).
    pub has_portfolios: bool,
    /// "All Portfolios" or the portfolio's name.
    pub scope_name: String,
    pub holdings: Vec<PortfolioHolding>,
    /// Value history inside the card's range, oldest first (30-minute steps for 1D when the
    /// day can be replayed). Empty for kinds that don't draw it.
    pub history: Vec<PortfolioSnapshot>,
    pub total_value: f64,
    /// Value now against 24 hours ago at today's quantities.
    pub day_change: Option<PortfolioValueChange>,
    /// Profit over the card's range, net of money moved in or out.
    pub period_change: Option<PortfolioPeriodChange>,
    pub formatter: PortfolioValueFormatter,
    /// No prices yet.
    pub loading: bool,
    /// The daily history is being rebuilt.
    pub loading_history: bool,
    pub error: Option<String>,
    /// Holdings no source could price.
    pub unpriced: usize,
    pub performance: Performance,
    /// Some holding's start price is still being fetched.
    pub loading_performance: bool,
    pub now_ms: i64,
    pub utc_offset_secs: i64,
}

/// How an asset's own market moved over a range, in the market's quote currency.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AssetMove {
    pub start: f64,
    pub latest: f64,
}

/// A holding's return over a range.
#[derive(Clone, Debug, PartialEq)]
pub struct Performer {
    pub asset_key: String,
    pub ticker: String,
    /// The market's price at the start of the range and now, in its quote currency.
    pub start_price: f64,
    pub price: f64,
    /// `price / start_price − 1`.
    pub change: f64,
    /// What today's position gained or lost over the range, in the reporting currency. It
    /// assumes the quantity held did not change; `None` while the holding has no value.
    pub amount: Option<f64>,
}

/// The ranking of a portfolio's holdings over a range.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Performance {
    /// Best first, every holding that could be compared.
    pub ranked: Vec<Performer>,
    /// Holdings whose market has no price at the start of the range (or no price at all).
    pub unpriced: usize,
}

/// Ranks `holdings` by how their markets moved (`moves`, keyed by asset key). The return comes
/// from the market's own candles, so it needs no currency conversion; the amount is today's
/// value scaled by that return. A holding without a usable move is counted as unpriced.
pub fn performance(
    holdings: &[PortfolioHolding],
    moves: &HashMap<String, AssetMove>,
) -> Performance {
    let mut ranked = Vec::new();
    let mut unpriced = 0;
    for holding in holdings.iter().filter(|holding| holding.quantity > 0.0) {
        match moves.get(&holding.asset.key) {
            Some(movement)
                if movement.start > 0.0
                    && movement.start.is_finite()
                    && movement.latest.is_finite() =>
            {
                let change = movement.latest / movement.start - 1.0;
                ranked.push(Performer {
                    asset_key: holding.asset.key.clone(),
                    ticker: holding.asset.display_ticker(),
                    start_price: movement.start,
                    price: movement.latest,
                    change,
                    // Today's value scaled back to the start of the range divides by the
                    // price ratio, so a latest price of zero has no amount.
                    amount: holding
                        .current_value
                        .filter(|_| movement.latest > 0.0)
                        .map(|value| value * change / (1.0 + change)),
                });
            }
            _ => unpriced += 1,
        }
    }
    ranked.sort_by(|a, b| {
        b.change
            .total_cmp(&a.change)
            .then_with(|| a.asset_key.cmp(&b.asset_key))
    });
    Performance { ranked, unpriced }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;
    use crate::model::portfolio::{PortfolioAsset, PortfolioCurrency};

    fn holding(symbol: &str, quantity: f64, price: Option<f64>) -> PortfolioHolding {
        PortfolioHolding {
            asset: PortfolioAsset {
                key: format!("binance:{symbol}USDT"),
                symbol: format!("{symbol}/USDT"),
                name: symbol.to_owned(),
                source: DataSource::Binance,
                quote_currency: PortfolioCurrency::Usd,
                metadata: HashMap::new(),
            },
            quantity,
            cost_basis: 0.0,
            average_cost: 0.0,
            realized_pnl: 0.0,
            current_price: price,
            previous_day_price: None,
            current_value: price.map(|price| price * quantity),
            unrealized_pnl: None,
            allocation: 0.0,
            is_price_stale: false,
        }
    }

    fn moves(rows: &[(&str, f64, f64)]) -> HashMap<String, AssetMove> {
        rows.iter()
            .map(|(symbol, start, latest)| {
                (
                    format!("binance:{symbol}USDT"),
                    AssetMove {
                        start: *start,
                        latest: *latest,
                    },
                )
            })
            .collect()
    }

    #[test]
    fn a_card_keeps_its_identity_across_ranges() {
        let mut card = PortfolioCard::new(CardKind::Allocation, Some(3));
        let key = card.key();
        card.range = PortfolioHistoryRange::OneYear;
        assert_eq!(card.key(), key);
        assert_eq!(key, "portfolio:allocation:3");
        assert_eq!(
            PortfolioCard::new(CardKind::TopPerformers, None).key(),
            "portfolio:top-performers:all"
        );
        assert!(card.same_card(&PortfolioCard::new(CardKind::Allocation, Some(3))));
        assert!(!card.same_card(&PortfolioCard::new(CardKind::Allocation, None)));
    }

    #[test]
    fn a_card_without_range_or_scope_decodes_with_defaults() {
        let card: PortfolioCard = serde_json::from_str(r#"{"kind":"value_chart"}"#).unwrap();
        assert_eq!(card.portfolio_id, None);
        assert_eq!(card.range, PortfolioHistoryRange::OneMonth);
        let saved = serde_json::to_string(&card).unwrap();
        assert_eq!(serde_json::from_str::<PortfolioCard>(&saved).unwrap(), card);
    }

    #[test]
    fn a_latest_price_of_zero_gives_no_infinite_amount() {
        let result = performance(
            &[holding("ETH", 2.0, Some(110.0))],
            &moves(&[("ETH", 100.0, 0.0)]),
        );
        assert_eq!(result.ranked.len(), 1);
        assert_eq!(result.ranked[0].change, -1.0);
        assert_eq!(result.ranked[0].amount, None);
    }

    #[test]
    fn performers_rank_best_first_and_scale_todays_value() {
        let holdings = [
            holding("ETH", 2.0, Some(110.0)),
            holding("BTC", 1.0, Some(300.0)),
            holding("SOL", 10.0, Some(45.0)),
        ];
        let result = performance(
            &holdings,
            &moves(&[
                ("ETH", 100.0, 110.0),
                ("BTC", 200.0, 300.0),
                ("SOL", 50.0, 45.0),
            ]),
        );
        let order: Vec<&str> = result.ranked.iter().map(|p| p.ticker.as_str()).collect();
        assert_eq!(order, ["BTC", "ETH", "SOL"]);
        assert!((result.ranked[0].change - 0.5).abs() < 1e-12);
        // BTC is worth 300 now and was worth 200 at +50 %: it gained 100.
        assert!((result.ranked[0].amount.unwrap() - 100.0).abs() < 1e-9);
        // SOL is worth 450 now after a −10 % move: it lost 50.
        assert!((result.ranked[2].amount.unwrap() + 50.0).abs() < 1e-9);
        assert_eq!(result.unpriced, 0);
    }

    #[test]
    fn holdings_without_a_usable_move_are_unpriced_not_ranked() {
        let holdings = [
            holding("ETH", 1.0, None),
            holding("BTC", 1.0, Some(10.0)),
            holding("SOL", 1.0, Some(10.0)),
            holding("ADA", 0.0, Some(10.0)),
        ];
        let result = performance(&holdings, &moves(&[("BTC", 0.0, 10.0)]));
        assert!(result.ranked.is_empty());
        assert_eq!(result.unpriced, 3, "a sold-out holding is not counted");
    }

    #[test]
    fn a_holding_without_a_value_still_ranks_but_has_no_amount() {
        let result = performance(&[holding("ETH", 1.0, None)], &moves(&[("ETH", 10.0, 20.0)]));
        assert_eq!(result.ranked.len(), 1);
        assert_eq!(result.ranked[0].amount, None);
    }

    #[test]
    fn equal_returns_rank_by_asset_key() {
        let holdings = [
            holding("ETH", 1.0, Some(20.0)),
            holding("BTC", 1.0, Some(20.0)),
        ];
        let result = performance(
            &holdings,
            &moves(&[("ETH", 10.0, 20.0), ("BTC", 10.0, 20.0)]),
        );
        assert_eq!(result.ranked[0].ticker, "BTC");
    }

    #[test]
    fn a_deleted_portfolio_reads_as_all() {
        let portfolios = [Portfolio::new(1, "Long term", 0)];
        assert_eq!(
            PortfolioCard::new(CardKind::Value, Some(1)).scope_name(&portfolios),
            "Long term"
        );
        assert_eq!(
            PortfolioCard::new(CardKind::Value, Some(9)).scope_name(&portfolios),
            "All Portfolios"
        );
    }
}

//! A price alert before it is saved: what the price-alert sheet and an agent's create or edit
//! both build. The fields are kept as typed; parsing, hints, the summary and the saved form are
//! derived here, so the sheet and MCP follow one set of rules and the rules are testable without
//! a window.

use crate::model::alert::message::AlertMessageContext;
use crate::model::alert::{
    AlertAsset, AlertCondition, AlertCurrency, AlertFrequency, AlertState, PriceAlert,
};
use crate::model::watchlist::quote::{Freshness, WatchlistQuote};
use crate::model::webhook::EndpointId;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Direction {
    Above,
    Below,
    Rises,
    Falls,
}

impl Direction {
    pub const ALL: [Self; 4] = [Self::Above, Self::Below, Self::Rises, Self::Falls];

    pub fn title(self) -> &'static str {
        match self {
            Self::Above => "Above",
            Self::Below => "Below",
            Self::Rises => "Rises",
            Self::Falls => "Falls",
        }
    }

    pub fn is_percent(self) -> bool {
        matches!(self, Self::Rises | Self::Falls)
    }

    pub fn blurb(self) -> &'static str {
        match self {
            Self::Above => "Notify me when the price crosses above a level.",
            Self::Below => "Notify me when the price crosses below a level.",
            Self::Rises => "Notify me when the price climbs by a percentage from now.",
            Self::Falls => "Notify me when the price drops by a percentage from now.",
        }
    }
}

/// A quick target a few percent from the current price.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TargetOffset {
    pub percent: i32,
    pub target_text: String,
}

impl TargetOffset {
    /// "+5%" / "−5%".
    pub fn label(&self) -> String {
        format!(
            "{}{}%",
            if self.percent > 0 { "+" } else { "−" },
            self.percent.abs()
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HintTone {
    Warning,
    Neutral,
}

/// A price alert being made or edited, before it is saved: the fields as typed, the market it
/// is priced against, and the rules that turn them into an alert.
#[derive(Clone, Debug)]
pub struct PriceAlertDraft {
    asset: AlertAsset,
    existing: Option<PriceAlert>,
    pub direction: Direction,
    pub target: String,
    pub percent: String,
    currency: AlertCurrency,
    pub frequency: AlertFrequency,
    pub note: String,
    pub sends_to_webhooks: bool,
    pub webhook_ids: Vec<EndpointId>,
    pub webhook_message: String,
    /// The price a percentage alert counts from; fixed when the sheet opens (or kept from
    /// the saved alert).
    reference: Option<f64>,
    /// The market now, in the chosen currency; the host refreshes it while the sheet is open.
    current_price: Option<f64>,
}

impl PriceAlertDraft {
    /// `default_currency` is the asset's own quote currency when it is one an alert can use.
    pub fn new(
        asset: AlertAsset,
        existing: Option<PriceAlert>,
        default_currency: Option<AlertCurrency>,
    ) -> Self {
        let mut state = Self {
            asset,
            direction: Direction::Above,
            target: String::new(),
            percent: String::new(),
            currency: default_currency.unwrap_or(AlertCurrency::Usd),
            frequency: AlertFrequency::Once,
            note: String::new(),
            sends_to_webhooks: false,
            webhook_ids: Vec::new(),
            webhook_message: String::new(),
            reference: None,
            current_price: None,
            existing: None,
        };
        if let Some(alert) = existing {
            state.currency = alert.currency;
            state.frequency = alert.frequency;
            state.note = alert.note.clone();
            state.sends_to_webhooks = !alert.webhook_endpoint_ids.is_empty();
            state.webhook_ids = alert.webhook_endpoint_ids.clone();
            state.webhook_message = alert.webhook_message.clone().unwrap_or_default();
            match &alert.condition {
                AlertCondition::CrossesAbove { target } => state.target = plain(*target),
                AlertCondition::CrossesBelow { target } => {
                    state.direction = Direction::Below;
                    state.target = plain(*target);
                }
                AlertCondition::RisesBy {
                    percent, reference, ..
                } => {
                    state.direction = Direction::Rises;
                    state.percent = plain(*percent);
                    state.reference = Some(*reference);
                }
                AlertCondition::FallsBy {
                    percent, reference, ..
                } => {
                    state.direction = Direction::Falls;
                    state.percent = plain(*percent);
                    state.reference = Some(*reference);
                }
                AlertCondition::Unsupported { .. } => {}
            }
            state.existing = Some(alert);
        }
        state
    }

    pub fn is_new(&self) -> bool {
        self.existing.is_none()
    }

    pub fn asset(&self) -> &AlertAsset {
        &self.asset
    }

    pub fn currency(&self) -> AlertCurrency {
        self.currency
    }

    pub fn current_price(&self) -> Option<f64> {
        self.current_price
    }

    pub fn reference(&self) -> Option<f64> {
        self.reference
    }

    /// Picks the currency. The price and the percentage reference were in the old one, so both
    /// are dropped until the host feeds a price in the new one.
    pub fn set_currency(&mut self, currency: AlertCurrency) {
        if currency != self.currency {
            self.currency = currency;
            self.current_price = None;
            self.reference = None;
        }
    }

    /// Feeds the market price (the sheet's host calls this every frame while it is open). A
    /// percentage alert counts from the first price it sees, and again after a currency change.
    pub fn set_current_price(&mut self, price: Option<f64>) {
        if let Some(price) = price {
            self.current_price = Some(price);
            if self.reference.is_none() {
                self.reference = Some(price);
            }
        }
    }

    /// Feeds the market's quote (USD; `rates` are USD → currency). A price that is not live, or a
    /// currency with no FX rate yet, leaves the draft without one rather than a wrong number.
    pub fn feed_quote(
        &mut self,
        quote: Option<&WatchlistQuote>,
        rates: &HashMap<AlertCurrency, f64>,
    ) {
        self.set_current_price(converted_price(quote, self.currency, rates));
    }

    /// The chosen currency is not the quotes' own, so the draft's host needs FX rates.
    pub fn needs_fx_rates(&self) -> bool {
        self.currency != AlertCurrency::Usd
    }

    // MARK: Parsing and hints

    fn parse_positive(text: &str) -> Option<f64> {
        text.trim()
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite() && *value > 0.0)
    }

    pub fn parsed_target(&self) -> Option<f64> {
        Self::parse_positive(&self.target)
    }

    pub fn parsed_percent(&self) -> Option<f64> {
        Self::parse_positive(&self.percent)
    }

    /// Targets a few percent away from the current price, on the side the alert watches.
    pub fn offset_options(&self) -> Vec<TargetOffset> {
        let Some(price) = self.current_price else {
            return Vec::new();
        };
        let sign = if self.direction == Direction::Below {
            -1
        } else {
            1
        };
        [1, 2, 5, 10]
            .into_iter()
            .map(|step| {
                let percent = step * sign;
                let level = price * (1.0 + f64::from(percent) / 100.0);
                TargetOffset {
                    percent,
                    target_text: self.currency.plain_price_text(level),
                }
            })
            .collect()
    }

    /// How the typed level sits against the market, and a warning when the alert could not
    /// fire soon.
    pub fn target_hint(&self) -> Option<(String, HintTone)> {
        let level = self.parsed_target()?;
        let price = self.current_price.filter(|price| *price > 0.0)?;
        let change = (level - price) / price * 100.0;
        let signed = format!(
            "{}{:.1}%",
            if change >= 0.0 { "+" } else { "−" },
            change.abs()
        );
        if self.direction == Direction::Above && level <= price {
            return Some((
                format!(
                    "{signed} from now. The price is already above this level; it fires after dropping below and crossing back up."
                ),
                HintTone::Warning,
            ));
        }
        if self.direction == Direction::Below && level >= price {
            return Some((
                format!(
                    "{signed} from now. The price is already below this level; it fires after rising above and crossing back down."
                ),
                HintTone::Warning,
            ));
        }
        Some((
            format!("{signed} from the current price"),
            HintTone::Neutral,
        ))
    }

    /// The rule being built, or `None` while the sheet is not complete.
    pub fn condition(&self) -> Option<AlertCondition> {
        match self.direction {
            Direction::Above => self
                .parsed_target()
                .map(|target| AlertCondition::CrossesAbove { target }),
            Direction::Below => self
                .parsed_target()
                .map(|target| AlertCondition::CrossesBelow { target }),
            Direction::Rises => {
                let (percent, reference) = (self.parsed_percent()?, self.reference?);
                Some(AlertCondition::rises_by(percent, reference))
            }
            Direction::Falls => {
                let (percent, reference) = (self.parsed_percent()?, self.reference?);
                Some(AlertCondition::falls_by(percent, reference))
            }
        }
    }

    pub fn can_save(&self) -> bool {
        self.condition().is_some()
    }

    /// "BTC crosses above $70,000.00 · once", shown beside the Save button.
    pub fn summary(&self) -> Option<String> {
        let condition = self.condition()?;
        let ticker = &self.asset.symbol;
        let price = |value: f64| self.currency.format_price(value);
        let what = match &condition {
            AlertCondition::CrossesAbove { target } => {
                format!("{ticker} crosses above {}", price(*target))
            }
            AlertCondition::CrossesBelow { target } => {
                format!("{ticker} crosses below {}", price(*target))
            }
            AlertCondition::RisesBy {
                percent, target, ..
            } => {
                format!("{ticker} rises {}% to {}", plain(*percent), price(*target))
            }
            AlertCondition::FallsBy {
                percent, target, ..
            } => {
                format!("{ticker} falls {}% to {}", plain(*percent), price(*target))
            }
            AlertCondition::Unsupported { .. } => return None,
        };
        Some(format!(
            "{what} · {}",
            if self.frequency == AlertFrequency::Once {
                "once"
            } else {
                "every time"
            }
        ))
    }

    /// The saved form: the existing alert updated in place (id, creation time kept), or a
    /// new one; always active. Webhooks off means none, whatever was picked before.
    pub fn result(&self, now_ms: i64) -> Option<PriceAlert> {
        let condition = self.condition()?;
        let mut alert = self
            .existing
            .clone()
            .unwrap_or_else(|| PriceAlert::new(self.asset.clone(), condition.clone(), now_ms));
        alert.condition = condition;
        alert.currency = self.currency;
        alert.frequency = self.frequency;
        alert.note = self.note.clone();
        alert.webhook_endpoint_ids = if self.sends_to_webhooks {
            self.webhook_ids.clone()
        } else {
            Vec::new()
        };
        let message = self.webhook_message.trim();
        alert.webhook_message = (!message.is_empty()).then(|| self.webhook_message.clone());
        alert.state = AlertState::Active;
        Some(alert)
    }

    /// Example values for the webhook message preview: what this alert would send for its
    /// own market.
    pub fn example_context(&self, now_ms: i64) -> AlertMessageContext {
        AlertMessageContext {
            ticker: Some(self.asset.instrument.api_symbol()),
            exchange: Some(self.asset.source().display_name().to_owned()),
            interval: Some("1".to_owned()),
            // A converted quote carries float noise (`83356.00999999998`); show what a person
            // would read, as the alert states prices.
            close: self
                .current_price
                .and_then(|price| self.currency.plain_price_text(price).parse().ok()),
            time_ms: Some(now_ms),
            now_ms,
            ..AlertMessageContext::default()
        }
    }
}

/// A plain, locale-independent number: the fields are parsed with `str::parse`.
fn plain(value: f64) -> String {
    format!("{value}")
}

/// The live USD quote in `currency` (`rates` are USD → currency), or `None` while it is stale,
/// missing, or the rate is unknown.
pub fn converted_price(
    quote: Option<&WatchlistQuote>,
    currency: AlertCurrency,
    rates: &HashMap<AlertCurrency, f64>,
) -> Option<f64> {
    let quote = quote.filter(|quote| quote.freshness == Freshness::Live)?;
    let usd = quote
        .last
        .filter(|price| price.is_finite() && *price > 0.0)?;
    if currency == AlertCurrency::Usd {
        return Some(usd);
    }
    rates
        .get(&currency)
        .copied()
        .filter(|rate| rate.is_finite() && *rate > 0.0)
        .map(|rate| usd * rate)
}

#[cfg(test)]
mod tests;

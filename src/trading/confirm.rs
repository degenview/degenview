//! The lines of the confirmation shown before an order that costs real money is sent. They state
//! what the order is and, where the venue has said so, what it will cost: the venue's own figures
//! from its preview, labelled as estimates, never the sheet's guess.

use degenview_brokers::Decimal;

use super::draft::{Estimate, PreviewResult, TradeDraft};
use super::model::{MarketRules, OrderClass};
use crate::model::broker_ticket::display;

/// A price with at least the market's precision (`90,000.00`, not `90,000`).
pub fn price_text(value: Decimal, increment: Decimal) -> String {
    let mut shown = value;
    let places = increment.normalize().scale();
    if shown.scale() < places {
        shown.rescale(places);
    }
    let text = shown.to_string();
    let (whole, fraction) = text
        .split_once('.')
        .map_or((text.as_str(), None), |(w, f)| (w, Some(f)));
    let digits = display(whole.parse::<Decimal>().unwrap_or_default());
    match fraction {
        Some(fraction) => format!("{digits}.{fraction}"),
        None => digits,
    }
}

/// `(label, value)` rows, in the order a person checks an order.
pub fn lines(
    draft: &TradeDraft,
    rules: &MarketRules,
    estimate: &Estimate,
    preview: Option<&PreviewResult>,
) -> Vec<(String, String)> {
    let mut rows = vec![
        ("Market".to_owned(), rules.name.clone()),
        ("Side".to_owned(), draft.side_label().to_owned()),
        ("Type".to_owned(), draft.class.label().to_owned()),
    ];
    let size = draft.size_text.trim();
    if draft.spend_in_quote {
        rows.push(("Spend".to_owned(), format!("{size} {}", rules.quote_asset)));
    } else {
        rows.push(("Size".to_owned(), format!("{size} {}", rules.base_asset)));
    }
    let price = |text: &str| {
        crate::model::broker_ticket::parse_decimal(text)
            .ok()
            .flatten()
            .map_or_else(
                || text.trim().to_owned(),
                |value| {
                    format!(
                        "{} {}",
                        price_text(value, rules.price_increment),
                        rules.quote_asset
                    )
                },
            )
    };
    if draft.uses_stop_price() {
        rows.push(("Stop price".to_owned(), price(&draft.stop_text)));
    }
    if draft.uses_limit_price() {
        let label = if draft.class == OrderClass::StopLimit {
            "Limit price"
        } else {
            "Price"
        };
        rows.push((label.to_owned(), price(&draft.price_text)));
    }
    if draft.post_only {
        rows.push(("Post only".to_owned(), "Rests; never takes".to_owned()));
    }
    let total = preview.and_then(|p| p.total).or(estimate.notional);
    if let Some(total) = total {
        let label = if preview.and_then(|p| p.total).is_some() {
            "Total (venue's estimate)"
        } else {
            "Total (estimate)"
        };
        rows.push((
            label.to_owned(),
            format!("≈ {} {}", display(total), rules.quote_asset),
        ));
    }
    if let Some(fee) = preview.and_then(|p| p.fee).or(estimate.fee) {
        rows.push((
            "Fee (estimate)".to_owned(),
            format!("≈ {} {}", display(fee), rules.quote_asset),
        ));
    }
    if let Some(preview) = preview {
        for warning in &preview.warnings {
            rows.push(("Note".to_owned(), warning.clone()));
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trading::model::TicketCaps;
    use degenview_brokers::Side;

    fn rules() -> MarketRules {
        MarketRules {
            name: "BTC-USD".to_owned(),
            base_asset: "BTC".to_owned(),
            quote_asset: "USD".to_owned(),
            price_increment: "0.01".parse().unwrap(),
            size_increment: "0.00000001".parse().unwrap(),
            min_size: "0.00000001".parse().unwrap(),
            min_notional: Decimal::ZERO,
            caps: TicketCaps {
                order_classes: vec![OrderClass::Limit],
                time_in_force: Vec::new(),
                post_only: true,
                reduce_only: false,
                brackets: false,
                quote_sized_market: true,
                shorting: false,
                edit_in_place: false,
            },
        }
    }

    #[test]
    fn a_price_has_the_markets_precision_and_grouped_thousands() {
        let inc: Decimal = "0.01".parse().unwrap();
        assert_eq!(price_text("90000".parse().unwrap(), inc), "90,000.00");
        assert_eq!(price_text("90000.5".parse().unwrap(), inc), "90,000.50");
        assert_eq!(price_text("0.12345".parse().unwrap(), inc), "0.12345");
        assert_eq!(price_text("7".parse().unwrap(), "1".parse().unwrap()), "7");
    }

    #[test]
    fn the_confirmation_states_the_order_and_the_venues_own_figures() {
        let mut draft = TradeDraft::new(Side::Buy);
        draft.size_text = "0.001".to_owned();
        draft.price_text = "90000".to_owned();
        draft.post_only = true;
        let estimate = Estimate {
            notional: Some("90".parse().unwrap()),
            fee: Some("0.4".parse().unwrap()),
            ..Estimate::default()
        };
        let preview = PreviewResult {
            accepted: true,
            total: Some("90.54".parse().unwrap()),
            fee: Some("0.54".parse().unwrap()),
            warnings: vec!["Large order".to_owned()],
            ..PreviewResult::default()
        };
        let rows = lines(&draft, &rules(), &estimate, Some(&preview));
        let get = |label: &str| {
            rows.iter()
                .find(|(l, _)| l == label)
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(get("Market"), Some("BTC-USD"));
        assert_eq!(get("Side"), Some("Buy"));
        assert_eq!(get("Type"), Some("Limit"));
        assert_eq!(get("Size"), Some("0.001 BTC"));
        assert_eq!(get("Price"), Some("90,000.00 USD"));
        assert_eq!(get("Post only"), Some("Rests; never takes"));
        assert_eq!(
            get("Total (venue's estimate)"),
            Some("≈ 90.54 USD"),
            "the venue's, not ours"
        );
        assert_eq!(get("Fee (estimate)"), Some("≈ 0.54 USD"));
        assert_eq!(get("Note"), Some("Large order"));
        // Without a preview the sheet's own figures are labelled as such.
        let rows = lines(&draft, &rules(), &estimate, None);
        assert!(
            rows.iter()
                .any(|(l, v)| l == "Total (estimate)" && v == "≈ 90 USD")
        );
    }

    #[test]
    fn spending_an_amount_says_spend_not_size() {
        let mut draft = TradeDraft::new(Side::Buy);
        draft.class = OrderClass::Market;
        draft.spend_in_quote = true;
        draft.size_text = "100".to_owned();
        let rows = lines(&draft, &rules(), &Estimate::default(), None);
        assert!(rows.iter().any(|(l, v)| l == "Spend" && v == "100 USD"));
        assert!(!rows.iter().any(|(l, _)| l == "Size"));
    }
}

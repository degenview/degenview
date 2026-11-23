//! The lines of the confirmation shown before an order that costs real money is sent. They state
//! what the order is and, where the venue has said so, what it will cost: the venue's own figures
//! from its preview, labelled as estimates, never the sheet's guess.

use super::draft::{Estimate, PreviewResult, TradeDraft};
use super::model::{MarketRules, OrderClass, VenueKind};
use crate::i18n::tr;
use crate::model::broker::ticket::{display, price_at};

/// Whether an order must be reviewed before it is sent. Only an order that costs real money is: a
/// market order always, whatever the settings say, and a resting one (limit, stop) when the user
/// asked to confirm those too.
pub fn needs_review(kind: VenueKind, class: OrderClass, confirm_resting: bool) -> bool {
    kind.is_real_money() && (class == OrderClass::Market || confirm_resting)
}

/// `(label, value)` rows, in the order a person checks an order.
pub fn lines(
    draft: &TradeDraft,
    rules: &MarketRules,
    account: Option<&str>,
    estimate: &Estimate,
    preview: Option<&PreviewResult>,
) -> Vec<(String, String)> {
    let mut rows = Vec::new();
    if let Some(account) = account {
        rows.push((tr!("Account").to_owned(), account.to_owned()));
    }
    rows.extend([
        (tr!("Market").to_owned(), rules.name.clone()),
        (tr!("Side").to_owned(), draft.side_label().to_owned()),
        (tr!("Type").to_owned(), draft.class.label().to_owned()),
    ]);
    let size = draft.size_text.trim();
    if draft.spend_in_quote {
        rows.push((
            tr!("Spend").to_owned(),
            format!("{size} {}", rules.quote_asset),
        ));
    } else {
        rows.push((
            tr!("Size").to_owned(),
            format!("{size} {}", rules.base_asset),
        ));
    }
    let price = |text: &str| {
        degenview_brokers::model::ticket::parse_decimal(text)
            .ok()
            .flatten()
            .map_or_else(
                || text.trim().to_owned(),
                |value| {
                    format!(
                        "{} {}",
                        price_at(value, rules.price_increment),
                        rules.quote_asset
                    )
                },
            )
    };
    if draft.uses_stop_price() {
        rows.push((tr!("Stop price").to_owned(), price(&draft.stop_text)));
    }
    if draft.uses_limit_price() {
        let label = if draft.class == OrderClass::StopLimit {
            tr!("Limit price")
        } else {
            tr!("Price")
        };
        rows.push((label.to_owned(), price(&draft.price_text)));
    }
    if draft.post_only {
        rows.push((
            tr!("Post only").to_owned(),
            tr!("Rests; never takes").to_owned(),
        ));
    }
    let total = preview.and_then(|p| p.total).or(estimate.notional);
    if let Some(total) = total {
        let label = if preview.and_then(|p| p.total).is_some() {
            tr!("Total (venue's estimate)")
        } else {
            tr!("Total (estimate)")
        };
        rows.push((
            label.to_owned(),
            format!("≈ {} {}", display(total), rules.quote_asset),
        ));
    }
    if let Some(fee) = preview.and_then(|p| p.fee).or(estimate.fee) {
        rows.push((
            tr!("Fee (estimate)").to_owned(),
            format!("≈ {} {}", display(fee), rules.quote_asset),
        ));
    }
    if let Some(preview) = preview {
        for warning in &preview.warnings {
            rows.push((tr!("Note").to_owned(), warning.clone()));
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trading::model::TicketCaps;
    use degenview_brokers::{Decimal, Side};

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
        assert_eq!(price_at("90000".parse().unwrap(), inc), "90,000.00");
        assert_eq!(price_at("90000.5".parse().unwrap(), inc), "90,000.50");
        assert_eq!(price_at("0.12345".parse().unwrap(), inc), "0.12345");
        assert_eq!(price_at("7".parse().unwrap(), "1".parse().unwrap()), "7");
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
        let rows = lines(
            &draft,
            &rules(),
            Some("Default portfolio"),
            &estimate,
            Some(&preview),
        );
        let get = |label: &str| {
            rows.iter()
                .find(|(l, _)| l == label)
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(get("Account"), Some("Default portfolio"));
        assert_eq!(
            rows[0].0, "Account",
            "the account is the first thing checked"
        );
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
        let rows = lines(&draft, &rules(), None, &estimate, None);
        assert!(
            rows.iter()
                .any(|(l, v)| l == "Total (estimate)" && v == "≈ 90 USD")
        );
    }

    #[test]
    fn a_market_order_always_asks_and_a_resting_one_asks_when_told_to_and_only_for_real_money() {
        use VenueKind::{Live, Simulated, Test};
        assert!(!needs_review(Live, OrderClass::Limit, false));
        assert!(needs_review(Live, OrderClass::Limit, true));
        assert!(
            needs_review(Live, OrderClass::Market, false),
            "whatever the settings say"
        );
        for harmless in [Simulated, Test] {
            assert!(
                !needs_review(harmless, OrderClass::Market, true),
                "{harmless:?} costs nothing"
            );
        }
    }

    #[test]
    fn spending_an_amount_says_spend_not_size() {
        let mut draft = TradeDraft::new(Side::Buy);
        draft.class = OrderClass::Market;
        draft.spend_in_quote = true;
        draft.size_text = "100".to_owned();
        let rows = lines(&draft, &rules(), None, &Estimate::default(), None);
        assert!(rows.iter().any(|(l, v)| l == "Spend" && v == "100 USD"));
        assert!(!rows.iter().any(|(l, _)| l == "Size"));
    }

    #[test]
    fn the_labels_follow_the_language_and_the_values_do_not() {
        use crate::i18n::{Locale, with_locale};
        let mut draft = TradeDraft::new(Side::Buy);
        draft.size_text = "0.001".to_owned();
        draft.price_text = "90000".to_owned();
        let rows = |locale| {
            with_locale(locale, || {
                lines(&draft, &rules(), Some("Main"), &Estimate::default(), None)
            })
        };
        let english = rows(Locale::ENGLISH);
        let pseudo = rows(Locale::PSEUDO);
        assert_eq!(english.len(), pseudo.len());
        for (plain, shown) in english.iter().zip(&pseudo) {
            assert_ne!(plain.0, shown.0, "{} is a label", plain.0);
        }
        assert_eq!(english[0].1, "Main");
        assert_eq!(pseudo[0].1, "Main", "the account's name is the user's");
        assert_eq!(english[1].1, pseudo[1].1, "the market is provider data");
    }
}

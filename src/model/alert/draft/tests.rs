use super::*;
use crate::model::alert::message::{DEFAULT_PRICE_ALERT_TEMPLATE, render};
use crate::model::{DataSource, InstrumentId};

fn asset() -> AlertAsset {
    AlertAsset::new(
        InstrumentId::new(DataSource::Binance, "BTCUSDT"),
        "BTC",
        "Bitcoin",
    )
}

fn state() -> PriceAlertDraft {
    PriceAlertDraft::new(asset(), None, None)
}

#[test]
fn an_empty_sheet_cannot_be_saved() {
    let state = state();
    assert!(state.is_new() && !state.can_save());
    assert_eq!(state.summary(), None);
    assert_eq!(state.current_price(), None);
}

#[test]
fn targets_parse_as_positive_finite_numbers() {
    let mut state = state();
    for (text, expected) in [
        ("70000", Some(70_000.0)),
        ("  0.5 ", Some(0.5)),
        ("1e5", Some(100_000.0)),
        ("0", None),
        ("-3", None),
        ("abc", None),
        ("", None),
        ("NaN", None),
        ("inf", None),
    ] {
        state.target = text.into();
        assert_eq!(state.parsed_target(), expected, "{text:?}");
    }
}

#[test]
fn a_price_level_builds_a_crossing_condition_and_a_summary() {
    let mut state = state();
    state.target = "70000".into();
    assert_eq!(
        state.condition(),
        Some(AlertCondition::CrossesAbove { target: 70_000.0 })
    );
    assert_eq!(
        state.summary().as_deref(),
        Some("BTC crosses above $70,000.00 · once")
    );
    state.direction = Direction::Below;
    state.frequency = AlertFrequency::EveryTime;
    assert_eq!(
        state.summary().as_deref(),
        Some("BTC crosses below $70,000.00 · every time")
    );
}

#[test]
fn percentage_alerts_need_a_reference_price() {
    let mut state = state();
    state.direction = Direction::Rises;
    state.percent = "10".into();
    assert!(!state.can_save(), "no price yet, nothing to count from");
    state.set_current_price(Some(100.0));
    assert_eq!(state.reference(), Some(100.0));
    let condition = state.condition().unwrap();
    assert!((condition.target().unwrap() - 110.0).abs() < 1e-9);
    assert_eq!(
        state.summary().as_deref(),
        Some("BTC rises 10% to $110.00 · once")
    );
    // The reference is fixed when first seen; later prices do not move it.
    state.set_current_price(Some(200.0));
    assert_eq!(state.reference(), Some(100.0));
    assert_eq!(state.current_price(), Some(200.0));
    state.direction = Direction::Falls;
    state.percent = "25".into();
    assert!((state.condition().unwrap().target().unwrap() - 75.0).abs() < 1e-9);
    assert_eq!(
        state.summary().as_deref(),
        Some("BTC falls 25% to $75.00 · once")
    );
}

#[test]
fn a_saved_alert_reopens_exactly_as_it_was_made() {
    let mut saved = PriceAlert::new(asset(), AlertCondition::rises_by(5.0, 80.0), 7);
    saved.currency = AlertCurrency::Eur;
    saved.frequency = AlertFrequency::EveryTime;
    saved.note = "swing".into();
    saved.webhook_endpoint_ids = vec![3, 4];
    saved.webhook_message = Some("{{ticker}} up".into());
    let state = PriceAlertDraft::new(asset(), Some(saved.clone()), None);
    assert!(!state.is_new());
    assert_eq!(state.direction, Direction::Rises);
    assert_eq!(state.percent, "5");
    assert_eq!(
        state.reference(),
        Some(80.0),
        "a saved percentage alert keeps its price"
    );
    assert_eq!(state.currency(), AlertCurrency::Eur);
    assert!(state.sends_to_webhooks);
    assert_eq!(state.webhook_ids, vec![3, 4]);
    let result = state.result(99).unwrap();
    assert_eq!(result.id, saved.id);
    assert_eq!(result.created_at_ms, 7, "editing keeps the creation time");
    assert_eq!(result.condition, saved.condition);
    assert_eq!(result.note, "swing");
    assert_eq!(
        result.state,
        AlertState::Active,
        "saving re-enables a triggered alert"
    );
}

#[test]
fn webhooks_off_means_none_even_if_some_were_picked() {
    let mut state = state();
    state.target = "100".into();
    state.sends_to_webhooks = true;
    state.webhook_ids = vec![6];
    state.webhook_message = "  hello  ".into();
    let on = state.result(1).unwrap();
    assert_eq!(on.webhook_endpoint_ids, vec![6]);
    assert_eq!(
        on.webhook_message.as_deref(),
        Some("  hello  "),
        "kept as typed"
    );
    state.sends_to_webhooks = false;
    assert!(state.result(1).unwrap().webhook_endpoint_ids.is_empty());
    state.sends_to_webhooks = true;
    state.webhook_message = "   ".into();
    assert_eq!(state.result(1).unwrap().webhook_message, None);
}

#[test]
fn quick_targets_sit_on_the_side_the_alert_watches() {
    let mut state = state();
    assert!(
        state.offset_options().is_empty(),
        "no price, no suggestions"
    );
    state.set_current_price(Some(1_000.0));
    let up = state.offset_options();
    assert_eq!(
        up.iter().map(TargetOffset::label).collect::<Vec<_>>(),
        ["+1%", "+2%", "+5%", "+10%"]
    );
    assert_eq!(up[0].target_text, "1010.00");
    assert_eq!(up[3].target_text, "1100.00");
    state.direction = Direction::Below;
    let down = state.offset_options();
    assert_eq!(down[0].label(), "−1%");
    assert_eq!(down[0].target_text, "990.00");
    // Every suggestion parses back as a usable target.
    state.target = down[2].target_text.clone();
    assert!((state.parsed_target().unwrap() - 950.0).abs() < 1e-9);
}

#[test]
fn the_hint_warns_when_the_price_is_already_past_the_level() {
    let mut state = state();
    state.set_current_price(Some(100.0));
    state.target = "120".into();
    let (text, tone) = state.target_hint().unwrap();
    assert_eq!(tone, HintTone::Neutral);
    assert_eq!(text, "+20.0% from the current price");

    state.target = "90".into();
    let (text, tone) = state.target_hint().unwrap();
    assert_eq!(tone, HintTone::Warning);
    assert!(text.starts_with("−10.0% from now. The price is already above this level"));

    state.direction = Direction::Below;
    state.target = "110".into();
    let (text, tone) = state.target_hint().unwrap();
    assert_eq!(tone, HintTone::Warning);
    assert!(text.contains("already below this level"));

    state.target = "".into();
    assert_eq!(state.target_hint(), None);
    state.target = "90".into();
    state.set_current_price(None);
    state.current_price = None;
    assert_eq!(state.target_hint(), None, "no price, no hint");
}

#[test]
fn changing_currency_drops_the_old_price_and_reference() {
    let mut state = state();
    state.set_current_price(Some(100.0));
    state.set_currency(AlertCurrency::Usd);
    assert_eq!(state.current_price(), Some(100.0), "same currency, kept");
    state.set_currency(AlertCurrency::Eur);
    assert_eq!(state.current_price(), None);
    assert_eq!(state.reference(), None);
    state.set_current_price(Some(92.0));
    assert_eq!(state.reference(), Some(92.0));
}

fn live(last: f64) -> WatchlistQuote {
    WatchlistQuote {
        last: Some(last),
        freshness: Freshness::Live,
        ..WatchlistQuote::default()
    }
}

#[test]
fn the_price_is_the_live_quote_in_the_chosen_currency() {
    let rates = HashMap::from([(AlertCurrency::Eur, 0.9)]);
    let quote = live(100.0);
    assert_eq!(
        converted_price(Some(&quote), AlertCurrency::Usd, &rates),
        Some(100.0)
    );
    assert_eq!(
        converted_price(Some(&quote), AlertCurrency::Eur, &rates),
        Some(90.0)
    );
    assert_eq!(
        converted_price(Some(&quote), AlertCurrency::Gbp, &rates),
        None,
        "no rate yet, no price"
    );
    let stale = WatchlistQuote {
        freshness: Freshness::Stale,
        ..live(100.0)
    };
    assert_eq!(
        converted_price(Some(&stale), AlertCurrency::Usd, &rates),
        None
    );
    assert_eq!(converted_price(None, AlertCurrency::Usd, &rates), None);
}

#[test]
fn the_default_currency_is_the_assets_own_when_alertable() {
    let eur = PriceAlertDraft::new(asset(), None, Some(AlertCurrency::Eur));
    assert_eq!(eur.currency(), AlertCurrency::Eur);
    assert_eq!(state().currency(), AlertCurrency::Usd);
}

#[test]
fn the_webhook_preview_renders_with_this_alerts_own_market() {
    let mut state = state();
    state.set_current_price(Some(67_432.5));
    let context = state.example_context(1_714_571_130_000);
    assert_eq!(
        render(DEFAULT_PRICE_ALERT_TEMPLATE, &context),
        "BTCUSDT alert: 67432.5"
    );
    assert_eq!(
        render(
            "{{exchange}} {{ticker}} {{interval}} {{time}} {{open}}",
            &context
        ),
        "Binance BTCUSDT 1 2024-05-01T13:45:30Z {{open}}",
        "a value the alert does not have stays a placeholder"
    );
}

#[test]
fn directions_describe_themselves() {
    assert!(Direction::Rises.is_percent() && !Direction::Above.is_percent());
    assert_eq!(Direction::Below.title(), "Below");
    assert!(Direction::Falls.blurb().contains("drops by a percentage"));
}

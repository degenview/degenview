//! A value `request.security` returns that is a collection is the chart's own each time: the site
//! remembers it for the next bar and for a second call in the same bar, and the script changing
//! what it got (`array.push`) must never show in what it gets next.

use super::lookahead::{hourly, plot_values, session_with};
use crate::model::Kline;
use crate::pine::compiler::program::{BarEvent, BarPhase};
use crate::pine::tests::bench::run_script;

/// The plot `size` of `source` (a script that ends in `plot(.., "size")`) over `chart`.
fn sizes(source: &str, chart: &[Kline]) -> Vec<Option<f64>> {
    let session = run_script(
        &format!("//@version=6\nindicator(\"T\")\n{source}"),
        chart,
        false,
        |_| {},
    );
    plot_values(&session, "size")
}

fn always(size: f64, bars: usize) -> Vec<Option<f64>> {
    vec![Some(size); bars]
}

#[test]
fn an_array_the_script_pushes_to_is_a_fresh_copy_on_every_bar() {
    let chart = hourly(12);
    for options in ["", ", lookahead = barmerge.lookahead_on"] {
        let source = format!(
            "a = request.security(syminfo.tickerid, \"240\", array.from(close){options})\nif not na(a)\n    array.push(a, 1.0)\nplot(na(a) ? na : array.size(a), \"size\")"
        );
        // `array.from(close)` holds one value; after the push, two, on every bar.
        let values = sizes(&source, &chart);
        assert_eq!(
            values[3..],
            always(2.0, 9)[..],
            "lookahead options {options:?}"
        );
    }
}

#[test]
fn a_second_call_in_a_bar_does_not_see_what_the_script_did_to_the_first() {
    // One call site run twice in a bar (a loop): the second answer is the bar's cached one.
    let chart = hourly(12);
    for options in ["", ", lookahead = barmerge.lookahead_on"] {
        let source = format!(
            "float second = na\nfor i = 0 to 1\n    a = request.security(syminfo.tickerid, \"240\", array.from(close){options})\n    if not na(a)\n        if i == 0\n            array.push(a, 1.0)\n        else\n            second := array.size(a)\nplot(second, \"size\")"
        );
        assert_eq!(
            sizes(&source, &chart)[3..],
            always(1.0, 9)[..],
            "{options:?}"
        );
    }
}

#[test]
fn a_tuple_of_arrays_and_an_object_with_an_array_are_copies_too() {
    let chart = hourly(12);
    let tuple = "[a, b] = request.security(syminfo.tickerid, \"240\", [array.from(close), array.from(open)], lookahead = barmerge.lookahead_on)\narray.push(a, 1.0)\narray.push(b, 2.0)\nplot(array.size(a) + array.size(b), \"size\")";
    assert_eq!(sizes(tuple, &chart)[3..], always(4.0, 9)[..]);
    let object = "type Holder\n    array<float> items\nmake() =>\n    Holder.new(array.from(close))\nb = request.security(syminfo.tickerid, \"240\", make(), lookahead = barmerge.lookahead_on)\narray.push(b.items, 1.0)\nplot(array.size(b.items), \"size\")";
    assert_eq!(sizes(object, &chart)[3..], always(2.0, 9)[..]);
}

#[test]
fn another_symbols_array_is_a_fresh_copy_on_every_bar() {
    let chart = hourly(12);
    for options in ["", ", lookahead = barmerge.lookahead_on"] {
        let source = format!(
            "//@version=6\nindicator(\"T\")\na = request.security(\"ETHUSDT\", \"240\", array.from(close){options})\nif not na(a)\n    array.push(a, 1.0)\nplot(na(a) ? na : array.size(a), \"size\")"
        );
        let mut session = session_with(&source, Some(Box::new(super::lookahead::EthFourHour)));
        session.evaluate(&chart).expect("script runs");
        assert_eq!(
            plot_values(&session, "size")[3..],
            always(2.0, 9)[..],
            "{options:?}"
        );
    }
}

#[test]
fn the_intrabars_of_a_bar_are_a_fresh_array_for_each_call_in_it() {
    // Two one minute chart bars with 15 second intrabars from a provider; one call site, run twice.
    let source = "//@version=6\nindicator(\"T\")\nfloat second = na\nfor i = 0 to 1\n    a = request.security_lower_tf(syminfo.tickerid, \"15S\", close)\n    if i == 0\n        array.push(a, 1.0)\n    else\n        second := array.size(a)\nplot(second, \"size\")";
    let mut session = session_with(
        source,
        Some(Box::new(super::lookahead::Intrabars::fifteen_seconds())),
    );
    let chart = super::lookahead::minute_chart(2);
    session.evaluate(&chart).expect("script runs");
    assert_eq!(plot_values(&session, "size"), [Some(4.0), Some(4.0)]);
}

#[test]
fn an_array_survives_a_live_bars_ticks_unchanged() {
    // The same live bar ticked twice (the second rolls the first back): each tick gets its own copy.
    let chart = hourly(6);
    let source = "//@version=6\nindicator(\"T\")\na = request.security(syminfo.tickerid, \"240\", array.from(close), lookahead = barmerge.lookahead_on)\nif not na(a)\n    array.push(a, 1.0)\nplot(na(a) ? na : array.size(a), \"size\")";
    let mut session = session_with(source, None);
    session
        .load_history(&chart[..5], true, None)
        .expect("history runs");
    for is_new in [true, false, false] {
        session
            .execute(
                BarEvent {
                    candle: chart[5].clone(),
                    phase: BarPhase::RealtimeTick { is_new },
                },
                true,
                None,
            )
            .expect("a tick runs");
        assert_eq!(plot_values(&session, "size").last(), Some(&Some(2.0)));
    }
}

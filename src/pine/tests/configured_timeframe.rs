//! A chart tells its script the timeframe it is set to; the script does not read it off the gap
//! between two bars. A bucket cut short by midnight (the last of a `59s` day lasts 24 seconds)
//! would otherwise shorten the chart's length for the next bar, and `request.security_lower_tf`
//! to a timeframe the chart still has would be refused.

use crate::model::Kline;
use crate::pine::compiler::NoLibraries;
use crate::pine::compiler::program::SymbolInfo;
use crate::pine::runtime::execution::ExecutionController;
use crate::pine::runtime::support::{SecurityDataProvider, SecurityKey};
use crate::pine::syntax::token::Limits;
use std::collections::HashMap;

const DAY: i64 = 86_400_000;
/// Where a `59s` day's last bucket opens: 24 seconds before midnight.
const SHORT_BUCKET: i64 = DAY - 24_000;

const SOURCE: &str = "//@version=6\nindicator(\"T\")\n\
    plot(timeframe.in_seconds())\n\
    plot(array.size(request.security_lower_tf(syminfo.tickerid, \"30S\", close)))";

/// `SOURCE` with the finer timeframe the provider's candles really have, so a candle ends where the
/// timeframe says and a bar takes the ones that end inside it.
const COUNT_SOURCE: &str = "//@version=6\nindicator(\"T\")\n\
    plot(timeframe.in_seconds())\n\
    plot(array.size(request.security_lower_tf(syminfo.tickerid, \"8S\", close)))";

fn bar(open_time: i64, close_time: i64) -> Kline {
    Kline {
        open_time,
        close_time,
        open: 1.0,
        high: 1.0,
        low: 1.0,
        close: 1.0,
        volume: 1.0,
    }
}

/// `59s` bars: two before midnight, the 24 s bucket that ends the day, and three of the next.
fn bars() -> Vec<Kline> {
    let whole = |open: i64| bar(open, open + 58_999);
    vec![
        whole(SHORT_BUCKET - 2 * 59_000),
        whole(SHORT_BUCKET - 59_000),
        bar(SHORT_BUCKET, DAY - 1),
        whole(DAY),
        whole(DAY + 59_000),
        whole(DAY + 118_000),
    ]
}

/// A candle every eight seconds, whatever length was asked for: enough to count which ones a chart
/// bar takes (`COUNT_SOURCE` asks for eight).
struct EightSeconds;

impl SecurityDataProvider for EightSeconds {
    fn candles(&self, _: &SecurityKey) -> Option<Vec<Kline>> {
        Some(
            (10_000..11_000)
                .map(|index| bar(index * 8_000, index * 8_000 + 7_999))
                .collect(),
        )
    }
}

fn controller() -> ExecutionController {
    controller_for(SOURCE)
}

fn controller_for(source: &str) -> ExecutionController {
    ExecutionController::new(
        source,
        HashMap::new(),
        SymbolInfo {
            ticker: "BTCUSDT".to_owned(),
            ticker_id: "BINANCE:BTCUSDT".to_owned(),
            ..SymbolInfo::default()
        },
        None,
        Limits::default(),
        &NoLibraries,
        Some(Box::new(EightSeconds)),
        None,
    )
    .expect("compiles")
}

fn plot(update: &crate::pine::runtime::execution::ExecutionUpdate, index: usize) -> Vec<f64> {
    update.output.plots[index]
        .values
        .iter()
        .map(|value| value.expect("a value on every bar"))
        .collect()
}

#[test]
fn a_cut_short_bucket_does_not_shorten_the_charts_length_or_halt_the_script() {
    let mut chart = controller().with_timeframe_seconds(59.0);
    let update = chart.update(&bars(), None, DAY + 200_000);
    assert!(update.error.is_none(), "{:?}", update.error);
    assert_eq!(plot(&update, 0), vec![59.0; 6]);
}

#[test]
fn a_cut_short_bucket_takes_only_its_own_intrabars() {
    let mut chart = controller_for(COUNT_SOURCE).with_timeframe_seconds(59.0);
    let update = chart.update(&bars(), None, DAY + 200_000);
    let sizes = plot(&update, 1);
    // The 24 s bucket holds the candles opening at 86,376, 86,384 and 86,392 s; the ones at
    // 86,400 s and after belong to the next bar, which is 59 s long. A candle belongs to the bar its
    // last instant falls in: the one opening at 86,456 s ends at 86,464 s, inside the bar after.
    assert_eq!(sizes[2], 3.0);
    assert_eq!(sizes[3], 7.0, "86,400 .. 86,448 s, every eight seconds");
}

#[test]
fn without_a_configured_length_the_gap_between_bars_halts_the_script() {
    let mut chart = controller();
    let update = chart.update(&bars(), None, DAY + 200_000);
    // The first bar of the next day is 24 s after the one before it: a length the chart does not
    // have, which a 30 s request is then longer than.
    let error = update.error.expect("the script halts");
    assert_eq!(error.code, "PINE4021");
}

#[test]
fn a_price_based_charts_candle_length_comes_before_the_timeframe() {
    let mut chart = controller()
        .with_timeframe_seconds(59.0)
        .with_bar_seconds(3_600.0);
    let update = chart.update(&bars(), None, DAY + 200_000);
    assert_eq!(plot(&update, 0), vec![3_600.0; 6]);
}

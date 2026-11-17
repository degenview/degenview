//! The line that says a chart of the tab cannot draw the highlighted timeframe.

use crate::i18n::{tr, tr_n};
use crate::model::{DataSource, Timeframe};

/// "CoinGecko can't chart 1m", naming up to two sources and counting more; `None` when every
/// source can.
pub(super) fn cannot_chart(sources: &[DataSource], timeframe: Timeframe) -> Option<String> {
    let mut refusing: Vec<DataSource> = Vec::new();
    for source in sources {
        if !source.serves(timeframe) && !refusing.contains(source) {
            refusing.push(*source);
        }
    }
    (!refusing.is_empty()).then(|| sentence(&refusing, timeframe))
}

fn sentence(refusing: &[DataSource], timeframe: Timeframe) -> String {
    match refusing {
        [one] => tr!(
            "{source} can't chart {timeframe}",
            source = one.display_name(),
            timeframe = timeframe
        ),
        [first, second] => tr!(
            "{first} and {second} can't chart {timeframe}",
            first = first.display_name(),
            second = second.display_name(),
            timeframe = timeframe
        ),
        many => tr_n!(
            many.len(),
            "{count} source can't chart {timeframe}",
            "{count} sources can't chart {timeframe}",
            timeframe = timeframe
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_source_is_named() {
        let sources = [
            DataSource::Binance,
            DataSource::CoinGecko,
            DataSource::CoinGecko,
        ];
        assert_eq!(
            cannot_chart(&sources, Timeframe::MINUTE_1).as_deref(),
            Some("CoinGecko can't chart 1m")
        );
    }

    #[test]
    fn nothing_is_said_when_every_source_can() {
        assert_eq!(
            cannot_chart(
                &[DataSource::Binance, DataSource::Polymarket],
                Timeframe::MINUTE_1
            ),
            None
        );
        assert_eq!(cannot_chart(&[], Timeframe::MINUTE_1), None);
    }

    #[test]
    fn two_sources_are_named_and_more_are_counted() {
        let (a, b, c) = (
            DataSource::CoinGecko,
            DataSource::Alpaca,
            DataSource::Binance,
        );
        assert_eq!(
            sentence(&[a, b], Timeframe::MINUTE_5),
            "CoinGecko and Alpaca (IEX) can't chart 5m"
        );
        assert_eq!(
            sentence(&[a, b, c], Timeframe::MINUTE_5),
            "3 sources can't chart 5m"
        );
    }
}

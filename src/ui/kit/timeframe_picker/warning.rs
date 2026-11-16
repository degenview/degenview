//! The line that says a chart of the tab cannot draw the highlighted timeframe.

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
    let who = match refusing {
        [one] => one.display_name().to_owned(),
        [first, second] => format!("{} and {}", first.display_name(), second.display_name()),
        many => format!("{} sources", many.len()),
    };
    format!("{who} can't chart {timeframe}")
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

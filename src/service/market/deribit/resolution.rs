//! The `resolution` Deribit is asked for, for the candle lengths it cuts where this app does.

use crate::model::{DataSource, Timeframe};

/// Deribit's `resolution` for a candle length it serves itself: its minutes, `"1"` to `"120"`. Its
/// longer ones (`"180"`, `"360"`, `"720"`, `"1D"`) open at 02:00 or 08:00 UTC, so they are not
/// natives, and `None` here.
pub(super) fn resolution(timeframe: Timeframe) -> Option<String> {
    DataSource::Deribit
        .natives()
        .contains(&timeframe)
        .then(|| (timeframe.nominal_ms() / 60_000).to_string())
}

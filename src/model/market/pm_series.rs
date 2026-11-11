use serde::{Deserialize, Serialize};

/// One choice of a prediction-market event charted as a line: the market id (a CLOB token id on
/// Polymarket, `"SERIES/MARKET"` on Kalshi) and the label drawn at the end of its line.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PmSeries {
    #[serde(rename = "tokenID")]
    pub token_id: String,
    pub label: String,
}

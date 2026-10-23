//! The per-chart state a saved layout restores. Lives in `model/` (not `workspace.rs`) so the
//! `SavedView` model can reference it without depending on the workspace.

use crate::model::pine_instance::PineInstance;
use crate::model::portfolio_card::PortfolioCard;
use crate::model::viewport::SavedViewport;
use crate::model::{ChartAppearance, DataSource, Drawing, IndicatorConfig};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PersistedChart {
    pub id: u64,
    pub symbol: String,
    pub source: DataSource,
    pub log_scale: bool,
    pub y_zoom: f64,
    pub indicators: IndicatorConfig,
    #[serde(default)]
    pub appearance: ChartAppearance,
    /// Decoded one by one: a drawing this version cannot read is kept, not fatal to the chart.
    #[serde(default, with = "crate::model::drawings::lenient")]
    pub drawings: Vec<Drawing>,
    /// The Pine scripts applied to the chart, in order; old rows decode as none.
    #[serde(default)]
    pub pine_instances: Vec<PineInstance>,
    /// The scroll-zoom candle count at save time; old rows decode as `0` (auto).
    #[serde(default)]
    pub candle_count: i64,
    /// Where the chart was scrolled and a price range placed by hand; old rows decode as the
    /// newest bars on autoscale.
    #[serde(default)]
    pub viewport: SavedViewport,
    /// An optional title shown instead of the provider id (prediction events and DEX pairs).
    #[serde(default)]
    pub title: Option<String>,
    /// Prediction markets: the choices drawn together as lines; empty for a single market.
    #[serde(default)]
    pub pm_series: Vec<super::PmSeries>,
    /// Provider artwork retained so a chart added to a watchlist keeps its image.
    #[serde(default)]
    pub image_url: Option<String>,
    /// A portfolio card (`symbol` is its key and `source` a placeholder); `None` for a market.
    #[serde(default)]
    pub card: Option<PortfolioCard>,
}

impl PersistedChart {
    /// The chart's scripts.
    pub fn instances(&self) -> Vec<PineInstance> {
        self.pine_instances.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chart() -> PersistedChart {
        PersistedChart {
            id: 1,
            symbol: "BTCUSDT".into(),
            source: DataSource::Binance,
            log_scale: false,
            y_zoom: 1.0,
            indicators: IndicatorConfig::default(),
            appearance: ChartAppearance::default(),
            drawings: Vec::new(),
            pine_instances: Vec::new(),
            candle_count: 0,
            viewport: SavedViewport::default(),
            title: None,
            pm_series: Vec::new(),
            image_url: None,
            card: None,
        }
    }

    #[test]
    fn a_card_chart_round_trips_and_an_old_row_has_none() {
        use crate::model::portfolio_card::{CardKind, PortfolioCard};
        let mut saved = chart();
        saved.card = Some(PortfolioCard::new(CardKind::Allocation, Some(2)));
        let encoded = serde_json::to_string(&saved).unwrap();
        let decoded: PersistedChart = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, saved);

        let mut row = serde_json::to_value(chart()).unwrap();
        row.as_object_mut().unwrap().remove("card");
        let restored: PersistedChart = serde_json::from_value(row).unwrap();
        assert_eq!(restored.card, None);
    }

    #[test]
    fn a_scrolled_chart_keeps_its_place_and_an_old_row_follows_the_newest_bar() {
        use crate::model::viewport::PriceRange;
        let mut saved = chart();
        saved.viewport = SavedViewport {
            right_offset: -12.5,
            price_range: Some(PriceRange::new(60_000.0, 70_000.0)),
        };
        let decoded: PersistedChart =
            serde_json::from_str(&serde_json::to_string(&saved).unwrap()).unwrap();
        assert_eq!(decoded, saved);

        let mut row = serde_json::to_value(chart()).unwrap();
        row.as_object_mut().unwrap().remove("viewport");
        let restored: PersistedChart = serde_json::from_value(row).unwrap();
        assert_eq!(restored.viewport, SavedViewport::default());
    }

    fn anchors() -> Vec<crate::model::Anchor> {
        (0..4)
            .map(|index| crate::model::Anchor {
                time_ms: index * 60_000,
                price: 100.0 + index as f64 * 3.5,
            })
            .collect()
    }

    /// One drawing of every tool, as a chart would save them.
    fn every_drawing() -> Vec<crate::model::Drawing> {
        use crate::model::{Drawing, Placement, Tool};
        let anchors = anchors();
        Tool::GROUPED
            .iter()
            .enumerate()
            .map(|(index, tool)| {
                let count = match tool.spec().placement {
                    Placement::Points(count) => usize::from(count),
                    Placement::Single => 1,
                    _ => 3,
                };
                let mut drawing = Drawing::build(
                    *tool,
                    index as u64 + 1,
                    &anchors[..count],
                    tool.default_style(),
                )
                .expect("builds");
                drawing.set_text("note");
                drawing
            })
            .chain([
                {
                    let mut locked = Drawing::build(
                        Tool::TrendLine,
                        90,
                        &anchors[..2],
                        Tool::TrendLine.default_style(),
                    )
                    .expect("builds");
                    locked.set_locked(true);
                    locked
                },
                Drawing::Fib {
                    id: 91,
                    a: anchors[1],
                    b: anchors[2],
                    locked: false,
                    hidden: true,
                    visibility: crate::model::Visibility::ALL
                        .with(crate::model::Timeframe::Week, false),
                },
            ])
            .collect()
    }

    #[test]
    fn every_drawing_survives_a_save_and_reopen() {
        let mut saved = chart();
        saved.drawings = every_drawing();
        assert!(saved.drawings.len() > 16);
        let restored: PersistedChart =
            serde_json::from_str(&serde_json::to_string(&saved).unwrap()).unwrap();
        assert_eq!(restored, saved);
        // Locked, hidden and per-timeframe visibility came back as saved.
        assert!(restored.drawings.iter().any(crate::model::Drawing::locked));
        assert!(restored.drawings.iter().any(crate::model::Drawing::hidden));
    }

    #[test]
    fn a_row_saved_by_a_version_without_drawing_styles_still_opens() {
        // The exact drawings the previous version wrote, in a chart row.
        let mut row = serde_json::to_value(chart()).unwrap();
        row["drawings"] = serde_json::json!([
            {"TrendLine": {"id": 1, "a": {"time_ms": 0, "price": 1.0}, "b": {"time_ms": 60000, "price": 2.0}, "locked": false}},
            {"Brush": {"id": 2, "points": [{"time_ms": 0, "price": 1.0}, {"time_ms": 1000, "price": 1.5}], "width": 2.0, "locked": true}},
            {"Ruler": {"id": 3, "a": {"time_ms": 0, "price": 1.0}, "b": {"time_ms": 60000, "price": 2.0}}}
        ]);
        let restored: PersistedChart = serde_json::from_value(row).unwrap();
        assert_eq!(restored.drawings.len(), 3);
        assert!(restored.drawings[1].locked());
        assert!(
            restored
                .drawings
                .iter()
                .all(|d| !d.hidden() && d.visibility().is_all())
        );
        // And a chart row with no drawings field at all reads as none.
        let mut bare = serde_json::to_value(chart()).unwrap();
        bare.as_object_mut().unwrap().remove("drawings");
        let none: PersistedChart = serde_json::from_value(bare).unwrap();
        assert!(none.drawings.is_empty());
    }

    #[test]
    fn a_drawing_this_version_cannot_read_neither_costs_the_chart_nor_is_lost() {
        let mut row = serde_json::to_value(chart()).unwrap();
        let future =
            serde_json::json!({"Shape": {"id": 8, "geometry": {"hologram": {"depth": 3}}}});
        let known = serde_json::to_value(&every_drawing()[0]).unwrap();
        row["drawings"] = serde_json::json!([known.clone(), future.clone(), "garbage"]);
        let restored: PersistedChart = serde_json::from_value(row).unwrap();
        // The chart opens with the drawing it can read; the others ride along untouched.
        assert_eq!(restored.drawings.len(), 3);
        assert!(restored.drawings[0].tool().is_some());
        assert!(restored.drawings[1].tool().is_none());
        let resaved = serde_json::to_value(&restored).unwrap();
        assert_eq!(resaved["drawings"][1], future);
        assert_eq!(resaved["drawings"][2], serde_json::json!("garbage"));
        assert_eq!(resaved["drawings"][0], known);
    }

    #[test]
    fn invalid_geometry_is_kept_but_never_reaches_the_renderer() {
        let mut row = serde_json::to_value(chart()).unwrap();
        // A path with no points and a rectangle whose price is not a number.
        row["drawings"] = serde_json::json!([
            {"Shape": {"id": 1, "geometry": {"path": {"points": []}}}},
            {"Shape": {"id": 2, "geometry": {"rectangle": {"a": {"time_ms": 0, "price": null}, "b": {"time_ms": 1, "price": 2.0}}}}}
        ]);
        let restored: PersistedChart = serde_json::from_value(row).unwrap();
        assert_eq!(restored.drawings.len(), 2);
        assert!(restored.drawings.iter().all(|d| d.tool().is_none()));
    }

    #[test]
    fn old_chart_without_appearance_uses_swift_defaults() {
        let mut row = serde_json::to_value(chart()).unwrap();
        row.as_object_mut().unwrap().remove("appearance");
        let restored: PersistedChart = serde_json::from_value(row).unwrap();
        assert_eq!(restored.appearance, ChartAppearance::default());
    }

    #[test]
    fn the_chart_type_and_its_settings_survive_a_save() {
        use crate::model::chart_type::{ChartType, SizeMethod};
        let mut saved = chart();
        saved.appearance.style.chart_type = ChartType::Renko;
        saved.appearance.style.renko.size.method = SizeMethod::Traditional;
        saved.appearance.style.renko.size.amount = 10.0;
        saved.appearance.style.candles.up_body = Some([9, 8, 7, 255]);
        let restored: PersistedChart =
            serde_json::from_str(&serde_json::to_string(&saved).unwrap()).unwrap();
        assert_eq!(restored, saved);
        assert_eq!(restored.appearance.style.renko.size.amount, 10.0);
        // Switching away and back finds Renko as it was left.
        let mut style = restored.appearance.style;
        style.chart_type = ChartType::Candles;
        style.chart_type = ChartType::Renko;
        assert_eq!(style.renko.size.amount, 10.0);
    }

    #[test]
    fn an_old_appearance_without_a_style_reads_as_plain_candles() {
        let old: ChartAppearance =
            serde_json::from_str(r#"{"bullish":[1,2,3],"bearish":[4,5,6],"axis_decimals":2}"#)
                .unwrap();
        assert_eq!(old.bullish, [1, 2, 3]);
        assert_eq!(old.style, crate::model::ChartStyle::default());
        assert_eq!(old.style.chart_type, crate::model::ChartType::Candles);
    }

    #[test]
    fn an_old_appearance_counts_the_zeros_of_a_tiny_price() {
        let old: ChartAppearance =
            serde_json::from_str(r#"{"bullish":[1,2,3],"bearish":[4,5,6],"axis_decimals":2}"#)
                .unwrap();
        assert_eq!(old.axis_small_prices, crate::model::SmallPrices::ZeroCount);
        assert_eq!(old.price_format().decimals, Some(2));
    }

    #[test]
    fn how_a_tiny_price_reads_survives_a_round_trip() {
        let appearance = ChartAppearance {
            axis_small_prices: crate::model::SmallPrices::Decimals,
            ..ChartAppearance::default()
        };
        let json = serde_json::to_string(&appearance).unwrap();
        assert!(json.contains(r#""axis_small_prices":"decimals""#), "{json}");
        let back: ChartAppearance = serde_json::from_str(&json).unwrap();
        assert_eq!(back, appearance);
    }
}

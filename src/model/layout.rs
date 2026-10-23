use crate::model::{DataSource, Timeframe};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// One ordered column in the chart grid. Chart membership is keyed by the
/// stable chart id, not by a symbol.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ChartColumn {
    pub id: u64,
    pub chart_ids: Vec<u64>,
}

impl ChartColumn {
    pub fn new(id: u64, chart_ids: Vec<u64>) -> Self {
        Self { id, chart_ids }
    }

    /// Resolve persisted columns against the charts that actually exist. With no
    /// columns yet, lay the charts out as a two-column row-major grid. Stale
    /// membership is repaired without dropping charts. `next_id` mints ids for
    /// any columns created here.
    pub fn resolved(
        persisted: Option<&[ChartColumn]>,
        chart_ids: &[u64],
        next_id: &mut u64,
    ) -> Vec<ChartColumn> {
        if chart_ids.is_empty() {
            return Vec::new();
        }

        let Some(persisted) = persisted.filter(|columns| !columns.is_empty()) else {
            let count = chart_ids.len().min(2);
            let mut columns = (0..count)
                .map(|_| {
                    let id = *next_id;
                    *next_id += 1;
                    ChartColumn::new(id, Vec::new())
                })
                .collect::<Vec<_>>();
            for (index, chart_id) in chart_ids.iter().enumerate() {
                columns[index % count].chart_ids.push(*chart_id);
            }
            return columns;
        };

        let valid: HashSet<u64> = chart_ids.iter().copied().collect();
        let mut seen: HashSet<u64> = HashSet::new();
        let mut columns: Vec<ChartColumn> = persisted
            .iter()
            .filter_map(|column| {
                let mut repaired = column.clone();
                repaired
                    .chart_ids
                    .retain(|id| valid.contains(id) && seen.insert(*id));
                (!repaired.chart_ids.is_empty()).then_some(repaired)
            })
            .collect();

        for chart_id in chart_ids.iter().copied().filter(|id| !seen.contains(id)) {
            if columns.len() < chart_ids.len().min(2) {
                let id = *next_id;
                *next_id += 1;
                columns.push(ChartColumn::new(id, vec![chart_id]));
            } else if let Some(shortest) = columns
                .iter()
                .map(|column| column.chart_ids.len())
                .enumerate()
                .min_by_key(|(index, len)| (*len, *index))
                .map(|(index, _)| index)
            {
                columns[shortest].chart_ids.push(chart_id);
            } else {
                let id = *next_id;
                *next_id += 1;
                columns.push(ChartColumn::new(id, vec![chart_id]));
            }
        }
        columns
    }
}

/// The per-chart configuration that participates in a layout fingerprint.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ChartConfig {
    pub id: u64,
    pub symbol: String,
    pub source: DataSource,
    #[serde(default)]
    pub scale_mode: super::price_scale::PriceScaleMode,
    #[serde(default)]
    pub appearance: super::ChartAppearance,
    /// Set for a portfolio card; its kind, scope and range are part of the layout.
    #[serde(default)]
    pub card: Option<super::portfolio_card::PortfolioCard>,
}

/// The part of a workspace a saved layout owns, reduced to what decides
/// "has this changed?". Zoom, focus, and watchlist edits are excluded.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LayoutSnapshot {
    pub timeframe: Timeframe,
    pub configs: Vec<ChartConfig>,
    pub columns: Vec<Vec<u64>>,
    /// Linking is part of the layout: changing it is a change to be saved.
    #[serde(default)]
    pub sync: super::ChartSyncSettings,
}

/// Grid sizing math, ported from the Swift `ChartLayout` constants.
pub struct ChartLayout;

impl ChartLayout {
    pub const GRID_MIN_COLUMN_WIDTH: f32 = 280.0;
    pub const GRID_OUTER_PADDING: f32 = 4.0;

    /// Equal card height for `row_count` stacked rows separated by `gap`, filling `available`
    /// exactly so the space above the first card and below the last are the panel's own margins.
    pub fn grid_card_height(available: f32, row_count: usize, gap: f32) -> f32 {
        if row_count == 0 {
            return available.max(0.0);
        }
        let gaps = row_count.saturating_sub(1) as f32 * gap;
        ((available - gaps) / row_count as f32).max(0.0)
    }

    /// A trailing column may be created only while every column remains usable.
    pub fn can_add_column(available_width: f32, current_column_count: usize) -> bool {
        let candidate = current_column_count + 1;
        if candidate == 0 {
            return false;
        }
        let usable = (available_width - Self::GRID_OUTER_PADDING * 2.0).max(0.0);
        usable / candidate as f32 >= Self::GRID_MIN_COLUMN_WIDTH
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolved_ids(persisted: Option<&[ChartColumn]>, ids: &[u64]) -> Vec<Vec<u64>> {
        let mut next_id = 1000;
        ChartColumn::resolved(persisted, ids, &mut next_id)
            .into_iter()
            .map(|column| column.chart_ids)
            .collect()
    }

    #[test]
    fn no_persisted_columns_gives_two_column_row_major_grid() {
        let ids: Vec<u64> = (1..=5).collect();
        assert_eq!(resolved_ids(None, &ids), vec![vec![1, 3, 5], vec![2, 4]]);
    }

    #[test]
    fn resolution_removes_stale_duplicate_and_empty_membership() {
        let ids: Vec<u64> = vec![1, 2, 3, 4];
        let columns = vec![
            ChartColumn::new(10, vec![1, 2, 99]),
            ChartColumn::new(11, vec![2]),
            ChartColumn::new(12, vec![3]),
        ];
        let resolved = resolved_ids(Some(&columns), &ids);
        let flat: HashSet<u64> = resolved.iter().flatten().copied().collect();
        assert_eq!(flat, ids.iter().copied().collect());
        assert_eq!(resolved.iter().flatten().count(), ids.len());
        assert!(!resolved.iter().any(|column| column.is_empty()));
        assert!(!resolved.iter().flatten().any(|id| *id == 99));
    }

    #[test]
    fn card_heights_and_gaps_fill_the_available_height() {
        let height = ChartLayout::grid_card_height(500.0, 3, 6.0);
        assert!((height * 3.0 + 2.0 * 6.0 - 500.0).abs() < 0.0001);
        assert_eq!(ChartLayout::grid_card_height(500.0, 1, 6.0), 500.0);
    }

    #[test]
    fn responsive_column_limit_uses_minimum_usable_width() {
        assert!(ChartLayout::can_add_column(900.0, 2));
        assert!(!ChartLayout::can_add_column(839.0, 2));
    }

    #[test]
    fn layout_snapshot_round_trips() {
        let snapshot = LayoutSnapshot {
            timeframe: Timeframe::Hour,
            configs: vec![
                ChartConfig {
                    id: 1,
                    symbol: "BTCUSDT".into(),
                    source: DataSource::Binance,
                    scale_mode: Default::default(),
                    appearance: crate::model::ChartAppearance::default(),
                    card: None,
                },
                ChartConfig {
                    id: 2,
                    symbol: "ETHUSDT".into(),
                    source: DataSource::Binance,
                    scale_mode: crate::model::price_scale::PriceScaleMode::Logarithmic,
                    appearance: crate::model::ChartAppearance::default(),
                    card: None,
                },
            ],
            columns: vec![vec![1], vec![2]],
            sync: crate::model::ChartSyncSettings {
                horizontal_position: true,
                horizontal_zoom: true,
            },
        };
        let encoded = serde_json::to_string(&snapshot).expect("serialize");
        let decoded: LayoutSnapshot = serde_json::from_str(&encoded).expect("deserialize");
        assert_eq!(decoded, snapshot);
    }

    #[test]
    fn a_chart_type_change_is_a_layout_change_and_a_restyle_is_too() {
        let base = LayoutSnapshot {
            timeframe: Timeframe::Hour,
            configs: vec![ChartConfig {
                id: 1,
                symbol: "BTCUSDT".into(),
                source: DataSource::Binance,
                scale_mode: Default::default(),
                appearance: crate::model::ChartAppearance::default(),
                card: None,
            }],
            columns: vec![vec![1]],
            sync: Default::default(),
        };
        let mut other = base.clone();
        assert_eq!(base, other);
        let mut linked = base.clone();
        linked.sync.horizontal_zoom = false;
        assert_ne!(base, linked, "unlinking charts is a change to the layout");
        other.configs[0].appearance.style.chart_type = crate::model::ChartType::HeikinAshi;
        assert_ne!(
            base, other,
            "the chart type is part of the layout's fingerprint"
        );
        let encoded = serde_json::to_string(&other).unwrap();
        assert_eq!(
            serde_json::from_str::<LayoutSnapshot>(&encoded).unwrap(),
            other
        );
    }
}

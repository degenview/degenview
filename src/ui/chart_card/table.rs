//! Feeds Table view: the bars this chart draws and the values its visible indicators and
//! scripts have on them.
//!
//! Built from the same `bars` the painter, the indicators and Pine read (constructed for a
//! price-based chart type, cut at the replay position), only while a table is open, and only
//! when something it shows has changed.

use super::ChartCard;
use crate::model::chart::kind::ChartType;
use crate::model::chart::table::{Extra, TableFeed, ValueKind, align_right};
use crate::model::pine::instance::VisualOrder;
use crate::model::{IndicatorConfig, IndicatorSeries, Kline};
use crate::pine::program::VisualOutput;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

impl ChartCard {
    /// Whether the workspace has a table open on this chart. Closing it drops the feed.
    pub fn set_table_wanted(&mut self, wanted: bool) {
        self.table_wanted = wanted;
        if !wanted {
            self.table_feed = None;
            self.table_stamp = 0;
        }
    }

    /// What the table shows, as of the last frame.
    pub fn table_feed(&self) -> Option<Arc<TableFeed>> {
        self.table_feed.clone()
    }

    pub(super) fn feed_table(
        &mut self,
        bars: &[Kline],
        chart_type: ChartType,
        line: bool,
        provides_volume: bool,
        indicators: &IndicatorConfig,
        outputs: &[(Arc<VisualOutput>, VisualOrder, u64)],
    ) {
        if !self.table_wanted {
            return;
        }
        let drawn = indicators.drawn();
        let stamp = stamp(bars, chart_type, &drawn, outputs);
        if stamp == self.table_stamp && self.table_feed.is_some() {
            return;
        }
        let mut extras = Vec::new();
        let series = IndicatorSeries::compute(bars, 0, bars.len(), &drawn);
        if drawn.show_ema {
            extras.push(Extra {
                title: format!("EMA {}", drawn.ema_period),
                kind: ValueKind::Price,
                values: series.ema.clone(),
            });
        }
        if let Some(bands) = &series.bollinger {
            for (title, values) in [
                ("Bollinger upper", &bands.upper),
                ("Bollinger basis", &bands.middle),
                ("Bollinger lower", &bands.lower),
            ] {
                extras.push(Extra {
                    title: title.to_owned(),
                    kind: ValueKind::Price,
                    values: values.clone(),
                });
            }
        }
        if drawn.show_rsi {
            extras.push(Extra {
                title: "RSI".to_owned(),
                kind: ValueKind::Plain,
                values: series.rsi.clone(),
            });
        }
        if drawn.show_trend_flips {
            extras.push(Extra {
                title: "Supertrend flip".to_owned(),
                kind: ValueKind::Flip,
                values: series
                    .trend_flips
                    .iter()
                    .map(|flip| flip.map(|bullish| if bullish { 1.0 } else { -1.0 }))
                    .collect(),
            });
        }
        // The scripts the chart draws, a column per plot.
        for (output, _, instance_id) in outputs {
            let name = self
                .instances
                .iter()
                .find(|instance| instance.id == *instance_id)
                .map_or("Script", |instance| instance.name.as_str());
            for plot in output.plots.iter().filter(|plot| plot.display != 0) {
                let title = match &plot.title {
                    Some(title) if !title.is_empty() => format!("{name}: {title}"),
                    _ => format!("{name}: plot {}", plot.id + 1),
                };
                extras.push(Extra {
                    title,
                    kind: if output.overlay || line {
                        ValueKind::Price
                    } else {
                        ValueKind::Plain
                    },
                    values: align_right(&plot.values, bars.len()),
                });
            }
        }
        self.table_stamp = stamp;
        self.table_feed = Some(Arc::new(TableFeed {
            bars_label: chart_type.label().to_owned(),
            ohlc: !line,
            volume: provides_volume,
            bars: bars.to_vec(),
            extras,
        }));
    }
}

/// A fingerprint of what the table shows: the bars' ends, the indicators and each script run.
fn stamp(
    bars: &[Kline],
    chart_type: ChartType,
    indicators: &IndicatorConfig,
    outputs: &[(Arc<VisualOutput>, VisualOrder, u64)],
) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bars.len().hash(&mut hasher);
    if let Some(last) = bars.last() {
        last.open_time.hash(&mut hasher);
        for value in [last.open, last.high, last.low, last.close, last.volume] {
            value.to_bits().hash(&mut hasher);
        }
    }
    if let Some(first) = bars.first() {
        first.open_time.hash(&mut hasher);
    }
    std::mem::discriminant(&chart_type).hash(&mut hasher);
    format!("{indicators:?}").hash(&mut hasher);
    for (output, _, id) in outputs {
        id.hash(&mut hasher);
        Arc::as_ptr(output).hash(&mut hasher);
    }
    hasher.finish().max(1)
}

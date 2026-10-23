//! Pine scripts and built-in indicators on a chart: the views the app and MCP take, and applying, changing and removing them.

use super::ChartWorkspace;
use crate::model::{BuiltIn, EMA_PERIODS, IndicatorConfig, InstrumentId, Kline, Timeframe};
use crate::ui::pine_settings_popover::{self, PineSettingsPopover};
use crate::ui::{self};
use eframe::egui;
use std::collections::HashMap;

/// The focused chart's identity, read by the Pine integration (alert subscription + tick loop).
pub struct PineChartTarget {
    pub chart_id: u64,
    pub instrument: InstrumentId,
    pub timeframe: Timeframe,
    pub instances: Vec<crate::model::pine_instance::PineInstance>,
}

/// What a chart's run of one applied Pine script has produced so far. A script runs only while its
/// chart is drawn, so a chart in a window nobody has looked at yet has none.
#[derive(Clone, Debug)]
pub struct PineRunView {
    /// The last run's failure; the chart keeps showing the previous good output meanwhile.
    pub error: Option<crate::pine::token::Diagnostic>,
    pub bar_count: usize,
    /// What the script lets the user restyle (empty until it has run).
    pub style_rows: Vec<crate::pine::style_overrides::StyleRow>,
    pub is_strategy: bool,
    /// The backtest report, only when it was asked for (it can be large).
    pub strategy: Option<crate::pine::strategy::StrategyReport>,
}

/// One Pine script applied to a chart, with the context it runs in.
#[derive(Clone, Debug)]
pub struct PineInstanceView {
    pub chart_id: u64,
    pub instrument: InstrumentId,
    pub timeframe: Timeframe,
    pub instance: crate::model::pine_instance::PineInstance,
    pub run: Option<PineRunView>,
}

/// What [`ChartWorkspace::apply_indicator`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndicatorOutcome {
    Added,
    /// Already applied; it was shown again or its EMA period changed.
    Updated,
    AlreadyApplied,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndicatorError {
    NoSuchChart,
    /// Volume on a line chart (prediction markets, CoinMarketCap).
    UnsupportedOnLineChart,
    /// A period for an indicator that has none (only EMA takes one).
    NoParameters,
    /// An EMA period outside `EMA_PERIODS`.
    InvalidEmaPeriod,
}

/// What the Pine alert runtime needs from one chart: its market, the bars its scripts run over
/// and the scripts. Those are the market's candles, or the bars the chart type constructs from
/// them, so a script's alert sees what the chart shows.
pub struct ChartFeed {
    pub chart_id: u64,
    pub symbol_key: String,
    pub timeframe: String,
    pub bars: Vec<Kline>,
    pub run: crate::host::pine_subscription_host::ChartRun,
    pub instances: Vec<crate::model::pine_instance::PineInstance>,
}

impl ChartWorkspace {
    /// The Pine scripts applied to a chart (or just `only` instance), with each one's latest run.
    /// `None` when the chart does not exist. Cloning the strategy report is opt-in.
    pub fn pine_instance_views(
        &self,
        chart_id: u64,
        only: Option<u64>,
        with_report: bool,
    ) -> Option<Vec<PineInstanceView>> {
        let chart = self.charts.iter().find(|chart| chart.id == chart_id)?;
        let instrument = InstrumentId::new(chart.source, &chart.symbol);
        Some(
            chart
                .chart
                .pine_instances()
                .iter()
                .filter(|instance| only.is_none_or(|id| instance.id == id))
                .map(|instance| PineInstanceView {
                    chart_id,
                    instrument: instrument.clone(),
                    timeframe: self.timeframe,
                    instance: instance.clone(),
                    run: chart
                        .chart
                        .pine_status(instance.id)
                        .map(|status| PineRunView {
                            error: status.error.cloned(),
                            bar_count: status.output.bar_count,
                            style_rows: chart.chart.pine_style_rows(instance.id),
                            is_strategy: status.output.strategy.is_some(),
                            strategy: if with_report {
                                status.output.strategy.clone()
                            } else {
                                None
                            },
                        }),
                })
                .collect(),
        )
    }

    /// Applies a built-in indicator to a chart, as the chart's indicator menu does: it is drawn
    /// (shown again if its eye was off), and an EMA takes `ema_period`. Each indicator exists
    /// once per chart, so applying one again only updates it. Returns the chart's resulting
    /// configuration.
    pub fn apply_indicator(
        &mut self,
        chart_id: u64,
        kind: BuiltIn,
        ema_period: Option<usize>,
    ) -> Result<(IndicatorOutcome, IndicatorConfig), IndicatorError> {
        let chart = self
            .charts
            .iter_mut()
            .find(|chart| chart.id == chart_id)
            .ok_or(IndicatorError::NoSuchChart)?;
        if !kind.supports_line_charts() && chart.source.is_line_chart() {
            return Err(IndicatorError::UnsupportedOnLineChart);
        }
        if let Some(period) = ema_period {
            if kind != BuiltIn::Ema {
                return Err(IndicatorError::NoParameters);
            }
            if !EMA_PERIODS.contains(&period) {
                return Err(IndicatorError::InvalidEmaPeriod);
            }
        }
        let before = chart.indicators;
        kind.set_applied(&mut chart.indicators, true);
        if let Some(period) = ema_period {
            chart.indicators.ema_period = period;
        }
        let outcome = if !kind.applied(&before) {
            IndicatorOutcome::Added
        } else if chart.indicators != before {
            IndicatorOutcome::Updated
        } else {
            IndicatorOutcome::AlreadyApplied
        };
        Ok((outcome, chart.indicators))
    }

    /// Adds a Pine script to the focused chart, after the ones already there (the Script
    /// Manager's "Add to Chart").
    pub fn add_pine_on_focused(&mut self, script_id: Option<String>, name: &str, source: &str) {
        if let Some(id) = self.resolved_focused_id()
            && let Some(chart) = self.charts.iter_mut().find(|chart| chart.id == id)
        {
            chart.chart.add_pine_instance(script_id, name, source);
        }
    }

    /// Applies a saved script to a market chart for an agent, as the chart's Scripts tab does,
    /// with the inputs it was given. Undo takes it off again. `None` when the chart is not a
    /// market chart in this workspace.
    pub fn add_pine_instance_to(
        &mut self,
        chart_id: u64,
        script_id: String,
        name: &str,
        source: &str,
        inputs: &HashMap<String, crate::pine::program::InputValue>,
        visible: bool,
    ) -> Option<u64> {
        if !self
            .charts
            .iter()
            .any(|chart| chart.id == chart_id && chart.is_market())
        {
            return None;
        }
        self.remember_indicators(chart_id);
        let chart = self.charts.iter_mut().find(|chart| chart.id == chart_id)?;
        let id = chart.chart.add_pine_instance(Some(script_id), name, source);
        chart.chart.set_pine_instance_inputs(id, inputs);
        chart.chart.set_pine_visible(id, visible);
        Some(id)
    }

    /// Changes an applied script's inputs and/or visibility. Returns whether anything differed;
    /// only then does it take an undo step.
    pub fn change_pine_instance(
        &mut self,
        chart_id: u64,
        instance_id: u64,
        inputs: Option<&HashMap<String, crate::pine::program::InputValue>>,
        visible: Option<bool>,
    ) -> Option<bool> {
        let chart = self.charts.iter().find(|chart| chart.id == chart_id)?;
        let instance = chart
            .chart
            .pine_instances()
            .iter()
            .find(|instance| instance.id == instance_id)?;
        let changed = inputs.is_some_and(|inputs| *inputs != instance.input_values())
            || visible.is_some_and(|visible| visible != instance.visible);
        if changed {
            self.remember_indicators(chart_id);
            let chart = self.charts.iter_mut().find(|chart| chart.id == chart_id)?;
            if let Some(inputs) = inputs {
                chart.chart.set_pine_instance_inputs(instance_id, inputs);
            }
            if let Some(visible) = visible {
                chart.chart.set_pine_visible(instance_id, visible);
            }
        }
        Some(changed)
    }

    /// Takes an applied script off a chart (its saved source and alerts are left alone). Undo
    /// brings it back. Returns whether the instance existed.
    pub fn remove_pine_instance_from(&mut self, chart_id: u64, instance_id: u64) -> bool {
        let exists = self.charts.iter().any(|chart| {
            chart.id == chart_id
                && chart
                    .chart
                    .pine_instances()
                    .iter()
                    .any(|instance| instance.id == instance_id)
        });
        if exists {
            self.remember_indicators(chart_id);
            if let Some(chart) = self.charts.iter_mut().find(|chart| chart.id == chart_id) {
                chart.chart.remove_pine_instance(instance_id);
            }
        }
        exists
    }

    /// The focused chart's identity for the Pine integration (alert subscription creation).
    pub fn focused_pine_target(&self) -> Option<PineChartTarget> {
        let id = self.resolved_focused_id()?;
        let chart = self.charts.iter().find(|chart| chart.id == id)?;
        Some(PineChartTarget {
            chart_id: chart.id,
            instrument: InstrumentId::new(chart.source, &chart.symbol),
            timeframe: self.timeframe,
            instances: chart.chart.pine_instances().to_vec(),
        })
    }

    /// Each chart's `(chart_id, symbol_key, timeframe)`, so script alerts can say whether their
    /// chart is open and still on their market.
    pub fn pine_datasets(&self) -> Vec<(u64, String, String)> {
        self.charts
            .iter()
            .filter(|chart| chart.is_market())
            .map(|chart| {
                (
                    chart.id,
                    InstrumentId::new(chart.source, &chart.symbol).key(),
                    self.timeframe.interval().to_owned(),
                )
            })
            .collect()
    }

    /// Every chart's `(symbol_key, candles)`, so the Pine subscription tick loop fires on any open
    /// chart, not just the focused one.
    pub fn charts_candles(&self) -> Vec<ChartFeed> {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_millis() as i64);
        self.charts
            .iter()
            .filter(|chart| chart.is_market())
            .map(|chart| {
                let (bars, run) = chart.chart.pine_feed(&chart.candles, chart.source, now_ms);
                ChartFeed {
                    chart_id: chart.id,
                    symbol_key: InstrumentId::new(chart.source, &chart.symbol).key(),
                    timeframe: self.timeframe.interval().to_owned(),
                    bars,
                    run,
                    instances: chart.chart.pine_instances().to_vec(),
                }
            })
            .collect()
    }

    /// Opens an applied script's Inputs and Style popover beside `anchor`, or puts it away when it
    /// is already up for that script.
    pub(super) fn toggle_script_popover(
        &mut self,
        chart_id: u64,
        instance_id: u64,
        anchor: egui::Rect,
    ) {
        if self
            .script_popover
            .as_ref()
            .is_some_and(|popover| popover.is_for(chart_id, instance_id))
        {
            self.script_popover = None;
            return;
        }
        self.script_popover = self
            .charts
            .iter()
            .find(|chart| chart.id == chart_id)
            .and_then(|chart| {
                let instance = chart
                    .chart
                    .pine_instances()
                    .iter()
                    .find(|instance| instance.id == instance_id)?;
                Some(PineSettingsPopover::open(
                    chart_id,
                    instance,
                    chart.chart.pine_style_rows(instance_id),
                    anchor,
                ))
            });
    }

    /// Draws the script popover, if one is up, and applies what it returns.
    pub(super) fn show_script_popover(&mut self, ctx: &egui::Context) {
        let Some(mut popover) = self.script_popover.take() else {
            ui::chart_indicator_strip::set_open_script_popover(ctx, None);
            return;
        };
        let (chart_id, instance_id) = (popover.chart_id(), popover.instance_id());
        let applied = self
            .charts
            .iter()
            .find(|chart| chart.id == chart_id)
            .is_some_and(|chart| {
                chart
                    .chart
                    .pine_instances()
                    .iter()
                    .any(|instance| instance.id == instance_id)
            });
        if !applied {
            ui::chart_indicator_strip::set_open_script_popover(ctx, None);
            return;
        }
        match popover.show(ctx) {
            pine_settings_popover::Outcome::Open => {
                ui::chart_indicator_strip::set_open_script_popover(
                    ctx,
                    Some((chart_id, instance_id)),
                );
                self.script_popover = Some(popover);
            }
            pine_settings_popover::Outcome::Cancel => {
                ui::chart_indicator_strip::set_open_script_popover(ctx, None);
            }
            pine_settings_popover::Outcome::Apply { inputs, style } => {
                ui::chart_indicator_strip::set_open_script_popover(ctx, None);
                if let Some(chart) = self.charts.iter_mut().find(|chart| chart.id == chart_id) {
                    chart.chart.set_pine_instance_inputs(instance_id, &inputs);
                    chart.chart.set_pine_instance_style(instance_id, style);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;
    use crate::workspace::test_support::empty_workspace;

    #[test]
    fn agent_script_changes_are_undoable_and_only_real_changes_take_a_step() {
        use crate::pine::program::InputValue;
        let ctx = egui::Context::default();
        let mut workspace = empty_workspace(7);
        let btc = InstrumentId::new(DataSource::Binance, "BTCUSDT");
        let chart_id = workspace.add_chart_for(&btc, &ctx).chart_id;
        let base = workspace.undo.len();

        let inputs = HashMap::from([("len".to_owned(), InputValue::Int(50))]);
        assert_eq!(
            workspace.add_pine_instance_to(999, "s1".into(), "SMA", "src", &inputs, true),
            None
        );
        let id = workspace
            .add_pine_instance_to(chart_id, "s1".into(), "SMA", "src", &inputs, false)
            .expect("market chart");
        let instance = |workspace: &ChartWorkspace| {
            workspace
                .pine_instance_views(chart_id, None, false)
                .unwrap()
                .into_iter()
                .map(|view| view.instance)
                .collect::<Vec<_>>()
        };
        let applied = instance(&workspace);
        assert_eq!(applied.len(), 1);
        assert_eq!(applied[0].script_id.as_deref(), Some("s1"));
        assert!(!applied[0].visible);
        assert_eq!(applied[0].input_values(), inputs);

        // Saying what is already true is not a change, so it is not an undo step.
        assert_eq!(
            workspace.change_pine_instance(chart_id, id, Some(&inputs), Some(false)),
            Some(false)
        );
        assert_eq!(workspace.undo.len(), base + 1);
        assert_eq!(
            workspace.change_pine_instance(chart_id, 99, None, Some(true)),
            None
        );
        assert_eq!(
            workspace.change_pine_instance(chart_id, id, Some(&HashMap::new()), Some(true)),
            Some(true)
        );
        assert!(instance(&workspace)[0].visible);
        assert!(instance(&workspace)[0].inputs.is_empty());

        assert!(workspace.remove_pine_instance_from(chart_id, id));
        assert!(!workspace.remove_pine_instance_from(chart_id, id));
        assert!(instance(&workspace).is_empty());

        workspace.undo();
        assert_eq!(instance(&workspace).len(), 1);
        workspace.undo();
        assert!(!instance(&workspace)[0].visible);
        assert_eq!(instance(&workspace)[0].input_values(), inputs);
        workspace.undo();
        assert!(instance(&workspace).is_empty());
    }

    #[test]
    fn apply_indicator_adds_updates_and_validates() {
        let ctx = egui::Context::default();
        let mut workspace = empty_workspace(5);
        let btc = workspace
            .add_chart_for(&InstrumentId::new(DataSource::Binance, "BTCUSDT"), &ctx)
            .chart_id;

        let (outcome, config) = workspace.apply_indicator(btc, BuiltIn::Rsi, None).unwrap();
        assert_eq!(outcome, IndicatorOutcome::Added);
        assert!(config.show_rsi);
        assert_eq!(
            workspace
                .apply_indicator(btc, BuiltIn::Rsi, None)
                .unwrap()
                .0,
            IndicatorOutcome::AlreadyApplied
        );

        // EMA: added with a period, then re-configured; an unknown period changes nothing.
        let (outcome, config) = workspace
            .apply_indicator(btc, BuiltIn::Ema, Some(50))
            .unwrap();
        assert_eq!((outcome, config.ema_period), (IndicatorOutcome::Added, 50));
        let (outcome, config) = workspace
            .apply_indicator(btc, BuiltIn::Ema, Some(200))
            .unwrap();
        assert_eq!(
            (outcome, config.ema_period),
            (IndicatorOutcome::Updated, 200)
        );
        assert_eq!(
            workspace.apply_indicator(btc, BuiltIn::Ema, Some(21)),
            Err(IndicatorError::InvalidEmaPeriod)
        );
        assert_eq!(workspace.chart_summaries()[0].indicators.ema_period, 200);

        // Only EMA takes a parameter.
        assert_eq!(
            workspace.apply_indicator(btc, BuiltIn::Bollinger, Some(20)),
            Err(IndicatorError::NoParameters)
        );
        assert!(!workspace.chart_summaries()[0].indicators.show_bollinger);

        // A hidden indicator is shown again by applying it.
        BuiltIn::Rsi.toggle_visible(&mut workspace.charts[0].indicators);
        assert!(workspace.charts[0].indicators.hide_rsi);
        assert_eq!(
            workspace
                .apply_indicator(btc, BuiltIn::Rsi, None)
                .unwrap()
                .0,
            IndicatorOutcome::Updated
        );
        assert!(!workspace.charts[0].indicators.hide_rsi);

        assert_eq!(
            workspace.apply_indicator(999, BuiltIn::Rsi, None),
            Err(IndicatorError::NoSuchChart)
        );
    }

    #[test]
    fn volume_is_refused_on_line_charts_but_others_are_not() {
        let ctx = egui::Context::default();
        let mut workspace = empty_workspace(6);
        let market = workspace
            .add_chart_for(
                &InstrumentId::new(DataSource::Polymarket, "some-market"),
                &ctx,
            )
            .chart_id;
        assert_eq!(
            workspace.apply_indicator(market, BuiltIn::Volume, None),
            Err(IndicatorError::UnsupportedOnLineChart)
        );
        assert!(
            workspace
                .apply_indicator(market, BuiltIn::Ema, None)
                .is_ok()
        );
    }

    #[test]
    fn indicators_stay_on_their_own_chart_and_persist() {
        let ctx = egui::Context::default();
        let mut workspace = empty_workspace(8);
        let a = workspace
            .add_chart_for(&InstrumentId::new(DataSource::Binance, "BTCUSDT"), &ctx)
            .chart_id;
        let b = workspace
            .add_chart_for(&InstrumentId::new(DataSource::Binance, "ETHUSDT"), &ctx)
            .chart_id;
        workspace
            .apply_indicator(a, BuiltIn::Bollinger, None)
            .unwrap();
        let persisted = workspace.persisted();
        let of = |id| {
            persisted
                .charts
                .iter()
                .find(|c| c.id == id)
                .unwrap()
                .indicators
        };
        assert!(of(a).show_bollinger);
        assert!(!of(b).show_bollinger);
    }
}

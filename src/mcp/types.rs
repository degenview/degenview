//! The data the chart tools return. Serializable and schema-described, free of egui and HTTP,
//! built from the workspace's own summaries — never a second chart model.

use schemars::JsonSchema;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct WorkspaceInfo {
    /// Stable id: survives restarts, never reused. Pass it to the other chart tools.
    pub workspace_id: String,
    pub title: String,
    pub chart_count: usize,
    /// Whether this is the workspace whose window last had focus.
    pub active: bool,
    /// The timeframe every chart in the workspace uses (`1H`, `1D`, `1W`, `1M`, `3M`, `1Y`).
    pub timeframe: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct WorkspacesResult {
    pub workspaces: Vec<WorkspaceInfo>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct IndicatorsInfo {
    pub volume: bool,
    pub rsi: bool,
    pub ema: bool,
    pub ema_period: usize,
    pub bollinger: bool,
    pub trend_flips: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct PineInstanceInfo {
    pub instance_id: String,
    pub script_name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ChartInfo {
    /// Unique within its workspace.
    pub chart_id: String,
    /// Source-qualified market, e.g. `BINANCE:BTCUSDT`.
    pub instrument: String,
    pub source: String,
    pub symbol: String,
    pub display_symbol: String,
    pub timeframe: String,
    /// `candles` or `line`.
    pub chart_type: String,
    pub indicators: IndicatorsInfo,
    pub pine_instances: Vec<PineInstanceInfo>,
    /// `loading`, `ready` or `error`.
    pub status: String,
    pub error: Option<String>,
    pub focused: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ChartsResult {
    pub workspace_id: String,
    pub charts: Vec<ChartInfo>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct AddChartResult {
    pub workspace_id: String,
    pub chart_id: String,
    pub instrument: String,
    pub source: String,
    pub symbol: String,
    pub display_symbol: String,
    /// The workspace's timeframe, which the new chart uses.
    pub timeframe: String,
    /// `created`, or `already_present` when the workspace already charted this market (that
    /// chart is returned and focused, so a retried request is harmless).
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct IndicatorParameterInfo {
    pub name: String,
    /// Always `integer` today.
    pub value_type: String,
    pub default: i64,
    /// The only values accepted.
    pub allowed_values: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct IndicatorSpecInfo {
    /// Stable id to pass to `degenview_add_indicator`.
    pub id: String,
    pub title: String,
    pub description: String,
    /// `overlay` (drawn on the price pane) or `pane` (a pane of its own).
    pub placement: String,
    /// Chart types it works on: `candles`, `line`.
    pub chart_types: Vec<String>,
    /// Empty for indicators with fixed settings.
    pub parameters: Vec<IndicatorParameterInfo>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct IndicatorsResult {
    pub indicators: Vec<IndicatorSpecInfo>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct AddIndicatorResult {
    pub workspace_id: String,
    pub chart_id: String,
    pub indicator: String,
    /// `added`, `updated` (shown again or EMA period changed) or `already_applied`.
    pub status: String,
    /// The chart's indicators after the change.
    pub indicators: IndicatorsInfo,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct AlertInfo {
    /// Stable id; pass it to the pause, resume and delete tools.
    pub alert_id: String,
    /// Source-qualified market, e.g. `BINANCE:BTCUSDT`.
    pub instrument: String,
    pub source: String,
    pub symbol: String,
    /// `crosses_above`, `crosses_below`, `rises_by`, `falls_by`, or `unsupported`.
    pub condition: String,
    /// The price that triggers it, as a decimal string in `currency`.
    pub target_price: Option<String>,
    /// For `rises_by` / `falls_by`: the percentage, and the price it counts from.
    pub percent: Option<String>,
    pub reference_price: Option<String>,
    pub currency: String,
    /// `once` or `every_time`.
    pub frequency: String,
    /// `active`, `paused`, `triggered` (a once-alert that fired) or `unsupported`.
    pub status: String,
    pub note: String,
    /// Whether the alert posts to webhook endpoints (which are never exposed here).
    pub sends_to_webhooks: bool,
    pub created_at: String,
    pub updated_at: String,
    pub last_triggered_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct AlertsResult {
    pub alerts: Vec<AlertInfo>,
    /// All alerts, including any beyond the returned page.
    pub total: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct CreateAlertResult {
    /// `created`, `queued` (accepted but not yet visible in the list) or `already_exists`
    /// (an identical alert is returned instead of a duplicate).
    pub status: String,
    pub alert: AlertInfo,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct UpdateAlertResult {
    /// `updated`, `queued` (accepted but not yet visible in the list) or `unchanged` (the
    /// request matched the alert as it is, so nothing was queued).
    pub status: String,
    /// The alert as edited.
    pub alert: AlertInfo,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct AlertChangeResult {
    pub alert_id: String,
    /// `paused`, `active`, `deleted`, `already_paused`, `already_active`, or `queued`
    /// (accepted but not yet applied).
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct PineCompileSummary {
    /// `valid`, `warning`, `error` or `not_compiled`.
    pub status: String,
    /// Whether that compile was of the script's current saved source (not an older one).
    pub current: bool,
    pub compiled_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct PineScriptInfo {
    pub script_id: String,
    pub name: String,
    /// `indicator`, `strategy` or `library`.
    pub script_type: String,
    /// Whether unsaved text differing from the saved source is kept for it.
    pub has_draft: bool,
    pub is_favorite: bool,
    pub modified_at: String,
    /// SHA-256 of the saved source.
    pub source_hash: String,
    pub last_compile: Option<PineCompileSummary>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct PineScriptsResult {
    pub scripts: Vec<PineScriptInfo>,
    pub total: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct PineScriptSource {
    pub script_id: String,
    pub name: String,
    pub script_type: String,
    /// Which text this is: `saved` or `draft`. Never one standing in for the other.
    pub version: String,
    pub source: String,
    /// SHA-256 of `source`: pass it back as the expected revision when editing.
    pub source_hash: String,
    pub size_chars: usize,
    /// The latest saved revision id, when the script has history.
    pub revision_id: Option<String>,
    pub has_draft: bool,
    pub draft_modified_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct PineDiagnosticInfo {
    /// `error` or `warning`.
    pub severity: String,
    /// The compiler's own code, e.g. `PINE0002`.
    pub code: String,
    /// `lexical`, `syntax`, `semantic`, `unsupported`, `resource`, `runtime` or `cancellation`.
    pub category: String,
    pub message: String,
    /// 1-based position where it starts and ends.
    pub line: usize,
    pub column: usize,
    pub end_line: usize,
    pub end_column: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct PineCompileResult {
    /// No errors (warnings do not fail a compile).
    pub success: bool,
    pub script_id: Option<String>,
    /// `saved`, `draft`, or `inline` for source passed in directly.
    pub version: String,
    pub source_hash: String,
    /// What the script declares itself to be: `indicator`, `strategy` or `library`.
    pub script_type: Option<String>,
    pub pine_version: Option<i64>,
    pub title: Option<String>,
    pub error_count: usize,
    pub warning_count: usize,
    pub diagnostics: Vec<PineDiagnosticInfo>,
    pub diagnostics_truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct PineInputOption {
    pub value: serde_json::Value,
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct PineInputInfo {
    /// The script's variable name for it; the key to use when setting it.
    pub id: String,
    pub title: Option<String>,
    /// `integer`, `float`, `bool`, `string`, `color` or `source`.
    pub value_type: String,
    pub default: serde_json::Value,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub step: Option<f64>,
    /// Allowed values when the input is a fixed list.
    pub options: Option<Vec<PineInputOption>>,
    pub group: Option<String>,
    pub inline: Option<String>,
    pub tooltip: Option<String>,
    pub confirm: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct PineInputsResult {
    pub script_id: String,
    pub version: String,
    pub source_hash: String,
    /// False when the script has errors; the inputs found before them are still listed.
    pub compiled: bool,
    pub inputs: Vec<PineInputInfo>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct AppliedPineInfo {
    pub instance_id: String,
    /// The Script Manager script it was applied from, when it came from one.
    pub script_id: Option<String>,
    pub script_name: String,
    pub visible: bool,
    /// SHA-256 of the source this instance runs (its own copy, taken when it was applied).
    pub source_hash: String,
    /// `ok`, `error` (the last run failed) or `not_run` (the chart has not computed it yet:
    /// scripts run only while their chart is drawn).
    pub run_status: String,
    pub error: Option<PineDiagnosticInfo>,
    /// Known once it has run.
    pub is_strategy: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct AppliedPineList {
    pub workspace_id: String,
    pub chart_id: String,
    pub instrument: String,
    pub timeframe: String,
    pub instances: Vec<AppliedPineInfo>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct PineInstanceChangeResult {
    /// `added`, `updated`, `unchanged` or `removed`.
    pub status: String,
    pub instance_id: String,
    /// The scripts on the chart afterwards. A script runs once its chart draws it, so a new or
    /// changed one reads `not_run` until then.
    pub chart: AppliedPineList,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct AppliedPineDetail {
    pub workspace_id: String,
    pub chart_id: String,
    pub instrument: String,
    pub timeframe: String,
    pub instance: AppliedPineInfo,
    /// The user's input overrides by input id, as `{kind, value}`; an input with no entry runs at
    /// its default (see `degenview_get_pine_inputs`).
    pub inputs: std::collections::BTreeMap<String, serde_json::Value>,
    /// Style overrides by `kind.id.attribute`.
    pub style_overrides: std::collections::BTreeMap<String, String>,
    /// Bars the last run covered.
    pub bar_count: Option<usize>,
    /// How the instance's source compares with its Script Manager script now: `unchanged`,
    /// `changed`, `deleted`, or `unlinked` (not applied from a saved script).
    pub script_status: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct PineStyleOverridesInfo {
    pub visible: Option<bool>,
    /// `#RRGGBBAA`.
    pub color: Option<String>,
    pub width: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct PineStyleRowInfo {
    /// `kind.output_id`, the prefix of this row's override keys.
    pub id: String,
    /// `plot`, `hline`, `fill`, `marker`, `candle`, `bgcolor` or `barcolor`.
    pub kind: String,
    pub title: String,
    /// The script's own color when it is one fixed color; absent when it varies per bar.
    pub default_color: Option<String>,
    pub default_width: Option<usize>,
    pub overrides: PineStyleOverridesInfo,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct PineStylesResult {
    pub workspace_id: String,
    pub chart_id: String,
    pub instance_id: String,
    /// False until the script has run: its outputs are what can be styled.
    pub available: bool,
    pub reason: Option<String>,
    pub rows: Vec<PineStyleRowInfo>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct PineReportContext {
    /// Where the numbers come from: `chart_instance` (the chart's own run).
    pub kind: String,
    pub script_id: Option<String>,
    pub script_name: String,
    /// SHA-256 of the source that produced the report.
    pub source_hash: String,
    /// `unchanged`, `changed`, `deleted` or `unlinked` against the saved script now.
    pub script_status: String,
    pub instrument: String,
    pub timeframe: String,
    pub bar_count: usize,
    pub inputs: std::collections::BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct PineReportMetrics {
    pub net_profit: Option<f64>,
    pub gross_profit: Option<f64>,
    pub gross_loss: Option<f64>,
    pub closed_trades: usize,
    pub winning_trades: usize,
    pub losing_trades: usize,
    pub win_rate: Option<f64>,
    pub profit_factor: Option<f64>,
    pub average_trade: Option<f64>,
    pub final_equity: Option<f64>,
    /// Fractional return on the initial capital (0.1 = 10%).
    pub return_fraction: Option<f64>,
    pub max_drawdown: Option<f64>,
    pub max_drawdown_fraction: Option<f64>,
    pub open_profit: Option<f64>,
    pub open_trades: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct PineStrategySettingsInfo {
    pub initial_capital: f64,
    pub quantity_type: String,
    pub quantity_value: f64,
    pub commission_type: String,
    pub commission_value: f64,
    pub slippage_ticks: usize,
    pub pyramiding: usize,
    pub currency: Option<String>,
    pub process_orders_on_close: bool,
    pub calc_on_every_tick: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct PineTradeInfo {
    pub id: usize,
    pub entry_id: String,
    pub exit_id: String,
    /// `long` or `short`.
    pub direction: String,
    pub quantity: f64,
    pub entry_bar: usize,
    pub entry_time: String,
    pub entry_price: f64,
    pub exit_bar: usize,
    pub exit_time: String,
    pub exit_price: f64,
    pub commission: f64,
    pub profit: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct PineOpenTradeInfo {
    pub entry_id: String,
    pub direction: String,
    pub quantity: f64,
    pub entry_bar: usize,
    pub entry_time: String,
    pub entry_price: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct PineReportResult {
    pub workspace_id: String,
    pub chart_id: String,
    pub instance_id: String,
    /// False when there is no report to give; `reason` says why. Never an old or invented one.
    pub available: bool,
    pub reason: Option<String>,
    pub context: Option<PineReportContext>,
    pub metrics: Option<PineReportMetrics>,
    pub settings: Option<PineStrategySettingsInfo>,
    /// One page of closed trades, oldest first.
    pub trades: Vec<PineTradeInfo>,
    pub trades_total: usize,
    pub trades_offset: usize,
    pub open_trades: Vec<PineOpenTradeInfo>,
    /// Equity after each bar, thinned to at most 200 points, when asked for.
    pub equity_curve: Vec<f64>,
    pub equity_points_total: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct PineAlertInfo {
    pub subscription_id: String,
    pub script_id: Option<String>,
    pub script_name: String,
    /// Source-qualified market the script watches, e.g. `binance:BTCUSDT`.
    pub instrument: String,
    pub timeframe: String,
    /// `active`, `paused`, `script_changed` or `script_deleted`.
    pub state: String,
    pub note: String,
    pub chart_id: String,
    pub instance_id: Option<String>,
    pub source_hash: String,
    /// Whether it posts to webhook endpoints (which are never exposed here).
    pub sends_to_webhooks: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct PineAlertsResult {
    pub subscriptions: Vec<PineAlertInfo>,
    pub total: usize,
    /// DegenView does not keep a log of fired script alerts, so none can be returned.
    pub event_history_available: bool,
    pub recent_events: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct PineEditResult {
    pub script_id: String,
    /// `update_draft` or `save`.
    pub action: String,
    /// `updated` (draft written), `saved`, or `unchanged` (nothing needed doing).
    pub status: String,
    /// The revision (source hash) that was current before: what the caller's
    /// `expected_revision` matched.
    pub previous_revision: String,
    /// The revision now: the draft's hash after an update, the saved source's after a save.
    /// Pass it as `expected_revision` on the next edit.
    pub revision: String,
    /// The saved revision id, after a save.
    pub saved_revision_id: Option<String>,
    /// Whether the script was open in the Script Manager, whose editor now shows this text.
    pub applied_to_open_editor: bool,
    /// Whether an unsaved draft remains afterwards.
    pub has_draft: bool,
    /// The compile result of the text now current.
    pub compile: Option<PineCompileResult>,
}

/// The built-in indicators the app actually has, from the same list the chart menu uses.
pub fn indicator_catalog() -> Vec<IndicatorSpecInfo> {
    use crate::model::{
        BOLLINGER_MULTIPLIER, BOLLINGER_PERIOD, BuiltIn, EMA_DEFAULT_PERIOD, EMA_PERIODS,
        RSI_OVERBOUGHT, RSI_OVERSOLD, RSI_PERIOD, SUPERTREND_MULTIPLIER, SUPERTREND_PERIOD,
    };
    BuiltIn::ALL
        .into_iter()
        .map(|kind| IndicatorSpecInfo {
            id: kind.id().to_owned(),
            title: kind.menu_title().to_owned(),
            description: match kind {
                BuiltIn::Volume => "Traded volume bars under the price.".to_owned(),
                BuiltIn::Rsi => format!(
                    "Relative Strength Index over {RSI_PERIOD} periods, with {RSI_OVERSOLD}/{RSI_OVERBOUGHT} levels. Fixed settings."
                ),
                BuiltIn::Ema => "Exponential moving average of closes.".to_owned(),
                BuiltIn::Bollinger => format!(
                    "Bollinger Bands: {BOLLINGER_PERIOD}-period average with bands {BOLLINGER_MULTIPLIER} standard deviations out. Fixed settings."
                ),
                BuiltIn::TrendFlips => format!(
                    "Supertrend ({SUPERTREND_PERIOD} periods, {SUPERTREND_MULTIPLIER}x ATR) marking where the trend flips. Fixed settings."
                ),
            },
            placement: if kind.is_overlay() { "overlay" } else { "pane" }.to_owned(),
            chart_types: if kind.supports_line_charts() {
                vec!["candles".to_owned(), "line".to_owned()]
            } else {
                vec!["candles".to_owned()]
            },
            parameters: if kind == BuiltIn::Ema {
                vec![IndicatorParameterInfo {
                    name: "period".to_owned(),
                    value_type: "integer".to_owned(),
                    default: EMA_DEFAULT_PERIOD as i64,
                    allowed_values: EMA_PERIODS.iter().map(|p| *p as i64).collect(),
                }]
            } else {
                Vec::new()
            },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_lists_exactly_the_built_in_indicators() {
        let catalog = indicator_catalog();
        let ids: Vec<_> = catalog.iter().map(|spec| spec.id.as_str()).collect();
        assert_eq!(ids, ["volume", "rsi", "ema", "bollinger", "trend_flips"]);

        let ema = catalog.iter().find(|spec| spec.id == "ema").unwrap();
        assert_eq!(ema.placement, "overlay");
        assert_eq!(ema.parameters.len(), 1);
        assert_eq!(ema.parameters[0].allowed_values, [9, 20, 50, 100, 200]);
        assert!(
            ema.parameters[0]
                .allowed_values
                .contains(&ema.parameters[0].default)
        );

        let volume = catalog.iter().find(|spec| spec.id == "volume").unwrap();
        assert_eq!(volume.placement, "pane");
        assert_eq!(volume.chart_types, ["candles"]);
        for spec in catalog.iter().filter(|spec| spec.id != "ema") {
            assert!(spec.parameters.is_empty(), "{} has no parameters", spec.id);
        }
    }
}

// MARK: Candles and drawings

/// One candle: `[time, open, high, low, close, volume]`, in the order `columns` names. The volume
/// is `null` for a market whose provider reports none.
pub type CandleRow = (String, f64, f64, f64, f64, Option<f64>);

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct CandleRange {
    /// Start of the oldest returned bar.
    pub from: String,
    /// Start of the newest returned bar.
    pub to: String,
}

/// Plain figures over the returned bars, so an agent need not scan them for the extremes.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct CandleSummary {
    /// The highest high.
    pub high: f64,
    /// The lowest low.
    pub low: f64,
    pub first_open: f64,
    pub last_close: f64,
    /// `last_close` against `first_open`, in percent; null when the first open is zero.
    pub change_percent: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct CandlesResult {
    pub workspace_id: String,
    pub chart_id: String,
    pub instrument: String,
    /// The workspace timeframe, i.e. the length of each bar.
    pub timeframe: String,
    /// How the chart draws its bars. The candles below are always the market's own.
    pub chart_type: String,
    /// The meaning of each position in a `bars` row.
    pub columns: Vec<String>,
    /// Oldest first. Times are ISO-8601 UTC; pass them back as a drawing's `time`.
    pub bars: Vec<CandleRow>,
    pub count: usize,
    /// How many bars the chart holds in all (up to the replay position).
    pub loaded_bars: usize,
    pub range: Option<CandleRange>,
    pub summary: Option<CandleSummary>,
    /// Whether the chart holds bars older than the first returned. Page back with `before`.
    pub has_older_loaded: bool,
    /// Whether the market can serve more history than the chart holds. DegenView loads it only
    /// as someone scrolls the chart back, so asking again will not fetch it.
    pub older_history_available: bool,
    /// Whether the newest returned bar is still forming (its period has not ended).
    pub last_bar_forming: bool,
    /// Anything that changes how to read the bars.
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct DrawingAnchorInfo {
    pub time: String,
    pub price: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct DrawingStyleInfo {
    /// `#RRGGBBAA`.
    pub color: String,
    pub width: f64,
    /// `solid`, `dashed` or `dotted`.
    pub dash: String,
    pub extend_left: bool,
    pub extend_right: bool,
    /// `#RRGGBBAA` when the drawing is filled.
    pub fill_color: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct DrawingInfo {
    pub drawing_id: String,
    /// `trend_line`, `rectangle`, `fib_retracement`, `brush`, … (the app's own tool names).
    pub tool: String,
    /// The first 20 points, in placement order.
    pub anchors: Vec<DrawingAnchorInfo>,
    pub anchor_count: usize,
    /// Null for drawings that carry no style (the ruler).
    pub style: Option<DrawingStyleInfo>,
    pub text: Option<String>,
    pub locked: bool,
    pub hidden: bool,
    /// Whether this server can change it: trend lines and rectangles that are not locked.
    pub editable: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct DrawingsResult {
    pub workspace_id: String,
    pub chart_id: String,
    /// In stacking order, topmost last.
    pub drawings: Vec<DrawingInfo>,
    pub total: usize,
    pub truncated: bool,
    /// Rows this version of DegenView cannot read; they are kept, not shown.
    pub unreadable: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct DrawingsChangeResult {
    pub workspace_id: String,
    pub chart_id: String,
    /// `added`, `updated`, `unchanged` or `removed`.
    pub status: String,
    /// The drawings added, changed or removed.
    pub drawing_ids: Vec<String>,
    /// The added or changed drawings as they are now (empty after a removal).
    pub drawings: Vec<DrawingInfo>,
    /// Drawings the chart holds afterwards.
    pub total_drawings: usize,
}

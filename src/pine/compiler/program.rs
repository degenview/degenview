//! The compiled program and the script's declaration, inputs, bar events, symbol facts, theme
//! and visual output, ported from `Pine/Model/*` + `PineCompiledProgram`.

use std::collections::{HashMap, HashSet};

use crate::model::Kline;
use crate::pine::outputs::{
    ArrowOutput, BoxOutput, CandleOutput, ColorOutput, DISPLAY_PANE, FillOutput, HorizontalLine,
    LabelOutput, LineOutput, LinefillOutput, MarkerOutput, PlotOutput, PolylineOutput, TableOutput,
};
use crate::pine::runtime::state::AlertEvent;
use crate::pine::strategy::{StrategyReport, StrategySettings};
use crate::pine::syntax::ast::{Expression, Statement};
use crate::pine::{Diagnostic, ValueType};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ScriptType {
    #[default]
    Indicator,
    Strategy,
    Library,
}

impl ScriptType {}

#[derive(Clone, Debug, PartialEq)]
pub enum InputValue {
    Int(i64),
    Float(f64),
    Bool(bool),
    String(String),
    Color(u32),
    Source(String),
}

/// What an input is edited with, beyond what its value says: the string inputs of `input.symbol`,
/// `input.timeframe`, `input.session` and `input.text_area` and the float of `input.price` hold the
/// same kinds of value as `input.string` and `input.float` but are not typed the same way.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum InputControl {
    #[default]
    Standard,
    Price,
    Symbol,
    Timeframe,
    Session,
    TextArea,
}

#[derive(Clone, Debug, PartialEq)]
pub struct InputDefinition {
    pub id: String,
    pub kind: ValueType,
    pub control: InputControl,
    pub default_value: InputValue,
    pub title: Option<String>,
    pub tooltip: Option<String>,
    pub group: Option<String>,
    pub inline: Option<String>,
    pub confirm: bool,
    pub min_value: Option<f64>,
    pub max_value: Option<f64>,
    pub step: Option<f64>,
    pub options: Option<Vec<InputValue>>,
    /// What to show for each of `options`, when it differs from the value.
    pub option_titles: Option<Vec<String>>,
    /// `active`: the condition, on other inputs, under which the user can change this input. None
    /// when it can always be changed.
    pub active: Option<Expression>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct InputSchema {
    pub inputs: Vec<InputDefinition>,
}

impl InputSchema {
    /// Whether the user can change `input` while the inputs hold `values` (an input with no entry
    /// holds its default). A greyed-out input still holds its value, and the script reads it.
    pub fn is_active(&self, input: &InputDefinition, values: &HashMap<String, InputValue>) -> bool {
        input.active.as_ref().is_none_or(|condition| {
            crate::pine::inputs::activity::holds(condition, &self.inputs, values)
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DeclarationMetadata {
    pub script_type: ScriptType,
    pub pine_version: Option<i64>,
    pub title: String,
    pub short_title: Option<String>,
    pub overlay: bool,
    pub format: Option<String>,
    pub precision: Option<i64>,
    pub max_bars_back: Option<i64>,
    pub max_lines_count: Option<i64>,
    pub max_labels_count: Option<i64>,
    pub max_boxes_count: Option<i64>,
    pub max_polylines_count: Option<i64>,
    /// `behind_chart`: an overlay script's plots and drawings sit behind the candles.
    pub behind_chart: Option<bool>,
    /// Present for `strategy()` scripts.
    pub strategy: Option<StrategySettings>,
}

impl Default for DeclarationMetadata {
    fn default() -> Self {
        Self {
            script_type: ScriptType::Indicator,
            pine_version: None,
            title: String::new(),
            short_title: None,
            overlay: false,
            format: None,
            precision: None,
            max_bars_back: None,
            max_lines_count: None,
            max_labels_count: None,
            max_boxes_count: None,
            max_polylines_count: None,
            behind_chart: None,
            strategy: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct BarEvent {
    pub candle: Kline,
    pub phase: BarPhase,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BarPhase {
    Historical,
    RealtimeTick { is_new: bool },
    RealtimeClose { is_new: bool },
}

impl BarPhase {
    /// Whether the bar's values are final.
    pub fn is_confirmed(self) -> bool {
        !matches!(self, Self::RealtimeTick { .. })
    }

    pub fn is_realtime(self) -> bool {
        !matches!(self, Self::Historical)
    }
}

/// Symbol facts a script can read through `syminfo.*`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SymbolInfo {
    pub ticker: String,
    pub ticker_id: String,
    pub currency: String,
    pub kind: String,
    /// The asset a pair is priced in terms of (`BTC` of `BTCUSDT`); None when not a pair.
    pub base_currency: Option<String>,
    /// How the chart the script runs on is drawn: `chart.is_*` reads this, and a script on a
    /// price-based chart is fed that chart's constructed bars.
    pub chart: crate::model::ChartKind,
}

impl Default for SymbolInfo {
    fn default() -> Self {
        Self {
            ticker: String::new(),
            ticker_id: String::new(),
            currency: "USD".to_owned(),
            kind: "crypto".to_owned(),
            base_currency: None,
            chart: crate::model::ChartKind::Standard,
        }
    }
}

/// `chart.fg_color` / `chart.bg_color`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ChartTheme {
    pub foreground: u32,
    pub background: u32,
}

impl ChartTheme {
    pub const DARK: Self = Self {
        foreground: 0xd1d4_dcff,
        background: 0x1317_22ff,
    };
    pub const LIGHT: Self = Self {
        foreground: 0x1317_22ff,
        background: 0xffff_ffff,
    };
}

impl Default for ChartTheme {
    fn default() -> Self {
        Self::DARK
    }
}

/// The rendered output of one full script run.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VisualOutput {
    pub overlay: bool,
    /// Number of bars the script ran over.
    pub bar_count: usize,
    pub plots: Vec<PlotOutput>,
    pub hlines: Vec<HorizontalLine>,
    pub markers: Vec<MarkerOutput>,
    pub arrows: Vec<ArrowOutput>,
    pub backgrounds: Vec<ColorOutput>,
    pub bar_colors: Vec<ColorOutput>,
    pub fills: Vec<FillOutput>,
    pub lines: Vec<LineOutput>,
    pub labels: Vec<LabelOutput>,
    pub boxes: Vec<BoxOutput>,
    pub linefills: Vec<LinefillOutput>,
    pub polylines: Vec<PolylineOutput>,
    pub tables: Vec<TableOutput>,
    pub candles: Vec<CandleOutput>,
    pub alerts: Vec<AlertEvent>,
    pub strategy: Option<StrategyReport>,
    /// `behind_chart` of an overlay script.
    pub behind_chart: bool,
}

impl VisualOutput {
    /// Where the script draws (its own pane or the main chart, behind the candles or over them)
    /// and nothing it drew: what stands in for its output while a run over other bars is going.
    pub fn placement(&self) -> Self {
        Self {
            overlay: self.overlay,
            behind_chart: self.behind_chart,
            ..Self::default()
        }
    }

    /// Whether a script with its own pane asked (`force_overlay`) for some of its plots, markers,
    /// candles or drawings to be drawn on the main chart.
    pub fn has_forced_overlay(&self) -> bool {
        self.plots.iter().any(|plot| plot.force_overlay)
            || self.markers.iter().any(|marker| marker.force_overlay)
            || self.arrows.iter().any(|arrow| arrow.force_overlay)
            || self.candles.iter().any(|candle| candle.force_overlay)
            || self.lines.iter().any(|line| line.force_overlay)
            || self.labels.iter().any(|label| label.force_overlay)
            || self.boxes.iter().any(|boxed| boxed.force_overlay)
            || self.tables.iter().any(|table| table.force_overlay)
            || self.polylines.iter().any(|polyline| polyline.force_overlay)
    }

    /// Splits a pane script's output in two: what stays in its pane, and what it forced onto the
    /// main chart. A forced plot stays in the pane output with its pane display cleared, so a
    /// `fill()` between a forced plot and a pane plot still finds it; a `fill()` between two
    /// forced plots goes to the main chart with them, where their prices are on scale. Forced
    /// lines, labels, boxes, tables and polylines move to the main chart and leave the pane (a
    /// drawing has no display bit, and one left behind would stretch the pane's scale and catch
    /// its hover); a linefill goes with its lines when both are forced, else it stays in the pane,
    /// where it is not drawn because one of its lines is gone. Each side keeps the script's own
    /// identities, so style overrides and rows still match.
    pub fn split_forced_overlay(&self) -> (Self, Self) {
        let mut pane = self.clone();
        let mut forced = Self {
            overlay: true,
            bar_count: self.bar_count,
            behind_chart: false,
            ..Self::default()
        };
        for plot in &mut pane.plots {
            if plot.force_overlay {
                forced.plots.push(plot.clone());
                plot.display &= !DISPLAY_PANE;
            }
        }
        for marker in &mut pane.markers {
            if marker.force_overlay {
                forced.markers.push(marker.clone());
                marker.display &= !DISPLAY_PANE;
            }
        }
        for arrow in &mut pane.arrows {
            if arrow.force_overlay {
                forced.arrows.push(arrow.clone());
                arrow.display &= !DISPLAY_PANE;
            }
        }
        for candle in &mut pane.candles {
            if candle.force_overlay {
                forced.candles.push(candle.clone());
                candle.display &= !DISPLAY_PANE;
            }
        }
        let is_forced = |id: usize| forced.plots.iter().any(|plot| plot.id == id);
        let (on_chart, in_pane) = pane
            .fills
            .into_iter()
            .partition(|fill| is_forced(fill.plot_a) && is_forced(fill.plot_b));
        pane.fills = in_pane;
        forced.fills = on_chart;
        (forced.lines, pane.lines) = pane.lines.into_iter().partition(|line| line.force_overlay);
        (forced.labels, pane.labels) = pane
            .labels
            .into_iter()
            .partition(|label| label.force_overlay);
        (forced.boxes, pane.boxes) = pane
            .boxes
            .into_iter()
            .partition(|boxed| boxed.force_overlay);
        (forced.tables, pane.tables) = pane
            .tables
            .into_iter()
            .partition(|table| table.force_overlay);
        (forced.polylines, pane.polylines) = pane
            .polylines
            .into_iter()
            .partition(|polyline| polyline.force_overlay);
        let line_is_forced = |id: usize| forced.lines.iter().any(|line| line.id == id);
        (forced.linefills, pane.linefills) = pane
            .linefills
            .into_iter()
            .partition(|fill| line_is_forced(fill.line1) && line_is_forced(fill.line2));
        (pane, forced)
    }
}

#[derive(Clone, Debug)]
pub struct Import {
    pub path: String,
    pub alias: String,
    pub library: Option<Box<CompiledProgram>>,
}

/// Where a statement ends. Statement ranges are anchored at their first token, so a block's extent
/// is only known from here. Keyed by the UTF-16 offset the statement starts at.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StatementEnd {
    pub offset: usize,
    pub end: crate::pine::SourcePosition,
}

#[derive(Clone, Debug)]
pub struct CompiledProgram {
    pub source: String,
    pub statements: Vec<Statement>,
    pub declaration: DeclarationMetadata,
    pub input_schema: InputSchema,
    pub diagnostics: Vec<Diagnostic>,
    /// Names declared with `method`, which `value.name(…)` resolves ahead of builtins.
    pub method_names: HashSet<String>,
    /// The function or method each call site was resolved to at compile time: call-site number to
    /// position among the top-level functions. A call the checker could not settle is absent.
    pub resolutions: HashMap<usize, usize>,
    /// Names a library declares with `export`.
    pub exported_names: HashSet<String>,
    /// The libraries this script imports, with the compiled program of each that was found.
    pub imports: Vec<Import>,
    /// Where each statement ends, sorted by start offset.
    pub statement_ends: Vec<StatementEnd>,
}

impl CompiledProgram {
    pub fn is_valid(&self) -> bool {
        !self
            .diagnostics
            .iter()
            .any(|d| d.severity == crate::pine::DiagnosticSeverity::Error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pine::outputs::{
        BoxOutput, LabelOutput, LabelStyle, LineExtend, LineOutput, LineStyle, LinefillOutput,
        PolylineOutput, Size, TableOutput, TablePosition, TextAlign, YLocation,
    };

    #[test]
    fn bar_phase_flags() {
        assert!(!BarPhase::Historical.is_realtime());
        assert!(BarPhase::Historical.is_confirmed());
        assert!(BarPhase::RealtimeTick { is_new: false }.is_realtime());
        assert!(!BarPhase::RealtimeTick { is_new: false }.is_confirmed());
        assert!(BarPhase::RealtimeClose { is_new: true }.is_confirmed());
    }

    #[test]
    fn forced_overlay_visuals_split_off_a_pane_script() {
        use crate::pine::outputs::{CandleOutput, DISPLAY_ALL, FillOutput, PlotOutput, PlotStyle};
        let plot = |id: usize, force_overlay: bool| PlotOutput {
            id,
            title: None,
            values: vec![Some(1.0)],
            color: 0,
            line_width: 1,
            style: PlotStyle::Line,
            colors: vec![Some(0)],
            display: DISPLAY_ALL,
            hist_base: 0.0,
            offset: 0,
            force_overlay,
        };
        let fill = |id: usize, plot_a: usize, plot_b: usize| FillOutput {
            id,
            plot_a,
            plot_b,
            colors: vec![Some(0)],
            title: None,
            gradients: Vec::new(),
        };
        let candle = CandleOutput {
            id: 2,
            title: None,
            bars: vec![None],
            display: DISPLAY_ALL,
            bar_style: false,
            editable: true,
            show_last: None,
            force_overlay: true,
        };
        let output = VisualOutput {
            overlay: false,
            bar_count: 1,
            plots: vec![plot(0, false), plot(1, true), plot(3, true)],
            fills: vec![fill(10, 1, 3), fill(11, 0, 1)],
            candles: vec![candle],
            ..VisualOutput::default()
        };
        assert!(output.has_forced_overlay());
        let (pane, forced) = output.split_forced_overlay();
        assert!(forced.overlay && !forced.behind_chart);
        assert_eq!(forced.plots.len(), 2);
        assert_eq!(forced.plots[0].id, 1);
        assert_eq!(forced.candles.len(), 1);
        // The pane keeps every plot (a fill may still need the forced one) but no longer draws it.
        assert_eq!(pane.plots.len(), 3);
        // A fill between two forced plots is drawn with them on the main chart; one that reaches
        // into the pane stays there.
        assert_eq!(forced.fills.iter().map(|f| f.id).collect::<Vec<_>>(), [10]);
        assert_eq!(pane.fills.iter().map(|f| f.id).collect::<Vec<_>>(), [11]);
        assert_ne!(pane.plots[0].display & DISPLAY_PANE, 0);
        assert_eq!(pane.plots[1].display & DISPLAY_PANE, 0);
        assert_eq!(pane.candles[0].display & DISPLAY_PANE, 0);
        assert!(!VisualOutput::default().has_forced_overlay());
    }

    fn line(id: usize, force_overlay: bool) -> LineOutput {
        LineOutput {
            id,
            x1: 0,
            y1: 1.0,
            x2: 1,
            y2: 2.0,
            color: 0,
            width: 1,
            style: LineStyle::Solid,
            extend: LineExtend::None,
            time_anchored: false,
            force_overlay,
        }
    }

    fn label(id: usize, force_overlay: bool) -> LabelOutput {
        LabelOutput {
            id,
            x: 0,
            y: 1.0,
            text: String::new(),
            color: None,
            text_color: 0,
            style: LabelStyle::LabelDown,
            size: Size::Normal,
            tooltip: None,
            text_align: TextAlign::Center,
            yloc: YLocation::Price,
            time_anchored: false,
            force_overlay,
        }
    }

    fn boxed(id: usize, force_overlay: bool) -> BoxOutput {
        BoxOutput {
            id,
            left: 0,
            top: 2.0,
            right: 1,
            bottom: 1.0,
            border_color: None,
            border_width: 1,
            background_color: None,
            border_style: LineStyle::Solid,
            extend: LineExtend::None,
            text: String::new(),
            text_color: 0,
            text_size: Size::Normal,
            text_horizontal_align: TextAlign::Center,
            text_vertical_align: TextAlign::Center,
            time_anchored: false,
            force_overlay,
        }
    }

    fn table(id: usize, force_overlay: bool) -> TableOutput {
        TableOutput {
            id,
            position: TablePosition::TopRight,
            columns: 1,
            rows: 1,
            background_color: None,
            border_color: None,
            border_width: 0,
            frame_color: None,
            frame_width: 0,
            cells: Vec::new(),
            force_overlay,
        }
    }

    fn polyline(id: usize, force_overlay: bool) -> PolylineOutput {
        PolylineOutput {
            id,
            points: Vec::new(),
            closed: false,
            line_color: None,
            fill_color: None,
            style: LineStyle::Solid,
            width: 1,
            curved: false,
            force_overlay,
        }
    }

    fn linefill(id: usize, line1: usize, line2: usize) -> LinefillOutput {
        LinefillOutput {
            id,
            line1,
            line2,
            color: 0,
        }
    }

    fn ids<T>(items: &[T], id: impl Fn(&T) -> usize) -> Vec<usize> {
        items.iter().map(id).collect()
    }

    #[test]
    fn forced_drawings_move_to_the_main_chart_and_leave_the_pane() {
        let output = VisualOutput {
            overlay: false,
            bar_count: 1,
            lines: vec![line(0, true), line(1, false)],
            labels: vec![label(2, false), label(3, true)],
            boxes: vec![boxed(4, true), boxed(5, false)],
            tables: vec![table(6, false), table(7, true)],
            polylines: vec![polyline(8, true), polyline(9, false)],
            ..VisualOutput::default()
        };
        assert!(output.has_forced_overlay());
        let (pane, forced) = output.split_forced_overlay();
        assert!(forced.overlay && forced.bar_count == 1);
        assert_eq!(ids(&forced.lines, |l| l.id), [0]);
        assert_eq!(ids(&pane.lines, |l| l.id), [1]);
        assert_eq!(ids(&forced.labels, |l| l.id), [3]);
        assert_eq!(ids(&pane.labels, |l| l.id), [2]);
        assert_eq!(ids(&forced.boxes, |b| b.id), [4]);
        assert_eq!(ids(&pane.boxes, |b| b.id), [5]);
        assert_eq!(ids(&forced.tables, |t| t.id), [7]);
        assert_eq!(ids(&pane.tables, |t| t.id), [6]);
        assert_eq!(ids(&forced.polylines, |p| p.id), [8]);
        assert_eq!(ids(&pane.polylines, |p| p.id), [9]);
    }

    #[test]
    fn a_linefill_follows_its_lines_only_when_both_are_forced() {
        let output = VisualOutput {
            overlay: false,
            lines: vec![line(0, true), line(1, true), line(2, true), line(3, false)],
            linefills: vec![linefill(10, 0, 1), linefill(11, 1, 3), linefill(12, 3, 3)],
            ..VisualOutput::default()
        };
        let (pane, forced) = output.split_forced_overlay();
        assert_eq!(ids(&forced.linefills, |f| f.id), [10]);
        assert_eq!(ids(&pane.linefills, |f| f.id), [11, 12]);
    }

    #[test]
    fn a_forced_table_alone_is_a_forced_overlay() {
        let output = VisualOutput {
            overlay: false,
            tables: vec![table(0, true)],
            ..VisualOutput::default()
        };
        assert!(output.has_forced_overlay());
        let unforced = VisualOutput {
            tables: vec![table(0, false)],
            lines: vec![line(1, false)],
            labels: vec![label(2, false)],
            boxes: vec![boxed(3, false)],
            polylines: vec![polyline(4, false)],
            ..VisualOutput::default()
        };
        assert!(!unforced.has_forced_overlay());
    }
}

//! What the Screenshot Studio works from: a chart, or a layout of them, frozen at the moment of
//! capture. The Studio is a modal, so nothing here changes while it is open, and everything the
//! composer reads that could change underneath it (candles, drawings, script outputs, pictures,
//! the logo) is owned here, cut at the replay and decoded.
//!
//! A [`ChartCapture`] also remembers the view it started with as a [`Base`], which the Width and
//! Height dials and the drag measure from. A portion is a chart captured at the size of the
//! selection, showing exactly the bars and prices the selection covered.

use crate::model::chart::price_scale::PriceScaleMode;
use crate::model::chart::snapshot::framing::Base;
use crate::model::chart::viewport::zoom::Selection;
use crate::model::{DataSource, Drawing, IndicatorConfig, Kline, Timeframe};
use crate::service::image::icon;
use crate::ui::chart::card::{
    ChartCard, FrozenOutputs, SnapshotError, SnapshotFrame, SnapshotInput,
};
use crate::ui::chart::snapshot_scene::Images;
use eframe::egui::{self, ColorImage, Rect, Vec2, load::ImagePoll, load::SizeHint};
use std::sync::Arc;

/// The width, in pixels, a logo is asked for: a vector logo is drawn at this size, a bitmap one
/// is kept as it is. Larger than any header shows it at 2×.
const LOGO_PIXELS: u32 = 256;

/// What a chart is made of besides its card: the workspace's data for it, as the chart shows it.
pub struct ChartParts {
    pub chart_id: u64,
    pub symbol: String,
    pub source: DataSource,
    pub timeframe: Timeframe,
    /// The market's name as the header reads it.
    pub title: String,
    /// The candles as the chart shows them (cut at the replay's time while it runs).
    pub candles: Vec<Kline>,
    pub drawings: Vec<Drawing>,
    pub indicators: IndicatorConfig,
    /// The drawings whose note is open on the chart (a pin's).
    pub open_notes: Vec<u64>,
    /// The pictures the drawings show, decoded.
    pub images: Images,
    pub replay_active: bool,
    /// Draw the paper-trading orders and positions, if the chart shows them.
    pub include_trading: bool,
    pub scale_mode: PriceScaleMode,
    /// The market's logo, if the icon cache has it already; the header's monogram stands in
    /// otherwise.
    pub logo: Option<Arc<ColorImage>>,
}

impl ChartParts {
    /// What the card paints this chart from. `frame` says where it departs from the live chart.
    pub fn input<'a>(&'a self, frame: SnapshotFrame<'a>) -> SnapshotInput<'a> {
        SnapshotInput {
            symbol: &self.symbol,
            source: self.source,
            candles: &self.candles,
            scale_mode: self.scale_mode,
            indicators: &self.indicators,
            drawings: &self.drawings,
            open_notes: &self.open_notes,
            images: Some(&self.images),
            hide_all_drawings: false,
            replay_active: self.replay_active,
            include_trading: self.include_trading,
            frame,
        }
    }
}

/// One chart, frozen.
pub struct ChartCapture {
    /// Boxed: a layout holds several of these, a single capture one, and the data is large.
    parts: Box<ChartParts>,
    outputs: FrozenOutputs,
    /// The capture's size in points: the chart's own, or the selection's.
    size: Vec2,
    base: Base,
}

impl ChartCapture {
    /// Freezes `card` with `parts`, as `pixels_per_point` draws it (the scale text is measured
    /// at). `portion` is a rectangle in points from the chart's top-left corner, as laid out in a
    /// frame of [`ChartCard::snapshot_size`]: the capture then holds only what it covers.
    ///
    /// Fails when there is nothing to draw, or when the portion is too small to hold a bar.
    pub fn new(
        card: &ChartCard,
        parts: ChartParts,
        pixels_per_point: f32,
        portion: Option<Rect>,
    ) -> Result<Self, SnapshotError> {
        let outputs = card.snapshot_outputs();
        let full = card.snapshot_size();
        let measure = |frame: SnapshotFrame, size: Vec2| {
            card.snapshot_geometry(&parts.input(frame), size, pixels_per_point)
        };
        let mut size = full;
        let mut geometry = measure(
            SnapshotFrame {
                outputs: Some(&outputs),
                ..SnapshotFrame::default()
            },
            size,
        )?;
        if let Some(portion) = portion {
            let rect = portion.intersect(Rect::from_min_size(egui::Pos2::ZERO, full));
            let plot = geometry.view.plot;
            let selection = Selection::between(
                (rect.min.x, rect.min.y),
                (rect.max.x, rect.max.y),
                (plot.left(), plot.top(), plot.right(), plot.bottom()),
            );
            let viewport = geometry
                .view
                .viewport_for(&selection)
                .ok_or(SnapshotError::Empty)?;
            size = rect.size();
            geometry = measure(
                SnapshotFrame {
                    viewport: Some(viewport),
                    outputs: Some(&outputs),
                    ..SnapshotFrame::default()
                },
                size,
            )?;
        }
        let base = geometry.view.framing_base();
        Ok(Self {
            parts: Box::new(parts),
            outputs,
            size,
            base,
        })
    }

    /// What the card paints this chart from. `frame` says where it departs from the live chart.
    pub fn input<'a>(&'a self, frame: SnapshotFrame<'a>) -> SnapshotInput<'a> {
        self.parts.input(frame)
    }

    pub fn chart_id(&self) -> u64 {
        self.parts.chart_id
    }

    /// The capture's size in points.
    pub fn size(&self) -> Vec2 {
        self.size
    }

    /// The view the capture started with.
    pub fn base(&self) -> &Base {
        &self.base
    }

    pub fn outputs(&self) -> &FrozenOutputs {
        &self.outputs
    }

    pub fn images(&self) -> &Images {
        &self.parts.images
    }

    pub fn symbol(&self) -> &str {
        &self.parts.symbol
    }

    pub fn title(&self) -> &str {
        &self.parts.title
    }

    pub fn source(&self) -> DataSource {
        self.parts.source
    }

    pub fn timeframe(&self) -> Timeframe {
        self.parts.timeframe
    }

    pub fn logo(&self) -> Option<&Arc<ColorImage>> {
        self.parts.logo.as_ref()
    }

    /// Whether any built-in indicator or script has something on the chart: what the Include
    /// toggle for indicators can switch off.
    pub fn has_indicators(&self) -> bool {
        let drawn = self.parts.indicators.drawn();
        let built_in = drawn.show_volume
            || drawn.show_rsi
            || drawn.show_ema
            || drawn.show_bollinger
            || drawn.show_trend_flips;
        built_in || !self.outputs.is_empty()
    }

    /// Whether any drawing is visible on this chart (not hidden, shown on its timeframe, and
    /// readable), as the frame decides it.
    pub fn has_drawings(&self) -> bool {
        self.parts
            .drawings
            .iter()
            .any(|drawing| drawing.is_visible(self.parts.timeframe))
    }
}

/// Several charts as the workspace arranged them.
pub struct LayoutCapture {
    /// Each chart and the rectangle it had, in points from the layout's top-left corner.
    pub charts: Vec<(ChartCapture, Rect)>,
    /// The layout's size in points.
    pub size: Vec2,
    /// The part of the layout the image shows, for a selection that spans charts; the whole
    /// layout when `None`.
    pub crop: Option<Rect>,
    /// The chart the title block names: the one that was focused.
    pub focused: usize,
}

impl LayoutCapture {
    /// The part of the layout the image shows.
    pub fn view(&self) -> Rect {
        self.crop
            .unwrap_or_else(|| Rect::from_min_size(egui::Pos2::ZERO, self.size))
    }

    /// The chart the title block names.
    pub fn focused(&self) -> Option<&ChartCapture> {
        self.charts
            .get(self.focused)
            .or_else(|| self.charts.first())
            .map(|(chart, _)| chart)
    }
}

/// What the Studio captured.
pub enum Capture {
    Single(ChartCapture),
    Layout(LayoutCapture),
}

impl Capture {
    /// The image's content in points, before any style or margin is added.
    pub fn size(&self) -> Vec2 {
        match self {
            Self::Single(chart) => chart.size(),
            Self::Layout(layout) => layout.view().size(),
        }
    }

    /// The chart whose title the Styled header carries.
    pub fn headline(&self) -> Option<&ChartCapture> {
        match self {
            Self::Single(chart) => Some(chart),
            Self::Layout(layout) => layout.focused(),
        }
    }

    pub fn has_indicators(&self) -> bool {
        self.each(ChartCapture::has_indicators)
    }

    pub fn has_drawings(&self) -> bool {
        self.each(ChartCapture::has_drawings)
    }

    fn each(&self, mut test: impl FnMut(&ChartCapture) -> bool) -> bool {
        match self {
            Self::Single(chart) => test(chart),
            Self::Layout(layout) => layout.charts.iter().any(|(chart, _)| test(chart)),
        }
    }

    /// The pictures every chart's drawings show, by name: what the scene's context is handed.
    pub fn images(&self) -> Images {
        match self {
            Self::Single(chart) => chart.images().clone(),
            Self::Layout(layout) => layout
                .charts
                .iter()
                .flat_map(|(chart, _)| chart.images().iter())
                .map(|(name, image)| (name.clone(), Arc::clone(image)))
                .collect(),
        }
    }
}

/// The logo the icon cache holds for a chart's market, decoded; `None` while it has none, when
/// the header draws a monogram. Asks the cache to resolve one for next time.
pub fn cached_logo_image(
    ctx: &egui::Context,
    symbol: &str,
    source: DataSource,
) -> Option<Arc<ColorImage>> {
    if source == DataSource::CoinMarketCap {
        return None;
    }
    let Some(url) = icon::get_url(source, symbol) else {
        icon::request_with_base(source, symbol, &source.base_asset(symbol));
        return None;
    };
    match ctx.try_load_image(&url, SizeHint::Width(LOGO_PIXELS)) {
        Ok(ImagePoll::Ready { image }) => Some(image),
        _ => None,
    }
}

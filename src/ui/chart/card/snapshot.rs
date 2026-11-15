//! The chart as an image: the same frame the card draws, painted from what the card holds onto a
//! context of its own, so it needs no window, no input and no frame of the app's.
//!
//! Nothing here changes the card. It reads the last run of each script instead of running it
//! again, builds the constructed bars without keeping them, and draws drawings and markers with
//! nothing to point at: no crosshair, selection, hover, drag, button or tooltip.

use super::frame::{
    Decor, FrameData, FrameLayout, Overlays, PaneSlot, Placed, paint_pane_title,
    split_forced_overlays,
};
use super::overlays::OverlayFrame;
use super::*;
use crate::model::chart::snapshot::SnapshotDimensions;
use crate::model::{Drawing, Tool};
use crate::service::snapshot::raster::Scene;
use crate::ui::chart::canvas::Restyle;
use crate::ui::chart::snapshot_scene::{self, Images};

/// What a snapshot is made from besides the card itself: the data the workspace holds.
pub struct SnapshotInput<'a> {
    pub symbol: &'a str,
    pub source: DataSource,
    /// The market's candles as the chart shows them (cut at the replay's time while it runs).
    pub candles: &'a [Kline],
    pub scale_mode: PriceScaleMode,
    pub indicators: &'a IndicatorConfig,
    pub drawings: &'a [Drawing],
    /// The drawings whose note is open on the chart (a pin's).
    pub open_notes: &'a [u64],
    /// The pictures the drawings show, decoded: a snapshot has no store to ask for them.
    pub images: Option<&'a Images>,
    pub hide_all_drawings: bool,
    pub replay_active: bool,
    /// Draw the paper-trading orders and positions, if the chart shows them.
    pub include_trading: bool,
    pub frame: SnapshotFrame<'a>,
}

/// Where a snapshot departs from the live frame. `Default` departs nowhere. Everything here is
/// the snapshot's alone: it is drawn instead of the card's own state, which it never changes.
#[derive(Clone)]
pub struct SnapshotFrame<'a> {
    /// The bars and prices to draw instead of the card's own (a manual price range wins, as on
    /// the chart). It may show fewer bars than a chart ever does.
    pub viewport: Option<ChartViewport>,
    /// The scripts' outputs as they were frozen, instead of their last runs now.
    pub outputs: Option<&'a FrozenOutputs>,
    /// Draw the built-in indicators and the scripts' outputs, panes included. Off, the price
    /// chart takes the whole height.
    pub include_indicators: bool,
    pub restyle: Restyle,
}

impl Default for SnapshotFrame<'_> {
    fn default() -> Self {
        Self {
            viewport: None,
            outputs: None,
            include_indicators: true,
            restyle: Restyle::default(),
        }
    }
}

/// What the scripts' last runs drew, kept as it was: a snapshot given these draws them whatever
/// the scripts do afterwards. Cheap to clone (the outputs are shared).
#[derive(Clone)]
pub struct FrozenOutputs(Vec<Placed>);

/// Where a snapshot puts its parts, in points from the corner of the frame it is drawn in: what
/// a caller needs to frame a picture of it, read the way [`ChartCard::paint_snapshot`] lays it out.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the Screenshot Studio, next in this stack, is its caller"
    )
)]
pub struct SnapshotGeometry {
    pub view: ViewGeometry,
    pub axis_width: f32,
    pub time_axis_height: f32,
}

/// How many of each visual a script's last run put on the chart.
#[cfg(test)]
#[derive(Debug)]
pub struct VisualCounts {
    pub labels: usize,
    pub lines: usize,
    pub boxes: usize,
    pub tables: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SnapshotError {
    /// Nothing is drawn yet: no candles, or the chart has not been laid out.
    Empty,
    /// This chart is not a plot (a gauge, a card).
    Unsupported,
}

impl SnapshotError {
    pub const fn message(self) -> &'static str {
        match self {
            Self::Empty => "There is nothing on this chart to take a picture of yet.",
            Self::Unsupported => "This chart cannot be saved as an image.",
        }
    }
}

struct SnapshotOverlays<'a> {
    card: &'a ChartCard,
    painter: egui::Painter,
    include_trading: bool,
}

impl Overlays for SnapshotOverlays<'_> {
    fn pane(&mut self, painter: &egui::Painter, layout: &FrameLayout, slot: &PaneSlot) {
        paint_pane_title(painter, layout, slot, false);
    }

    fn finish(
        &mut self,
        _data: &FrameData,
        layout: &FrameLayout,
        view: &View,
        _overlays: &OverlayFrame,
    ) {
        let painter = self.painter.with_clip_rect(layout.plot);
        lines::paint(
            &painter,
            layout.plot,
            &view.scale,
            &self.card.alert_lines,
            self.card.appearance.price_format(),
            &self.card.alert_open,
        );
        if self.include_trading {
            self.card.paper.paint_markers(
                &painter,
                layout.plot,
                &view.scale,
                painter
                    .ctx()
                    .style_of(painter.ctx().theme())
                    .visuals
                    .text_color(),
            );
        }
    }
}

impl ChartCard {
    /// How big the chart was at the last frame, in points.
    pub fn snapshot_size(&self) -> Vec2 {
        // Without the header's strip: a snapshot has no header over its plot.
        Vec2::new(
            self.body_size.x,
            (self.body_size.y - self.top_inset).max(1.0),
        )
    }

    /// What instance `id`'s last run drew, counted.
    #[cfg(test)]
    pub fn pine_visual_for_tests(&self, id: u64) -> VisualCounts {
        let output = self.pine.styled_for_tests(id).expect("the script ran");
        VisualCounts {
            labels: output.labels.len(),
            lines: output.lines.len(),
            boxes: output.boxes.len(),
            tables: output.tables.len(),
        }
    }

    /// Pretends the chart was drawn at `size`.
    #[cfg(test)]
    pub fn set_snapshot_size_for_tests(&mut self, size: Vec2) {
        self.body_size = size;
    }

    /// The scripts' last runs, in the order the chart draws them. Unlike [`Self::pine_outputs`]
    /// it starts and requests nothing.
    pub(super) fn last_pine_outputs(&self) -> Vec<Placed> {
        self.pine.last_outputs(&self.instances)
    }

    /// The scripts' last runs, frozen for a snapshot to draw whatever happens to them next.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the Screenshot Studio, next in this stack, is its caller"
        )
    )]
    pub fn snapshot_outputs(&self) -> FrozenOutputs {
        FrozenOutputs(self.last_pine_outputs())
    }

    /// Lays the snapshot out in `rect` of `painter` and hands what it finds to `then`, the one
    /// path painting and measuring share, so a measure is what the paint does.
    fn with_snapshot_frame<R>(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        input: &SnapshotInput,
        then: impl FnOnce(&FrameData, &FrameLayout, &View) -> R,
    ) -> Result<R, SnapshotError> {
        let candles = input.candles;
        if candles.is_empty() || rect.width() < 1.0 || rect.height() < 1.0 {
            return Err(SnapshotError::Empty);
        }
        let source = input.source;
        if source == DataSource::CoinMarketCap && crate::model::market::cmc::is_latest(input.symbol)
        {
            return Err(SnapshotError::Unsupported);
        }
        let frame = &input.frame;
        let style = CanvasStyle::resolve(&self.appearance.canvas).restyled(frame.restyle);
        let line = source.is_line_chart();
        let chart_type = if line {
            ChartType::Candles
        } else {
            self.appearance.style.effective_type(source)
        };
        let constructed = self.constructed_for(candles, chart_type);
        let bars: &[Kline] = constructed
            .as_ref()
            .map_or(candles, |built| built.klines.as_slice());
        let outputs = match (frame.include_indicators, frame.outputs) {
            (false, _) => Vec::new(),
            (true, Some(frozen)) => split_forced_overlays(frozen.0.clone(), line),
            (true, None) => split_forced_overlays(self.last_pine_outputs(), line),
        };
        let indicators = if frame.include_indicators {
            *input.indicators
        } else {
            IndicatorConfig::default()
        };
        let viewport = frame.viewport.as_ref().unwrap_or(&self.viewport);
        let extent = self.extent_of(viewport, &outputs, line, bars.len());
        let framing = Framing {
            viewport,
            extent: &extent,
        };
        let data = FrameData {
            symbol: input.symbol,
            source,
            line,
            chart_type,
            scale_mode: input.scale_mode,
            candles,
            bars,
            constructed: constructed.as_deref(),
            outputs: &outputs,
            indicators: &indicators,
        };
        let layout = self.frame_layout(&data, framing, rect, 0.0, painter, &style);
        let view = self
            .view(
                framing,
                bars,
                (layout.plot, 0.0),
                input.scale_mode,
                chart_type,
            )
            .ok_or(SnapshotError::Empty)?;
        Ok(then(&data, &layout, &view))
    }

    /// Paints the chart into `rect` of `painter`, as it stands.
    pub fn paint_snapshot(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        input: &SnapshotInput,
    ) -> Result<(), SnapshotError> {
        self.with_snapshot_frame(painter, rect, input, |data, layout, view| {
            canvas::paint_background(painter, rect, layout.style.fill);
            let mut layer = DrawingLayer::default();
            layer.set_open_notes(input.open_notes);
            let mut drawing = DrawingFrame::new(input.drawings, Tool::Cursor, None, 1).with_view(
                self.timeframe.unwrap_or(Timeframe::Day),
                input.hide_all_drawings,
            );
            let mut overlays = SnapshotOverlays {
                card: self,
                painter: painter.clone(),
                include_trading: input.include_trading,
            };
            self.paint_frame(
                painter,
                data,
                layout,
                view,
                &mut layer,
                &mut drawing,
                &Decor {
                    chart_id: 0,
                    replay_active: input.replay_active,
                    time_lock: false,
                    crosshair: None,
                },
                &mut overlays,
            );
        })
    }

    /// Where [`Self::paint_snapshot`] would put the chart in a frame of `size` points at
    /// `pixels_per_point`. Text is measured at that scale, so ask at the scale the picture will
    /// be drawn at. Paints and changes nothing.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the Screenshot Studio, next in this stack, is its caller"
        )
    )]
    pub fn snapshot_geometry(
        &self,
        input: &SnapshotInput,
        size: Vec2,
        pixels_per_point: f32,
    ) -> Result<SnapshotGeometry, SnapshotError> {
        snapshot_scene::measure(size, pixels_per_point, |painter, rect| {
            self.with_snapshot_frame(painter, rect, input, |data, layout, view| {
                SnapshotGeometry {
                    view: view.geometry(layout.plot, data.bars.len()),
                    axis_width: layout.rect.right() - layout.plot.right(),
                    time_axis_height: layout.style.time_axis_height(),
                }
            })
        })
    }

    /// The chart tessellated for a `dimensions` image: painted on a context of its own at the
    /// image's scale, so text and lines are laid out for those pixels, not stretched to them.
    pub fn snapshot_scene(
        &self,
        input: &SnapshotInput,
        dimensions: SnapshotDimensions,
        background: Color32,
    ) -> Result<Scene, SnapshotError> {
        snapshot_scene::render(
            (dimensions.width, dimensions.height),
            dimensions.pixels_per_point,
            background,
            input.images,
            |painter, rect| self.paint_snapshot(painter, rect, input),
        )
    }
}

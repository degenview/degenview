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
use super::*;
use crate::model::snapshot::SnapshotDimensions;
use crate::model::{Drawing, Tool};
use crate::service::snapshot_raster::Scene;

/// What a snapshot is made from besides the card itself: the data the workspace holds.
pub struct SnapshotInput<'a> {
    pub symbol: &'a str,
    pub source: DataSource,
    /// The market's candles as the chart shows them (cut at the replay's time while it runs).
    pub candles: &'a [Kline],
    pub log_scale: bool,
    pub indicators: &'a IndicatorConfig,
    pub drawings: &'a [Drawing],
    /// The drawings whose note is open on the chart (a pin's).
    pub open_notes: &'a [u64],
    pub hide_all_drawings: bool,
    pub replay_active: bool,
    /// Draw the paper-trading orders and positions, if the chart shows them.
    pub include_trading: bool,
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

    fn finish(&mut self, _data: &FrameData, layout: &FrameLayout, view: &View) {
        let painter = self.painter.with_clip_rect(layout.plot);
        alert_lines::paint(
            &painter,
            layout.plot,
            &view.scale,
            &self.card.alert_lines,
            self.card.appearance.price_format(),
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
        self.body_size
    }

    /// What instance `id`'s last run drew, counted.
    #[cfg(test)]
    pub fn pine_visual_for_tests(&self, id: u64) -> VisualCounts {
        let output = &self.runs[&id].styled;
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
        self.instances
            .iter()
            .filter(|instance| instance.visible)
            .filter_map(|instance| {
                let run = self
                    .runs
                    .get(&instance.id)
                    .filter(|run| run.placement_known)?;
                Some((Arc::clone(&run.styled), instance.visual_order, instance.id))
            })
            .collect()
    }

    /// Paints the chart into `rect` of `painter`, as it stands.
    pub fn paint_snapshot(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        input: &SnapshotInput,
    ) -> Result<(), SnapshotError> {
        let candles = input.candles;
        if candles.is_empty() || rect.width() < 1.0 || rect.height() < 1.0 {
            return Err(SnapshotError::Empty);
        }
        let source = input.source;
        if source == DataSource::CoinMarketCap && crate::service::cmc::is_latest(input.symbol) {
            return Err(SnapshotError::Unsupported);
        }
        let style = CanvasStyle::resolve(&self.appearance.canvas);
        canvas_render::paint_background(painter, rect, style.fill);
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
        let outputs = split_forced_overlays(self.last_pine_outputs(), line);
        let data = FrameData {
            symbol: input.symbol,
            source,
            line,
            chart_type,
            log_scale: input.log_scale,
            candles,
            bars,
            constructed: constructed.as_deref(),
            outputs: &outputs,
            indicators: input.indicators,
        };
        let layout = self.frame_layout(&data, rect, painter, &style);
        let Some(view) = self.view(bars, layout.plot, input.log_scale, chart_type) else {
            return Err(SnapshotError::Empty);
        };
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
            &data,
            &layout,
            &view,
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
        Ok(())
    }

    /// The chart tessellated for a `dimensions` image: painted on a context of its own at the
    /// image's scale, so text and lines are laid out for those pixels, not stretched to them.
    pub fn snapshot_scene(
        &self,
        input: &SnapshotInput,
        dimensions: SnapshotDimensions,
        background: Color32,
    ) -> Result<Scene, SnapshotError> {
        let scale = dimensions.pixels_per_point;
        let size = Vec2::new(
            dimensions.width as f32 / scale,
            dimensions.height as f32 / scale,
        );
        let ctx = egui::Context::default();
        theme::install_offscreen(&ctx);
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            viewports: std::iter::once((
                egui::ViewportId::ROOT,
                egui::ViewportInfo {
                    native_pixels_per_point: Some(scale),
                    ..Default::default()
                },
            ))
            .collect(),
            ..Default::default()
        };
        let mut result = Ok(());
        let mut output = ctx.run_ui(raw, |ui| {
            let rect = Rect::from_min_size(Pos2::ZERO, size);
            result = self.paint_snapshot(&ui.painter_at(rect), rect, input);
        });
        if let Err(error) = result {
            output.textures_delta.clear();
            return Err(error);
        }
        Ok(Scene::new(
            ctx.tessellate(output.shapes, output.pixels_per_point),
            output.textures_delta,
            output.pixels_per_point,
            (dimensions.width, dimensions.height),
            background,
        ))
    }
}

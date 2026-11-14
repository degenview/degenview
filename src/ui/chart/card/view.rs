//! What a frame shows: the slice of the bars in view, the price range it is scaled to, and the
//! plot area under the header.

use super::*;

/// The geometry of the last frame, as a test needs it to aim a pointer.
#[cfg(test)]
#[derive(Clone, Copy)]
pub(super) struct Probe {
    pub(super) plot: Rect,
    pub(super) start: usize,
    pub(super) lead: f32,
    pub(super) slot_width: f32,
    pub(super) minimum: f64,
    pub(super) maximum: f64,
}

/// The visible slice of the data and the scales that map it onto the plot.
pub(super) struct View {
    pub(super) start: usize,
    pub(super) end: usize,
    pub(super) scale: PriceScale,
    pub(super) slot_width: f32,
    pub(super) lead: f32,
}

impl View {
    pub(super) fn projection<'a>(
        &'a self,
        plot: Rect,
        candles: &'a [Kline],
        format: PriceFormat,
        ctx: &egui::Context,
    ) -> Projection<'a> {
        Projection {
            plot,
            pixels_per_point: ctx.pixels_per_point(),
            measure: TextMeasure::new(ctx),
            format,
            scale: &self.scale,
            times: TimeIndex::new(candles),
            start: self.start,
            slot_width: self.slot_width,
            lead: self.lead,
        }
    }
}

impl ChartCard {
    /// Tells the card how much of the plot's top the header covers (zero when there is none). The
    /// plot reaches the card's top edge; the price scale starts below the header.
    pub fn set_top_inset(&mut self, inset: f32) {
        self.top_inset = inset.max(0.0);
    }

    /// `plot` less the part the header covers: what top-anchored readouts and tags are placed in.
    pub(in crate::ui) fn below_header(&self, plot: Rect) -> Rect {
        Rect::from_min_max(
            Pos2::new(plot.min.x, (plot.min.y + self.top_inset).min(plot.max.y)),
            plot.max,
        )
    }

    /// Visible range and price scale for the current pan/zoom, if anything is shown.
    pub(super) fn view(
        &self,
        candles: &[Kline],
        (plot, top_inset): (Rect, f32),
        mode: PriceScaleMode,
        chart_type: ChartType,
    ) -> Option<View> {
        let window = self.window(candles.len());
        let (start, end) = (window.start, window.end);
        let shown = &candles[start..end];
        if shown.is_empty() {
            return None;
        }
        let (mut low, mut high) = self.value_range(shown, chart_type);
        if low > high {
            (low, high) = (0.0, 1.0);
        }
        // Every line has to fit, not only the first choice the candles come from.
        let (from, to) = (shown[0].open_time, shown[shown.len() - 1].open_time);
        for line in &self.series {
            for candle in self
                .series_points(line)
                .filter(|candle| (from..=to).contains(&candle.open_time))
            {
                low = low.min(candle.low);
                high = high.max(candle.high);
            }
        }
        // So does every comparison line drawn on the chart's own scale, rebased as it will be drawn.
        let reference = chart::price_scale::reference_from(shown[0].close);
        if let Some((from, to)) = self
            .overlay_frame(candles, start, end, chart_type, reference)
            .extent()
        {
            low = low.min(from);
            high = high.max(to);
        }
        // Whatever an overlay script draws has to fit too.
        for value in &self.script_extent.values {
            low = low.min(*value);
            high = high.max(*value);
        }
        // A logarithm takes no price at or below zero. Such a view is drawn linear, the mode
        // staying chosen so the log scale comes back with the data.
        let logarithmic = mode.is_logarithmic() && chart::price_scale::can_draw_logarithmic(low);
        let (minimum, maximum) =
            chart::canvas::price_range(low, high, self.appearance.canvas.margins, logarithmic);
        let PriceRange { minimum, maximum } = self
            .viewport
            .vertical
            .resolve(PriceRange::new(minimum, maximum), logarithmic);
        Some(View {
            start,
            end,
            scale: PriceScale {
                minimum,
                maximum,
                // Below what covers the plot's top, so the bars clear the header by default.
                top: plot.top() + top_inset,
                height: (plot.height() - top_inset).max(1.0),
                logarithmic,
                mode,
                // Percent and Indexed measure from the first bar shown, so a pan moves the
                // reference and the axis follows; the map itself does not change.
                reference,
            },
            slot_width: self.slot_width(plot),
            lead: window.lead,
        })
    }

    /// The open times of the first and last candle the plot shows: what `chart.left_visible_bar_time`
    /// and `chart.right_visible_bar_time` read. A run that reads them is given this range.
    pub(crate) fn visible_open_range(&self, candles: &[Kline]) -> Option<(i64, i64)> {
        let window = self.window(candles.len());
        candles
            .get(window.start..window.end)
            .and_then(|visible| Some((visible.first()?.open_time, visible.last()?.open_time)))
    }

    /// The bars the plot shows out of `len`, and where they sit in its slots.
    fn window(&self, len: usize) -> Window {
        chart::canvas::visible_window(
            len,
            self.viewport.horizontal.visible,
            self.viewport.horizontal.right_offset,
            self.reserved_slots(),
        )
    }

    /// The prices the main series spans over `shown`, which the scale has to reach. Only what is
    /// drawn counts: a line chart follows its one value, not the bars' wicks.
    pub(super) fn value_range(&self, shown: &[Kline], chart_type: ChartType) -> (f64, f64) {
        let style = &self.appearance.style;
        let reach = |value: fn(&Kline) -> f64| {
            shown
                .iter()
                .map(value)
                .filter(|value| value.is_finite())
                .fold((f64::MAX, f64::MIN), |(low, high), value| {
                    (low.min(value), high.max(value))
                })
        };
        match chart_type.data_class() {
            DataClass::StandardLine => match chart_type {
                ChartType::HlcArea => {
                    let (low, _) = reach(|bar| bar.low);
                    let (_, high) = reach(|bar| bar.high);
                    (low, high)
                }
                _ => {
                    let source = style.value_source();
                    shown
                        .iter()
                        .map(|bar| source.value(bar))
                        .filter(|value| value.is_finite())
                        .fold((f64::MAX, f64::MIN), |(low, high), value| {
                            (low.min(value), high.max(value))
                        })
                }
            },
            _ => {
                let (low, _) = reach(|bar| bar.low);
                let (_, high) = reach(|bar| bar.high);
                (low, high)
            }
        }
    }

    /// The width of one candle slot: the plot shared by the visible candles and the free slots an
    /// overlay script reserves past the last one.
    pub(super) fn slot_width(&self, plot: Rect) -> f32 {
        plot.width() / (self.viewport.horizontal.visible + self.reserved_slots()) as f32
    }

    /// Back to the newest bars at the natural scale, autoscale on.
    pub fn reset_view(&mut self) {
        self.viewport.reset();
        self.gesture = None;
        self.newest_bar = None;
        self.forget_zoom_history();
    }

    /// Whether the price axis follows the bars on screen (no range placed by dragging).
    #[cfg(test)]
    pub(crate) fn prices_are_autoscaled(&self) -> bool {
        self.viewport.vertical.manual.is_none()
    }

    /// Places the price range as a drag of the chart would.
    #[cfg(test)]
    pub(crate) fn place_prices_for_test(&mut self, minimum: f64, maximum: f64) {
        self.viewport.vertical.manual = Some(crate::model::chart::viewport::PriceRange::new(
            minimum, maximum,
        ));
    }

    /// Price scale back to autoscale at the natural zoom, leaving the time window where it is.
    pub fn reset_price_scale(&mut self) {
        self.viewport.vertical.reset();
    }
}

//! The live chart's frame, around the shared painter in [`super::frame`]: what `body_inner` sets up
//! before the layout and settles after the paint. The pure pieces are plain functions so they can
//! be tested without a UI.

use super::frame::FrameLayout;
use super::*;
use crate::ui::paper_chart_trading::{MarkerMenu, PaperOverlayAction};

/// What one call to [`ChartCard::body`] draws and reports on.
pub(super) struct BodyArgs<'a> {
    pub symbol: &'a str,
    pub candles: &'a [Kline],
    pub error: Option<&'a str>,
    pub log_scale: bool,
    pub source: DataSource,
    pub indicators: &'a IndicatorConfig,
    pub crosshair: Option<&'a CrosshairInfo>,
    pub chart_id: u64,
}

/// What the live overlays collected while painting, for the card to apply once the paint is over.
pub(super) struct OverlayResults {
    pub pane_actions: Vec<(u64, PaneAction)>,
    pub alert_menu: Option<alert_lines::LineMenu>,
    pub alert_moves: Vec<alert_lines::AlertMove>,
    pub paper: (Vec<PaperOverlayAction>, Option<MarkerMenu>),
    pub navigation: Option<chart_buttons::NavAction>,
}

/// What a script's `syminfo.*` and `chart.is_*` read for this market and chart type.
pub(super) fn symbol_info(symbol: &str, source: DataSource, chart_type: ChartType) -> SymbolInfo {
    SymbolInfo {
        ticker: symbol.to_owned(),
        ticker_id: format!("{}:{}", source.watchlist_slug(), symbol),
        currency: "USD".to_owned(),
        kind: if source == DataSource::Alpaca {
            "stock"
        } else {
            "crypto"
        }
        .to_owned(),
        base_currency: None,
        chart: chart_type.kind(),
    }
}

/// Bars that follow price carry no length of their own: scripts read the candles'. `None` for a
/// chart drawn from the candles themselves, which scripts already see with their own length.
pub(super) fn source_bar_seconds(candles: &[Kline], constructed: bool) -> Option<f64> {
    if !constructed {
        return None;
    }
    let last = candles.last()?;
    let seconds = (last.close_time - last.open_time + 1) as f64 / 1000.0;
    (seconds > 0.0).then(|| seconds.round())
}

impl ChartCard {
    /// What stands in for the chart when there is nothing to plot: the error, nothing, or the CMC
    /// gauge. `true` when it did, and the frame ends there.
    pub(super) fn paint_placeholder(
        &self,
        ui: &egui::Ui,
        response: &Response,
        painter: &egui::Painter,
        args: &BodyArgs,
    ) -> bool {
        let rect = response.rect;
        if let Some(message) = args.error
            && args.candles.is_empty()
        {
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                message,
                FontId::proportional(13.0),
                theme::DOWN,
            );
            return true;
        }
        if args.candles.is_empty() {
            return true;
        }
        if args.source == DataSource::CoinMarketCap && crate::service::cmc::is_latest(args.symbol) {
            if let Some(reading) = crate::service::cmc::latest_reading(args.symbol) {
                let description =
                    crate::ui::cmc_gauge::paint(ui, painter, rect, args.symbol, &reading);
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Panel, true, &description)
                });
            }
            return true;
        }
        false
    }

    /// Overlay scripts may draw past the last bar (a stack to the right of the chart) and above
    /// or below the candles; the line chart draws none of that.
    pub(super) fn refresh_script_extent(
        &mut self,
        outputs: &[frame::Placed],
        line: bool,
        bars: usize,
    ) {
        let shown_count = canvas::visible_window(
            bars,
            self.viewport.horizontal.visible,
            self.viewport.horizontal.right_offset,
            usize::from(self.appearance.canvas.margins.right),
        )
        .count();
        self.script_extent = if line {
            OverlayExtent::default()
        } else {
            overlay_extent(
                outputs
                    .iter()
                    .filter(|(output, _, _)| output.overlay)
                    .map(|(output, _, _)| &**output),
                shown_count,
            )
        };
    }

    /// The panes the layout drew are the card's panes; a maximized one that is gone is dropped.
    pub(super) fn keep_panes_in_step(&mut self, layout: &FrameLayout) {
        self.pane_ids = layout.pane_specs.iter().map(|pane| pane.id).collect();
        if self
            .maximized_pane
            .is_some_and(|id| !self.pane_ids.contains(&id))
        {
            self.maximized_pane = None;
        }
    }

    /// The drawing layer's pointer, then the chart's own navigation, on the layout this frame
    /// painted. Returns the regions the pointer is answered against.
    pub(super) fn handle_frame_input(
        &mut self,
        ui: &mut egui::Ui,
        response: &Response,
        data: &frame::FrameData,
        layout: &FrameLayout,
        drawing: &mut DrawingFrame,
        replay: &mut ReplayChartFrame,
    ) -> navigation::Regions {
        let (bars, plot) = (data.bars, layout.plot);
        replay.picked_ms = None;
        if layout.main_visible
            && let Some(view) = self.view(bars, plot, data.log_scale, data.chart_type)
        {
            if replay.selecting {
                self.layer.clear_hover();
            } else {
                self.layer.update(
                    ui,
                    response,
                    &view.projection(plot, bars, self.appearance.price_format(), ui.ctx()),
                    drawing,
                );
            }
        }
        let regions = navigation::Regions {
            plot,
            main: layout.rect,
            chart: layout.full_rect,
        };
        let navigation = navigation::Navigation {
            regions,
            shown: self
                .view(bars, plot, data.log_scale, data.chart_type)
                .map(|view| PriceRange::new(view.scale.minimum, view.scale.maximum)),
            total: bars.len(),
            slot_width: self.slot_width(plot),
            log_scale: data.log_scale,
            drawing_claims: self.layer.claims_drag(),
            response,
        };
        drawing.escape_consumed |= self.handle_input(ui, &navigation);
        self.handle_axis_double_click(response, &regions);
        regions
    }

    /// What the overlays asked for while painting.
    pub(super) fn apply_overlay_results(&mut self, results: OverlayResults, bars: usize) {
        let OverlayResults {
            pane_actions,
            alert_menu,
            alert_moves,
            paper,
            navigation,
        } = results;
        for (id, action) in pane_actions {
            self.apply_pane_action(id, action);
        }
        if alert_menu.is_some() {
            self.alert_menu = alert_menu;
        }
        self.alert_moves.extend(alert_moves);
        self.paper.actions.extend(paper.0);
        self.paper.menu = paper.1.or(self.paper.menu.take());
        if let Some(action) = navigation {
            self.apply_navigation(action, bars);
        }
    }
}

/// Where the pointer is on the plot, for the workspace's crosshair and the other charts'.
pub(super) fn report_hover(
    pointer: &mut ChartPointer,
    response: &Response,
    plot: Rect,
    view: &View,
    projection: &Projection,
) {
    if let Some(position) = response.hover_pos().filter(|point| plot.contains(*point)) {
        pointer.hover_x_fraction = Some(((position.x - plot.left()) / plot.width()) as f64);
        pointer.hover_price = Some(view.scale.price(position.y));
        pointer.hover_anchor = Some(projection.anchor(position));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candle(open_time: i64, close_time: i64) -> Kline {
        Kline {
            open_time,
            close_time,
            open: 1.0,
            high: 2.0,
            low: 0.5,
            close: 1.5,
            volume: 1.0,
        }
    }

    #[test]
    fn only_alpaca_is_a_stock() {
        assert_eq!(
            symbol_info("AAPL", DataSource::Alpaca, ChartType::Candles).kind,
            "stock"
        );
        for source in [
            DataSource::Binance,
            DataSource::Coinbase,
            DataSource::Kalshi,
        ] {
            assert_eq!(
                symbol_info("X", source, ChartType::Candles).kind,
                "crypto",
                "{source:?}"
            );
        }
    }

    #[test]
    fn the_ticker_id_is_the_sources_slug_and_the_symbol() {
        let info = symbol_info("BTCUSDT", DataSource::Binance, ChartType::Candles);
        assert_eq!(info.ticker, "BTCUSDT");
        assert_eq!(info.ticker_id, "BINANCE:BTCUSDT");
        assert_eq!(info.currency, "USD");
        assert_eq!(info.base_currency, None);
    }

    #[test]
    fn the_symbol_info_carries_the_chart_type() {
        let renko = symbol_info("X", DataSource::Binance, ChartType::Renko);
        assert_eq!(renko.chart, ChartType::Renko.kind());
        let standard = symbol_info("X", DataSource::Binance, ChartType::Candles);
        assert_eq!(standard.chart, ChartType::Candles.kind());
        assert_ne!(renko.chart, standard.chart);
    }

    #[test]
    fn a_chart_of_the_candles_themselves_has_no_source_bar_length() {
        assert_eq!(source_bar_seconds(&[candle(0, 3_599_999)], false), None);
    }

    #[test]
    fn constructed_bars_read_the_length_of_the_last_candle() {
        // An hour: open to close inclusive of the last millisecond.
        assert_eq!(
            source_bar_seconds(&[candle(0, 3_599_999)], true),
            Some(3600.0)
        );
        // The last candle decides, and the length is rounded to whole seconds.
        let candles = [candle(0, 59_999), candle(60_000, 60_000 + 899_999)];
        assert_eq!(source_bar_seconds(&candles, true), Some(900.0));
        assert_eq!(source_bar_seconds(&[candle(0, 1_499)], true), Some(2.0));
    }

    #[test]
    fn no_candles_or_an_empty_span_give_no_source_bar_length() {
        assert_eq!(source_bar_seconds(&[], true), None);
        assert_eq!(source_bar_seconds(&[candle(1_000, 500)], true), None);
    }
}

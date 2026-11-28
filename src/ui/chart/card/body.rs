//! The live chart's frame, around the shared painter in [`super::frame`]: [`ChartCard::body`], and
//! what `body_inner` sets up before the layout and settles after the paint. The pure pieces are
//! plain functions so they can be tested without a UI.

use super::frame::FrameLayout;
use super::*;
use crate::i18n::tr;
use crate::model::chart::sync::TimeAxis;
use crate::pine::time::bucket::seconds_of;
use crate::ui::paper::chart_trading::{MarkerMenu, PaperOverlayAction};

/// What one call to [`ChartCard::body`] draws and reports on.
pub(super) struct BodyArgs<'a> {
    pub symbol: &'a str,
    pub candles: &'a [Kline],
    pub error: Option<&'a ProviderError>,
    pub scale_mode: PriceScaleMode,
    pub source: DataSource,
    pub indicators: &'a IndicatorConfig,
    pub crosshair: Option<&'a CrosshairInfo>,
    pub chart_id: u64,
}

/// What the live overlays collected while painting, for the card to apply once the paint is over.
pub(super) struct OverlayResults {
    pub pane_actions: Vec<(u64, PaneAction)>,
    pub alert_menu: Option<lines::LineMenu>,
    pub alert_moves: Vec<lines::AlertMove>,
    pub alert_toggled: Vec<crate::model::alert::AlertId>,
    pub alert_deleted: Vec<crate::model::alert::AlertId>,
    pub paper: (Vec<PaperOverlayAction>, Option<MarkerMenu>),
    pub live: Vec<crate::ui::broker::chart_trading::BrokerOverlayAction>,
    pub navigation: Option<buttons::NavAction>,
    pub scale_mode: Option<PriceScaleMode>,
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
        grid: source.grid(),
    }
}

/// Bars that follow price carry no length of their own: scripts read the candles', which are the
/// chart's timeframe, else (a chart with none set) those of the newest candle. `None` for a chart
/// drawn from the candles themselves, which scripts already see with their own length.
pub(super) fn source_bar_seconds(
    candles: &[Kline],
    timeframe: Option<Timeframe>,
    constructed: bool,
) -> Option<f64> {
    if !constructed {
        return None;
    }
    if let Some(timeframe) = timeframe {
        return Some(seconds_of(timeframe));
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
        if let Some(error) = args.error
            && args.candles.is_empty()
        {
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                error.to_string(),
                FontId::proportional(13.0),
                theme::DOWN,
            );
            return true;
        }
        if args.candles.is_empty() {
            return true;
        }
        if args.source == DataSource::CoinMarketCap
            && crate::model::market::cmc::is_latest(args.symbol)
        {
            if let Some(reading) = crate::service::market::cmc::latest_reading(args.symbol) {
                let description =
                    crate::ui::chart::cmc_gauge::paint(ui, painter, rect, args.symbol, &reading);
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Panel, true, &description)
                });
            }
            return true;
        }
        false
    }

    /// How far the overlay scripts in `outputs` reach for `viewport` over `bars` bars: overlay
    /// scripts may draw past the last bar (a stack to the right of the chart) and above or below
    /// the candles; the line chart draws none of that.
    pub(super) fn extent_of(
        &self,
        viewport: &ChartViewport,
        outputs: &[frame::Placed],
        line: bool,
        bars: usize,
    ) -> OverlayExtent {
        if line {
            return OverlayExtent::default();
        }
        let shown_count = chart::canvas::visible_window(
            bars,
            viewport.horizontal.visible,
            viewport.horizontal.right_offset,
            usize::from(self.appearance.canvas.margins.right),
        )
        .count();
        overlay_extent(
            outputs
                .iter()
                .filter(|(output, _, _)| output.overlay)
                .map(|(output, _, _)| &**output),
            shown_count,
        )
    }

    pub(super) fn refresh_script_extent(
        &mut self,
        outputs: &[frame::Placed],
        line: bool,
        bars: usize,
    ) {
        self.script_extent = self.extent_of(&self.viewport, outputs, line, bars);
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
        replay.picked_range = None;
        let view = self.view(
            self.framing(),
            bars,
            (plot, layout.top_inset),
            data.scale_mode,
            data.chart_type,
        );
        let regions = navigation::Regions {
            plot,
            main: layout.rect,
            chart: layout.full_rect,
        };
        // A long touch is egui's right-click. Only a press the values tooltip has taken (the
        // layer runs before `handle_hold`, so on the release frame it still has) swallows it: a
        // press on a drawing, off the plot, with the option off or under a cursor that uses the
        // press itself keeps its menu.
        drawing.long_press_values = self.values_hold_claims_drag();
        // The Zoom In tool and Select portion own the pointer while they are on: nothing is drawn
        // or picked.
        let held = self.pointer_is_held(layout.main_visible, replay.selecting);
        let zooming = self.zoom_is_live(layout.main_visible, replay.selecting);
        if layout.main_visible
            && let Some(view) = &view
        {
            if replay.selecting || held {
                self.layer.clear_hover();
            } else {
                self.layer.update(
                    ui,
                    response,
                    &view.projection(
                        plot,
                        bars,
                        self.appearance.price_format(),
                        self.bar_calendar(),
                        ui.ctx(),
                    ),
                    drawing,
                );
            }
        }
        drawing.escape_consumed |=
            self.handle_zoom(ui, response, view.as_ref(), &regions, bars.len(), zooming);
        // A range being picked (replay start, Pine Logs) has the drag to itself: no stroke over the
        // bars it selects, and one begun before the pick started is let go.
        let marks_allowed = layout.main_visible && !held && !replay.selecting;
        if !marks_allowed {
            let now = ui.input(|input| input.time);
            self.demo.end(now);
        }
        if marks_allowed && let Some(view) = &view {
            self.handle_demonstration(
                ui,
                response,
                &view.projection(
                    plot,
                    bars,
                    self.appearance.price_format(),
                    self.bar_calendar(),
                    ui.ctx(),
                ),
                plot,
            );
        }
        // Not while a drawing tool is armed: its press places anchors and drags out shapes, which
        // the drawing layer (run above, before this) has already begun by the time a hold shows.
        if layout.main_visible && !held && drawing.tool == crate::model::Tool::Cursor {
            let blocked =
                replay.selecting || self.layer.claims_drag() || self.demonstration_claims_drag();
            self.handle_hold(ui, response, plot, blocked);
        } else {
            self.hold = Default::default();
        }
        let navigation = navigation::Navigation {
            regions,
            shown: view
                .as_ref()
                .map(|view| PriceRange::new(view.scale.minimum, view.scale.maximum)),
            total: bars.len(),
            slot_width: self.slot_width(self.framing(), plot),
            reserved: self.reserved_slots_with(self.framing().extent),
            scale_height: view
                .as_ref()
                .map_or_else(|| plot.height(), |view| view.scale.height),
            logarithmic: view.as_ref().is_some_and(|view| view.scale.logarithmic),
            // Dragging over bars for the logs pane is not a pan.
            drawing_claims: self.layer.claims_drag()
                || self.demonstration_claims_drag()
                || self.values_hold_claims_drag()
                || self.zoom_claims_drag()
                || self.snapshot_select
                || (replay.selecting && self.pick_kind == overlay::PickKind::LogsRange),
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
            alert_toggled,
            alert_deleted,
            paper,
            live,
            navigation,
            scale_mode,
        } = results;
        for (id, action) in pane_actions {
            self.apply_pane_action(id, action);
        }
        if alert_menu.is_some() {
            self.alert_menu = alert_menu;
        }
        self.alert_moves.extend(alert_moves);
        for id in alert_toggled {
            if !self.alert_open.remove(&id) {
                self.alert_open.insert(id);
            }
        }
        self.alert_deletes.extend(alert_deleted);
        self.live.actions.extend(live);
        self.paper.actions.extend(paper.0);
        self.paper.menu = paper.1.or(self.paper.menu.take());
        if let Some(action) = navigation {
            self.apply_navigation(action, bars);
        }
        if scale_mode.is_some() {
            self.scale_request = scale_mode;
        }
    }
}

/// Where the pointer is on the plot, for the workspace's crosshair and the other charts'. `time`
/// is the chart's time scale, given only when its slots are time (see
/// [`ChartCard::takes_part_in_sync`]): the other charts find the pointer's column by it.
pub(super) fn report_hover(
    pointer: &mut ChartPointer,
    response: &Response,
    plot: Rect,
    view: &View,
    projection: &Projection,
    time: Option<TimeAxis>,
) {
    if let Some(position) = response.hover_pos().filter(|point| plot.contains(*point)) {
        pointer.hover_x_fraction = Some(((position.x - plot.left()) / plot.width()) as f64);
        pointer.hover_time = time.and_then(|axis| {
            let slots = view.geometry(plot, axis.len()).slots();
            axis.time_at(slots.at(position.x))
        });
        pointer.hover_price = Some(view.scale.price(position.y));
        pointer.hover_anchor = Some(projection.anchor(position));
    }
}

impl ChartCard {
    /// Plot body. Reports the pointer (click + hover) back to the workspace.
    #[allow(clippy::too_many_arguments)]
    /// The chart; when a refresh failed but there is data to show, the last result stays up with
    /// a small warning along the bottom.
    pub fn body(
        &mut self,
        ui: &mut egui::Ui,
        symbol: &str,
        candles: &[Kline],
        loading: bool,
        error: Option<&ProviderError>,
        scale_mode: PriceScaleMode,
        source: DataSource,
        indicators: &IndicatorConfig,
        crosshair: Option<&CrosshairInfo>,
        chart_id: u64,
        drawing: &mut DrawingFrame,
        replay: &mut ReplayChartFrame,
    ) -> ChartPointer {
        self.sync_baseline = None;
        let args = BodyArgs {
            symbol,
            candles,
            error,
            scale_mode,
            source,
            indicators,
            crosshair,
            chart_id,
        };
        let _ = loading;
        let pointer = self.body_inner(ui, &args, drawing, replay);
        self.end_sync_frame(source, candles);
        if let (Some(error), false, Some(rect)) = (error, candles.is_empty(), pointer.rect) {
            refresh_warning(ui, rect, error, source);
        }
        pointer
    }

    fn body_inner(
        &mut self,
        ui: &mut egui::Ui,
        args: &BodyArgs,
        drawing: &mut DrawingFrame,
        replay: &mut ReplayChartFrame,
    ) -> ChartPointer {
        let BodyArgs {
            symbol,
            candles,
            scale_mode,
            source,
            indicators,
            crosshair,
            chart_id,
            ..
        } = *args;
        // An open scale menu owns the keyboard: Escape closes it and nothing else, and no draft
        // takes the Enter or Delete meant for one of its rows.
        if scale_menu::is_open(ui.ctx(), chart_id) {
            drawing.keys_blocked = true;
            scale_menu::close_on_escape(ui.ctx(), chart_id);
        }
        let line = source.is_line_chart();
        let mut pointer = ChartPointer::default();
        let available = ui.available_size();
        let size = Vec2::new(available.x.max(1.0), available.y.max(50.0));
        let (response, painter) = ui.allocate_painter(size, Sense::click_and_drag());
        let response = crate::ui::accessibility::label_as(
            response,
            egui::WidgetType::Panel,
            tr!(
                "{title} {source} chart",
                title = self.display_title(symbol, source),
                source = source.display_name()
            ),
        );
        let rect = response.rect;
        pointer.rect = Some(rect);
        self.body_rect = rect;
        self.axis_width = 0.0;
        let style = CanvasStyle::resolve(&self.appearance.canvas);
        let full_rect = rect;
        canvas::paint_background(&painter, rect, style.fill);

        if self.paint_placeholder(ui, &response, &painter, args) {
            return pointer;
        }

        // A script that isn't an overlay (RSI, MACD …) gets a pane under the candles, of the height
        // the user dragged it to or its part of 30% of the chart; the candles keep the rest and the
        // time axis. On a line chart every script draws over the line instead.
        // A price-based chart type draws bars it constructed from the candles, and the scripts
        // and indicators on it read those same bars; every other type draws the candles as they
        // are. Either way `bars` is what the chart is, and `candles` stays the market.
        let chart_type = if line {
            ChartType::Candles
        } else {
            self.appearance.style.effective_type(source)
        };
        self.drawn_type = chart_type;
        self.grid = source.grid();
        let constructed = self.constructed_series(candles, chart_type);
        // Editing a price-based type's construction (box size, reversal) rebuilds its bars, which
        // is a different series for a stored view as much as another type is.
        let drawn = constructed.as_ref().map(|built| built.construction);
        if drawn != self.drawn_series {
            self.drawn_series = drawn;
            self.new_series();
        }
        let bars: &[Kline] = constructed
            .as_ref()
            .map_or(candles, |built| built.klines.as_slice());
        self.keep_view_on_its_bars(bars);
        self.begin_sync_frame();
        let symbol_info = body::symbol_info(symbol, source, chart_type);
        let source_bar_seconds =
            body::source_bar_seconds(candles, self.timeframe, constructed.is_some());
        let placed = self.pine_outputs(ui.ctx(), &symbol_info, bars, source_bar_seconds);
        let outputs = frame::split_forced_overlays(placed, line);
        self.feed_table(
            bars,
            chart_type,
            line,
            source.provides_volume(),
            indicators,
            &outputs,
        );
        self.refresh_script_extent(&outputs, line, bars.len());
        let data = frame::FrameData {
            symbol,
            source,
            line,
            chart_type,
            scale_mode,
            candles,
            bars,
            constructed: constructed.as_deref(),
            outputs: &outputs,
            indicators,
            countdown: self.countdown,
        };
        let layout = self.frame_layout(
            &data,
            self.framing(),
            full_rect,
            self.top_inset,
            &painter,
            &style,
        );
        self.keep_panes_in_step(&layout);
        self.handle_pane_dividers(ui, chart_id, &layout, replay.selecting);
        let plot = layout.plot;
        self.axis_width = full_rect.max.x - plot.max.x;
        let pane_rects = &layout.pane_rects;
        let chart_hovered = ui.rect_contains_pointer(full_rect);
        let regions = self.handle_frame_input(ui, &response, &data, &layout, drawing, replay);

        let Some(view) = self.view(
            self.framing(),
            bars,
            (plot, layout.top_inset),
            scale_mode,
            chart_type,
        ) else {
            return pointer;
        };
        self.near_oldest = view.start <= self.viewport.horizontal.visible * NEAR_OLDEST_SCREENS;
        #[cfg(test)]
        {
            self.probe = Some(Probe {
                plot,
                start: view.start,
                lead: view.lead,
                slot_width: view.slot_width,
                minimum: view.scale.minimum,
                maximum: view.scale.maximum,
            });
        }
        let format = self.appearance.price_format();
        let projection = view.projection(plot, bars, format, self.bar_calendar(), ui.ctx());
        pointer.context = self.context_request(
            &response,
            &regions,
            pane_rects,
            &projection,
            &view.scale,
            drawing,
        );
        let mut layer = std::mem::take(&mut self.layer);
        let mut overlays = LiveOverlays {
            ui,
            response: &response,
            card: self,
            replay,
            chart_id,
            chart_hovered,
            pane_actions: Vec::new(),
            alert_menu: None,
            alert_moves: Vec::new(),
            alert_toggled: Vec::new(),
            alert_deleted: Vec::new(),
            paper: (Vec::new(), None),
            live: Vec::new(),
            navigation: None,
            scale_mode: None,
            overlay_action: None,
            on_overlay_card: false,
        };
        self.paint_frame(
            &painter,
            &data,
            &layout,
            &view,
            &mut layer,
            drawing,
            &frame::Decor {
                chart_id,
                replay_active: overlays.replay.active,
                time_lock: true,
                crosshair,
            },
            &mut overlays,
        );
        let LiveOverlays {
            pane_actions,
            alert_menu,
            alert_moves,
            alert_toggled,
            alert_deleted,
            paper,
            live,
            navigation,
            scale_mode,
            overlay_action,
            on_overlay_card,
            ..
        } = overlays;
        pointer.overlay_action = overlay_action;
        pointer.on_overlay_card = on_overlay_card;
        self.layer = layer;
        if layout.main_visible && self.cursor.caps().highlights {
            let now = ui.input(|input| input.time);
            self.paint_demonstration(&painter, plot, &projection, now);
        }
        self.apply_overlay_results(
            OverlayResults {
                pane_actions,
                alert_menu,
                alert_moves,
                alert_toggled,
                alert_deleted,
                paper,
                live,
                navigation,
                scale_mode,
            },
            bars.len(),
        );
        let time = self.takes_part_in_sync(source).then(|| TimeAxis::new(bars));
        body::report_hover(&mut pointer, &response, plot, &view, &projection, time);

        pointer.clicked = response.clicked();
        pointer
    }

    /// "Click a candle" mode: marker at the candle under the pointer, click picks it.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn handle_start_pick(
        &self,
        ui: &egui::Ui,
        response: &Response,
        painter: &egui::Painter,
        plot: Rect,
        time_of: &dyn Fn(usize) -> i64,
        view: &View,
        replay: &mut ReplayChartFrame,
    ) {
        if self.pick_kind == overlay::PickKind::LogsRange {
            self.handle_range_pick(ui, response, painter, plot, time_of, view, replay);
            return;
        }
        let Some(position) = response.hover_pos().filter(|point| plot.contains(*point)) else {
            return;
        };
        let Some(index) = overlay::candle_at(
            position.x,
            plot.left(),
            view.slot_width,
            view.lead,
            view.start,
            view.end,
        ) else {
            return;
        };
        let open_time = time_of(index);
        let x = plot.left()
            + view.slot_width * ((index - view.start) as f32 + 0.5)
            + view.slot_width * view.lead;
        overlay::paint_selection(
            painter,
            self.below_header(plot),
            x,
            open_time,
            self.pick_kind,
        );
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        if response.clicked() {
            replay.picked_ms = Some(open_time);
        }
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
        assert_eq!(
            source_bar_seconds(&[candle(0, 3_599_999)], None, false),
            None
        );
    }

    #[test]
    fn constructed_bars_without_a_timeframe_read_the_length_of_the_last_candle() {
        // An hour: open to close inclusive of the last millisecond.
        assert_eq!(
            source_bar_seconds(&[candle(0, 3_599_999)], None, true),
            Some(3600.0)
        );
        // The last candle decides, and the length is rounded to whole seconds.
        let candles = [candle(0, 59_999), candle(60_000, 60_000 + 899_999)];
        assert_eq!(source_bar_seconds(&candles, None, true), Some(900.0));
        assert_eq!(
            source_bar_seconds(&[candle(0, 1_499)], None, true),
            Some(2.0)
        );
    }

    #[test]
    fn constructed_bars_read_the_charts_timeframe_not_a_candle_cut_short_by_midnight() {
        // A 59 s chart's last candle of the day is 24 s long.
        let candles = [candle(0, 58_999), candle(86_376_000, 86_376_000 + 23_999)];
        let timeframe = Timeframe::new(59, crate::model::TimeUnit::Second).unwrap();
        assert_eq!(
            source_bar_seconds(&candles, Some(timeframe), true),
            Some(59.0)
        );
        assert_eq!(source_bar_seconds(&candles, Some(timeframe), false), None);
    }

    #[test]
    fn no_candles_or_an_empty_span_give_no_source_bar_length() {
        assert_eq!(source_bar_seconds(&[], None, true), None);
        assert_eq!(source_bar_seconds(&[candle(1_000, 500)], None, true), None);
    }

    #[test]
    fn a_stock_chart_hands_scripts_the_session_grid_and_every_other_market_the_utc_one() {
        use crate::model::market::bucket::Grid;
        let grid = |source| symbol_info("X", source, ChartType::Candles).grid;
        assert_eq!(grid(DataSource::Alpaca), Grid::UsEquities);
        for source in [
            DataSource::Binance,
            DataSource::Coinbase,
            DataSource::Okx,
            DataSource::DexScreener,
        ] {
            assert_eq!(grid(source), Grid::Utc, "{source:?}");
        }
    }
}

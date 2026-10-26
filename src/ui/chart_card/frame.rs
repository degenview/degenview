//! One frame of the plot, drawn the same way for the live chart and for a snapshot.
//!
//! [`ChartCard::frame_layout`] fixes where everything goes (panes, axis, plot) and
//! [`ChartCard::paint_frame`] paints it, bottom to top. Neither touches input or the card's own
//! state, so a snapshot runs them on a throwaway painter. What only the live chart does (the
//! drawing tools' selection toolbar, pane buttons, interactive alert tags and paper markers, the
//! navigation pill, a replay pick) comes in through [`Overlays`] at the exact points the live
//! chart draws it, so the live layering is unchanged.

use super::*;
use crate::model::canvas::PaneHeights;

/// A script's output, how it is ordered against the chart, and the instance it belongs to.
pub(super) type Placed = (Arc<VisualOutput>, VisualOrder, u64);

/// A script with its own pane may send some of its visuals to the main chart (`force_overlay`):
/// they become an overlay output of their own, in the script's place.
pub(super) fn split_forced_overlays(placed: Vec<Placed>, line: bool) -> Vec<Placed> {
    placed
        .into_iter()
        .flat_map(|(output, order, id)| {
            if !line && !output.overlay && output.has_forced_overlay() {
                let (pane, forced) = output.split_forced_overlay();
                vec![(Arc::new(pane), order, id), (Arc::new(forced), order, id)]
            } else {
                vec![(output, order, id)]
            }
        })
        .collect()
}

/// What one frame is drawn from.
pub(super) struct FrameData<'a> {
    pub symbol: &'a str,
    pub source: DataSource,
    pub line: bool,
    pub chart_type: ChartType,
    pub scale_mode: PriceScaleMode,
    /// The market's candles (replay-cut).
    pub candles: &'a [Kline],
    /// What the chart draws: the constructed bars of a price-based type, else the candles.
    pub bars: &'a [Kline],
    pub constructed: Option<&'a ConstructedSeries>,
    pub outputs: &'a [Placed],
    pub indicators: &'a IndicatorConfig,
}

impl FrameData<'_> {
    pub(super) fn synthetic(&self) -> Option<&[SyntheticBar]> {
        self.constructed.map(|built| built.series.bars.as_slice())
    }
}

/// Where a frame's parts go.
pub(super) struct FrameLayout<'a> {
    pub style: CanvasStyle,
    /// The whole card.
    pub full_rect: Rect,
    /// The card above its script panes.
    pub rect: Rect,
    pub plot: Rect,
    pub heights: PaneHeights,
    pub pane_specs: Vec<PaneSpec>,
    pub pane_rects: Vec<Rect>,
    pub pane_outputs: Vec<(u64, &'a Arc<VisualOutput>)>,
    pub overlay_outputs: Vec<(&'a Arc<VisualOutput>, VisualOrder)>,
    pub bar_color_outputs: Vec<&'a Arc<VisualOutput>>,
    /// The pane that fills the chart, if it still exists.
    pub maximized: Option<u64>,
    /// Whether the price chart has room to be drawn; if not only its time axis stays.
    pub main_visible: bool,
}

/// What only some frames show.
pub(super) struct Decor<'a> {
    pub chart_id: u64,
    /// The watermark names the replay.
    pub replay_active: bool,
    pub time_lock: bool,
    pub crosshair: Option<&'a CrosshairInfo>,
}

/// One script pane, as its overlay hook sees it.
pub(super) struct PaneSlot<'a> {
    pub index: usize,
    pub count: usize,
    pub id: u64,
    pub rect: Rect,
    pub collapsed: bool,
    pub maximized: bool,
    pub name: &'a str,
}

/// What the live chart adds to a frame, in the places it adds it.
pub(super) trait Overlays {
    /// After the drawings: the selection toolbar and the drawing menu.
    fn after_drawings(
        &mut self,
        _layer: &mut DrawingLayer,
        _projection: &Projection,
        _drawing: &mut DrawingFrame,
        _layout: &FrameLayout,
    ) {
    }

    /// After the script tables: a replay pick and the replay badge.
    fn after_tables(
        &mut self,
        _main_painter: &egui::Painter,
        _data: &FrameData,
        _layout: &FrameLayout,
        _view: &View,
    ) {
    }

    /// For each script pane, once its contents and axis edge are painted.
    fn pane(&mut self, _painter: &egui::Painter, _layout: &FrameLayout, _slot: &PaneSlot) {}

    /// Last, over the whole plot: alert lines, trading markers, the navigation pill.
    fn finish(&mut self, _data: &FrameData, _layout: &FrameLayout, _view: &View) {}
}

/// The title a script pane carries when its buttons are wanted: the buttons' own cue. A snapshot
/// shows it as the idle chart does.
pub(super) fn paint_pane_title(
    painter: &egui::Painter,
    layout: &FrameLayout,
    slot: &PaneSlot,
    hovered: bool,
) {
    let buttons_wanted =
        layout.style.pane_buttons.wanted(hovered) || slot.collapsed || slot.maximized;
    if buttons_wanted && chart_buttons::pane_fits(slot.rect, layout.plot.right()) {
        painter.text(
            Pos2::new(slot.rect.left() + 8.0, slot.rect.top() + 6.0),
            Align2::LEFT_TOP,
            truncated(slot.name, 36),
            FontId::proportional(11.0),
            layout.style.text_color,
        );
    }
}

impl ChartCard {
    /// Splits `rect` into the price chart, the script panes under it and the axes.
    pub(super) fn frame_layout<'a>(
        &self,
        data: &FrameData<'a>,
        rect: Rect,
        painter: &egui::Painter,
        style: &CanvasStyle,
    ) -> FrameLayout<'a> {
        let (line, bars, candles, scale_mode, chart_type) = (
            data.line,
            data.bars,
            data.candles,
            data.scale_mode,
            data.chart_type,
        );
        let full_rect = rect;
        let pane_outputs: Vec<(u64, &Arc<VisualOutput>)> = data
            .outputs
            .iter()
            .filter(|(output, _, _)| !output.overlay && !line)
            .map(|(output, _, id)| (*id, output))
            .collect();
        let overlay_outputs: Vec<(&Arc<VisualOutput>, VisualOrder)> = data
            .outputs
            .iter()
            .filter(|(output, _, _)| output.overlay || line)
            .map(|(output, order, _)| (output, *order))
            .collect();
        // `barcolor()` colours the main chart's bars from wherever the script runs, in its own
        // pane too; the first script (in the order they were applied) that colours a bar wins.
        let bar_color_outputs: Vec<&Arc<VisualOutput>> = if line {
            Vec::new()
        } else {
            data.outputs.iter().map(|(output, _, _)| output).collect()
        };
        let pane_specs: Vec<PaneSpec> = pane_outputs
            .iter()
            .map(|(id, _)| PaneSpec {
                id: *id,
                collapsed: self
                    .instances
                    .iter()
                    .any(|instance| instance.id == *id && instance.pane_collapsed),
            })
            .collect();
        let maximized = self
            .maximized_pane
            .filter(|id| pane_specs.iter().any(|pane| pane.id == *id));
        let heights = canvas::split_panes(
            rect.height(),
            &pane_specs,
            maximized,
            style.time_axis_height(),
        );
        let pane_total: f32 = heights.panes.iter().sum();
        let pane_area_top = rect.max.y - pane_total;
        let rect = Rect::from_min_max(rect.min, Pos2::new(rect.max.x, pane_area_top));
        let mut pane_top = pane_area_top;
        let pane_rects: Vec<Rect> = heights
            .panes
            .iter()
            .map(|height| {
                let pane = Rect::from_min_max(
                    Pos2::new(rect.min.x, pane_top),
                    Pos2::new(rect.max.x, pane_top + height),
                );
                pane_top += height;
                pane
            })
            .collect();
        let time_axis_height = style.time_axis_height();
        let plot_bottom = (rect.max.y - time_axis_height).max(rect.min.y);
        // The axis is as wide as the widest label it holds. The scale does not depend on the
        // plot's width, so a plot of the final height and any width gives the labels' range.
        let axis_width = {
            let provisional = Rect::from_min_max(rect.min, Pos2::new(rect.max.x, plot_bottom));
            let font = style.price_font();
            let measure = |text: &str| {
                price_text::PriceText::new(painter, text, font.clone(), style.text_color)
                    .size()
                    .x
            };
            let mut widest = 0.0f32;
            if let Some(view) = self.view(bars, provisional, scale_mode, chart_type) {
                let scale = &view.scale;
                let budget = canvas::tick_budget(provisional.height(), style.price_tick_spacing());
                let AxisTicks { prices, step } = scale.axis_ticks(budget);
                let format = self.appearance.price_format();
                for price in &prices {
                    widest = widest.max(measure(&scale.tick_label(format, *price, step)));
                }
                // The tags: the range's ends bound the crosshair's price, and the last closes
                // sit in the range or off it, but are written in full wherever they are.
                let tagged = [scale.minimum, scale.maximum]
                    .into_iter()
                    .chain(bars.last().map(|bar| bar.close))
                    .chain(candles.last().map(|bar| bar.close));
                for price in tagged {
                    widest = widest.max(measure(&scale.tag_label(format, price)));
                }
                let shown = view.end - view.start;
                for (pane_rect, (_, pine)) in pane_rects.iter().zip(&pane_outputs) {
                    let height = (pane_rect.height() - 16.0).max(0.0);
                    for (_, label) in pane_labels(pine, shown, height, style) {
                        widest = widest.max(measure(&label));
                    }
                }
            }
            canvas::axis_width(
                widest,
                style.min_axis_width(PRICE_AXIS_WIDTH),
                148.0 * (style.price_size / 10.0).max(1.0),
            )
        };
        let plot = Rect::from_min_max(rect.min, Pos2::new(rect.max.x - axis_width, plot_bottom));
        // A pane maximized over the chart, or a card too short to plot in, leaves the price
        // chart nothing to draw: only its time axis stays.
        let main_visible = plot.height() >= MIN_PLOT_HEIGHT;
        FrameLayout {
            style: *style,
            full_rect,
            rect,
            plot,
            heights,
            pane_specs,
            pane_rects,
            pane_outputs,
            overlay_outputs,
            bar_color_outputs,
            maximized,
            main_visible,
        }
    }

    /// Paints the frame: grid, series, indicators, scripts, drawings, axes and panes.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn paint_frame(
        &self,
        painter: &egui::Painter,
        data: &FrameData,
        layout: &FrameLayout,
        view: &View,
        layer: &mut DrawingLayer,
        drawing: &mut DrawingFrame,
        decor: &Decor,
        overlays: &mut dyn Overlays,
    ) {
        let (line, bars, candles, chart_type) =
            (data.line, data.bars, data.candles, data.chart_type);
        let (style, plot, rect, full_rect) =
            (&layout.style, layout.plot, layout.rect, layout.full_rect);
        let main_visible = layout.main_visible;
        let main_painter = if main_visible {
            painter.clone()
        } else {
            painter.with_clip_rect(Rect::NOTHING)
        };
        let synthetic_bars = data.synthetic();
        let shown = &bars[view.start..view.end];
        let shown_synthetic = synthetic_bars.map(|all| &all[view.start..view.end]);
        let (scale, slot_width, lead) = (&view.scale, view.slot_width, view.lead);
        let x_for =
            |index: usize| plot.left() + slot_width * (index as f32 + 0.5) + slot_width * lead;
        let body_half_width = candle_body_half_width(slot_width);
        let x_for_index =
            |index: i64| plot.left() + slot_width * (index as f32 + 0.5) + slot_width * lead;
        let y_for = |price: f64| scale.y(price);
        let candle_style = CandleStyle {
            body_width: body_half_width * 2.0,
            wick_width: 1.0,
            min_body_height: 1.0,
        };
        // Bars lying after the last one shown: the scripts' series are indexed from the end.
        let tail = bars.len() - view.end;
        let pine_layers: Vec<(PineLayer, bool)> = layout
            .overlay_outputs
            .iter()
            .map(|(pine, order)| {
                (
                    PineLayer {
                        pine,
                        candles: shown,
                        tail,
                        in_pane: false,
                        plot,
                        slot_width,
                        x_for_index: &x_for_index,
                        y_for: &y_for,
                        candle_style,
                    },
                    order.behind(pine.behind_chart),
                )
            })
            .collect();

        let ticks = canvas::time_ticks(view.start, shown.len(), slot_width);
        let tick_xs: Vec<f32> = ticks.iter().map(|index| x_for(*index)).collect();
        self.draw_grid(&main_painter, plot, rect, scale, style, &tick_xs);
        canvas_render::paint_scale_lines(
            painter,
            plot,
            rect.right(),
            full_rect.bottom(),
            style.scale_line,
        );
        if self
            .appearance
            .canvas
            .watermark
            .mode
            .shows(decor.replay_active)
        {
            let title = self.display_title(data.symbol, data.source);
            let mut subtitle = self
                .timeframe
                .map_or_else(String::new, |timeframe| format!("{} · ", timeframe.label()));
            subtitle.push_str(data.source.display_name());
            if decor.replay_active {
                subtitle.push_str(" · Replay");
            }
            canvas_render::paint_watermark(&main_painter, plot, &title, &subtitle, style.watermark);
        }
        // A bar the pan cuts at the plot's edge, or a price panned off the plot, stops there: it
        // must not run over the axes.
        let clipped = main_painter.with_clip_rect(plot);
        for (pine_layer, _) in &pine_layers {
            pine_layer.draw_background(&clipped);
        }
        // Visuals the script (or the user) put behind the chart come first, so the main series
        // draws over them.
        for (pine_layer, behind) in &pine_layers {
            if *behind {
                pine_layer.draw_behind(&clipped);
            }
        }
        if line && self.series.is_empty() {
            self.draw_line(&clipped, shown, &x_for, scale);
        } else if line {
            self.draw_series(&clipped, plot, shown, &x_for, scale);
        } else {
            // The first script that recolours a bar wins it, in the order the scripts were
            // applied.
            let recolour = |index: usize| {
                layout.bar_color_outputs.iter().find_map(|pine| {
                    pine_chart_render::bar_color_at(pine, shown.len(), tail, index)
                })
            };
            let bar_color: Option<&dyn Fn(usize) -> Option<Color32>> =
                (!layout.bar_color_outputs.is_empty()).then_some(&recolour);
            series_render::paint(&SeriesFrame {
                painter: &clipped,
                plot,
                bars: shown,
                synthetic: shown_synthetic,
                box_size: data.constructed.and_then(|built| built.series.size),
                previous_close: view.start.checked_sub(1).map(|index| bars[index].close),
                slot_width,
                x_for: &x_for,
                scale,
                palette: Palette::new(self.appearance.bullish, self.appearance.bearish),
                style: &self.appearance.style,
                chart_type,
                recolor: bar_color,
                price_format: self.appearance.price_format(),
            });
        }

        let drawn_indicators = data.indicators.drawn();
        let series = IndicatorSeries::compute(bars, view.start, shown.len(), &drawn_indicators);
        if !line && drawn_indicators.show_volume {
            self.draw_volume(&clipped, plot, &x_for, slot_width, shown);
        }
        if drawn_indicators.show_ema {
            self.draw_ema(&clipped, &x_for, scale, &series.ema);
        }
        if let Some(bands) = &series.bollinger {
            self.draw_bollinger(&clipped, &x_for, scale, bands);
        }
        if !series.rsi.is_empty() {
            self.draw_rsi(&clipped, plot, &x_for, &series.rsi);
        }
        if !series.trend_flips.is_empty() {
            self.draw_trend_flips(&clipped, plot, &x_for, scale, shown, &series.trend_flips);
        }
        for (pine_layer, behind) in &pine_layers {
            if *behind {
                pine_layer.draw_trades_in_front(&clipped);
            } else {
                pine_layer.draw_foreground(&clipped);
            }
        }

        let projection = view.projection(plot, bars, self.appearance.price_format(), painter.ctx());
        layer.paint(&clipped, &projection, drawing);
        overlays.after_drawings(layer, &projection, drawing, layout);
        for (pine_layer, _) in &pine_layers {
            pine_layer.draw_tables(&clipped);
        }
        overlays.after_tables(&main_painter, data, layout, view);

        self.draw_last_price(
            &main_painter,
            plot,
            rect,
            scale,
            candles,
            shown_synthetic.and_then(|_| bars.last()),
            style,
        );
        self.draw_time_axis(painter, rect, shown, &ticks, &x_for, plot.right(), style);
        // The tags of the lines at a price or a time sit on the axes, which the drawings'
        // clipped painter cannot reach.
        layer.paint_tags(&main_painter, rect, plot, &projection, style);
        for (index, (pane_rect, (id, pine))) in layout
            .pane_rects
            .iter()
            .zip(&layout.pane_outputs)
            .enumerate()
        {
            let collapsed = layout.pane_specs[index].collapsed;
            let maximized = layout.maximized == Some(*id);
            if index > 0 || !layout.heights.main_collapsed || pane_rect.top() > rect.bottom() {
                // The line between this pane and what lies above it.
                if let Some(separator) = style.separator {
                    canvas_render::horizontal(
                        painter,
                        (pane_rect.left(), pane_rect.right()),
                        pane_rect.top(),
                        separator,
                    );
                }
            }
            if !collapsed {
                draw_script_pane(
                    painter,
                    *pane_rect,
                    plot,
                    pine,
                    shown,
                    tail,
                    slot_width,
                    &x_for_index,
                    style,
                    &tick_xs,
                );
            }
            // The price axis' edge runs on through the pane.
            canvas_render::vertical(
                painter,
                plot.right(),
                (pane_rect.top(), pane_rect.bottom()),
                canvas_render::solid(style.scale_line),
            );
            if *id != PREVIEW_INSTANCE {
                let name = self
                    .instances
                    .iter()
                    .find(|instance| instance.id == *id)
                    .map_or("", |instance| instance.name.as_str());
                overlays.pane(
                    painter,
                    layout,
                    &PaneSlot {
                        index,
                        count: layout.pane_rects.len(),
                        id: *id,
                        rect: *pane_rect,
                        collapsed,
                        maximized,
                        name,
                    },
                );
            }
        }

        if decor.time_lock {
            self.draw_time_lock(
                painter,
                &projection,
                plot,
                rect,
                &layout.pane_rects,
                style,
                main_visible,
            );
        }
        if decor.time_lock {
            self.draw_logs_range(
                painter,
                &projection,
                plot,
                &layout.pane_rects,
                style,
                main_visible,
            );
        }
        if decor.time_lock {
            self.draw_bar_marker(
                painter,
                &projection,
                plot,
                rect,
                &layout.pane_rects,
                style,
                main_visible,
            );
        }
        if let Some(crosshair) = decor.crosshair {
            self.draw_crosshair(
                painter,
                plot,
                rect,
                &layout.pane_rects,
                crosshair,
                decor.chart_id,
                scale,
                shown,
                slot_width,
                lead,
                data.source.provides_volume(),
                style,
                main_visible,
                &CrosshairBars {
                    chart_type,
                    synthetic: shown_synthetic,
                    source_candles: candles,
                    pine: pine_layers
                        .iter()
                        .map(|(pine_layer, _)| pine_layer.pine)
                        .collect(),
                    shown_len: shown.len(),
                    tail,
                },
            );
        }

        if main_visible {
            overlays.finish(data, layout, view);
        }
    }
}

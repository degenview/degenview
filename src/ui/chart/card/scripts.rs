//! The Pine scripts on a chart card: their instances and settings, the outputs the host's runs
//! hand back, the pane buttons, and the own pane an `overlay=false` script draws in.

use super::*;

impl ChartCard {
    pub fn pine_instances(&self) -> &[PineInstance] {
        &self.instances
    }

    /// Replaces every instance (restoring a saved chart).
    pub fn set_pine_instances(&mut self, instances: Vec<PineInstance>) {
        self.instances = instances;
        self.pine.reset();
    }

    /// Applies a script to the chart, after the ones already there. Returns the new instance's id.
    pub fn add_pine_instance(
        &mut self,
        script_id: Option<String>,
        name: &str,
        source: &str,
    ) -> u64 {
        let id = self
            .instances
            .iter()
            .map(|i| i.id)
            .max()
            .map_or(1, |max| max + 1);
        self.instances
            .push(PineInstance::new(id, script_id, name, source));
        id
    }

    pub fn remove_pine_instance(&mut self, id: u64) {
        self.instances.retain(|instance| instance.id != id);
        self.pine.remove(id);
    }

    /// Moves an instance to `index` (clamped); draw order follows list order.
    pub fn move_pine_instance(&mut self, id: u64, index: usize) {
        if let Some(from) = self.instances.iter().position(|i| i.id == id) {
            let instance = self.instances.remove(from);
            let to = index.min(self.instances.len());
            self.instances.insert(to, instance);
        }
    }

    /// Sends an instance's visuals in front of or behind the chart's own candles; a restyle, so it
    /// never reruns the script.
    pub fn set_pine_visual_order(&mut self, id: u64, order: VisualOrder) {
        if let Some(instance) = self.instances.iter_mut().find(|i| i.id == id) {
            instance.visual_order = order;
        }
    }

    pub fn set_pine_visible(&mut self, id: u64, visible: bool) {
        if let Some(instance) = self.instances.iter_mut().find(|i| i.id == id) {
            instance.visible = visible;
        }
    }

    /// Sets an instance's style overrides; the next frame draws with them.
    pub fn set_pine_instance_style(&mut self, id: u64, overrides: HashMap<String, String>) {
        if let Some(instance) = self.instances.iter_mut().find(|i| i.id == id) {
            instance.style_overrides = overrides;
        }
    }

    /// What the user can restyle in an instance's output, as the script itself draws it. Empty
    /// until the script has run.
    pub fn pine_style_rows(&self, id: u64) -> Vec<StyleRow> {
        self.pine.style_rows(id)
    }

    /// Sets an instance's input overrides; its next run uses them.
    pub fn set_pine_instance_inputs(&mut self, id: u64, inputs: &HashMap<String, InputValue>) {
        if let Some(instance) = self.instances.iter_mut().find(|i| i.id == id) {
            instance.set_input_values(inputs);
        }
    }

    /// The messages an instance's script wrote with `log.*()` in its current run.
    pub fn pine_logs(&self, id: u64) -> Option<PineLogs<'_>> {
        self.pine.logs(id)
    }

    /// Forgets the messages kept for an instance's run. The run goes on and writes more.
    pub fn clear_pine_logs(&mut self, id: u64) {
        self.pine.clear_logs(id);
    }

    /// Whether an instance keeps the messages it writes in realtime.
    pub fn pine_logging(&self, id: u64) -> bool {
        self.pine.logging(id)
    }

    /// Turns the keeping of an instance's realtime messages on or off. Messages already kept stay.
    pub fn set_pine_logging(&mut self, id: u64, enabled: bool) {
        self.pine.set_logging(id, enabled);
    }

    pub fn pine_status(&self, id: u64) -> Option<InstanceStatus<'_>> {
        self.pine.status(id)
    }

    /// Whether the instance's latest run has been going on long enough to show a busy state. A
    /// hidden instance has no run, so it is never busy.
    pub fn pine_busy(&self, id: u64, now: Instant) -> bool {
        self.pine.busy(id, now)
    }

    // The single-script preview (Pine Studio): one instance, id 0, that keeps its last good
    // output while a newer source fails to run.

    /// Sets (or clears) the one script the preview chart runs.
    pub fn set_pine_source(&mut self, source: Option<String>) {
        match source {
            Some(source) => match self.instances.first_mut() {
                Some(instance) if instance.id == PREVIEW_INSTANCE => instance.source = source,
                _ => {
                    let mut instance =
                        PineInstance::new(PREVIEW_INSTANCE, None, "Preview", &source);
                    instance.set_input_values(&self.preview_inputs);
                    self.instances = vec![instance];
                    self.pine.stop_all();
                }
            },
            None => {
                self.instances.clear();
                self.pine.stop_all();
            }
        }
    }

    /// Sets the input overrides the preview script runs with.
    pub fn set_pine_inputs(&mut self, inputs: HashMap<String, InputValue>) {
        self.set_pine_instance_inputs(PREVIEW_INSTANCE, &inputs);
        self.preview_inputs = inputs;
    }

    /// The preview run's runtime failure, if it stopped early.
    pub fn pine_error(&self) -> Option<&Diagnostic> {
        self.pine.error(PREVIEW_INSTANCE)
    }

    /// Whether a preview run has finished cleanly, so a later failure leaves real output up.
    pub fn has_valid_pine_output(&self) -> bool {
        self.pine.has_valid_output(PREVIEW_INSTANCE)
    }

    /// The preview run's output (strategy report, alerts), for the Report tab.
    pub fn pine_visual(&self) -> Option<&VisualOutput> {
        self.pine.output(PREVIEW_INSTANCE)
    }

    /// The visible instances' outputs in order, from the runs the host keeps for them: this frame
    /// hands them the bars it draws, so a run starts again when what the script reads changed.
    pub(super) fn pine_outputs(
        &mut self,
        ctx: &egui::Context,
        symbol_info: &SymbolInfo,
        bars: &[Kline],
        bar_seconds: Option<f64>,
    ) -> Vec<frame::Placed> {
        let chart = ChartFrame {
            bars,
            symbol: symbol_info,
            bar_seconds,
            // What `chart.bg_color` and `chart.fg_color` read: the colours the chart is drawn on.
            theme: if theme::is_light() {
                ChartTheme::LIGHT
            } else {
                ChartTheme::DARK
            },
            visible_range: self.visible_open_range(bars),
        };
        self.pine.update(ctx, &chart, &self.instances)
    }

    /// Applies a pane button. Order, collapse and removal act on the script's own instance, so
    /// there is one list of what is on the chart and in what order.
    pub(super) fn apply_pane_action(&mut self, id: u64, action: PaneAction) {
        match action {
            PaneAction::MoveUp | PaneAction::MoveDown => {
                let Some(position) = self.pane_ids.iter().position(|pane| *pane == id) else {
                    return;
                };
                let neighbour = if action == PaneAction::MoveUp {
                    position.checked_sub(1)
                } else {
                    Some(position + 1)
                }
                .and_then(|index| self.pane_ids.get(index).copied());
                // Panes are drawn in the order of their scripts, so a pane trades places with its
                // neighbour by taking the neighbour's place in the instance list.
                if let Some(other) = neighbour
                    && let Some(index) = self.instances.iter().position(|i| i.id == other)
                {
                    self.move_pine_instance(id, index);
                    // The next click, before the next frame, sees the new order.
                    if let Some(at) = self.pane_ids.iter().position(|pane| *pane == other) {
                        self.pane_ids.swap(position, at);
                    }
                }
            }
            PaneAction::ToggleCollapse => {
                if let Some(instance) = self.instances.iter_mut().find(|i| i.id == id) {
                    instance.pane_collapsed = !instance.pane_collapsed;
                }
                if self.maximized_pane == Some(id) {
                    self.maximized_pane = None;
                }
            }
            PaneAction::ToggleMaximize => {
                if self.maximized_pane == Some(id) {
                    self.maximized_pane = None;
                } else {
                    // A maximized pane is expanded: that is the state it returns to.
                    if let Some(instance) = self.instances.iter_mut().find(|i| i.id == id) {
                        instance.pane_collapsed = false;
                    }
                    self.maximized_pane = Some(id);
                }
            }
            PaneAction::Remove => {
                self.remove_pine_instance(id);
                if self.maximized_pane == Some(id) {
                    self.maximized_pane = None;
                }
            }
        }
    }
}

/// The value labels a script pane writes on its axis, with the value each stands at. The painter
/// and the axis width both read this, so the axis is as wide as the labels it draws.
pub(super) fn pane_labels(
    pine: &VisualOutput,
    shown: usize,
    plot_height: f32,
    style: &CanvasStyle,
) -> Vec<(f64, String)> {
    let (low, high) = crate::ui::pine::geometry::pane_value_range(pine, shown, 0.1);
    // A pane is short: its labels sit closer than the price chart's, and it keeps at least two.
    let budget = chart::canvas::tick_budget(plot_height, style.pane_tick_spacing()).max(2);
    let ticks = chart::canvas::price_ticks(low, high, false, budget);
    let step = tick_step(&ticks);
    ticks
        .into_iter()
        .map(|value| {
            // A tick that is zero up to rounding reads "0", not eight decimals of nothing.
            let label = if value.abs() < (high - low).abs() * 1e-9 {
                "0".to_owned()
            } else {
                PriceFormat::AUTO.format_tick(value, step)
            };
            (value, label)
        })
        .collect()
}

/// The own pane under the candles for `overlay=false` scripts, ported from `PineScriptPaneView`:
/// a value axis fitted to the script's outputs, sharing the price chart's horizontal layout so
/// bar slots line up with the candles above (the time axis stays up there). The pane takes the
/// canvas' grid, scale text and separator like the price chart does, each with its own scale.
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_script_pane(
    painter: &egui::Painter,
    pane: Rect,
    price_plot: Rect,
    pine: &VisualOutput,
    shown: &[Kline],
    tail: usize,
    slot_width: f32,
    x_for_index: &dyn Fn(i64) -> f32,
    style: &CanvasStyle,
    tick_xs: &[f32],
    pointer: Option<Pos2>,
) -> Option<HoverTarget> {
    // Top and bottom leave just enough room for the outermost axis labels.
    let plot = Rect::from_min_max(
        Pos2::new(price_plot.left(), pane.top() + 8.0),
        Pos2::new(price_plot.right(), pane.bottom() - 8.0),
    );
    if plot.height() <= 1.0 {
        return None;
    }
    let (low, high) = crate::ui::pine::geometry::pane_value_range(pine, shown.len(), 0.1);
    let span = (high - low).max(f64::EPSILON);
    let y_for = |value: f64| plot.bottom() - ((value - low) / span) as f32 * plot.height();
    let labels = pane_labels(pine, shown.len(), plot.height(), style);
    if let Some(grid) = style.vertical_grid {
        for x in tick_xs {
            if *x >= plot.left() && *x <= plot.right() {
                canvas::vertical(painter, *x, (plot.top(), plot.bottom()), grid);
            }
        }
    }
    for (value, label) in labels {
        let y = y_for(value);
        if !y.is_finite() || y < plot.top() - 0.5 || y > plot.bottom() + 0.5 {
            continue;
        }
        if let Some(grid) = style.horizontal_grid {
            canvas::horizontal(painter, (plot.left(), plot.right()), y, grid);
        }
        price_text::paint(
            painter,
            Pos2::new(pane.right() - 7.0, y),
            Align2::RIGHT_CENTER,
            &label,
            style.price_font(),
            style.text_color,
        );
    }
    let layer = PineLayer {
        pine,
        candles: shown,
        tail,
        in_pane: true,
        plot,
        slot_width,
        x_for_index,
        y_for: &y_for,
        candle_style: CandleStyle {
            body_width: candle_body_half_width(slot_width) * 2.0,
            wick_width: 1.0,
            min_body_height: 1.0,
        },
    };
    let clipped = painter.with_clip_rect(plot.expand2(Vec2::new(0.0, 0.0)));
    layer.draw_background(&clipped);
    layer.draw_foreground(&clipped);
    // Tables pin to the plot's corners and are never price-anchored, so they sit outside the
    // series clip.
    layer.draw_tables(painter);
    // The tooltip of a label or cell under the pointer, measured only while it is over the pane.
    let pointer = pointer.filter(|pointer| plot.contains(*pointer) && layer.has_tooltips())?;
    chart_render::target_at(&layer.hover_targets(painter), pointer).cloned()
}

//! The chart's right-click menu, from the click to the effect.
//!
//! A chart card reports a right-click (`ChartPointer::context`); this module turns it into a
//! [`ChartContext`] snapshot, builds the menu from the chart's current state
//! (`model::chart_context`), draws it (`ui::chart_context_menu`), and runs the chosen command.
//! Every effect goes through the owners that already exist: the drawing undo stack, the paper
//! panel's ticket, the alert editor, the settings sheet, the template dialogs. Opening the menu
//! and choosing an entry never place an order, save an alert or touch another chart: the
//! snapshot is checked against the live workspace first, and a stale one is refused with a
//! message.

use super::*;
use crate::model::alert::AlertCondition;
use crate::model::chart_clipboard::DrawingClip;
use crate::model::chart_context::{
    self, ChartContext, Command, Facts, Live, OrderFacts, PaneFacts, PaperAccess, TemplateRow,
};
use crate::model::chart_template::ApplyScope;
use crate::model::object_tree::{self, IndicatorKey, Key as TreeKey};
use crate::model::paper::{PaperOrderSide, PaperOrderType};
use crate::ui::ContextRequest;
use crate::ui::chart_context_menu::{self, MenuState, Outcome};
use crate::ui::chart_table_view::{self, TableAction, TableViewState};
use crate::ui::chart_templates::{self, TemplateState};
use crate::ui::object_tree::{self as tree_view, TreeAction, TreeState};
use eframe::egui::{Context, Id, Pos2};

/// How long a message stays up.
const NOTICE_LIFETIME: Duration = Duration::from_secs(4);

pub(super) struct OpenMenu {
    context: ChartContext,
    at: Pos2,
    state: MenuState,
    serial: u64,
    /// The templates the submenu lists, read once when the menu opened.
    templates: Vec<TemplateRow>,
}

pub(super) struct Notice {
    pub(super) text: String,
    pub(super) error: bool,
    at: Instant,
}

/// Everything the chart menus keep between frames.
#[derive(Default)]
pub(super) struct ContextState {
    menu: Option<OpenMenu>,
    serial: u64,
    pub(super) notice: Option<Notice>,
    /// The chart a Table view or Object Tree shows, when open.
    pub(super) table_chart: Option<u64>,
    pub(super) tree_chart: Option<u64>,
    /// A bulk indicator removal waiting for its confirmation.
    confirm_remove_indicators: Option<u64>,
    /// The template dialogs opened from the menu, and the chart they act on.
    templates: TemplateState,
    templates_chart: Option<u64>,
    /// The price alerts the app holds, for the chart's alert lines and their menu.
    pub(super) alerts: Vec<PriceAlert>,
    /// The last time and price the pointer was over a chart's plot, where a pasted copy lands.
    hover: Option<(u64, crate::model::Anchor)>,
    table: TableViewState,
    tree: TreeState,
}

/// Where the copied drawings wait: shared by every tab and window of the app.
fn clip_id() -> Id {
    Id::new("degenview-drawing-clip")
}

fn stored_clip(ctx: &Context) -> Option<DrawingClip> {
    ctx.data(|data| data.get_temp::<DrawingClip>(clip_id()))
}

impl ChartWorkspace {
    // MARK: Opening

    /// Opens the menu for a right-click a chart reported.
    pub(super) fn open_context_menu(&mut self, chart_id: u64, request: ContextRequest) {
        let Some(chart) = self.charts.iter().find(|chart| chart.id == chart_id) else {
            return;
        };
        if !chart.is_market() {
            return;
        }
        let current = Some(chart.chart.appearance());
        let context = ChartContext {
            chart_id,
            symbol: chart.symbol.clone(),
            source: chart.source,
            time_ms: request.time_ms,
            price: request.price,
            target: request.target,
        };
        self.focus_chart(chart_id);
        self.context.serial += 1;
        let templates = if matches!(context.target, chart_context::Target::Background) {
            self.context
                .templates
                .entries(&self.store)
                .into_iter()
                .map(|entry| TemplateRow {
                    id: entry.id,
                    name: entry.name,
                    built_in: entry.built_in,
                    // Applying it would leave the chart as it is: it is the current look.
                    current: current.as_ref().is_some_and(|look| {
                        ApplyScope::AppearanceAndCanvas.merge(look, &entry.appearance) == *look
                    }),
                })
                .collect()
        } else {
            Vec::new()
        };
        self.context.menu = Some(OpenMenu {
            context,
            at: request.at,
            state: MenuState::default(),
            serial: self.context.serial,
            templates,
        });
    }

    /// Whether a chart menu is up, so Escape closes it before it disarms a tool.
    pub(super) fn context_menu_open(&self) -> bool {
        self.context.menu.is_some()
    }

    /// Remembers where the pointer is on a chart, for a keyboard paste.
    pub(super) fn note_hover(&mut self, chart_id: u64, anchor: Option<crate::model::Anchor>) {
        if let Some(anchor) = anchor {
            self.context.hover = Some((chart_id, anchor));
        }
    }

    /// Hands the workspace the price alerts the app holds.
    pub fn set_price_alerts(&mut self, alerts: &[PriceAlert]) {
        if self.context.alerts != alerts {
            self.context.alerts = alerts.to_vec();
        }
    }

    // MARK: Messages

    pub(super) fn say(&mut self, text: impl Into<String>, error: bool) {
        self.context.notice = Some(Notice {
            text: text.into(),
            error,
            at: Instant::now(),
        });
    }

    fn show_notice(&mut self, ctx: &Context) {
        if let Some((text, error)) = self.context.templates.take_notice() {
            self.say(text, error);
        }
        let Some(notice) = &self.context.notice else {
            return;
        };
        if notice.at.elapsed() > NOTICE_LIFETIME {
            self.context.notice = None;
            return;
        }
        ctx.request_repaint_after(Duration::from_millis(500));
        ui::chart_notice::show(
            ctx,
            Id::new(("chart-notice", self.id)),
            &notice.text,
            notice.error,
        );
    }

    // MARK: Facts

    pub(super) fn chart_index(&self, chart_id: u64) -> Option<usize> {
        self.charts.iter().position(|chart| chart.id == chart_id)
    }

    fn replay_cut(&self) -> Option<i64> {
        if self.replay.is_active() {
            self.replay.current_ms()
        } else {
            None
        }
    }

    /// Whether the chart can trade on paper at all. Having no account yet does not count against
    /// it: choosing an order entry creates the default account, as opening the paper panel does.
    fn paper_access(chart: &Chart) -> PaperAccess {
        if !chart.is_market() || !Self::supports_paper_trading(chart.source) {
            PaperAccess::Unsupported
        } else {
            PaperAccess::Available
        }
    }

    fn context_facts(
        &self,
        ctx: &Context,
        context: &ChartContext,
        templates: &[TemplateRow],
    ) -> Option<Facts> {
        let index = self.chart_index(context.chart_id)?;
        let replay_ms = self.replay_cut();
        let chart = &self.charts[index];
        let last_price = displayed_candles(&chart.candles, replay_ms)
            .last()
            .map(|candle| candle.close);
        let instrument = Self::paper_instrument(chart);
        let paper = Self::paper_access(chart);
        let quantity = if paper == PaperAccess::Unsupported {
            String::new()
        } else {
            crate::model::paper_ticket::TicketDraft::opening(
                instrument.clone(),
                last_price,
                PaperOrderSide::Buy,
            )
            .quantity_text
        };
        let applied = BuiltIn::ALL
            .iter()
            .filter(|indicator| indicator.applied(&chart.indicators))
            .count();
        let mut facts = Facts {
            price_format: chart.chart.appearance().price_format(),
            last_price,
            tick: instrument.tick_size,
            replay: self.replay.is_active() || self.replay.status == ReplayStatus::SelectingStart,
            alerts_supported: chart.is_market() && ui::supports_alerts(chart.source),
            paper,
            quantity,
            drawings: chart.drawings.iter().filter(|d| is_lockable(d)).count(),
            indicators: applied + chart.chart.pine_instances().len(),
            clipboard: stored_clip(ctx).is_some(),
            time_locked: chart.chart.time_lock().is_some(),
            table_open: self.context.table_chart == Some(chart.id),
            tree_open: self.context.tree_chart == Some(chart.id),
            log_scale: chart.log_scale,
            ..Facts::default()
        };
        match &context.target {
            chart_context::Target::Pane { instance } => {
                facts.pane = chart
                    .chart
                    .pine_instances()
                    .iter()
                    .find(|candidate| candidate.id == *instance)
                    .map(|instance| PaneFacts {
                        name: instance.name.clone(),
                        visible: instance.visible,
                        order: instance.visual_order,
                    });
            }
            chart_context::Target::Alert { id } => {
                facts.alert = self
                    .context
                    .alerts
                    .iter()
                    .find(|alert| alert.id == *id)
                    .map(|alert| alert.state);
            }
            chart_context::Target::Order { id } => {
                facts.order = chart
                    .chart
                    .paper
                    .orders
                    .iter()
                    .find(|order| order.id == *id)
                    .map(|order| OrderFacts {
                        cancelable: order.status.is_working(),
                    });
            }
            _ => {}
        }
        facts.templates = templates.to_vec();
        Some(facts)
    }

    fn live_state(&self, context: &ChartContext) -> Live {
        let chart = self
            .charts
            .iter()
            .find(|chart| chart.id == context.chart_id);
        Live {
            chart: chart.map(|chart| (chart.symbol.clone(), chart.source)),
            instances: chart
                .map(|chart| chart.chart.pine_instances().iter().map(|i| i.id).collect())
                .unwrap_or_default(),
            alerts: self.context.alerts.iter().map(|alert| alert.id).collect(),
            orders: chart
                .map(|chart| chart.chart.paper.orders.iter().map(|o| o.id).collect())
                .unwrap_or_default(),
            positions: chart
                .map(|chart| {
                    chart
                        .chart
                        .paper
                        .positions
                        .iter()
                        .map(|(position, _)| position.instrument.key.clone())
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    // MARK: Drawing the menu

    /// Draws the open menu and the dialogs it leads to, and runs what was chosen.
    pub(super) fn show_context_ui(
        &mut self,
        ctx: &Context,
        host: &mut PaperHost,
    ) -> Vec<WorkspaceAction> {
        let mut actions = self.apply_alert_moves();
        if let Some(mut open) = self.context.menu.take() {
            match self.context_facts(ctx, &open.context, &open.templates) {
                // The chart closed while the menu was up: it goes with it.
                None => {}
                Some(facts) => {
                    let entries = chart_context::menu(&open.context, &facts);
                    let id = Id::new(("chart-context-menu", self.id, open.serial));
                    let outcome = chart_context_menu::show(
                        ctx,
                        id,
                        open.at,
                        &entries,
                        &mut open.state,
                        ctx.content_rect(),
                    );
                    match outcome {
                        Outcome::Open => self.context.menu = Some(open),
                        Outcome::Close => {}
                        Outcome::Run(command) => {
                            actions.extend(self.run_context_command(
                                ctx,
                                host,
                                open.context,
                                command,
                            ));
                        }
                    }
                }
            }
        }
        self.confirm_indicators_removal(ctx);
        self.show_template_dialogs(ctx);
        self.poll_snapshot(ctx);
        self.show_snapshot_sheets(ctx);
        self.show_notice(ctx);
        actions
    }

    fn show_template_dialogs(&mut self, ctx: &Context) {
        if !self.context.templates.has_dialog() {
            return;
        }
        let Some(chart_id) = self.context.templates_chart else {
            return;
        };
        let store = self.store.clone();
        let Some(chart) = self.charts.iter_mut().find(|chart| chart.id == chart_id) else {
            self.context.templates_chart = None;
            return;
        };
        let _ =
            chart_templates::dialogs(ctx, &mut self.context.templates, &store, &mut chart.chart);
    }

    // MARK: Running a command

    fn run_context_command(
        &mut self,
        ctx: &Context,
        host: &mut PaperHost,
        context: ChartContext,
        command: Command,
    ) -> Vec<WorkspaceAction> {
        let mut actions = Vec::new();
        // The menu may have been open while the chart or its target changed.
        if let Err(stale) = chart_context::check(&context, &self.live_state(&context)) {
            self.say(stale.message(), true);
            return actions;
        }
        let chart_id = context.chart_id;
        let Some(index) = self.chart_index(chart_id) else {
            return actions;
        };
        match command {
            Command::CopyPrice => {
                let format = self.charts[index].chart.appearance().price_format();
                match context.price() {
                    Some(price) => {
                        let text = format.plain(price);
                        ctx.copy_text(text.clone());
                        self.say(format!("Copied {text}"), false);
                    }
                    None => self.say("There is no price at that point.", true),
                }
            }
            Command::Paste => self.paste(ctx, &context),
            Command::AddAlert => self.start_alert(&context, host),
            Command::QuickOrder(order) => {
                if let Some(price) = context.price() {
                    self.open_ticket(host, index, Some((order.side, order.kind)), price);
                }
            }
            Command::AddOrder => {
                if let Some(price) = context.price() {
                    let last = displayed_candles(&self.charts[index].candles, self.replay_cut())
                        .last()
                        .map(|candle| candle.close);
                    let tick = Self::paper_instrument(&self.charts[index]).tick_size;
                    let suggestion = last
                        .and_then(|last| chart_context::suggest_orders(price, last, tick))
                        .map(|[first, _]| (first.side, first.kind));
                    self.open_ticket(host, index, suggestion, price);
                }
            }
            Command::ToggleTimeLock => {
                let card = &mut self.charts[index].chart;
                if card.time_lock().is_some() {
                    card.set_time_lock(None);
                } else {
                    card.set_time_lock(context.time_ms);
                }
            }
            Command::ToggleTable => {
                self.context.table_chart = toggled(self.context.table_chart, chart_id);
            }
            Command::ToggleObjectTree => {
                self.context.tree_chart = toggled(self.context.tree_chart, chart_id);
            }
            Command::ApplyTemplate(id) => {
                let store = self.store.clone();
                let card = &mut self.charts[index].chart;
                match self.context.templates.apply(&store, id, card) {
                    Ok(name) => self.say(format!("Applied the template “{name}”."), false),
                    Err(message) => self.say(message, true),
                }
            }
            Command::SaveTemplate => {
                self.context.templates_chart = Some(chart_id);
                self.context.templates.open_save();
            }
            Command::ManageTemplates => {
                self.context.templates_chart = Some(chart_id);
                self.context.templates.open_manage();
            }
            Command::ResetTemplate => {
                self.context.templates_chart = Some(chart_id);
                self.context.templates.open_reset();
            }
            Command::RemoveDrawings => self.confirm_remove_drawings = Some(chart_id),
            Command::RemoveIndicators => self.context.confirm_remove_indicators = Some(chart_id),
            Command::OpenSettings => {
                self.settings_state = ChartSettingsState::default();
                self.settings_state.tab = chart_settings::Tab::Appearance;
                self.settings_chart = Some(chart_id);
            }
            Command::ResetPriceScale => self.charts[index].chart.reset_price_scale(),
            Command::ToggleLogScale => self.charts[index].log_scale ^= true,
            Command::ResetChartView => self.charts[index].chart.reset_view(),
            Command::PaneSettings(_) => {
                self.settings_state = ChartSettingsState::default();
                self.settings_state.tab = chart_settings::Tab::Scripts;
                self.settings_chart = Some(chart_id);
            }
            Command::PaneToggle(instance) => {
                let card = &mut self.charts[index].chart;
                let visible = card
                    .pine_instances()
                    .iter()
                    .find(|candidate| candidate.id == instance)
                    .is_some_and(|candidate| candidate.visible);
                card.set_pine_visible(instance, !visible);
            }
            Command::PaneRemove(instance) => {
                self.remember_indicators(chart_id);
                self.charts[index].chart.remove_pine_instance(instance);
            }
            Command::PaneOrder(instance, order) => {
                self.charts[index]
                    .chart
                    .set_pine_visual_order(instance, order);
            }
            Command::OpenScriptManager => actions.push(WorkspaceAction::OpenPineScripts),
            Command::EditAlert(id) => {
                if let Some(alert) = self.context.alerts.iter().find(|alert| alert.id == id) {
                    self.alert_editor = Some(PriceAlertEditorState::new(
                        alert.asset.clone(),
                        Some(alert.clone()),
                        Some(alert.currency),
                    ));
                }
            }
            Command::ToggleAlert(id) => {
                let next =
                    self.context.alerts.iter().find(|alert| alert.id == id).map(
                        |alert| match alert.state {
                            crate::model::alert::AlertState::Active => {
                                crate::model::alert::AlertState::Paused
                            }
                            _ => crate::model::alert::AlertState::Active,
                        },
                    );
                if let Some(state) = next {
                    actions.push(WorkspaceAction::SetAlertState { id, state });
                }
            }
            Command::DeleteAlert(id) => actions.push(WorkspaceAction::DeleteAlert(id)),
            Command::CancelOrder(id) => {
                Self::apply_paper_overlay_actions(host, vec![PaperOverlayAction::CancelOrder(id)]);
            }
            Command::ClosePosition(key) => {
                let account = self.charts[index]
                    .chart
                    .paper
                    .positions
                    .iter()
                    .find(|(position, _)| position.instrument.key == key)
                    .map(|(position, _)| position.account_id);
                if let Some(account_id) = account {
                    Self::apply_paper_overlay_actions(
                        host,
                        vec![PaperOverlayAction::ClosePosition {
                            account_id,
                            instrument_key: key,
                        }],
                    );
                }
            }
            Command::ShowPaperPanel => self.paper_panel_visible = true,
        }
        actions
    }

    /// Starts a price alert at the clicked price. The editor opens for the user to review; it
    /// saves nothing by itself.
    fn start_alert(&mut self, context: &ChartContext, _host: &PaperHost) {
        self.start_alert_at(context.chart_id, context.price());
    }

    /// Starts a price alert on `chart_id` at `price`: the click's price, or a horizontal line's.
    /// The editor opens for the user to review; it saves nothing by itself.
    pub(crate) fn start_alert_at(&mut self, chart_id: u64, price: Option<f64>) {
        let Some(index) = self.chart_index(chart_id) else {
            return;
        };
        let chart = &self.charts[index];
        if !chart.is_market() || !ui::supports_alerts(chart.source) {
            self.say("Alerts are not available for this chart.", true);
            return;
        }
        if self.replay.is_active() {
            self.say("Alerts are not available during replay.", true);
            return;
        }
        let Some(price) = price.filter(|price| price.is_finite()) else {
            self.say("There is no price at that point.", true);
            return;
        };
        let last = chart.candles.last().map(|candle| candle.close);
        self.new_alert_request = Some(AlertRequest {
            asset: Self::alert_asset(chart),
            price: Some(price),
            last,
        });
    }

    /// Opens the paper ticket for the user to review. `pending` is the side and type a price
    /// suggests; without it the ticket opens as a market order on the usual side.
    fn open_ticket(
        &mut self,
        host: &mut PaperHost,
        index: usize,
        pending: Option<(PaperOrderSide, PaperOrderType)>,
        price: f64,
    ) {
        if self.replay.is_active() {
            self.say("Paper orders are not available during replay.", true);
            return;
        }
        let chart = &self.charts[index];
        if Self::paper_access(chart) == PaperAccess::Unsupported {
            self.say("Paper trading is not available for this chart.", true);
            return;
        }
        // The first order on paper creates the default account, as opening the panel would.
        if let Err(error) = host.ensure_default_account() {
            self.say(error.message(), true);
            return;
        }
        let instrument = Self::paper_instrument(chart);
        self.paper_panel_visible = true;
        match pending {
            Some((side, kind)) => {
                self.paper_panel
                    .open_pending_ticket(host, instrument, side, kind, price);
            }
            None => {
                self.paper_panel
                    .open_ticket(host, instrument, Some(price), PaperOrderSide::Buy);
            }
        }
    }

    // MARK: Clipboard

    /// Copy and Paste from the keyboard: the platform reports them as events, not keys.
    pub(super) fn handle_clipboard(&mut self, ui: &egui::Ui) {
        let ctx = ui.ctx();
        // A selected table cell takes the clipboard keys for itself; the chart's draw this frame
        // handles them.
        let cell_selected = crate::ui::take_table_cell_selected(ctx);
        if self.add_chart.is_open() || ctx.egui_wants_keyboard_input() || cell_selected {
            return;
        }
        let (copy, paste) = ui.input(|input| {
            let mut copy = false;
            let mut paste = None;
            for event in &input.events {
                match event {
                    egui::Event::Copy => copy = true,
                    egui::Event::Paste(text) => paste = Some(text.clone()),
                    _ => {}
                }
            }
            (copy, paste)
        });
        if copy {
            self.copy_selection(ctx);
        }
        if let Some(text) = paste {
            self.paste_text(ctx, &text);
        }
    }

    pub(super) fn copy_selection(&mut self, ctx: &Context) {
        let Some((chart_id, ids)) = self.selected_drawing.clone() else {
            return;
        };
        let Some(chart) = self.charts.iter().find(|chart| chart.id == chart_id) else {
            return;
        };
        let selected = chart
            .drawings
            .iter()
            .filter(|drawing| ids.contains(&drawing.id()));
        let Some(clip) = DrawingClip::copy(&chart.symbol, selected) else {
            return;
        };
        let count = clip.count();
        ctx.copy_text(clip.encode());
        ctx.data_mut(|data| data.insert_temp(clip_id(), clip));
        self.say(
            format!(
                "Copied {count} drawing{}.",
                if count == 1 { "" } else { "s" }
            ),
            false,
        );
    }

    /// A paste from the keyboard: DegenView's own copy lands under the pointer on the chart it
    /// was last over; anything else on the clipboard is not ours and is ignored.
    fn paste_text(&mut self, ctx: &Context, text: &str) {
        use crate::model::chart_clipboard::ClipError;
        let clip = match DrawingClip::decode(text) {
            Ok(clip) => clip,
            // Not DegenView's own copy: a link to an X post becomes a card;
            // anything else on the clipboard is not ours.
            Err(ClipError::NotDrawings) => {
                if let Ok(link) = crate::model::link_card::Link::parse(text) {
                    self.place_pasted_link(link);
                }
                return;
            }
            Err(error) => {
                self.say(error.to_string(), true);
                return;
            }
        };
        ctx.data_mut(|data| data.insert_temp(clip_id(), clip.clone()));
        let Some(chart_id) = self
            .context
            .hover
            .map(|(chart, _)| chart)
            .filter(|id| self.chart_index(*id).is_some())
            .or_else(|| self.resolved_focused_id())
        else {
            return;
        };
        let at = self
            .context
            .hover
            .filter(|(chart, _)| *chart == chart_id)
            .map(|(_, anchor)| anchor);
        self.place_clip(&clip, chart_id, at);
    }

    /// Paste from the menu: the copy waiting in the app clipboard, at the clicked point.
    fn paste(&mut self, ctx: &Context, context: &ChartContext) {
        let Some(clip) = stored_clip(ctx) else {
            self.say("There are no copied drawings to paste.", true);
            return;
        };
        let at = context
            .time_ms
            .zip(context.price())
            .map(|(time_ms, price)| crate::model::Anchor { time_ms, price });
        self.place_clip(&clip, context.chart_id, at);
    }

    /// Adds a copy of `clip` to a chart as one undoable step, its first point at `at` (or where
    /// it was copied from, when there is no point to put it at).
    fn place_clip(&mut self, clip: &DrawingClip, chart_id: u64, at: Option<crate::model::Anchor>) {
        let Some(index) = self.chart_index(chart_id) else {
            return;
        };
        let chart = &self.charts[index];
        if !chart.is_market() {
            return;
        }
        let Some(origin) = clip.origin() else {
            return;
        };
        let target = at.unwrap_or(origin);
        let drawings = match clip.placed(target, chart.log_scale, chart.next_drawing_id) {
            Ok(drawings) => drawings,
            Err(error) => {
                self.say(error.to_string(), true);
                return;
            }
        };
        let ids: Vec<u64> = drawings.iter().map(Drawing::id).collect();
        let count = ids.len();
        let chart = &mut self.charts[index];
        chart.next_drawing_id += count as u64;
        let commands = drawings.into_iter().map(DrawingCommand::Add).collect();
        self.apply_drawing_commands(chart_id, commands);
        // A paste while drawings are hidden would vanish; show them again.
        self.hide_all_drawings = false;
        self.selected_drawing = Some((chart_id, ids));
        self.say(
            format!(
                "Pasted {count} drawing{}.",
                if count == 1 { "" } else { "s" }
            ),
            false,
        );
    }

    // MARK: Panels

    /// Keeps the open panels honest: each follows its chart, and only that chart builds a feed.
    pub(super) fn sync_context_panels(&mut self) {
        for panel in [&mut self.context.table_chart, &mut self.context.tree_chart] {
            if panel.is_some_and(|id| !self.charts.iter().any(|chart| chart.id == id)) {
                *panel = None;
            }
        }
        let table = self.context.table_chart;
        let replay_ms = self.replay_cut();
        let replaying =
            self.replay.is_active() || self.replay.status == ReplayStatus::SelectingStart;
        for chart in &mut self.charts {
            chart.chart.set_table_wanted(table == Some(chart.id));
            let mut lines = if chart.is_market() {
                crate::model::alert_line::lines_for(
                    &self.context.alerts,
                    &InstrumentId::new(chart.source, &chart.symbol),
                    &self.alert_fx_rates,
                )
            } else {
                Vec::new()
            };
            if !lines.is_empty() {
                // A level moved while the replay rewinds the market would be judged against a
                // price that is not the market's.
                for line in &mut lines {
                    line.editable &= !replaying;
                }
                let last = displayed_candles(&chart.candles, replay_ms)
                    .last()
                    .map(|candle| candle.close);
                chart
                    .chart
                    .set_alert_market(last, Self::paper_instrument(chart).tick_size);
            }
            chart.chart.set_alert_lines(lines);
        }
    }

    /// Turns the alert tags dropped on a new level since the last frame into saves.
    fn apply_alert_moves(&mut self) -> Vec<WorkspaceAction> {
        let mut actions = Vec::new();
        for index in 0..self.charts.len() {
            for moved in self.charts[index].chart.take_alert_moves() {
                actions.extend(self.move_alert(index, moved));
            }
        }
        actions
    }

    /// The save for alert `moved.id` dragged to `moved.price` on chart `index`. The level is the
    /// drop's, in the alert's currency; the side is the one the level is on from the market's
    /// last price, as for "Add alert at this price". State, frequency, note and webhooks stay.
    fn move_alert(&mut self, index: usize, moved: ui::AlertMove) -> Option<WorkspaceAction> {
        if self.replay.is_active() || self.replay.status == ReplayStatus::SelectingStart {
            self.say("Alerts are not available during replay.", true);
            return None;
        }
        let chart = &self.charts[index];
        let market = InstrumentId::new(chart.source, &chart.symbol);
        let last = displayed_candles(&chart.candles, self.replay_cut())
            .last()
            .map(|candle| candle.close);
        let alert = self
            .context
            .alerts
            .iter()
            .find(|alert| alert.id == moved.id && alert.asset.instrument == market)?;
        let rate = if alert.currency == AlertCurrency::Usd {
            Some(1.0)
        } else {
            self.alert_fx_rates
                .get(&alert.currency)
                .copied()
                .filter(|rate| *rate > 0.0)
        };
        let Some(rate) = rate else {
            self.say(
                "The exchange rate for this alert is not available yet.",
                true,
            );
            return None;
        };
        let above = crate::model::alert_line::above_after_drag(
            alert.condition.crosses_above(),
            moved.price,
            last,
        );
        // The rate leaves float noise (`90.00000000000001`) the alert would show.
        let target: f64 = format!("{:.9e}", moved.price * rate)
            .parse()
            .unwrap_or(moved.price * rate);
        let condition = match alert.condition {
            AlertCondition::CrossesAbove { .. } | AlertCondition::CrossesBelow { .. } => {
                if above {
                    AlertCondition::CrossesAbove { target }
                } else {
                    AlertCondition::CrossesBelow { target }
                }
            }
            // A percentage alert has no level of its own to move.
            _ => return None,
        };
        if condition == alert.condition {
            return None;
        }
        let mut alert = alert.clone();
        alert.condition = condition;
        Some(WorkspaceAction::MoveAlert {
            alert: Box::new(alert),
            baseline: last.map(|last| last * rate),
        })
    }

    pub(super) fn show_table_panel(&mut self, ui: &mut egui::Ui) {
        let Some(chart_id) = self.context.table_chart else {
            return;
        };
        let Some(index) = self.chart_index(chart_id) else {
            return;
        };
        let chart = &self.charts[index];
        let title = format!("{} {}", chart.symbol, self.timeframe.label());
        let price = chart.chart.appearance().price_format();
        let feed = chart.chart.table_feed();
        let action =
            chart_table_view::show(ui, &mut self.context.table, &title, feed.as_deref(), price);
        if action == Some(TableAction::Close) {
            self.context.table_chart = None;
        }
    }

    pub(super) fn show_tree_panel(&mut self, ui: &mut egui::Ui) {
        let Some(chart_id) = self.context.tree_chart else {
            return;
        };
        let Some(index) = self.chart_index(chart_id) else {
            return;
        };
        let chart = &self.charts[index];
        let title = format!("{} · {}", chart.symbol, chart.source.display_name());
        let rows = object_tree::rows(
            &object_tree::Inputs {
                title: &chart.symbol,
                source: chart.source.display_name(),
                indicators: &chart.indicators,
                scripts: chart.chart.pine_instances(),
                drawings: &chart.drawings,
            },
            &self.context.tree.filter,
        );
        let selected: Vec<u64> = self
            .selected_drawing
            .as_ref()
            .filter(|(selected_chart, _)| *selected_chart == chart_id)
            .map(|(_, ids)| ids.clone())
            .unwrap_or_default();
        let actions = tree_view::show(ui, &mut self.context.tree, &title, &rows, &selected);
        for action in actions {
            self.apply_tree_action(chart_id, action);
        }
    }

    /// What the user did in the Object Tree, through the paths the chart's own controls use.
    fn apply_tree_action(&mut self, chart_id: u64, action: TreeAction) {
        let Some(index) = self.chart_index(chart_id) else {
            return;
        };
        let drawing = |workspace: &Self, id: u64| {
            workspace.charts[index]
                .drawings
                .iter()
                .find(|candidate| candidate.stored_id() == id)
                .cloned()
        };
        match action {
            TreeAction::Close => self.context.tree_chart = None,
            TreeAction::Select(TreeKey::Drawing(id)) => {
                self.selected_drawing = Some((chart_id, vec![id]));
                self.focus_chart(chart_id);
            }
            TreeAction::Select(_) => self.focus_chart(chart_id),
            TreeAction::Settings(TreeKey::Drawing(id)) => {
                self.open_drawing_properties(chart_id, id);
            }
            TreeAction::Settings(TreeKey::Indicator(key)) => {
                self.settings_state = ChartSettingsState::default();
                self.settings_state.tab = match key {
                    IndicatorKey::BuiltIn(_) => chart_settings::Tab::Indicators,
                    IndicatorKey::Script(_) => chart_settings::Tab::Scripts,
                };
                self.settings_chart = Some(chart_id);
            }
            TreeAction::Settings(_) => {}
            TreeAction::ToggleShown(TreeKey::Drawing(id)) => {
                if let Some(mut changed) = drawing(self, id) {
                    changed.set_hidden(!changed.hidden());
                    self.apply_drawing_commands(
                        chart_id,
                        vec![DrawingCommand::Replace {
                            id,
                            drawing: changed,
                        }],
                    );
                }
            }
            TreeAction::ToggleShown(TreeKey::Indicator(IndicatorKey::BuiltIn(indicator))) => {
                indicator.toggle_visible(&mut self.charts[index].indicators);
            }
            TreeAction::ToggleShown(TreeKey::Indicator(IndicatorKey::Script(id))) => {
                let card = &mut self.charts[index].chart;
                let visible = card
                    .pine_instances()
                    .iter()
                    .find(|candidate| candidate.id == id)
                    .is_some_and(|candidate| candidate.visible);
                card.set_pine_visible(id, !visible);
            }
            TreeAction::ToggleShown(_) => {}
            TreeAction::ToggleLocked(id) => {
                if let Some(mut changed) = drawing(self, id) {
                    changed.set_locked(!changed.locked());
                    self.apply_drawing_commands(
                        chart_id,
                        vec![DrawingCommand::Replace {
                            id,
                            drawing: changed,
                        }],
                    );
                }
            }
            TreeAction::Delete(TreeKey::Drawing(id)) => {
                if drawing(self, id).is_some_and(|d| !d.locked()) {
                    self.apply_drawing_commands(chart_id, vec![DrawingCommand::Remove(id)]);
                    if let Some((_, ids)) = &mut self.selected_drawing {
                        ids.retain(|selected| *selected != id);
                    }
                }
            }
            TreeAction::Delete(TreeKey::Indicator(IndicatorKey::BuiltIn(indicator))) => {
                self.remember_indicators(chart_id);
                indicator.set_applied(&mut self.charts[index].indicators, false);
            }
            TreeAction::Delete(TreeKey::Indicator(IndicatorKey::Script(id))) => {
                self.remember_indicators(chart_id);
                self.charts[index].chart.remove_pine_instance(id);
            }
            TreeAction::Delete(_) => {}
            TreeAction::Reorder(id, step) => {
                self.apply_drawing_commands(chart_id, vec![DrawingCommand::Reorder { id, step }]);
            }
        }
    }

    // MARK: Removing indicators

    /// Puts the chart's indicators and scripts on the undo stack before they change.
    pub(super) fn remember_indicators(&mut self, chart_id: u64) {
        let Some(chart) = self.charts.iter().find(|chart| chart.id == chart_id) else {
            return;
        };
        self.undo.push(UndoEntry {
            chart_id,
            before: Snapshot::Indicators {
                config: chart.indicators,
                scripts: chart.chart.pine_instances().to_vec(),
            },
        });
        self.redo.clear();
        self.last_restyle = None;
    }

    /// The confirmation behind "Remove N indicators". The scripts' saved source and their alert
    /// subscriptions are left alone: only the chart's instances go, and Undo brings them back.
    fn confirm_indicators_removal(&mut self, ctx: &Context) {
        let Some(chart_id) = self.context.confirm_remove_indicators else {
            return;
        };
        let Some(chart) = self.charts.iter().find(|chart| chart.id == chart_id) else {
            self.context.confirm_remove_indicators = None;
            return;
        };
        let applied = BuiltIn::ALL
            .iter()
            .filter(|indicator| indicator.applied(&chart.indicators))
            .count();
        let scripts = chart.chart.pine_instances().len();
        let count = applied + scripts;
        if count == 0 {
            self.context.confirm_remove_indicators = None;
            return;
        }
        let message = format!(
            "{} will be removed from this chart. Saved scripts are kept, and you can undo it.",
            chart_context::remove_indicators_label(count)
                .trim_start_matches("Remove ")
                .to_owned()
        );
        let result = ui::kit::confirm(
            ctx,
            Id::new(("remove-indicators", self.id)),
            &ui::kit::Confirm {
                icon: ui::icons::Icon::Trash,
                title: "Remove all indicators?",
                message: &message,
                destructive: "Remove All",
            },
        );
        match result {
            Some(ui::kit::ConfirmOutcome::Confirmed) => {
                self.context.confirm_remove_indicators = None;
                self.remember_indicators(chart_id);
                if let Some(chart) = self.charts.iter_mut().find(|chart| chart.id == chart_id) {
                    for indicator in BuiltIn::ALL {
                        indicator.set_applied(&mut chart.indicators, false);
                    }
                    chart.chart.set_pine_instances(Vec::new());
                }
            }
            Some(ui::kit::ConfirmOutcome::Cancelled) => {
                self.context.confirm_remove_indicators = None;
            }
            None => {}
        }
    }
}

/// `current` with `id` toggled in or out.
fn toggled(current: Option<u64>, id: u64) -> Option<u64> {
    if current == Some(id) { None } else { Some(id) }
}

#[cfg(test)]
mod tests;

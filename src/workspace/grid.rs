//! The chart grid: drawing the columns of chart cards, dragging cards between them, and applying what the pass collected.

use super::ChartWorkspace;
use super::alerts::AlertRequest;
use super::chart::displayed_candles;
use crate::host::paper_host::PaperHost;
use crate::model::{
    ChartColumn, ChartLayout, Drawing, DrawingCommand, DrawingStyle, InstrumentId, ReplayStatus,
    Tool,
};
use crate::ui::chart_settings::{self, ChartSettingsState};
use crate::ui::{self, ChartCardAction, CrosshairInfo, DrawingFrame, ReplayChartFrame, shortcut};
use eframe::egui;
use std::collections::{HashMap, HashSet};
use std::time::Instant;

/// A deferred mutation the grid collects while drawing, applied after the pass.
enum ChartCommand {
    Focus(u64),
    Settings(u64),
    Remove(u64),
    Indicator(u64, crate::ui::chart_indicator_strip::Action),
    Move {
        chart_id: u64,
        to_column: u64,
        before: Option<u64>,
    },
    NewColumn(u64),
    /// The chart-type selector of one chart card was used.
    SetChartType(u64, crate::model::ChartType),
    /// A row of one chart card's camera menu was picked.
    Snapshot(u64, crate::model::snapshot::SnapshotAction),
}

impl ChartWorkspace {
    pub(super) fn place_in_grid(&mut self, chart_id: u64) {
        if self.columns.is_empty() || self.columns.len() < 2 {
            let column_id = self.fresh_id();
            self.columns
                .push(ChartColumn::new(column_id, vec![chart_id]));
        } else if let Some(shortest) = self
            .columns
            .iter()
            .map(|column| column.chart_ids.len())
            .enumerate()
            .min_by_key(|(index, len)| (*len, *index))
            .map(|(index, _)| index)
        {
            self.columns[shortest].chart_ids.push(chart_id);
        }
    }

    fn move_chart(&mut self, chart_id: u64, to_column: u64, before: Option<u64>) {
        if before == Some(chart_id) || !self.columns.iter().any(|c| c.id == to_column) {
            return;
        }
        let original = self.columns.clone();

        for column in &mut self.columns {
            column.chart_ids.retain(|id| *id != chart_id);
        }
        self.columns
            .retain(|column| !column.chart_ids.is_empty() || column.id == to_column);

        let Some(column_index) = self
            .columns
            .iter()
            .position(|column| column.id == to_column)
        else {
            self.columns = original;
            return;
        };
        let insertion = before
            .and_then(|target| {
                self.columns[column_index]
                    .chart_ids
                    .iter()
                    .position(|id| *id == target)
            })
            .unwrap_or(self.columns[column_index].chart_ids.len());
        self.columns[column_index]
            .chart_ids
            .insert(insertion, chart_id);
    }

    fn move_to_new_trailing_column(&mut self, chart_id: u64) {
        if !self.charts.iter().any(|chart| chart.id == chart_id) {
            return;
        }
        for column in &mut self.columns {
            column.chart_ids.retain(|id| *id != chart_id);
        }
        self.columns.retain(|column| !column.chart_ids.is_empty());
        let column_id = self.fresh_id();
        self.columns
            .push(ChartColumn::new(column_id, vec![chart_id]));
    }

    /// While a chart is dragged: finds what is under the pointer, draws its cue (an insert
    /// line, or the new-column strip and its preview) and, on release, queues the move.
    #[allow(clippy::too_many_arguments)]
    fn drive_grid_drag(
        &mut self,
        ctx: &egui::Context,
        dragged: Option<u64>,
        grid_rect: egui::Rect,
        mut zones: ui::grid_drop::Zones,
        can_add_column: bool,
        previewed: Option<u64>,
        commands: &mut Vec<ChartCommand>,
    ) {
        use ui::grid_drop::{self, DropTarget};
        let Some(dragged) = dragged else {
            return;
        };
        if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            egui::DragAndDrop::clear_payload(ctx);
            self.grid_drag.reset();
            return;
        }
        if can_add_column && zones.new_column.is_none() {
            zones.new_column = Some(egui::Rect::from_min_max(
                egui::pos2(
                    grid_rect.right() - grid_drop::NEW_COLUMN_STRIP_WIDTH,
                    grid_rect.top(),
                ),
                grid_rect.right_bottom(),
            ));
        }
        let target = ctx
            .pointer_latest_pos()
            .and_then(|pos| zones.target_at(pos));
        if let Some(wait) = self.grid_drag.hover(dragged, target, Instant::now()) {
            ctx.request_repaint_after(wait);
        }

        // Cues sit above the cards, on the grid's own layer.
        let painter_ui = egui::Ui::new(
            ctx.clone(),
            egui::Id::new("grid-drop-cues"),
            egui::UiBuilder::new()
                .layer_id(egui::LayerId::new(
                    egui::Order::Foreground,
                    egui::Id::new("grid-drop-cues-layer"),
                ))
                .max_rect(grid_rect),
        );
        if can_add_column
            && previewed.is_none()
            && let Some(strip) = zones.new_column
        {
            grid_drop::new_column_strip(&painter_ui, strip, target == Some(DropTarget::NewColumn));
        }
        match target {
            Some(DropTarget::Before { chart, .. }) if chart != dragged => {
                if let Some((_, _, rect)) = zones.cards.iter().find(|(_, id, _)| *id == chart) {
                    grid_drop::insert_line_above(&painter_ui, *rect);
                }
            }
            Some(DropTarget::Append { column }) => {
                if let Some((_, rect)) = zones.columns.iter().find(|(id, _)| *id == column) {
                    grid_drop::insert_line_below(&painter_ui, *rect);
                }
            }
            _ => {}
        }
        if let Some(chart) = self.charts.iter().find(|chart| chart.id == dragged) {
            grid_drop::drag_label(ctx, &chart.display_title());
        }
        ctx.set_cursor_icon(egui::CursorIcon::Grabbing);

        if ctx.input(|input| input.pointer.any_released()) {
            match target {
                Some(DropTarget::Before { column, chart }) if chart != dragged => {
                    commands.push(ChartCommand::Move {
                        chart_id: dragged,
                        to_column: column,
                        before: Some(chart),
                    });
                }
                Some(DropTarget::Append { column }) => commands.push(ChartCommand::Move {
                    chart_id: dragged,
                    to_column: column,
                    before: None,
                }),
                Some(DropTarget::NewColumn) => commands.push(ChartCommand::NewColumn(dragged)),
                _ => {}
            }
            self.grid_drag.reset();
        }
    }

    pub(super) fn render_grid(&mut self, ui: &mut egui::Ui, paper_host: &mut PaperHost) {
        if self.charts.is_empty() {
            self.show_empty_state(ui);
            return;
        }
        // The next visit to the empty page starts from fresh data.
        self.empty_page.data = None;

        let available = ui.available_size();
        let column_count = self.columns.len().max(1);
        let max_rows = self
            .columns
            .iter()
            .map(|column| column.chart_ids.len())
            .max()
            .unwrap_or(0);
        let card_height =
            ChartLayout::grid_card_height(available.y, max_rows, ui.spacing().item_spacing.y);

        let plan = self.columns.clone();
        let index_map: HashMap<u64, usize> = self
            .charts
            .iter()
            .enumerate()
            .map(|(index, chart)| (chart.id, index))
            .collect();
        let focused = self.focused;
        let multi = self.charts.len() > 1;
        let crosshair = self.crosshair;
        let mut crosshair_update: Option<CrosshairInfo> = None;
        let mut hover_update: Option<(u64, crate::model::Anchor)> = None;
        let active_tool = self.active_tool;
        let selected_drawing = self.selected_drawing.clone();
        let (magnet, keep_drawing, hide_all_drawings) =
            (self.magnet, self.keep_drawing, self.hide_all_drawings);
        let replay_ms = if self.replay.is_active() {
            self.replay.current_ms()
        } else {
            None
        };
        let replay_selecting = self.replay.status == ReplayStatus::SelectingStart;
        let timeframe = self.timeframe;
        let hosting_note = match self.snapshot_hosting() {
            crate::host::snapshot_host::Hosting::Ready(_) => None,
            crate::host::snapshot_host::Hosting::NotConfigured => {
                Some("Set a snapshot host in Settings ▸ Snapshots to share.")
            }
        };
        let snapshot_shortcut =
            |action| shortcut::snapshot(action).map(|shortcut| shortcut.label());
        let snapshot_menu = ui::snapshot_menu::SnapshotMenu {
            busy: self.snapshot_busy(),
            hosting_unavailable: hosting_note,
            shortcut: &snapshot_shortcut,
        };
        let replay_active = self.replay.is_active();
        let mut replay_picked: Option<(u64, i64)> = None;
        // Linked navigation needs another chart to follow.
        let sync = self.sync;
        let linked = self.market_charts() > 1 && sync.is_linked();
        let mut navigated = false;

        let mut drawing_commands: Vec<(u64, Vec<DrawingCommand>)> = Vec::new();
        let mut drawing_select: Option<(u64, Vec<u64>)> = None;
        let mut drawing_disarm = false;
        let picture_work = self.content.work_snapshot(self.focused);
        let mut picture_used = false;
        let mut picture_replace: Option<(u64, u64)> = None;
        let mut picture_copy: Option<String> = None;
        let mut link_request: Option<(u64, u64, ui::LinkRequest)> = None;
        let mut drawing_properties_request: Option<(u64, u64)> = None;
        let mut drawing_style_copied: Option<(Tool, DrawingStyle)> = None;
        let style_clipboard = self.style_clipboard;
        let dialog_open = self.drawing_props.is_some()
            || self.confirm_remove_drawings.is_some()
            || self.content.sheet_open()
            || self.add_chart.is_open();
        let mut drawing_reveal = false;
        let mut drawing_copy = false;
        let mut drawing_alert: Option<(u64, f64)> = None;
        let drawing_defaults = self.drawing_templates().defaults().clone();
        let drawing_preview: Option<(u64, Drawing)> = self
            .drawing_props
            .as_ref()
            .map(|props| (props.chart_id, props.draft().clone()));
        let mut drawing_escape = false;
        let mut paper_actions = Vec::new();
        let mut quick_trade = None;
        let mut new_alert: Option<AlertRequest> = None;
        let mut context_request: Option<(u64, ui::ContextRequest)> = None;
        let (favorites, watchlist_memberships): (HashSet<InstrumentId>, Vec<_>) = {
            let store = self.watchlist.lock().expect("watchlist lock poisoned");
            let favorites = store
                .lists()
                .iter()
                .find(|list| list.is_favorites)
                .into_iter()
                .flat_map(|list| list.instruments())
                .map(|item| item.instrument.clone())
                .collect();
            let memberships = store
                .lists()
                .iter()
                .map(|list| {
                    (
                        list.id,
                        list.name.clone(),
                        list.instruments()
                            .map(|item| item.instrument.clone())
                            .collect::<HashSet<_>>(),
                    )
                })
                .collect();
            (favorites, memberships)
        };
        let mut favorite_toggles = Vec::new();
        let mut watchlist_changes = Vec::new();
        let card_env = self.card_env.clone();

        let mut commands: Vec<ChartCommand> = Vec::new();
        let can_add_column = ChartLayout::can_add_column(available.x, column_count);
        let ctx = ui.ctx().clone();
        // Nothing about a drag shows until a chart is being dragged.
        let dragged: Option<u64> = egui::DragAndDrop::payload::<u64>(&ctx).map(|payload| *payload);
        if dragged.is_none() {
            self.grid_drag.reset();
        }
        let previewed = if can_add_column {
            self.grid_drag.previewed()
        } else {
            None
        };
        let mut zones = ui::grid_drop::Zones::default();
        let grid_rect = ui.available_rect_before_wrap();

        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(available.x, available.y),
                egui::Layout::left_to_right(egui::Align::Min),
                |ui| {
                    ui.columns(column_count + usize::from(previewed.is_some()), |uis| {
                        for (column_index, column_ui) in uis.iter_mut().enumerate() {
                            let Some(column) = plan.get(column_index) else {
                                // The previewed new column: a dashed placeholder where it would be.
                                let rect = column_ui.max_rect();
                                let card = egui::Rect::from_min_size(
                                    rect.min,
                                    egui::vec2(rect.width(), card_height),
                                );
                                ui::grid_drop::new_column_placeholder(column_ui, card);
                                zones.new_column = Some(egui::Rect::from_min_size(
                                    rect.min,
                                    egui::vec2(rect.width(), available.y),
                                ));
                                continue;
                            };
                            let column_rect = column_ui.max_rect();
                            zones.columns.push((
                                column.id,
                                egui::Rect::from_min_size(
                                    column_rect.min,
                                    egui::vec2(column_rect.width(), available.y),
                                ),
                            ));
                            for &chart_id in &column.chart_ids {
                                let Some(&index) = index_map.get(&chart_id) else {
                                    continue;
                                };
                                let is_focused = multi && focused == Some(chart_id);
                                let card = egui::Frame::new()
                                    .fill(ui::theme::surface())
                                    .corner_radius(10.0)
                                    .inner_margin(egui::Margin::same(6))
                                    .stroke(egui::Stroke::new(
                                        1.0,
                                        if replay_ms.is_some() {
                                            ui::theme::REPLAY.gamma_multiply(0.45)
                                        } else if is_focused {
                                            ui::theme::ACCENT
                                        } else {
                                            ui::theme::border()
                                        },
                                    ));

                                let drawn = column_ui.allocate_ui_with_layout(
                                    egui::vec2(column_ui.available_width(), card_height),
                                    egui::Layout::top_down(egui::Align::Min),
                                    |ui| {
                                        if previewed == Some(chart_id) {
                                            ui.multiply_opacity(0.35);
                                        }
                                        let chart = &mut self.charts[index];
                                        if let Some(mut shown) = chart.card {
                                            let instrument = shown.instrument_id();
                                            let memberships: Vec<crate::ui::WatchlistMembership> =
                                                watchlist_memberships
                                                    .iter()
                                                    .map(|(id, name, instruments)| {
                                                        crate::ui::WatchlistMembership {
                                                            id: *id,
                                                            name: name.clone(),
                                                            contains_market: instruments
                                                                .contains(&instrument),
                                                        }
                                                    })
                                                    .collect();
                                            return card.show(ui, |ui| {
                                                let mut action = ChartCardAction::default();
                                                let data = card_env.as_ref().map(|env| {
                                                    let runtime = chart
                                                        .card_runtime
                                                        .get_or_insert_with(|| env.runtime());
                                                    env.view(runtime, &shown, ui.ctx())
                                                });
                                                let scope = data.as_ref().map_or_else(
                                                    || shown.scope_name(&[]),
                                                    |data| data.scope_name.clone(),
                                                );
                                                ui::portfolio_cards::header(
                                                    ui,
                                                    &shown,
                                                    &scope,
                                                    chart_id,
                                                    favorites.contains(&instrument),
                                                    &memberships,
                                                    &mut action,
                                                );
                                                if ui::portfolio_cards::body(
                                                    ui,
                                                    &mut shown,
                                                    data.as_ref(),
                                                    chart_id,
                                                ) {
                                                    action.focus_requested = true;
                                                }
                                                chart.card = Some(shown);
                                                action
                                            });
                                        }
                                        chart.chart.set_series_cutoff(replay_ms);
                                        chart.chart.set_timeframe(timeframe);
                                        let candles = displayed_candles(&chart.candles, replay_ms);
                                        if linked {
                                            chart.chart.follow_sync(
                                                &mut self.sync_hub,
                                                chart_id,
                                                sync,
                                                chart.source,
                                                candles,
                                            );
                                        }
                                        let instrument =
                                            InstrumentId::new(chart.source, &chart.symbol);
                                        let memberships: Vec<crate::ui::WatchlistMembership> =
                                            watchlist_memberships
                                                .iter()
                                                .map(|(id, name, instruments)| {
                                                    crate::ui::WatchlistMembership {
                                                        id: *id,
                                                        name: name.clone(),
                                                        contains_market: instruments
                                                            .contains(&instrument),
                                                    }
                                                })
                                                .collect();
                                        card.show(ui, |ui| {
                                            let mut action = ChartCardAction::default();
                                            chart.chart.header(
                                                ui,
                                                &chart.symbol,
                                                chart.source,
                                                &chart.indicators,
                                                candles,
                                                replay_ms.is_some(),
                                                favorites.contains(&instrument),
                                                &memberships,
                                                &mut action,
                                                &snapshot_menu,
                                                chart_id,
                                            );
                                            let selected = selected_drawing
                                                .as_ref()
                                                .filter(|(chart, _)| *chart == chart_id)
                                                .map(|(_, drawings)| drawings.clone())
                                                .unwrap_or_default();
                                            let (pending, place_now, replace_with) =
                                                picture_work.for_chart(chart_id);
                                            let mut frame = DrawingFrame::new(
                                                &chart.drawings,
                                                active_tool,
                                                None,
                                                chart.next_drawing_id,
                                            )
                                            .with_pending_content(pending)
                                            .with_picture_work(place_now, replace_with)
                                            .with_selection(selected)
                                            .with_view(timeframe, hide_all_drawings)
                                            .with_behavior(magnet, keep_drawing)
                                            .with_style_clipboard(style_clipboard)
                                            .with_keys_blocked(dialog_open)
                                            .with_defaults(&drawing_defaults)
                                            .with_preview(
                                                drawing_preview
                                                    .as_ref()
                                                    .filter(|(preview_chart, _)| {
                                                        *preview_chart == chart_id
                                                    })
                                                    .map(|(_, draft)| draft),
                                            );
                                            let mut replay_frame = ReplayChartFrame {
                                                selecting: replay_selecting,
                                                active: replay_active,
                                                picked_ms: None,
                                            };
                                            let pointer = chart.chart.body(
                                                ui,
                                                &chart.symbol,
                                                candles,
                                                chart.loading,
                                                chart.error.as_deref(),
                                                chart.scale_mode,
                                                chart.source,
                                                &chart.indicators,
                                                crosshair.as_ref(),
                                                chart_id,
                                                &mut frame,
                                                &mut replay_frame,
                                            );
                                            if let Some(picked_ms) = replay_frame.picked_ms {
                                                replay_picked = Some((chart_id, picked_ms));
                                            }
                                            if linked
                                                && chart.chart.publish_sync(
                                                    &mut self.sync_hub,
                                                    chart_id,
                                                    sync,
                                                )
                                            {
                                                navigated = true;
                                            }
                                            chart.next_drawing_id = frame.next_id;
                                            if let Some(select) = frame.select {
                                                drawing_select = Some((chart_id, select));
                                            }
                                            drawing_escape |= frame.escape_consumed;
                                            drawing_disarm |= frame.disarm;
                                            picture_used |= frame.content_used;
                                            if let Some(drawing_id) = frame.replace_request {
                                                picture_replace = Some((chart_id, drawing_id));
                                            }
                                            if frame.copy_image.is_some() {
                                                picture_copy = frame.copy_image.take();
                                            }
                                            if let Some((drawing_id, request)) = frame.link_request
                                            {
                                                link_request =
                                                    Some((chart_id, drawing_id, request));
                                            }
                                            drawing_reveal |= frame.reveal_all;
                                            drawing_copy |= frame.copy_requested;
                                            if let Some(price) = frame.alert_price {
                                                drawing_alert = Some((chart_id, price));
                                            }
                                            if frame.style_copied.is_some() {
                                                drawing_style_copied = frame.style_copied;
                                            }
                                            if let Some(drawing_id) = frame.open_properties {
                                                drawing_properties_request =
                                                    Some((chart_id, drawing_id));
                                            }
                                            let frame_commands =
                                                std::mem::take(&mut frame.commands);
                                            if !frame_commands.is_empty() {
                                                drawing_commands.push((chart_id, frame_commands));
                                            }
                                            if pointer.clicked {
                                                action.focus_requested = true;
                                            }
                                            if let Some(anchor) = pointer.hover_anchor {
                                                hover_update = Some((chart_id, anchor));
                                            }
                                            if let Some(request) = pointer.context.clone() {
                                                context_request = Some((chart_id, request));
                                            }
                                            if let (Some(x_fraction), Some(price)) =
                                                (pointer.hover_x_fraction, pointer.hover_price)
                                            {
                                                crosshair_update = Some(CrosshairInfo {
                                                    owner_id: chart_id,
                                                    x_fraction,
                                                    price,
                                                });
                                            }
                                            action
                                        })
                                    },
                                );

                                if drawn.inner.inner.focus_requested {
                                    commands.push(ChartCommand::Focus(chart_id));
                                }
                                if drawn.inner.inner.remove_requested {
                                    commands.push(ChartCommand::Remove(chart_id));
                                }
                                if let Some(chart_type) = drawn.inner.inner.chart_type {
                                    commands.push(ChartCommand::SetChartType(chart_id, chart_type));
                                }
                                if let Some(action) = drawn.inner.inner.snapshot {
                                    commands.push(ChartCommand::Snapshot(chart_id, action));
                                }
                                if drawn.inner.inner.settings_requested {
                                    commands.push(ChartCommand::Settings(chart_id));
                                }
                                if let Some(action) = drawn.inner.inner.indicator_action {
                                    commands.push(ChartCommand::Indicator(chart_id, action));
                                }
                                if drawn.inner.inner.new_alert {
                                    new_alert = Some(AlertRequest::market(Self::alert_asset(
                                        &self.charts[index],
                                    )));
                                }
                                if drawn.inner.inner.toggle_favorite {
                                    favorite_toggles.push(chart_id);
                                }
                                if let Some(change) = drawn.inner.inner.watchlist_change {
                                    watchlist_changes.push((chart_id, change));
                                }
                                if let Some(side) = drawn.inner.inner.quick_trade {
                                    let reference_price =
                                        displayed_candles(&self.charts[index].candles, replay_ms)
                                            .last()
                                            .map(|candle| candle.close);
                                    quick_trade = Some((chart_id, side, reference_price));
                                }
                                paper_actions.append(&mut self.charts[index].chart.paper.actions);
                                zones.cards.push((column.id, chart_id, drawn.response.rect));
                            }
                        }
                    });
                },
            );
        });

        self.sync_hub.end_frame();
        if navigated {
            // Charts drawn before the one being navigated hear of it one frame later: ask for it
            // at once.
            ctx.request_repaint();
        }

        self.drive_grid_drag(
            &ctx,
            dragged,
            grid_rect,
            zones,
            can_add_column,
            previewed,
            &mut commands,
        );

        if !favorite_toggles.is_empty() || !watchlist_changes.is_empty() {
            let mut store = self.watchlist.lock().expect("watchlist lock poisoned");
            let list_id = store.favorites();
            for chart_id in favorite_toggles {
                if let Some(chart) = self.charts.iter().find(|chart| chart.id == chart_id) {
                    let instrument = Self::chart_instrument(chart);
                    if favorites.contains(&instrument) {
                        store.remove_from_list(list_id, &instrument);
                    } else {
                        store.add_to_list(list_id, self.favorite_item(chart));
                    }
                }
            }
            for (chart_id, (list_id, included)) in watchlist_changes {
                if let Some(chart) = self.charts.iter().find(|chart| chart.id == chart_id) {
                    let instrument = Self::chart_instrument(chart);
                    if included {
                        store.add_to_list(list_id, self.favorite_item(chart));
                    } else {
                        store.remove_from_list(list_id, &instrument);
                    }
                }
            }
        }

        for command in commands {
            match command {
                ChartCommand::Focus(chart_id) => self.focus_chart(chart_id),
                ChartCommand::Snapshot(chart_id, action) => {
                    self.begin_snapshot(&ctx, chart_id, action);
                }
                ChartCommand::Remove(chart_id) => self.removing_chart = Some(chart_id),
                ChartCommand::Settings(chart_id) => {
                    self.settings_state = ChartSettingsState::default();
                    self.settings_chart = Some(chart_id);
                }
                ChartCommand::Indicator(chart_id, action) => {
                    use crate::ui::chart_indicator_strip::{Action, Kind};
                    match action {
                        Action::OpenIndicatorSettings => {
                            self.settings_state = ChartSettingsState::default();
                            self.settings_chart = Some(chart_id);
                        }
                        Action::ConfigureScript(id, anchor) => {
                            self.toggle_script_popover(chart_id, id, anchor);
                        }
                        Action::OpenPineLogs(id) => self.open_pine_logs(chart_id, Some(id)),
                        Action::OpenScriptSettings => {
                            self.settings_state = ChartSettingsState::default();
                            self.settings_state.tab = chart_settings::Tab::Scripts;
                            self.settings_chart = Some(chart_id);
                        }
                        _ => {
                            if let Some(chart) = self.charts.iter_mut().find(|c| c.id == chart_id) {
                                match action {
                                    Action::Add(kind) => {
                                        kind.set_applied(&mut chart.indicators, true)
                                    }
                                    Action::Remove(kind) => {
                                        kind.set_applied(&mut chart.indicators, false)
                                    }
                                    Action::EmaPeriod(period) => {
                                        chart.indicators.ema_period = period
                                    }
                                    Action::ToggleVisible(Kind::BuiltIn(kind)) => {
                                        kind.toggle_visible(&mut chart.indicators);
                                    }
                                    Action::ToggleVisible(Kind::Script(id)) => {
                                        if let Some(instance) =
                                            chart.chart.pine_instances().iter().find(|i| i.id == id)
                                        {
                                            chart.chart.set_pine_visible(id, !instance.visible);
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
                ChartCommand::Move {
                    chart_id,
                    to_column,
                    before,
                } => self.move_chart(chart_id, to_column, before),
                ChartCommand::NewColumn(chart_id) => self.move_to_new_trailing_column(chart_id),
                ChartCommand::SetChartType(chart_id, chart_type) => {
                    if let Some(chart) = self.charts.iter_mut().find(|chart| chart.id == chart_id)
                        && chart_type.supported_by(chart.source)
                    {
                        chart.chart.set_chart_type(chart_type);
                    }
                }
            }
        }

        self.crosshair = crosshair_update;
        if let Some((chart_id, anchor)) = hover_update {
            self.note_hover(chart_id, Some(anchor));
        }
        if let Some((chart_id, request)) = context_request {
            self.open_context_menu(chart_id, request);
        }

        if let Some((chart_id, picked_ms)) = replay_picked {
            let (symbol, timeline) = self
                .charts
                .iter()
                .find(|chart| chart.id == chart_id)
                .map(|chart| {
                    (
                        chart.symbol.clone(),
                        chart.candles.iter().map(|kline| kline.open_time).collect(),
                    )
                })
                .unwrap_or_default();
            self.replay.start(picked_ms, &symbol, timeline);
        }

        for (chart_id, drawing) in drawing_commands {
            self.apply_drawing_commands(chart_id, drawing);
        }
        Self::apply_paper_overlay_actions(paper_host, paper_actions);
        if new_alert.is_some() {
            self.new_alert_request = new_alert;
        }
        if let Some((chart_id, side, reference_price)) = quick_trade
            && let Some(chart) = self.charts.iter().find(|chart| chart.id == chart_id)
        {
            let instrument = Self::paper_instrument(chart);
            self.paper_panel
                .open_ticket(paper_host, instrument, reference_price, side);
        }
        if let Some((chart_id, select)) = drawing_select {
            self.selected_drawing = (!select.is_empty()).then_some((chart_id, select));
        }
        if drawing_disarm {
            self.active_tool = Tool::Cursor;
        }
        self.picture_work_done();
        if picture_used {
            self.picture_used();
            self.preview_placed_card(&ctx);
        }
        if let Some((chart_id, drawing_id)) = picture_replace {
            self.replace_picture_request(&ctx, chart_id, drawing_id);
        }
        if let Some(asset) = picture_copy {
            self.copy_picture(&ctx, &asset);
        }
        if let Some((chart_id, drawing_id, request)) = link_request {
            self.link_requested(&ctx, chart_id, drawing_id, request);
        }
        if drawing_reveal {
            self.hide_all_drawings = false;
        }
        if drawing_copy {
            self.copy_selection(&ctx);
        }
        if let Some((chart_id, price)) = drawing_alert {
            self.start_alert_at(chart_id, Some(price));
        }
        if drawing_style_copied.is_some() {
            self.style_clipboard = drawing_style_copied;
        }
        if let Some((chart_id, drawing_id)) = drawing_properties_request {
            self.open_drawing_properties(chart_id, drawing_id);
        }
        let esc_pressed = ui.input(|input| input.key_pressed(egui::Key::Escape));
        if esc_pressed && !drawing_escape && !self.date_sheet.is_open() && !self.context_menu_open()
        {
            if self.replay.status == ReplayStatus::SelectingStart {
                self.replay.cancel_selection();
            } else if self.active_tool != Tool::Cursor {
                self.active_tool = Tool::Cursor;
            } else {
                self.selected_drawing = None;
            }
        }
    }
}

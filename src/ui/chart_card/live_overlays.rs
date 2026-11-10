//! What the live chart adds to a frame on top of what a snapshot shows: everything the user can
//! point at.

use super::frame::{FrameData, FrameLayout, Overlays, PaneSlot, paint_pane_title};
use super::overlays::OverlayFrame;
use super::*;
use crate::ui::journal_markers;
use crate::ui::paper_chart_trading::{MarkerMenu, PaperOverlayAction};
use crate::ui::scale_menu;
use crate::ui::{kit, theme};
use degenview_journal::marks::TradeMark;

pub(super) struct LiveOverlays<'a> {
    pub ui: &'a mut egui::Ui,
    pub response: &'a Response,
    pub card: &'a ChartCard,
    pub replay: &'a mut ReplayChartFrame,
    pub chart_id: u64,
    pub chart_hovered: bool,
    pub pane_actions: Vec<(u64, PaneAction)>,
    pub alert_menu: Option<alert_lines::LineMenu>,
    pub alert_moves: Vec<alert_lines::AlertMove>,
    pub alert_toggled: Vec<crate::model::alert::AlertId>,
    pub alert_deleted: Vec<crate::model::alert::AlertId>,
    pub paper: (Vec<PaperOverlayAction>, Option<MarkerMenu>),
    pub navigation: Option<chart_buttons::NavAction>,
    pub scale_mode: Option<PriceScaleMode>,
    pub overlay_action: Option<chart_indicator_strip::Action>,
    pub on_overlay_card: bool,
    pub journal_open: Option<String>,
}

/// The widest a script tooltip gets before it wraps (inferred: TradingView does not document it).
const PINE_TOOLTIP_WIDTH: f32 = 320.0;

impl Overlays for LiveOverlays<'_> {
    fn after_drawings(
        &mut self,
        layer: &mut DrawingLayer,
        projection: &Projection,
        drawing: &mut DrawingFrame,
        layout: &FrameLayout,
    ) {
        if layout.main_visible
            && !self.replay.selecting
            && !self
                .card
                .zoom_is_live(layout.main_visible, self.replay.selecting)
        {
            layer.overlay(self.ui, projection, drawing);
        }
    }

    fn after_tables(
        &mut self,
        main_painter: &egui::Painter,
        data: &FrameData,
        layout: &FrameLayout,
        view: &View,
    ) {
        if self.replay.selecting {
            let synthetic_bars = data.synthetic();
            let pick_time = |index: usize| {
                synthetic_bars
                    .and_then(|all| all.get(index))
                    .map_or(data.bars[index].open_time, |bar| bar.open_time)
            };
            self.card.handle_start_pick(
                self.ui,
                self.response,
                main_painter,
                layout.plot,
                &pick_time,
                view,
                self.replay,
            );
        }
        if self.replay.active {
            replay_overlay::paint_active(main_painter, self.card.below_header(layout.plot));
        }
        self.card.paint_zoom(self.ui, main_painter, layout.plot);
    }

    fn journal_hover(&mut self, hovered: Option<&TradeMark>, idle: bool) {
        let Some(mark) = hovered else { return };
        self.ui
            .ctx()
            .set_cursor_icon(egui::CursorIcon::PointingHand);
        if let Some(pointer) = self.response.hover_pos() {
            let mut over = self.response.clone();
            let target = Rect::from_center_size(pointer, egui::vec2(16.0, 16.0));
            over.rect = target;
            over.interact_rect = target;
            over.on_hover_ui(|ui| {
                ui.set_max_width(260.0);
                for (index, line) in journal_markers::tooltip(mark).into_iter().enumerate() {
                    let (weight, color) = if index == 0 {
                        (kit::SEMIBOLD, theme::text_primary())
                    } else {
                        (kit::REGULAR, theme::text_secondary())
                    };
                    ui.label(kit::text(line, 12.0, weight, color));
                }
            });
        }
        if idle && self.response.clicked() {
            self.journal_open = Some(mark.key.clone());
        }
    }

    fn pointer(&self) -> Option<Pos2> {
        self.response.hover_pos()
    }

    fn pine_tooltip(&mut self, target: &HoverTarget) {
        // The canvas is one response; the tooltip is asked of a copy that covers the label or
        // cell, so egui places it there and keeps its own delay and "only when still" rules.
        let mut over = self.response.clone();
        over.rect = target.rect;
        over.interact_rect = target.rect;
        over.on_hover_ui(|ui| {
            ui.set_max_width(PINE_TOOLTIP_WIDTH);
            ui.label(kit::text(
                target.text.clone(),
                12.0,
                kit::REGULAR,
                theme::text_primary(),
            ));
        });
    }

    fn pane(&mut self, painter: &egui::Painter, layout: &FrameLayout, slot: &PaneSlot) {
        let hovered = self.ui.rect_contains_pointer(slot.rect);
        paint_pane_title(painter, layout, slot, hovered);
        if let Some(action) = chart_buttons::pane(
            self.ui,
            egui::Id::new(("pane-buttons", self.chart_id, slot.id)),
            slot.rect,
            layout.plot.right(),
            &layout.style,
            chart_buttons::PaneState {
                can_move_up: slot.index > 0,
                can_move_down: slot.index + 1 < slot.count,
                collapsed: slot.collapsed,
                maximized: slot.maximized,
            },
            hovered,
        ) {
            self.pane_actions.push((slot.id, action));
        }
    }

    fn finish(
        &mut self,
        _data: &FrameData,
        layout: &FrameLayout,
        view: &View,
        overlays: &OverlayFrame,
    ) {
        let (plot, scale) = (layout.plot, &view.scale);
        let alerts = alert_lines::show(
            self.ui,
            plot,
            scale,
            &self.card.alert_lines,
            self.card.appearance.price_format(),
            self.card.alert_market,
            &self.card.alert_open,
        );
        if alerts.menu.is_some() {
            self.alert_menu = alerts.menu;
        }
        self.alert_moves.extend(alerts.moves);
        self.alert_toggled.extend(alerts.toggled);
        self.alert_deleted.extend(alerts.deleted);
        self.paper = self
            .card
            .paper
            .show_markers(self.ui, self.card.below_header(plot), scale);
        if !self.replay.selecting
            && let Some(action) = chart_buttons::navigation(
                self.ui,
                egui::Id::new(("nav-buttons", self.chart_id)),
                plot,
                &layout.style,
                self.chart_hovered,
            )
        {
            self.navigation = Some(action);
        }
        let card = self.card.overlay_card(
            self.ui,
            self.chart_id,
            self.card.below_header(plot),
            view,
            overlays,
        );
        self.overlay_action = card.action;
        self.on_overlay_card = card.on_card;
        // The scale menu sits where the axes meet, which a chart with no plot to draw has not got.
        if layout.main_visible
            && self.card.scale_menu
            && let Some(cell) = scale_menu::corner(plot, layout.rect)
        {
            let state = scale_menu::ScaleButton {
                mode: scale.mode,
                logarithmic_drawn: scale.logarithmic,
            };
            if let Some(mode) =
                scale_menu::show(self.ui, scale_menu::id(self.chart_id), cell, &state)
            {
                self.scale_mode = Some(mode);
            }
        }
    }
}

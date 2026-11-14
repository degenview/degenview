//! The chart area: the preview chart itself, its empty and loading messages, and the stale and
//! error banners over it.

use super::{PreviewState, REFRESH_INTERVAL};
use crate::model::chart::price_scale::PriceScaleMode;
use crate::model::pine::script::ScriptType;
use crate::model::{DataSource, IndicatorConfig};
use crate::pine_studio::commands::Cmd;
use crate::pine_studio::workspace::PreviewInput;
use crate::service::keychain::api_keys::alpaca_configured;
use crate::ui::icons::{self, Icon};
use crate::ui::{DrawingFrame, ReplayChartFrame, theme};
use eframe::egui::{
    self, Align, Color32, CursorIcon, FontId, Id, Layout, Rect, RichText, Sense, Stroke, Ui, Vec2,
    pos2, vec2,
};
use std::cell::RefCell;

pub fn chart(
    ui: &mut Ui,
    preview: &mut PreviewState,
    input: &PreviewInput,
    cmds: &RefCell<Vec<Cmd>>,
) {
    let area = ui.max_rect();
    ui.painter().rect_filled(area, 0.0, theme::background());
    preview.source_changed(input);
    preview.maybe_apply(input);
    if std::mem::take(&mut preview.pending_settings) {
        cmds.borrow_mut().push(Cmd::OpenAlpacaSettings);
    }

    if input.script_type == ScriptType::Library {
        message(
            ui,
            area,
            Icon::Books,
            "Nothing to chart",
            "Libraries export functions — they don't draw on a chart.",
            None,
            cmds,
        );
        return;
    }
    if preview.market.source == DataSource::Alpaca && !alpaca_configured() {
        message(
            ui,
            area,
            Icon::Key,
            "Alpaca isn't set up",
            "Stock candles come from Alpaca. Add your keys in Settings to test on stocks.",
            Some(("Open Settings", Cmd::OpenAlpacaSettings)),
            cmds,
        );
        return;
    }

    let ctx = ui.ctx().clone();
    if preview.want_fetch() {
        preview.fetch(&ctx);
    } else if preview.inbox.is_none()
        && preview
            .last_fetch
            .is_some_and(|at| at.elapsed() >= REFRESH_INTERVAL)
    {
        // Quiet refresh: the candles stay up while the new ones load.
        preview.fetched_for = None;
        preview.fetch(&ctx);
        preview.loading = false;
    }
    ctx.request_repaint_after(REFRESH_INTERVAL);

    let inner = area.shrink(6.0);
    let mut chart_ui = ui.new_child(egui::UiBuilder::new().max_rect(inner));
    chart_ui.set_clip_rect(inner.intersect(ui.clip_rect()));
    let drawings: Vec<crate::model::Drawing> = Vec::new();
    let mut frame = DrawingFrame::new(&drawings, crate::model::Tool::default(), None, 1);
    let mut replay = ReplayChartFrame {
        selecting: preview.picking_bar || preview.picking_range,
        active: false,
        picked_ms: None,
        picked_range: None,
    };
    let candles = std::mem::take(&mut preview.candles);
    let symbol = preview.market.ticker.clone();
    let source = preview.market.source;
    let loading = preview.loading && candles.is_empty();
    let error = preview.error.clone();
    preview.card.body(
        &mut chart_ui,
        &symbol,
        &candles,
        loading,
        if candles.is_empty() {
            error.as_ref()
        } else {
            None
        },
        PriceScaleMode::Regular,
        source,
        &IndicatorConfig::default(),
        None,
        u64::MAX,
        &mut frame,
        &mut replay,
    );
    preview.candles = candles;
    if preview.picking_bar {
        if let Some(picked) = replay.picked_ms {
            preview.picked_bar = Some(picked);
            preview.cancel_bar_pick();
        } else if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
            preview.cancel_bar_pick();
        }
    }

    if preview.picking_range {
        if let Some(range) = replay.picked_range {
            preview.picked_range = Some(range);
            preview.cancel_range_pick();
            ctx.request_repaint();
        } else if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
            preview.cancel_range_pick();
        }
    }

    // The stale banner, over the top of the chart.
    if preview.stale {
        let rect = stale_banner(ui, area);
        let response = ui.interact(rect, Id::new("preview-stale"), Sense::click());
        if response.clicked() {
            cmds.borrow_mut().push(Cmd::SetDrawerTab(
                crate::pine_studio::library::prefs::DrawerTab::Problems,
            ));
        }
        response
            .on_hover_cursor(CursorIcon::PointingHand)
            .on_hover_text("Show problems");
    }
    // Refresh failures with candles on screen: a retry banner at the bottom.
    if let Some(error) = error.filter(|_| !preview.candles.is_empty())
        && error_banner(ui, area, &error.to_string())
    {
        preview.fetched_for = None;
    }
}

pub(super) fn message(
    ui: &mut Ui,
    area: Rect,
    icon: Icon,
    title: &str,
    detail: &str,
    action: Option<(&str, Cmd)>,
    cmds: &RefCell<Vec<Cmd>>,
) {
    let mut column = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(Rect::from_center_size(
                area.center(),
                vec2(260.0_f32.min(area.width() - 20.0), 200.0),
            ))
            .layout(Layout::top_down(Align::Center)),
    );
    column.spacing_mut().item_spacing.y = 8.0;
    icons::inline(&mut column, icon, 22.0, theme::text_secondary());
    column.label(RichText::new(title).size(15.0).color(theme::text_primary()));
    column.label(
        RichText::new(detail)
            .size(10.0)
            .color(theme::text_secondary()),
    );
    if let Some((label, cmd)) = action
        && column.add(egui::Button::new(label)).clicked()
    {
        cmds.borrow_mut().push(cmd);
    }
}

fn stale_banner(ui: &mut Ui, area: Rect) -> Rect {
    let text = "Fix the errors to update the chart — showing the last working version";
    let font = FontId::proportional(10.0);
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font, theme::text_primary());
    let size = vec2(galley.size().x + 20.0 + 18.0, 24.0);
    let rect = Rect::from_center_size(pos2(area.center().x, area.min.y + 8.0 + size.y / 2.0), size);
    let painter = ui.painter();
    painter.rect_filled(rect, 12.0, Color32::from_rgba_unmultiplied(40, 48, 52, 235));
    painter.rect_stroke(
        rect,
        12.0,
        Stroke::new(1.0, theme::with_alpha(theme::syntax::string(), 0.5)),
        egui::StrokeKind::Inside,
    );
    icons::paint(
        painter,
        Rect::from_center_size(pos2(rect.min.x + 14.0, rect.center().y), Vec2::splat(11.0)),
        Icon::XmarkOctagonFill,
        theme::syntax::string(),
    );
    painter.galley(
        pos2(rect.min.x + 26.0, rect.center().y - galley.size().y / 2.0),
        galley,
        theme::text_primary(),
    );
    rect
}

/// Returns whether Retry was clicked.
fn error_banner(ui: &mut Ui, area: Rect, message: &str) -> bool {
    let font = FontId::proportional(9.0);
    let galley =
        ui.painter()
            .layout_no_wrap(message.to_owned(), font.clone(), theme::text_secondary());
    let retry = ui
        .painter()
        .layout_no_wrap("Retry".to_owned(), font, theme::SYSTEM_ACCENT);
    let width =
        (galley.size().x + retry.size().x + 8.0 + 8.0 + 16.0 + 14.0).min(area.width() - 16.0);
    let rect = Rect::from_min_size(
        pos2(area.min.x + 8.0, area.max.y - 8.0 - 22.0),
        vec2(width, 22.0),
    );
    let painter = ui.painter();
    painter.rect_filled(rect, 4.0, Color32::from_rgba_unmultiplied(40, 48, 52, 235));
    icons::paint(
        painter,
        Rect::from_center_size(pos2(rect.min.x + 12.0, rect.center().y), Vec2::splat(11.0)),
        Icon::TriangleFill,
        theme::ORANGE,
    );
    painter.galley(
        pos2(rect.min.x + 24.0, rect.center().y - galley.size().y / 2.0),
        galley.clone(),
        theme::text_secondary(),
    );
    let retry_rect = Rect::from_min_size(
        pos2(rect.min.x + 24.0 + galley.size().x + 8.0, rect.min.y),
        vec2(retry.size().x + 6.0, 22.0),
    );
    painter.galley(
        pos2(retry_rect.min.x, rect.center().y - retry.size().y / 2.0),
        retry,
        theme::SYSTEM_ACCENT,
    );
    ui.interact(retry_rect, Id::new("preview-retry"), Sense::click())
        .on_hover_cursor(CursorIcon::PointingHand)
        .clicked()
}

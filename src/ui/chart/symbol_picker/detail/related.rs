//! "Also trades as": the other markets of the shown asset, in columns of artwork, name, how it
//! trades and where, with a hairline between rows.

use super::super::list::result_label;
use super::super::search::trading_kind;
use super::layout::RELATED_ROW;
use crate::i18n::tr;
use crate::service::SearchResult;
use crate::ui::chart::picker_kit::artwork::result_icon;
use crate::ui::kit::scroll::Beyond;
use crate::ui::kit::{self, MEDIUM, REGULAR, SEMIBOLD};
use crate::ui::{kit::source_logo, theme};
use eframe::egui::{self, Align, Layout, Rect, Sense, Stroke, Ui, UiBuilder, Vec2, pos2, vec2};

const ICON: f32 = 20.0;
const KIND: f32 = 46.0;
const VENUE: f32 = 90.0;
const LOGO: f32 = 14.0;

/// Draws the list into `room`, whose top is the chart's foot; the hairline above it spans
/// `panel`. Returns the market clicked.
pub fn show(
    ui: &mut Ui,
    panel: Rect,
    room: Rect,
    related: &[SearchResult],
) -> Option<SearchResult> {
    let mut y = room.top() + 12.0;
    ui.painter()
        .hline(panel.x_range(), y, Stroke::new(1.0, theme::separator()));
    y += 10.0;
    let heading = kit::galley(
        ui,
        tr!("ALSO TRADES AS"),
        11.5,
        SEMIBOLD,
        theme::text_tertiary(),
        room.width(),
    );
    ui.painter()
        .galley(pos2(room.left(), y), heading, theme::text_tertiary());
    y += 22.0;

    // The rows scroll inside what is left instead of running into the button.
    let rows = Rect::from_min_max(
        pos2(room.left() - 6.0, y),
        pos2(panel.right() - 4.0, room.bottom()),
    );
    let mut list = ui.new_child(
        UiBuilder::new()
            .max_rect(rows)
            .layout(Layout::top_down(Align::Min)),
    );
    list.set_clip_rect(rows.intersect(ui.clip_rect()));
    kit::scroll::keep_bars_visible(&mut list);
    let width = room.width() + 12.0;
    let mut clicked = None;
    let scrolled = egui::ScrollArea::vertical()
        .id_salt("symbol_picker_related")
        .auto_shrink([false, false])
        .show(&mut list, |list| {
            list.spacing_mut().item_spacing = Vec2::ZERO;
            for (index, hit) in related.iter().enumerate() {
                if row(list, hit, width, index + 1 < related.len()) {
                    clicked = Some(hit.clone());
                }
            }
        });
    let viewport = scrolled.inner_rect;
    let beyond = Beyond::of(
        scrolled.state.offset.y,
        scrolled.content_size.y,
        viewport.height(),
    );
    kit::scroll::fade_edges(ui.painter(), viewport, theme::chrome(), beyond);
    clicked
}

/// One other market. Returns whether it was clicked.
fn row(ui: &mut Ui, result: &SearchResult, width: f32, separated: bool) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(width, RELATED_ROW), Sense::click());
    let response = crate::ui::accessibility::label(
        response.on_hover_cursor(egui::CursorIcon::PointingHand),
        result_label(result),
    );
    if !ui.is_rect_visible(rect) {
        return response.clicked();
    }
    if response.hovered() {
        ui.painter().rect_filled(rect, 8.0, theme::ink(10));
    }
    let icon = Rect::from_center_size(
        pos2(rect.left() + 6.0 + ICON / 2.0, rect.center().y),
        Vec2::splat(ICON),
    );
    result_icon(ui, icon, result);

    let venue_left = rect.right() - 6.0 - VENUE;
    let kind_left = venue_left - KIND;
    let name_left = icon.right() + 10.0;
    let text = |ui: &Ui, left: f32, width: f32, text: &str, weight: f32, color| {
        let galley = kit::galley(ui, text, 13.0, weight, color, width.max(24.0));
        ui.painter().galley(
            pos2(left, rect.center().y - galley.size().y / 2.0),
            galley,
            color,
        );
    };
    text(
        ui,
        name_left,
        kind_left - 8.0 - name_left,
        result.ticker(),
        SEMIBOLD,
        theme::text_primary(),
    );
    text(
        ui,
        kind_left,
        KIND - 8.0,
        trading_kind(result.source),
        REGULAR,
        theme::text_secondary(),
    );
    let logo = Rect::from_center_size(
        pos2(venue_left + LOGO / 2.0, rect.center().y),
        Vec2::splat(LOGO),
    );
    source_logo::paint(ui, result.source, logo);
    text(
        ui,
        logo.right() + 6.0,
        VENUE - LOGO - 6.0,
        result.source.display_name(),
        MEDIUM,
        theme::text_secondary(),
    );
    if separated {
        ui.painter().hline(
            (rect.left() + 6.0)..=(rect.right() - 6.0),
            rect.bottom() - 0.5,
            Stroke::new(1.0, theme::separator()),
        );
    }
    response.clicked()
}

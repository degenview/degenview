//! The detail card under the rows: the selected market's price, tiles, lists and note.

use super::{WatchlistSidebarInput, WatchlistSidebarState};

use crate::model::watchlist;
use crate::model::watchlist::quote::WatchlistQuote;
use crate::model::watchlist::quote::{Direction, Freshness};
use crate::model::watchlist::{Watchlist, WatchlistFlag};
use crate::ui::icons::{self, Icon};
use crate::ui::kit;
use crate::ui::kit::source_logo;
use crate::ui::theme;
use crate::ui::watchlist::style;
use eframe::egui::{self, Color32, Id, Pos2, Rect, Sense, Vec2};

const DETAIL_PAD: f32 = 12.0;
const DETAIL_HEADER: f32 = 34.0;
const DETAIL_HERO: f32 = 52.0;
const DETAIL_TILE: f32 = 40.0;
const DETAIL_TILE_GAP: f32 = 6.0;
const DETAIL_LISTS: f32 = 22.0;
const DETAIL_NOTE: f32 = 28.0;
/// Space between the card's bands.
const DETAIL_GAP: f32 = 10.0;

/// The card's height: its bands are fixed, so only a provider's note changes it.
pub(super) fn detail_height(quote: Option<&WatchlistQuote>) -> f32 {
    let note = quote.is_some_and(|quote| quote.note.is_some());
    DETAIL_PAD * 2.0
        + DETAIL_HEADER
        + DETAIL_HERO
        + DETAIL_TILE * 2.0
        + DETAIL_TILE_GAP
        + DETAIL_LISTS
        + DETAIL_GAP * 3.0
        + if note { DETAIL_NOTE + DETAIL_GAP } else { 0.0 }
}

/// Paints `text` as a capsule centred on `center_y` with its right edge at `right`, and returns
/// the capsule's rect.
fn detail_pill(
    ui: &egui::Ui,
    right: f32,
    center_y: f32,
    text: &str,
    ink: Color32,
    fill: Color32,
    swatch: Option<WatchlistFlag>,
) -> Rect {
    let galley = kit::galley(ui, text, 11.0, kit::MEDIUM, ink, f32::INFINITY);
    let lead = if swatch.is_some() { 16.0 } else { 0.0 };
    let size = Vec2::new(galley.size().x + 16.0 + lead, 20.0);
    let rect = Rect::from_min_size(Pos2::new(right - size.x, center_y - size.y / 2.0), size);
    ui.painter().rect_filled(rect, size.y / 2.0, fill);
    if let Some(flag) = swatch {
        style::paint_swatch(
            ui.painter(),
            Rect::from_center_size(Pos2::new(rect.left() + 14.0, center_y), Vec2::splat(12.0)),
            flag,
        );
    }
    ui.painter().galley(
        Pos2::new(rect.left() + 8.0 + lead, center_y - galley.size().y / 2.0),
        galley,
        ink,
    );
    rect
}

/// One labelled fact in a tile; `dot` puts a status light before the value. Hovering shows the
/// whole value when it was cut short.
fn detail_tile(
    ui: &mut egui::Ui,
    rect: Rect,
    id: Id,
    label: &str,
    value: &str,
    dot: Option<Color32>,
) {
    ui.painter().rect_filled(rect, 8.0, theme::ink(8));
    let caption = kit::galley(
        ui,
        &label.to_uppercase(),
        9.5,
        kit::SEMIBOLD,
        theme::text_tertiary(),
        rect.width() - 20.0,
    );
    ui.painter().galley(
        Pos2::new(rect.left() + 10.0, rect.top() + 7.0),
        caption,
        theme::text_tertiary(),
    );
    let lead = if dot.is_some() { 12.0 } else { 0.0 };
    let color = if value == watchlist::quote::MISSING {
        theme::text_tertiary()
    } else {
        theme::text_primary()
    };
    let text = kit::galley(
        ui,
        value,
        12.5,
        kit::REGULAR,
        color,
        rect.width() - 20.0 - lead,
    );
    let at = Pos2::new(rect.left() + 10.0 + lead, rect.top() + 21.0);
    if let Some(dot) = dot {
        ui.painter().circle_filled(
            Pos2::new(rect.left() + 13.5, at.y + text.size().y / 2.0),
            3.0,
            dot,
        );
    }
    ui.painter().galley(at, text, color);
    ui.interact(rect, id, Sense::hover())
        .on_hover_text(format!("{label}: {value}"));
}

fn freshness_color(freshness: Freshness) -> Color32 {
    match freshness {
        Freshness::Live => theme::UP,
        Freshness::Stale => theme::ORANGE,
        Freshness::MarketClosed => theme::text_secondary(),
        Freshness::Unavailable | Freshness::Unsupported => theme::text_tertiary(),
    }
}

/// Facts about the highlighted market, from what the quote provider reports; a field it does not
/// supply is a dash.
pub(super) fn detail_panel(
    ui: &mut egui::Ui,
    state: &WatchlistSidebarState,
    input: &WatchlistSidebarInput,
    list: &Watchlist,
) {
    let Some(item) = state
        .selected_entry
        .and_then(|id| list.instruments().find(|item| item.id == id))
    else {
        return;
    };
    let quote = (input.quote)(&item.instrument);
    let quote = quote.as_ref();
    let source = item.instrument.source;
    let (card, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), detail_height(quote)),
        Sense::hover(),
    );
    ui.painter().rect_filled(card, 0.0, theme::chrome());
    let (left, right) = (card.left() + DETAIL_PAD, card.right() - DETAIL_PAD);
    let width = right - left;
    let mut y = card.top() + DETAIL_PAD;

    // Who: the provider's logo on a plate, the market's name, and its flag.
    let header_mid = y + DETAIL_HEADER / 2.0;
    let plate = Rect::from_center_size(Pos2::new(left + 17.0, header_mid), Vec2::splat(34.0));
    ui.painter().rect_filled(plate, 9.0, theme::ink(10));
    source_logo::paint(
        ui,
        source,
        Rect::from_center_size(plate.center(), Vec2::splat(22.0)),
    );
    let mut text_right = right;
    if let Some(flag) = input.flags.get(&item.instrument.key()) {
        let pill = detail_pill(
            ui,
            right,
            header_mid,
            flag.title(),
            theme::text_secondary(),
            theme::ink(10),
            Some(*flag),
        );
        text_right = pill.left() - 8.0;
    }
    let text_left = plate.right() + 10.0;
    let name = kit::galley(
        ui,
        &item.name,
        14.0,
        kit::SEMIBOLD,
        theme::text_primary(),
        text_right - text_left,
    );
    let subtitle = kit::galley(
        ui,
        source.display_name(),
        11.0,
        kit::REGULAR,
        theme::text_secondary(),
        text_right - text_left,
    );
    let block = name.size().y + subtitle.size().y;
    ui.painter().galley(
        Pos2::new(text_left, header_mid - block / 2.0),
        name.clone(),
        theme::text_primary(),
    );
    ui.painter().galley(
        Pos2::new(text_left, header_mid - block / 2.0 + name.size().y),
        subtitle,
        theme::text_secondary(),
    );
    y += DETAIL_HEADER + DETAIL_GAP;

    // What it is worth: the price large, how it moved beside it.
    let direction = watchlist::quote::direction(quote);
    let move_color = match direction {
        Direction::Up => theme::UP,
        Direction::Down => theme::DOWN,
        Direction::Flat => theme::text_secondary(),
    };
    let last = watchlist::quote::last_text(quote, Some(source));
    let percent = watchlist::quote::percent_text(quote, Some(source));
    let pill_text = if direction == Direction::Flat {
        percent
    } else {
        format!("{} {percent}", direction.glyph())
    };
    let hero_top = y;
    let pill = detail_pill(
        ui,
        right,
        hero_top + 15.0,
        &pill_text,
        move_color,
        theme::with_alpha(move_color, 0.14),
        None,
    );
    let price = kit::digits(
        ui,
        &last,
        24.0,
        kit::SEMIBOLD,
        theme::text_primary(),
        (pill.left() - 8.0 - left).max(40.0),
    );
    ui.painter().galley(
        Pos2::new(left, hero_top),
        price.clone(),
        theme::text_primary(),
    );
    let change = watchlist::quote::change_text(quote, Some(source));
    let change_galley = kit::galley(ui, &change, 12.0, kit::MEDIUM, move_color, width);
    let change_at = Pos2::new(left, hero_top + DETAIL_HERO - 18.0);
    let after = change_at.x + change_galley.size().x;
    ui.painter().galley(change_at, change_galley, move_color);
    if let Some(basis) = quote.and_then(|quote| quote.change_basis) {
        let basis = kit::galley(
            ui,
            &format!("· {}", basis.label()),
            11.0,
            kit::REGULAR,
            theme::text_tertiary(),
            (right - after - 6.0).max(10.0),
        );
        ui.painter().galley(
            Pos2::new(after + 6.0, change_at.y + 1.0),
            basis,
            theme::text_tertiary(),
        );
    }
    y += DETAIL_HERO + DETAIL_GAP;

    // The facts, two to a row.
    let freshness = quote.map_or(Freshness::Unavailable, |quote| quote.freshness);
    let updated = quote.and_then(|quote| quote.timestamp_ms).map_or_else(
        || watchlist::quote::MISSING.to_owned(),
        |timestamp| {
            crate::model::paper::format::timestamp(
                timestamp,
                crate::service::system::timezone::local_utc_offset_secs(timestamp),
            )
        },
    );
    let symbol = format!("{}:{}", source.watchlist_slug(), item.instrument.symbol);
    let volume = watchlist::quote::volume_text(quote);
    let tile_width = (width - DETAIL_TILE_GAP) / 2.0;
    let tiles = [
        ("Volume", volume.as_str(), None),
        (
            "Status",
            freshness.label(),
            Some(freshness_color(freshness)),
        ),
        ("Symbol", symbol.as_str(), None),
        ("Updated", updated.as_str(), None),
    ];
    for (index, (label, value, dot)) in tiles.into_iter().enumerate() {
        let (column, row) = (index % 2, index / 2);
        let rect = Rect::from_min_size(
            Pos2::new(
                left + column as f32 * (tile_width + DETAIL_TILE_GAP),
                y + row as f32 * (DETAIL_TILE + DETAIL_TILE_GAP),
            ),
            Vec2::new(tile_width, DETAIL_TILE),
        );
        detail_tile(
            ui,
            rect,
            Id::new(("watchlist-detail-tile", list.id, label)),
            label,
            value,
            dot,
        );
    }
    y += DETAIL_TILE * 2.0 + DETAIL_TILE_GAP + DETAIL_GAP;

    // Where else it lives.
    let names: Vec<&str> = input
        .lists
        .iter()
        .filter(|list| {
            list.instruments()
                .any(|candidate| candidate.instrument.is_same_market(&item.instrument))
        })
        .map(|list| list.name.as_str())
        .collect();
    let row_mid = y + DETAIL_LISTS / 2.0;
    let caption = kit::galley(
        ui,
        "IN LISTS",
        9.5,
        kit::SEMIBOLD,
        theme::text_tertiary(),
        width,
    );
    ui.painter().galley(
        Pos2::new(left, row_mid - caption.size().y / 2.0),
        caption.clone(),
        theme::text_tertiary(),
    );
    let mut x = left + caption.size().x + 10.0;
    if names.is_empty() {
        let none = kit::galley(
            ui,
            watchlist::quote::MISSING,
            12.0,
            kit::REGULAR,
            theme::text_tertiary(),
            20.0,
        );
        ui.painter().galley(
            Pos2::new(x, row_mid - none.size().y / 2.0),
            none,
            theme::text_tertiary(),
        );
    }
    for (index, name) in names.iter().enumerate() {
        let left_over = names.len() - index;
        let galley = kit::galley(
            ui,
            name,
            11.0,
            kit::MEDIUM,
            theme::text_secondary(),
            (right - x - 16.0).max(24.0),
        );
        let pill_width = galley.size().x + 16.0;
        // Keep room for a "+N" when the rest will not fit.
        let reserve = if left_over > 1 { 34.0 } else { 0.0 };
        if x + pill_width + reserve > right && index > 0 {
            detail_pill(
                ui,
                right.min(x + 34.0),
                row_mid,
                &format!("+{left_over}"),
                theme::text_tertiary(),
                theme::ink(8),
                None,
            );
            break;
        }
        let rect = detail_pill(
            ui,
            x + pill_width,
            row_mid,
            name,
            theme::text_secondary(),
            theme::ink(10),
            None,
        );
        x = rect.right() + 5.0;
    }
    y += DETAIL_LISTS + DETAIL_GAP;

    // What the provider said about the quote.
    if let Some(note) = quote.and_then(|quote| quote.note.as_ref()) {
        let rect = Rect::from_min_size(Pos2::new(left, y), Vec2::new(width, DETAIL_NOTE));
        ui.painter()
            .rect_filled(rect, 8.0, theme::with_alpha(theme::ORANGE, 0.12));
        icons::paint(
            ui.painter(),
            Rect::from_center_size(
                Pos2::new(rect.left() + 14.0, rect.center().y),
                Vec2::splat(12.0),
            ),
            Icon::InfoCircle,
            theme::ORANGE,
        );
        let text = kit::galley(
            ui,
            note,
            11.0,
            kit::REGULAR,
            theme::text_secondary(),
            rect.width() - 34.0,
        );
        ui.painter().galley(
            Pos2::new(rect.left() + 28.0, rect.center().y - text.size().y / 2.0),
            text,
            theme::text_secondary(),
        );
        ui.interact(
            rect,
            Id::new(("watchlist-detail-note", list.id)),
            Sense::hover(),
        )
        .on_hover_text(note);
    }
}

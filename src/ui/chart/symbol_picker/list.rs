//! The symbol picker's rows: one market per row on the list's shared columns: artwork, the
//! market's own name over the pair it is, what it trades, the exchange with its kind, and a
//! check for a market already on the tab or a plus to add it.

use super::columns::{self, Columns, END, ICON, MARK, PAD, VENUE_GAP, VENUE_LOGO};
use super::describe;
use super::search::market_kind;
use crate::i18n::tr;
use crate::model::DataSource;
use crate::model::market::asset_name::AssetNames;
use crate::service::SearchResult;
use crate::ui::chart::picker_kit::artwork::result_icon;
use crate::ui::icons::{self, Icon};
use crate::ui::kit::{self, MEDIUM, REGULAR, SEMIBOLD};
use crate::ui::{kit::source_logo, theme};
use eframe::egui::{self, Id, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};

pub const ROW_HEIGHT: f32 = 56.0;
pub const TITLE_SIZE: f32 = 15.0;
pub const SUBTITLE_SIZE: f32 = 13.0;
const DESCRIPTION_SIZE: f32 = 14.0;
/// The corner radius of the highlighted row's box.
const RADIUS: f32 = 10.0;

/// How a row is drawn this frame.
#[derive(Clone, Copy)]
pub struct RowState {
    /// The keyboard's (or the pointer's) row.
    pub highlighted: bool,
    /// The market is already on a chart of the tab (or in the watchlist being added to).
    pub on_chart: bool,
    /// The market is the focused chart's own or already compared on it.
    pub overlaid: bool,
    /// The focused chart can have a market drawn over it.
    pub can_overlay: bool,
    /// The picker is comparing: the check means "already compared", not "already a chart".
    pub comparing: bool,
    /// The picker is adding to a watchlist: the plus follows the market instead of charting it.
    pub watchlist: bool,
    /// How far the artwork is pushed in, so a lone prediction choice lines up with the artwork of
    /// the events around it.
    pub indent: f32,
}

impl RowState {
    /// What the plus (and the menu's last item) does to the market.
    fn add_label(self) -> &'static str {
        if self.watchlist {
            tr!("Add to watchlist")
        } else {
            tr!("Add as new chart")
        }
    }

    /// What the check at the right edge says.
    fn checked(self) -> bool {
        if self.comparing {
            self.overlaid
        } else {
            self.on_chart
        }
    }

    /// Whether the compare button shows: the chart takes overlays and this is not one already.
    fn offers_overlay(self) -> bool {
        self.can_overlay && !self.overlaid
    }
}

/// What a row asks for.
#[derive(Clone, Copy, Debug, Default)]
pub struct RowOutput {
    /// The row was clicked: use the market.
    pub pick: bool,
    /// The plus was clicked: add the market as a chart of its own.
    pub add: bool,
    /// The compare button was clicked: draw the market over the focused chart.
    pub overlay: bool,
    pub hovered: bool,
    /// The row was right-clicked: its menu is opening, and the row becomes the highlighted one.
    pub opened_menu: bool,
    /// The row's checkbox was clicked: tick or untick the choice (or every choice of an event).
    pub tick: bool,
    /// An event's header was clicked: show or hide its choices.
    pub fold: bool,
}

/// The first line of a row: the market's own name. A prediction market's name is a sentence,
/// which may hold the company separator.
pub fn name(result: &SearchResult) -> &str {
    if result.source.is_prediction_market() {
        &result.symbol
    } else {
        result.ticker()
    }
}

/// What the row's right-click menu offers: `Replace <chart's market>` while a chart is focused, and
/// always `Add as new chart`. The shortcuts are the keys that do the same.
pub const REPLACE_KEY: &str = "↵";
pub const ADD_KEY: &str = "⇧↵";
pub const COMPARE_KEY: &str = "⌥↵";

/// Draws one row on `columns`. `replacing` is the market on the focused chart, which a pick would
/// replace; `None` when there is no chart to replace.
pub fn row(
    ui: &mut Ui,
    result: &SearchResult,
    names: &AssetNames,
    state: RowState,
    replacing: Option<&str>,
    columns: &Columns,
) -> (Response, RowOutput) {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, ROW_HEIGHT), Sense::click());
    let mut out = RowOutput {
        hovered: response.hovered(),
        ..RowOutput::default()
    };
    let label = format!(
        "{}, {}, {}",
        result.symbol,
        result.source.display_name(),
        market_kind(result.source)
    );
    let response = crate::ui::accessibility::label(response, label);
    out.opened_menu = response.secondary_clicked();
    context_menu(ui, &response, replacing, state, &mut out);
    if !ui.is_rect_visible(rect) {
        return (response, out);
    }
    paint_background(ui, rect, state.highlighted, response.hovered());
    marks(ui, rect, result, state, &mut out);
    out.pick |= response.clicked() && !out.add && !out.overlay;

    let icon = Rect::from_center_size(
        pos2(
            rect.left() + PAD + state.indent + ICON / 2.0,
            rect.center().y,
        ),
        Vec2::splat(ICON),
    );
    result_icon(ui, icon, result);
    venue_block(ui, rect, columns, result.source);

    // A prediction market's name is a sentence: it takes the description's column too, and its
    // chance stands before the exchange.
    let text_left = rect.left() + columns.symbol + state.indent;
    if result.source.is_prediction_market() {
        let mut right = rect.left() + columns.title_width() + columns.symbol;
        if let Some(chance) = result.price {
            right = chance_label(ui, rect, right, chance) - 16.0;
        }
        two_lines(
            ui,
            rect,
            text_left,
            (right - text_left).max(40.0),
            name(result),
            &describe::pair_line(result),
        );
        return (response, out);
    }

    let described = describe::column(result, names);
    let pair = describe::pair_line(result);
    // Without a column of its own the description is the second line.
    let under = match columns.description {
        Some(_) => pair,
        None if described.is_empty() => pair,
        None => described.clone(),
    };
    two_lines(
        ui,
        rect,
        text_left,
        columns.symbol_width - state.indent,
        name(result),
        &under,
    );
    if let Some(left) = columns.description
        && !described.is_empty()
    {
        let galley = kit::galley(
            ui,
            &described,
            DESCRIPTION_SIZE,
            REGULAR,
            theme::text_primary(),
            columns.description_width,
        );
        ui.painter().galley(
            pos2(rect.left() + left, rect.center().y - galley.size().y / 2.0),
            galley,
            theme::text_primary(),
        );
    }
    (response, out)
}

/// A bold title over a quieter line, the pair of them centred in `rect`; a title alone is
/// centred by itself.
pub fn two_lines(ui: &Ui, rect: Rect, left: f32, width: f32, title: &str, under: &str) {
    let area = Rect::from_min_max(pos2(left, rect.top()), pos2(left + width, rect.bottom()));
    two_lines_sized(ui, area, (title, TITLE_SIZE, SEMIBOLD), under);
}

/// [`two_lines`] filling `area`, with the title's size and weight chosen.
pub fn two_lines_sized(ui: &Ui, area: Rect, (title, size, weight): (&str, f32, f32), under: &str) {
    let width = area.width();
    let title = kit::galley(ui, title, size, weight, theme::text_primary(), width);
    let under = (!under.is_empty()).then(|| {
        kit::galley(
            ui,
            under,
            SUBTITLE_SIZE,
            REGULAR,
            theme::text_secondary(),
            width,
        )
    });
    let block = title.size().y + under.as_ref().map_or(0.0, |under| under.size().y + 2.0);
    let top = area.center().y - block / 2.0;
    let under_top = top + title.size().y + 2.0;
    ui.painter()
        .galley(pos2(area.left(), top), title, theme::text_primary());
    if let Some(under) = under {
        ui.painter()
            .galley(pos2(area.left(), under_top), under, theme::text_secondary());
    }
}

/// The box around the keyboard's row, or the fill behind the pointer's.
pub fn paint_background(ui: &Ui, rect: Rect, highlighted: bool, hovered: bool) {
    let rect = rect.shrink2(vec2(0.0, 1.0));
    if highlighted {
        ui.painter()
            .rect_filled(rect, RADIUS, theme::selected_tint(false));
        ui.painter()
            .rect_stroke(rect, RADIUS, theme::selected_stroke(), StrokeKind::Inside);
    } else if hovered {
        ui.painter().rect_filled(rect, RADIUS, theme::ink(10));
    }
}

/// The hairline under a row of `rect`.
pub fn paint_separator(ui: &Ui, rect: Rect) {
    ui.painter().hline(
        (rect.left() + PAD)..=(rect.right() - END),
        rect.bottom() - 0.5,
        Stroke::new(1.0, theme::separator()),
    );
}

/// The compare button, the plus and the check at a row's right edge, as `state` shows them.
pub fn marks(ui: &mut Ui, rect: Rect, result: &SearchResult, state: RowState, out: &mut RowOutput) {
    let mark = Rect::from_center_size(
        pos2(rect.right() - END - MARK / 2.0, rect.center().y),
        Vec2::splat(MARK),
    );
    // The compare button sits left of the plus, a target of its own over the row's.
    let compare_mark = mark.translate(vec2(-MARK - 2.0, 0.0));
    if state.highlighted && state.offers_overlay() {
        let compare = ui.interact(compare_mark, row_id(ui, result, "compare"), Sense::click());
        let compare = compare
            .on_hover_text(tr!("Compare on this chart"))
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let hot = compare.hovered();
        if hot {
            ui.painter().rect_filled(compare_mark, 7.0, theme::ink(18));
        }
        icons::paint(
            ui.painter(),
            Rect::from_center_size(compare_mark.center(), Vec2::splat(15.0)),
            Icon::Compare,
            if hot {
                theme::text_primary()
            } else {
                theme::text_secondary()
            },
        );
        out.overlay |= compare.clicked();
    }
    // The plus is its own target, over the row's.
    if state.highlighted && !state.checked() {
        let plus = ui.interact(mark, row_id(ui, result, "add"), Sense::click());
        let plus = plus
            .on_hover_text(state.add_label())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let hot = plus.hovered();
        if hot {
            ui.painter().rect_filled(mark, 7.0, theme::ink(18));
        }
        icons::paint(
            ui.painter(),
            Rect::from_center_size(mark.center(), Vec2::splat(14.0)),
            Icon::Plus,
            if hot {
                theme::text_primary()
            } else {
                theme::text_secondary()
            },
        );
        out.add |= plus.clicked();
    } else if state.checked() {
        icons::paint(
            ui.painter(),
            Rect::from_center_size(mark.center(), Vec2::splat(15.0)),
            Icon::Check,
            theme::UP,
        );
    }
}

/// The venue column of a row: the exchange's logo and name over what it lists.
pub fn venue_block(ui: &Ui, rect: Rect, columns: &Columns, source: DataSource) {
    venue_lines(
        ui,
        rect,
        columns,
        source.display_name(),
        market_kind(source),
    );
    let logo = Rect::from_center_size(
        pos2(
            rect.left() + columns.venue + VENUE_LOGO / 2.0,
            rect.center().y,
        ),
        Vec2::splat(VENUE_LOGO),
    );
    source_logo::paint(ui, source, logo);
}

/// The two lines of a venue column, beside the room its logo takes.
pub fn venue_lines(ui: &Ui, rect: Rect, columns: &Columns, name: &str, kind: &str) {
    let name = kit::galley(
        ui,
        name,
        columns::VENUE_SIZE,
        MEDIUM,
        theme::text_primary(),
        columns.venue_width,
    );
    let kind = kit::galley(
        ui,
        kind,
        columns::VENUE_KIND_SIZE,
        REGULAR,
        theme::text_secondary(),
        columns.venue_width,
    );
    let left = rect.left() + columns.venue + VENUE_LOGO + VENUE_GAP;
    let top = rect.center().y - (name.size().y + kind.size().y + 1.0) / 2.0;
    let kind_top = top + name.size().y + 1.0;
    ui.painter()
        .galley(pos2(left, top), name, theme::text_primary());
    ui.painter()
        .galley(pos2(left, kind_top), kind, theme::text_secondary());
}

/// A prediction market's YES chance, right-aligned to `right`. Returns its left edge.
pub fn chance_label(ui: &Ui, rect: Rect, right: f32, chance: f64) -> f32 {
    let galley = kit::digits(
        ui,
        &crate::format::headline(chance, true),
        TITLE_SIZE,
        SEMIBOLD,
        theme::text_primary(),
        90.0,
    );
    let left = right - galley.size().x;
    ui.painter().galley(
        pos2(left, rect.center().y - galley.size().y / 2.0),
        galley,
        theme::text_primary(),
    );
    left
}

/// The menu a right-click on a row opens: use the market here, compare it on the chart, or add it
/// as a chart of its own. While comparing, Return compares, so Replace is not offered.
fn context_menu(
    ui: &mut Ui,
    response: &Response,
    replacing: Option<&str>,
    state: RowState,
    out: &mut RowOutput,
) {
    let replace = replacing
        .filter(|_| !state.comparing && !state.watchlist)
        .map(|current| tr!("Replace {market}", market = current));
    let compare = state.offers_overlay().then(|| match replacing {
        Some(current) => tr!("Compare with {market}", market = current),
        None => tr!("Compare").to_owned(),
    });
    let compare_key = if state.comparing {
        REPLACE_KEY
    } else {
        COMPARE_KEY
    };
    // A watchlist's own Return is the add, so the menu needs no second key for it.
    let add_key = if state.watchlist {
        Some(REPLACE_KEY)
    } else {
        Some(ADD_KEY)
    };
    let mut rows: Vec<(&str, Option<&str>)> = Vec::new();
    if let Some(replace) = &replace {
        rows.push((replace, Some(REPLACE_KEY)));
    }
    if let Some(compare) = &compare {
        rows.push((compare, Some(compare_key)));
    }
    rows.push((state.add_label(), add_key));
    kit::fitted_context_menu(ui, response, &rows, |ui| {
        if let Some(replace) = &replace
            && kit::menu_item_with_shortcut(
                ui,
                Icon::Exchange,
                replace,
                Some(REPLACE_KEY),
                true,
                false,
            )
        {
            out.pick = true;
            ui.close();
        }
        if let Some(compare) = &compare
            && kit::menu_item_with_shortcut(
                ui,
                Icon::Compare,
                compare,
                Some(compare_key),
                true,
                false,
            )
        {
            out.overlay = true;
            ui.close();
        }
        if kit::menu_item_with_shortcut(ui, Icon::Plus, state.add_label(), add_key, true, false) {
            out.add = true;
            ui.close();
        }
    });
}

pub fn row_id(ui: &Ui, result: &SearchResult, part: &str) -> Id {
    ui.id()
        .with((part, result.source as u8, result.full_symbol.as_str()))
}

/// What a screen reader calls a compact row of a market.
pub fn result_label(result: &SearchResult) -> String {
    tr!(
        "{symbol} on {venue}",
        symbol = result.symbol,
        venue = result.source.display_name()
    )
}

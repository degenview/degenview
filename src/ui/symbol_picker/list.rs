//! The symbol picker's rows: one market per row, laid out like the symbol search of the charting
//! sites the toolbar follows: artwork, the market's own name over what it trades, the exchange
//! with its kind at the right, and a check for a market already on the tab or a plus to add it.

use super::search::{market_kind, trading_kind};
use crate::model::DataSource;
use crate::service::SearchResult;
use crate::ui::add_chart_sheet::result_icon;
use crate::ui::icons::{self, Icon};
use crate::ui::kit::{self, MEDIUM, REGULAR, SEMIBOLD};
use crate::ui::{source_logo, theme};
use eframe::egui::{self, Id, Rect, Response, Sense, Ui, Vec2, pos2, vec2};

pub const ROW_HEIGHT: f32 = 56.0;
const ICON: f32 = 36.0;
const PAD: f32 = 12.0;
const LOGO: f32 = 18.0;
/// The column at the right edge that holds the check or the plus.
const MARK: f32 = 28.0;
const TITLE_SIZE: f32 = 15.0;
const SUBTITLE_SIZE: f32 = 13.0;
const VENUE_SIZE: f32 = 14.0;

/// How a row is drawn this frame.
#[derive(Clone, Copy)]
pub struct RowState {
    /// The keyboard's (or the pointer's) row.
    pub highlighted: bool,
    /// The market is already on a chart of the tab.
    pub on_chart: bool,
    /// The market is the focused chart's own or already compared on it.
    pub overlaid: bool,
    /// The focused chart can have a market drawn over it.
    pub can_overlay: bool,
    /// The picker is comparing: the check means "already compared", not "already a chart".
    pub comparing: bool,
}

impl RowState {
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
}

/// What the second line of a row says: the pair (`BTC / USDT`), the company, or the coin.
pub fn subtitle(result: &SearchResult) -> String {
    if let Some(company) = result.company() {
        return company.to_owned();
    }
    match (&result.base, &result.quote) {
        (Some(base), Some(quote)) => format!("{base} / {quote}"),
        _ if result.source == DataSource::CoinGecko => title_case(&result.full_symbol),
        _ => String::new(),
    }
}

/// `wrapped-bitcoin` → `Wrapped Bitcoin`.
fn title_case(id: &str) -> String {
    id.split('-')
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map_or_else(String::new, |first| {
                first.to_uppercase().chain(chars).collect()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// What the row's right-click menu offers: `Replace <chart's market>` while a chart is focused, and
/// always `Add as new chart`. The shortcuts are the keys that do the same.
const REPLACE_KEY: &str = "↵";
const ADD_KEY: &str = "⇧↵";
const ADD_LABEL: &str = "Add as new chart";
const COMPARE_KEY: &str = "⌥↵";
const COMPARE_LABEL: &str = "Compare";

/// Draws one row, `width` wide. `replacing` is the market on the focused chart, which a pick would
/// replace; `None` when there is no chart to replace.
pub fn row(
    ui: &mut Ui,
    result: &SearchResult,
    state: RowState,
    replacing: Option<&str>,
    width: f32,
) -> (Response, RowOutput) {
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
    let painter = ui.painter();
    if state.highlighted {
        painter.rect_filled(rect, 10.0, theme::selected_tint(false));
    } else if response.hovered() {
        painter.rect_filled(rect, 10.0, theme::ink(10));
    }

    let mark = Rect::from_center_size(
        pos2(rect.right() - PAD - MARK / 2.0, rect.center().y),
        Vec2::splat(MARK),
    );
    // The compare button sits left of the plus, a target of its own over the row's.
    let compare_mark = mark.translate(vec2(-MARK - 2.0, 0.0));
    if state.highlighted && state.offers_overlay() {
        let compare = ui.interact(compare_mark, row_id(ui, result, "compare"), Sense::click());
        let compare = compare
            .on_hover_text("Compare on this chart")
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
            .on_hover_text("Add as a new chart")
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
    out.pick |= response.clicked() && !out.add && !out.overlay;

    let painter = ui.painter();
    let icon = Rect::from_center_size(
        pos2(rect.left() + PAD + ICON / 2.0, rect.center().y),
        Vec2::splat(ICON),
    );
    result_icon(ui, icon, result);

    // The exchange: logo and name, with what it lists under the name, right-aligned.
    let venue_right = if state.can_overlay {
        compare_mark.left() - 10.0
    } else {
        mark.left() - 10.0
    };
    let venue = kit::galley(
        ui,
        result.source.display_name(),
        VENUE_SIZE,
        MEDIUM,
        theme::text_primary(),
        150.0,
    );
    let kind = kit::galley(
        ui,
        market_kind(result.source),
        SUBTITLE_SIZE,
        REGULAR,
        theme::text_secondary(),
        150.0,
    );
    let name_right = venue_right;
    let name_left = name_right - venue.size().x;
    let top = rect.center().y - (venue.size().y + kind.size().y + 2.0) / 2.0;
    painter.galley(pos2(name_left, top), venue, theme::text_primary());
    painter.galley(
        pos2(name_right - kind.size().x, top + 2.0 + 17.0),
        kind,
        theme::text_secondary(),
    );
    let logo_rect = Rect::from_min_size(pos2(name_left - 6.0 - LOGO, top - 0.5), Vec2::splat(LOGO));
    source_logo::paint(ui, result.source, logo_rect);

    // The market, as its exchange names it.
    let text_left = icon.right() + 12.0;
    let text_width = (logo_rect.left() - 16.0 - text_left).max(40.0);
    let title = kit::galley(
        ui,
        result.ticker(),
        TITLE_SIZE,
        SEMIBOLD,
        theme::text_primary(),
        text_width,
    );
    let sub = subtitle(result);
    let sub = (!sub.is_empty()).then(|| {
        kit::galley(
            ui,
            &sub,
            SUBTITLE_SIZE,
            REGULAR,
            theme::text_secondary(),
            text_width,
        )
    });
    let block = title.size().y + sub.as_ref().map_or(0.0, |sub| sub.size().y + 2.0);
    let top = rect.center().y - block / 2.0;
    let sub_top = top + title.size().y + 2.0;
    ui.painter()
        .galley(pos2(text_left, top), title, theme::text_primary());
    if let Some(sub) = sub {
        ui.painter()
            .galley(pos2(text_left, sub_top), sub, theme::text_secondary());
    }
    (response, out)
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
        .filter(|_| !state.comparing)
        .map(|current| format!("Replace {current}"));
    let compare = state.offers_overlay().then(|| match replacing {
        Some(current) => format!("Compare with {current}"),
        None => COMPARE_LABEL.to_owned(),
    });
    let compare_key = if state.comparing {
        REPLACE_KEY
    } else {
        COMPARE_KEY
    };
    let mut rows: Vec<(&str, Option<&str>)> = Vec::new();
    if let Some(replace) = &replace {
        rows.push((replace, Some(REPLACE_KEY)));
    }
    if let Some(compare) = &compare {
        rows.push((compare, Some(compare_key)));
    }
    rows.push((ADD_LABEL, Some(ADD_KEY)));
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
        if kit::menu_item_with_shortcut(ui, Icon::Plus, ADD_LABEL, Some(ADD_KEY), true, false) {
            out.add = true;
            ui.close();
        }
    });
}

fn row_id(ui: &Ui, result: &SearchResult, part: &str) -> Id {
    ui.id()
        .with((part, result.source as u8, result.full_symbol.as_str()))
}

/// A compact row for the detail panel's "also trades as" list: artwork, name, how it trades and
/// where. Returns whether it was clicked.
pub fn related_row(ui: &mut Ui, result: &SearchResult, width: f32) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(width, 44.0), Sense::click());
    let response = crate::ui::accessibility::label(
        response.on_hover_cursor(egui::CursorIcon::PointingHand),
        format!("{} on {}", result.symbol, result.source.display_name()),
    );
    if response.hovered() {
        ui.painter().rect_filled(rect, 8.0, theme::ink(10));
    }
    let icon = Rect::from_center_size(
        pos2(rect.left() + 12.0 + 12.0, rect.center().y),
        Vec2::splat(24.0),
    );
    result_icon(ui, icon, result);
    let name = kit::galley(
        ui,
        result.ticker(),
        TITLE_SIZE,
        SEMIBOLD,
        theme::text_primary(),
        width * 0.4,
    );
    ui.painter().galley(
        pos2(icon.right() + 12.0, rect.center().y - name.size().y / 2.0),
        name,
        theme::text_primary(),
    );
    // Right to left: the exchange, its logo, how it trades.
    let venue = kit::galley(
        ui,
        result.source.display_name(),
        SUBTITLE_SIZE + 1.0,
        REGULAR,
        theme::text_secondary(),
        110.0,
    );
    let venue_left = rect.right() - 12.0 - venue.size().x;
    ui.painter().galley(
        pos2(venue_left, rect.center().y - venue.size().y / 2.0),
        venue,
        theme::text_secondary(),
    );
    let logo = Rect::from_center_size(
        pos2(venue_left - 8.0 - 8.0, rect.center().y),
        Vec2::splat(16.0),
    );
    source_logo::paint(ui, result.source, logo);
    let kind = kit::galley(
        ui,
        trading_kind(result.source),
        SUBTITLE_SIZE + 1.0,
        REGULAR,
        theme::text_secondary(),
        80.0,
    );
    ui.painter().galley(
        pos2(
            logo.left() - 14.0 - kind.size().x,
            rect.center().y - kind.size().y / 2.0,
        ),
        kind,
        theme::text_secondary(),
    );
    response.clicked()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_row_names_the_pair_the_company_or_the_coin() {
        let pair = SearchResult::new(DataSource::Binance, "BTCUSDT", "BTCUSDT").pair("BTC", "USDT");
        assert_eq!(subtitle(&pair), "BTC / USDT");
        let stock = SearchResult::new(DataSource::Alpaca, "AAPL — Apple Inc.", "AAPL");
        assert_eq!(subtitle(&stock), "Apple Inc.");
        let coin = SearchResult::new(DataSource::CoinGecko, "WBTC", "wrapped-bitcoin");
        assert_eq!(subtitle(&coin), "Wrapped Bitcoin");
        let bare = SearchResult::new(DataSource::Binance, "BTC", "BTC");
        assert_eq!(subtitle(&bare), "");
    }
}

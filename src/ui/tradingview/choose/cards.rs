//! What there is to bring over: a heading for each kind and a card for each item, with a picture
//! of what it holds. The whole card is its switch.

use super::{heading, pictures};
use crate::i18n::tr;
use crate::model::tradingview::library::{Layout as TvLayout, Script, TvWatchlist};
use crate::model::tradingview::plan::Mappings;
use crate::model::tradingview::{TvSymbol, interval};
use crate::ui::icons::{self, Icon};
use crate::ui::kit::matching;
use crate::ui::kit::sheet_parts::card_background;
use crate::ui::kit::{self, ticker_icon};
use crate::ui::theme;
use crate::ui::tradingview::{Action, Content, Group, words};
use eframe::egui::{self, Color32, CursorIcon, Id, Rect, Sense, Ui, UiBuilder, Vec2, pos2, vec2};
use std::collections::HashSet;

const CARD_HEIGHT: f32 = 68.0;
const CARD_GAP: f32 = 8.0;
const PADDING: f32 = 12.0;
const PICTURE: Vec2 = Vec2::new(68.0, 44.0);
/// The room at a card's trailing edge for its switch.
const SWITCH_ROOM: f32 = 62.0;
const CHIP_HEIGHT: f32 = 18.0;
/// The markets a layout names before it counts the rest.
const CHIPS: usize = 3;

pub(super) fn show(ui: &mut Ui, content: &Content<'_>, actions: &mut Vec<Action>) {
    let library = content.library;
    let selection = content.selection;
    let mut first = true;
    let mut section = |ui: &mut Ui,
                       icon: Icon,
                       title: &str,
                       group: Group,
                       chosen: usize,
                       total: usize,
                       actions: &mut Vec<Action>| {
        if !std::mem::take(&mut first) {
            ui.add_space(20.0);
        }
        section_heading(ui, icon, title, group, chosen, total, actions);
        ui.add_space(10.0);
    };
    if !library.layouts.is_empty() {
        let chosen = library
            .layouts
            .iter()
            .filter(|layout| selection.layouts.contains(&layout.id))
            .count();
        section(
            ui,
            Icon::BarChart,
            tr!("Layouts"),
            Group::Layouts,
            chosen,
            library.layouts.len(),
            actions,
        );
        for layout in &library.layouts {
            let on = selection.layouts.contains(&layout.id);
            if layout_card(ui, layout, content.mappings, on) {
                actions.push(Action::ToggleLayout(layout.id.clone()));
            }
            ui.add_space(CARD_GAP);
        }
    }
    if !library.watchlists.is_empty() {
        let chosen = library
            .watchlists
            .iter()
            .filter(|list| selection.watchlists.contains(&list.id))
            .count();
        section(
            ui,
            Icon::List,
            tr!("Watchlists"),
            Group::Watchlists,
            chosen,
            library.watchlists.len(),
            actions,
        );
        for list in &library.watchlists {
            let on = selection.watchlists.contains(&list.id);
            if watchlist_card(ui, list, content.mappings, on) {
                actions.push(Action::ToggleWatchlist(list.id.clone()));
            }
            ui.add_space(CARD_GAP);
        }
    }
    if !library.scripts.is_empty() {
        let chosen = library
            .scripts
            .iter()
            .filter(|script| selection.scripts.contains(&script.id))
            .count();
        section(
            ui,
            Icon::Code,
            tr!("Pine scripts"),
            Group::Scripts,
            chosen,
            library.scripts.len(),
            actions,
        );
        for script in &library.scripts {
            let on = selection.scripts.contains(&script.id);
            if script_card(ui, script, on) {
                actions.push(Action::ToggleScript(script.id.clone()));
            }
            ui.add_space(CARD_GAP);
        }
    }
}

/// A kind's title, how many of it are chosen, and one switch for all of them.
fn section_heading(
    ui: &mut Ui,
    icon: Icon,
    title: &str,
    group: Group,
    chosen: usize,
    total: usize,
    actions: &mut Vec<Action>,
) {
    let mut all = None;
    heading(ui, icon, title, |ui, row, end| {
        let count = kit::galley(
            ui,
            &tr!("{count} of {total} selected", count = chosen, total = total),
            kit::size::CAPTION + 0.5,
            kit::REGULAR,
            theme::text_tertiary(),
            240.0,
        );
        ui.painter().galley(
            pos2(end + 10.0, row.center().y - count.size().y / 2.0 + 1.0),
            count,
            Color32::PLACEHOLDER,
        );
        // One item has its own switch right below.
        if total < 2 {
            return;
        }
        let on = chosen < total;
        let label = if on {
            tr!("Select all")
        } else {
            tr!("Select none")
        };
        let text = kit::galley(
            ui,
            label,
            kit::size::CAPTION + 0.5,
            kit::REGULAR,
            theme::text_secondary(),
            200.0,
        );
        let link = Rect::from_min_size(
            pos2(
                row.right() - text.size().x,
                row.center().y - text.size().y / 2.0,
            ),
            text.size(),
        );
        let response = ui
            .interact(
                link.expand(4.0),
                Id::new(("tradingview-all", group)),
                Sense::click(),
            )
            .on_hover_cursor(CursorIcon::PointingHand);
        let response = crate::ui::accessibility::label(response, label);
        let color = if response.hovered() {
            theme::text_primary()
        } else {
            theme::text_secondary()
        };
        ui.painter()
            .galley_with_override_text_color(link.min, text, color);
        ui.painter().hline(
            link.x_range(),
            link.bottom(),
            egui::Stroke::new(1.0, color.gamma_multiply(0.6)),
        );
        if response.clicked() {
            all = Some(on);
        }
    });
    if let Some(on) = all {
        actions.push(Action::SelectAll(group, on));
    }
}

/// A card of an item: its picture, its name over `detail`, and its switch. `true` when clicked.
fn card(
    ui: &mut Ui,
    id: Id,
    title: &str,
    on: bool,
    picture: impl FnOnce(&mut Ui, Rect),
    detail: impl FnOnce(&mut Ui, Rect),
) -> bool {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), CARD_HEIGHT), Sense::hover());
    let response = ui
        .interact(rect, id, Sense::click())
        .on_hover_cursor(CursorIcon::PointingHand);
    let response = crate::ui::accessibility::label(response, title);
    let hovered = response.hovered();
    if on {
        card_background(ui, rect, hovered.then(|| theme::card_border(true)));
    } else {
        // Left out: only its outline stays.
        ui.painter().rect_stroke(
            rect,
            12.0,
            egui::Stroke::new(1.0, theme::card_border(hovered)),
            egui::StrokeKind::Inside,
        );
    }
    if hovered {
        ui.painter()
            .rect_filled(rect, 12.0, theme::quaternary(0.25));
    }

    let frame = Rect::from_min_size(
        pos2(rect.left() + PADDING, rect.center().y - PICTURE.y / 2.0),
        PICTURE,
    );
    let text_left = frame.right() + 14.0;
    let text_right = rect.right() - SWITCH_ROOM;
    let name = kit::galley(
        ui,
        title,
        kit::size::BODY + 1.0,
        kit::SEMIBOLD,
        if on {
            theme::text_primary()
        } else {
            theme::text_secondary()
        },
        text_right - text_left,
    );
    ui.painter().galley(
        pos2(text_left, rect.center().y - 2.0 - name.size().y),
        name,
        Color32::PLACEHOLDER,
    );

    let mut inside = ui.new_child(UiBuilder::new().max_rect(rect));
    if !on {
        inside.multiply_opacity(0.45);
    }
    picture(&mut inside, frame);
    let line = Rect::from_min_max(
        pos2(text_left, rect.center().y + 3.0),
        pos2(text_right, rect.center().y + 3.0 + CHIP_HEIGHT),
    );
    inside.set_clip_rect(inside.clip_rect().intersect(Rect::from_x_y_ranges(
        frame.left()..=text_right,
        rect.y_range(),
    )));
    detail(&mut inside, line);

    let switch =
        Rect::from_center_size(pos2(rect.right() - 30.0, rect.center().y), vec2(32.0, 18.0));
    let progress = ui.ctx().animate_bool(id.with("switch"), on);
    kit::paint_switch(ui.painter(), switch, progress, false);
    response.clicked()
}

/// A line of secondary words from `left`, centred in `line`; returns where it ends.
fn words_at(ui: &Ui, line: Rect, left: f32, text: &str, color: Color32) -> f32 {
    let galley = kit::galley(
        ui,
        text,
        kit::size::CAPTION + 0.5,
        kit::REGULAR,
        color,
        (line.right() - left).max(24.0),
    );
    let right = left + galley.size().x;
    ui.painter().galley(
        pos2(left, line.center().y - galley.size().y / 2.0),
        galley,
        Color32::PLACEHOLDER,
    );
    right
}

/// The mark of `symbol`: its market's logo once it has one, its initials until then.
fn logo(ui: &mut Ui, rect: Rect, symbol: &TvSymbol, mappings: &Mappings) {
    match mappings.market(symbol) {
        Some(market) => ticker_icon(ui, rect, &market.name, market.source),
        None => matching::placeholder_icon(ui, rect, symbol.ticker()),
    }
}

/// A market of a layout as a chip; returns its right edge.
fn chip(ui: &mut Ui, line: Rect, left: f32, symbol: &TvSymbol, mappings: &Mappings) -> f32 {
    let name = kit::galley(
        ui,
        symbol.ticker(),
        kit::size::CAPTION,
        kit::MEDIUM,
        theme::text_primary(),
        120.0,
    );
    let rect = Rect::from_min_size(
        pos2(left, line.center().y - CHIP_HEIGHT / 2.0),
        vec2(name.size().x + 28.0, CHIP_HEIGHT),
    );
    ui.painter()
        .rect_filled(rect, CHIP_HEIGHT / 2.0, theme::quaternary(0.9));
    logo(
        ui,
        Rect::from_center_size(pos2(rect.left() + 10.0, rect.center().y), Vec2::splat(14.0)),
        symbol,
        mappings,
    );
    ui.painter().galley(
        pos2(rect.left() + 21.0, rect.center().y - name.size().y / 2.0),
        name,
        Color32::PLACEHOLDER,
    );
    rect.right()
}

fn layout_card(ui: &mut Ui, layout: &TvLayout, mappings: &Mappings, on: bool) -> bool {
    let mut symbols: Vec<&TvSymbol> = Vec::new();
    let mut intervals: Vec<String> = Vec::new();
    for chart in layout.shown_charts() {
        if !symbols.contains(&&chart.symbol) {
            symbols.push(&chart.symbol);
        }
        if let Some(label) = interval::timeframe(&chart.interval).map(|t| t.label())
            && !intervals.contains(&label)
        {
            intervals.push(label);
        }
    }
    let mut facts = vec![
        words::charts(layout.shown_charts().count()),
        words::drawings(layout.drawings.len()),
    ];
    if !intervals.is_empty() {
        intervals.truncate(CHIPS);
        facts.push(intervals.join(", "));
    }
    let title = layout.display_name();
    card(
        ui,
        Id::new(("tradingview-layout", &layout.id)),
        &title,
        on,
        |ui, rect| pictures::grid(ui, rect, layout),
        |ui, line| {
            let mut left = line.left();
            for symbol in symbols.iter().take(CHIPS) {
                left = chip(ui, line, left, symbol, mappings) + 6.0;
            }
            if symbols.len() > CHIPS {
                let more = format!("+{}", symbols.len() - CHIPS);
                left = words_at(ui, line, left, &more, theme::text_tertiary()) + 8.0;
            }
            let end = words_at(
                ui,
                line,
                left + 2.0,
                &facts.join(" \u{00B7} "),
                theme::text_secondary(),
            );
            if layout.is_rebuilt() {
                let at = pos2(end + 16.0, line.center().y);
                icons::paint(
                    ui.painter(),
                    Rect::from_center_size(at, Vec2::splat(12.0)),
                    Icon::InfoCircle,
                    theme::REPLAY,
                );
                words_at(
                    ui,
                    line,
                    at.x + 11.0,
                    tr!("Rebuilt from its drawings"),
                    theme::REPLAY,
                );
            }
        },
    )
}

/// The first symbol of each asset: one logo says it once, however many pairs name it.
fn distinct_assets<'a>(symbols: &[&'a TvSymbol], mappings: &Mappings) -> Vec<&'a TvSymbol> {
    let mut seen = HashSet::new();
    symbols
        .iter()
        .copied()
        .filter(|symbol| {
            let asset = match mappings.market(symbol) {
                Some(market) => market.source.base_asset(&market.symbol).to_string(),
                None => symbol.ticker().to_owned(),
            };
            seen.insert(asset)
        })
        .collect()
}

fn watchlist_card(ui: &mut Ui, list: &TvWatchlist, mappings: &Mappings, on: bool) -> bool {
    let symbols: Vec<&TvSymbol> = list.symbols().collect();
    let marks = distinct_assets(&symbols, mappings);
    card(
        ui,
        Id::new(("tradingview-watchlist", &list.id)),
        &list.name,
        on,
        |ui, rect| {
            pictures::logos(ui, rect, marks.len(), |ui, at, index| {
                logo(ui, at, marks[index], mappings);
            });
        },
        |ui, line| {
            let end = words_at(
                ui,
                line,
                line.left(),
                &words::markets(symbols.len()),
                theme::text_secondary(),
            );
            // As many names as could be read at a glance; the line cuts what does not fit.
            let names: Vec<&str> = symbols
                .iter()
                .take(8)
                .map(|symbol| symbol.ticker())
                .collect();
            words_at(
                ui,
                line,
                end + 12.0,
                &names.join(", "),
                theme::text_tertiary(),
            );
        },
    )
}

fn script_card(ui: &mut Ui, script: &Script, on: bool) -> bool {
    card(
        ui,
        Id::new(("tradingview-script", &script.id)),
        &script.name,
        on,
        |ui, rect| pictures::source(ui, rect, &script.source),
        |ui, line| {
            words_at(
                ui,
                line,
                line.left(),
                &words::lines(script.source.lines().count()),
                theme::text_secondary(),
            );
        },
    )
}

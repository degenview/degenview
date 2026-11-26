//! The first step: what to bring over, and which market each of its symbols becomes.

use super::{Action, Content, scrolling, words};
use crate::i18n::{tr, tr_n};
use crate::model::tradingview::TvSymbol;
use crate::model::tradingview::plan::{Match, Unmatched, Usage};
use crate::ui::kit::matching::{self, RowAction, RowState, Tally, section_heading};
use crate::ui::kit::{self, sheet, ticker_icon};
use crate::ui::theme;
use eframe::egui::{self, Align, Color32, Id, Layout, Rect, Sense, Stroke, Ui, pos2, vec2};

const ITEM_HEIGHT: f32 = 34.0;
/// Where a symbol row's market starts: TradingView's names run longer than a CSV's tickers.
const TARGET_LEFT: f32 = 300.0;

pub(super) fn show(ui: &mut Ui, height: f32, content: &Content<'_>, actions: &mut Vec<Action>) {
    let symbols = content.selection.symbols(content.library);
    let tally = tally(content, &symbols);
    scrolling(ui, height, ("tradingview-choose", content.opening), |ui| {
        note(ui, content, actions);
        items(ui, content, actions);
        if !symbols.is_empty() {
            ui.add_space(20.0);
            symbols_section(ui, content, &symbols, &tally, actions);
        }
    });
    footer(ui, content, &tally, actions);
}

pub(super) fn tally(content: &Content<'_>, symbols: &[(TvSymbol, Usage)]) -> Tally {
    let mut tally = Tally::default();
    for (symbol, _) in symbols {
        tally.total += 1;
        match content.mappings.status(symbol) {
            Match::Searching => tally.resolving += 1,
            Match::Skipped => tally.skipped += 1,
            Match::Unmatched(_) => tally.unmatched += 1,
            Match::Exact(_) | Match::Substitute(_) | Match::Chosen(_) => tally.matched += 1,
        }
    }
    tally
}

/// Why something may be missing, with the ways to look again.
fn note(ui: &mut Ui, content: &Content<'_>, actions: &mut Vec<Action>) {
    let text = tr!(
        "Only what the TradingView app has opened recently on this computer is here. Missing \
         something? Open it there, then reload."
    );
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let choose = tr!("Choose Folder\u{2026}");
        let reload = tr!("Reload");
        let buttons = kit::push_button_width(ui, None, choose)
            + kit::push_button_width(ui, None, reload)
            + 24.0;
        ui.allocate_ui_with_layout(
            vec2((ui.available_width() - buttons).max(200.0), 0.0),
            Layout::top_down(Align::Min),
            |ui| {
                ui.add(
                    egui::Label::new(kit::text(
                        text,
                        kit::size::CAPTION + 0.5,
                        kit::REGULAR,
                        theme::text_secondary(),
                    ))
                    .wrap(),
                );
            },
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if kit::push_button(ui, None, choose, true).clicked() {
                actions.push(Action::ChooseFolder);
            }
            if kit::push_button(ui, None, reload, true)
                .on_hover_text(content.folder.display().to_string())
                .clicked()
            {
                actions.push(Action::Reload);
            }
        });
    });
}

/// One thing to bring over: a row that is a switch.
struct Item {
    title: String,
    detail: String,
    on: bool,
    toggle: Action,
}

fn item_row(ui: &mut Ui, id: Id, item: &Item) -> bool {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), ITEM_HEIGHT), Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let response = crate::ui::accessibility::label(response, &item.title);
    if response.hovered() {
        ui.painter().rect_filled(rect, 8.0, theme::quaternary(0.35));
    }
    let toggle =
        Rect::from_center_size(pos2(rect.right() - 30.0, rect.center().y), vec2(32.0, 18.0));
    let progress = ui.ctx().animate_bool(id, item.on);
    kit::paint_switch(ui.painter(), toggle, progress, false);
    let left = rect.left() + 14.0;
    let room = toggle.left() - left - 16.0;
    let color = if item.on {
        theme::text_primary()
    } else {
        theme::text_secondary()
    };
    // The name, then what it holds beside it, on one line.
    let title = kit::galley(
        ui,
        &item.title,
        kit::size::BODY,
        kit::MEDIUM,
        color,
        room * 0.5,
    );
    let detail = kit::galley(
        ui,
        &item.detail,
        kit::size::CAPTION + 0.5,
        kit::REGULAR,
        theme::text_secondary(),
        (room - title.size().x - 12.0).max(40.0),
    );
    let title_width = title.size().x;
    ui.painter().galley(
        pos2(left, rect.center().y - title.size().y / 2.0),
        title,
        Color32::PLACEHOLDER,
    );
    ui.painter().galley(
        pos2(
            left + title_width + 12.0,
            rect.center().y - detail.size().y / 2.0,
        ),
        detail,
        Color32::PLACEHOLDER,
    );
    response.clicked()
}

/// A heading and one card with a row per item.
fn group(ui: &mut Ui, title: &str, items: Vec<Item>, actions: &mut Vec<Action>) {
    if items.is_empty() {
        return;
    }
    ui.add_space(14.0);
    ui.label(kit::text(
        title,
        kit::size::CAPTION + 0.5,
        kit::SEMIBOLD,
        theme::text_secondary(),
    ));
    ui.add_space(6.0);
    egui::Frame::new()
        .fill(theme::quaternary(0.4))
        .stroke(Stroke::new(1.0, theme::separator().gamma_multiply(0.6)))
        .corner_radius(kit::CARD_RADIUS)
        .inner_margin(egui::Margin::same(3))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            for (index, item) in items.into_iter().enumerate() {
                if index > 0 {
                    kit::divider(ui);
                }
                let id = Id::new(("tradingview-item", title, index));
                if item_row(ui, id, &item) {
                    actions.push(item.toggle);
                }
            }
        });
}

fn items(ui: &mut Ui, content: &Content<'_>, actions: &mut Vec<Action>) {
    let library = content.library;
    let selection = content.selection;
    let layouts = library
        .layouts
        .iter()
        .map(|layout| Item {
            title: layout.display_name(),
            detail: words::layout_detail(layout),
            on: selection.layouts.contains(&layout.id),
            toggle: Action::ToggleLayout(layout.id.clone()),
        })
        .collect();
    group(ui, tr!("Layouts"), layouts, actions);
    let watchlists = library
        .watchlists
        .iter()
        .map(|list| Item {
            title: list.name.clone(),
            detail: words::watchlist_detail(list),
            on: selection.watchlists.contains(&list.id),
            toggle: Action::ToggleWatchlist(list.id.clone()),
        })
        .collect();
    group(ui, tr!("Watchlists"), watchlists, actions);
    let scripts = library
        .scripts
        .iter()
        .map(|script| {
            let lines = script.source.lines().count();
            Item {
                title: script.name.clone(),
                detail: tr_n!(lines, "{count} line", "{count} lines"),
                on: selection.scripts.contains(&script.id),
                toggle: Action::ToggleScript(script.id.clone()),
            }
        })
        .collect();
    group(ui, tr!("Pine scripts"), scripts, actions);
}

fn symbols_section(
    ui: &mut Ui,
    content: &Content<'_>,
    symbols: &[(TvSymbol, Usage)],
    tally: &Tally,
    actions: &mut Vec<Action>,
) {
    let busy = (tally.resolving > 0).then_some(tr!("Searching exchanges\u{2026}"));
    section_heading(
        ui,
        tr!("Match symbols to markets"),
        tr!(
            "A symbol from an exchange DegenView doesn\u{2019}t have is matched to the same pair \
             elsewhere. Change or skip any of them."
        ),
        busy,
    );
    let caption = tr_n!(tally.total, "symbol matched", "symbols matched");
    matching::band(ui, tally, &caption);
    let can_retry = symbols.iter().any(|(symbol, _)| {
        matches!(
            content.mappings.status(symbol),
            Match::Unmatched(Unmatched::NeedsKey | Unmatched::Unreachable)
        )
    });
    if can_retry {
        ui.add_space(8.0);
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            if kit::push_button(ui, None, tr!("Search Again"), true).clicked() {
                actions.push(Action::Retry);
            }
        });
    }
    ui.add_space(10.0);
    for (symbol, usage) in symbols {
        symbol_row(ui, content, symbol, *usage, actions);
        ui.add_space(8.0);
    }
}

fn symbol_row(
    ui: &mut Ui,
    content: &Content<'_>,
    symbol: &TvSymbol,
    usage: Usage,
    actions: &mut Vec<Action>,
) {
    let status = content.mappings.status(symbol);
    // A pair is its own name; a stock or a DEX pair also has one worth saying.
    let detail = status.market().map(|market| {
        let source = market.source.display_name();
        if market.name == market.symbol {
            source.to_owned()
        } else {
            format!("{} \u{00B7} {source}", market.name)
        }
    });
    let state = match status {
        Match::Searching => RowState::Searching(tr!("Searching exchanges\u{2026}")),
        Match::Exact(market) | Match::Substitute(market) | Match::Chosen(market) => {
            RowState::Matched {
                source: market.source,
                ticker: &market.symbol,
                detail: detail.as_deref().unwrap_or(""),
                badge: match status {
                    Match::Exact(_) => Some((tr!("Auto-matched"), theme::text_secondary())),
                    Match::Substitute(_) => Some((tr!("Other exchange"), theme::REPLAY)),
                    _ => None,
                },
            }
        }
        Match::Skipped => RowState::Skipped {
            note: tr!("Left out of the import"),
        },
        Match::Unmatched(why) => RowState::Unmatched {
            note: words::unmatched(*why),
        },
    };
    let caption = words::usage(symbol.exchange(), usage);
    let clicked = matching::row(
        ui,
        Id::new(("tradingview-symbol", symbol.as_str())),
        &matching::Row {
            name: symbol.ticker(),
            caption: &caption,
            state,
            target_left: TARGET_LEFT,
        },
        |ui, rect| match status.market() {
            Some(market) => ticker_icon(ui, rect, &market.name, market.source),
            None => matching::placeholder_icon(ui, rect, symbol.ticker()),
        },
    );
    match clicked {
        Some(RowAction::Match) => actions.push(Action::Match(symbol.clone())),
        Some(RowAction::Skip) => actions.push(Action::Skip(symbol.clone())),
        Some(RowAction::Reset) => actions.push(Action::Reset(symbol.clone())),
        None => {}
    }
}

fn footer(ui: &mut Ui, content: &Content<'_>, tally: &Tally, actions: &mut Vec<Action>) {
    let selection = content.selection;
    let parts: Vec<String> = [
        (
            selection.layouts.len(),
            words::layouts as fn(usize) -> String,
        ),
        (selection.watchlists.len(), words::watchlists),
        (selection.scripts.len(), words::scripts),
    ]
    .into_iter()
    .filter(|(count, _)| *count > 0)
    .map(|(count, words)| words(count))
    .collect();
    let summary = if parts.is_empty() {
        tr!("Nothing selected").to_owned()
    } else {
        parts.join(" \u{00B7} ")
    };
    sheet::footer_split(
        ui,
        |ui| {
            ui.label(kit::text(
                summary,
                kit::size::BODY,
                kit::REGULAR,
                theme::text_secondary(),
            ));
        },
        |ui| {
            let ready = !selection.is_empty() && tally.resolving == 0;
            if kit::footer_button(
                ui,
                tr!("Continue to Preview"),
                kit::FooterButton::Primary,
                ready,
                150.0,
            )
            .clicked()
            {
                actions.push(Action::Preview);
            }
            if kit::footer_button(ui, tr!("Cancel"), kit::FooterButton::Secondary, true, 0.0)
                .clicked()
            {
                actions.push(Action::Close);
            }
        },
    );
}

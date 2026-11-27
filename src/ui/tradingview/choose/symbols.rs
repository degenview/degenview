//! Which market each TradingView symbol becomes: how they stand together, and a row for each.

use crate::i18n::{tr, tr_n};
use crate::model::tradingview::TvSymbol;
use crate::model::tradingview::plan::{Match, Unmatched, Usage};
use crate::ui::icons::Icon;
use crate::ui::kit::loading::LoadingIndicator;
use crate::ui::kit::matching::{self, RowAction, RowState, Tally};
use crate::ui::kit::{self, ticker_icon};
use crate::ui::theme;
use crate::ui::tradingview::{Action, Content, section_title, words};
use eframe::egui::{self, Align, Color32, Id, Layout, Rect, Ui, Vec2, pos2};

/// Where a symbol row's market starts: TradingView's names run longer than a CSV's tickers.
const TARGET_LEFT: f32 = 300.0;

/// Where it is kept whether only the rows still to match are listed.
pub(super) fn filter(opening: u64) -> Id {
    Id::new(("tradingview-filter", opening))
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

pub(super) fn show(
    ui: &mut Ui,
    content: &Content<'_>,
    symbols: &[(TvSymbol, Usage)],
    tally: &Tally,
    actions: &mut Vec<Action>,
) {
    section_title(
        ui,
        Icon::ArrowLeftRight,
        tr!("Match symbols to markets"),
        |ui, row, _| {
            if tally.resolving == 0 {
                return;
            }
            let label = kit::galley(
                ui,
                tr!("Searching exchanges\u{2026}"),
                kit::size::CAPTION + 0.5,
                kit::REGULAR,
                theme::text_secondary(),
                260.0,
            );
            let left = row.right() - label.size().x;
            ui.painter().galley(
                pos2(left, row.center().y - label.size().y / 2.0),
                label,
                Color32::PLACEHOLDER,
            );
            LoadingIndicator::new().size(13.0).paint_at(
                ui,
                Rect::from_center_size(pos2(left - 14.0, row.center().y), Vec2::splat(13.0)),
            );
        },
    );
    ui.add_space(4.0);
    ui.add(
        egui::Label::new(kit::text(
            tr!(
                "A symbol from an exchange DegenView doesn\u{2019}t have is matched to the same \
                 pair elsewhere. Change or skip any of them."
            ),
            kit::size::SUBHEADLINE,
            kit::REGULAR,
            theme::text_secondary(),
        ))
        .wrap(),
    );
    ui.add_space(12.0);
    let caption = tr_n!(tally.total, "symbol matched", "symbols matched");
    matching::band(ui, tally, &caption);

    let only_open = toolbar(ui, content, symbols, tally, actions);
    ui.add_space(10.0);
    for (symbol, usage) in symbols {
        if only_open && !matches!(content.mappings.status(symbol), Match::Unmatched(_)) {
            continue;
        }
        symbol_row(ui, content, symbol, *usage, actions);
        ui.add_space(8.0);
    }
}

/// Under the band: which rows to list, and searching again. Returns whether only the rows still
/// to match are listed.
fn toolbar(
    ui: &mut Ui,
    content: &Content<'_>,
    symbols: &[(TvSymbol, Usage)],
    tally: &Tally,
    actions: &mut Vec<Action>,
) -> bool {
    // A choice between lists that differ.
    let filterable = tally.unmatched > 0 && tally.unmatched < tally.total;
    let can_retry = content.unplaced
        || symbols.iter().any(|(symbol, _)| {
            matches!(
                content.mappings.status(symbol),
                Match::Unmatched(Unmatched::NeedsKey | Unmatched::Unreachable)
            )
        });
    if !filterable && !can_retry {
        return false;
    }
    let id = filter(content.opening);
    let mut only_open: bool = ui.data(|data| data.get_temp(id)).unwrap_or(false);
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        if filterable {
            let open = tr!("{count} to match", count = tally.unmatched);
            kit::segmented(
                ui,
                id,
                &[(false, tr!("All")), (true, open.as_str())],
                &mut only_open,
            );
            ui.data_mut(|data| data.insert_temp(id, only_open));
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if can_retry && kit::push_button(ui, None, tr!("Search Again"), true).clicked() {
                actions.push(Action::Retry);
            }
        });
    });
    filterable && only_open
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
                // The person's own pick is taken back; the search's can only be left out.
                other: if matches!(status, Match::Chosen(_)) {
                    RowAction::Reset
                } else {
                    RowAction::Skip
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

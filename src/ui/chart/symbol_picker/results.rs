//! The list area: the rows, the captions above groups of them and what stands in for them.

use super::notice::{NOTICE_HEIGHT, show_notice};
use super::*;

impl SymbolPicker {
    /// The rows, or what stands in for them. Returns a pick.
    pub(super) fn list(
        &mut self,
        ui: &mut Ui,
        area: Rect,
        listing: &Listing,
        rows: &Rows,
        env: &Env,
    ) -> Option<(Pick, PickKind)> {
        let mut picked = None;
        let lookup = self.lookup();
        // The provider notice sits at the bottom of the list's own area, under the rows.
        let notice = self.notice(listing, &lookup);
        let reserved = notice.as_ref().map_or(0.0, |_| NOTICE_HEIGHT + 20.0);
        let content = Rect::from_min_max(area.min, pos2(area.max.x, area.max.y - reserved));
        if let Some(notice) = &notice {
            let card = Rect::from_min_max(
                pos2(area.left() + PADDING - 6.0, content.bottom() + 6.0),
                pos2(area.right() - (PADDING - 6.0) - 8.0, area.bottom() - 14.0),
            );
            if show_notice(ui, card, notice) {
                self.settings_request = notice.settings;
            }
        }
        let mut body = ui
            .new_child(egui::UiBuilder::new().max_rect(content.shrink2(vec2(PADDING - 6.0, 6.0))));
        body.set_clip_rect(content);
        let empty_query = lookup.is_empty();
        let topics = self.scope == Scope::Predictions && empty_query;
        if rows.items.is_empty() && rows.note.is_none() && !topics {
            self.empty_state(&mut body, listing, &lookup);
            return None;
        }
        let pointer_moved = ui.input(|input| input.pointer.delta() != Vec2::ZERO);
        let reveal = std::mem::take(&mut self.reveal);
        let width = body.available_width();
        let replacing = self.replacing(env);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(&mut body, |ui| {
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                ui.set_width(width - 8.0);
                if topics {
                    self.topics(ui, rows);
                }
                for (index, item) in rows.items.iter().enumerate() {
                    if let Some(head) = rows.head_before(index) {
                        section_heading(ui, head);
                    }
                    let state = self.row_state(index, item, env);
                    let (response, out) = match item {
                        Item::Market(result) => {
                            list::row(ui, result, state, replacing, ui.available_width())
                        }
                        Item::Choice(result) => predictions::rows::choice(
                            ui,
                            result,
                            predictions::is_ticked(self.ticked.as_ref(), result),
                            state,
                            ui.available_width(),
                        ),
                        Item::Event(head) => predictions::rows::event_header(
                            ui,
                            head,
                            state.highlighted,
                            ui.available_width(),
                        ),
                        Item::Tile(tile) => dashboards::row(ui, tile, state, ui.available_width()),
                    };
                    if reveal && state.highlighted {
                        response.scroll_to_me(None);
                    }
                    if (out.hovered && pointer_moved) || out.opened_menu {
                        self.highlight = index;
                    }
                    if out.tick {
                        self.tick(item);
                    }
                    if out.fold
                        && let Item::Event(head) = item
                    {
                        self.events.fold(&head.title);
                    }
                    let kind = if out.overlay {
                        Some(PickKind::Overlay)
                    } else if out.add {
                        Some(self.add_kind())
                    } else if out.pick {
                        // A click on a market already drawn here, while comparing, does nothing.
                        (!(self.mode == PickMode::Compare && state.overlaid))
                            .then(|| self.default_kind())
                    } else {
                        None
                    };
                    if let Some(kind) = kind {
                        picked = match item {
                            Item::Market(result) | Item::Choice(result) => {
                                Some((Pick::Market(result.clone()), kind))
                            }
                            Item::Tile(tile) => Some((Self::tile_pick(tile), kind)),
                            Item::Event(_) => None,
                        };
                    }
                }
                if let Some(note) = &rows.note {
                    note_line(ui, note);
                }
            });
        picked
    }

    /// How `item`'s row is drawn: highlighted, already held, drawn over the chart.
    pub(super) fn row_state(&self, index: usize, item: &Item, env: &Env) -> RowState {
        let on_chart = match item {
            Item::Market(result) | Item::Choice(result) => {
                self.held(env, &InstrumentId::new(result.source, &result.full_symbol))
            }
            Item::Tile(tile) => match &tile.pick {
                TilePick::Gauge(result) => {
                    self.held(env, &InstrumentId::new(result.source, &result.full_symbol))
                }
                TilePick::Card(card) if self.mode == PickMode::Watchlist => {
                    self.held(env, &card.instrument_id())
                }
                TilePick::Card(card) => env.cards_on_tab.iter().any(|on| on.same_card(card)),
            },
            Item::Event(_) => false,
        };
        let market = item.market().filter(|_| !matches!(item, Item::Event(_)));
        RowState {
            highlighted: index == self.highlight,
            on_chart,
            overlaid: market.is_some_and(|result| Self::overlaid(env, result)),
            can_overlay: env.can_overlay
                && self.mode != PickMode::Watchlist
                && market.is_some_and(Self::comparable),
            comparing: self.mode == PickMode::Compare,
            watchlist: self.mode == PickMode::Watchlist,
            indent: if self.scope == Scope::Predictions && matches!(item, Item::Market(_)) {
                predictions::rows::ART_INDENT
            } else {
                0.0
            },
        }
    }

    /// The topic chips of the Predictions tab, above the trending events.
    pub(super) fn topics(&mut self, ui: &mut Ui, rows: &Rows) {
        section_heading(ui, tr!("Popular topics"));
        let width = ui.available_width() - 20.0;
        ui.horizontal(|ui| {
            ui.add_space(10.0);
            ui.vertical(|ui| {
                ui.set_width(width);
                if let Some(topic) = predictions::rows::topic_chips(ui, width) {
                    self.query = topic.to_owned();
                    self.search.ask_now();
                    self.highlight = 0;
                }
            });
        });
        ui.add_space(8.0);
        if rows.items.is_empty() {
            if self.trending.is_loading() {
                let (area, _) = ui.allocate_exact_size(vec2(width, 100.0), Sense::hover());
                ui.put(
                    Rect::from_center_size(area.center(), Vec2::splat(32.0)),
                    LoadingIndicator::new().size(32.0),
                );
            } else if self.lookup().venue == Some(DataSource::Kalshi) {
                note_line(
                    ui,
                    tr!("Kalshi cannot list its busiest markets. Search for one, or pick a topic."),
                );
            }
        }
    }

    pub(super) fn empty_state(&self, ui: &mut Ui, listing: &Listing, lookup: &Lookup) {
        let query = self.query.trim();
        let empty_query = lookup.is_empty();
        match self.scope {
            Scope::Watchlists => {
                let (title, detail) = if self.watchlists.is_empty() {
                    (
                        tr!("No watchlist markets yet"),
                        tr!("Crypto and stocks you add to a watchlist show up here.").to_owned(),
                    )
                } else {
                    (
                        tr!("No results"),
                        tr!(
                            "Nothing in your watchlists matches “{query}”.",
                            query = query
                        ),
                    )
                };
                widgets::no_results(ui, title, &detail);
                return;
            }
            Scope::Dashboards => {
                widgets::no_results(
                    ui,
                    tr!("No results"),
                    &tr!(
                        "No index or portfolio view matches “{query}”.",
                        query = query
                    ),
                );
                return;
            }
            Scope::All | Scope::Stocks | Scope::Crypto | Scope::Forex | Scope::Predictions => {}
        }
        if let Some(venue) = lookup.outside(self.scope) {
            widgets::no_results(
                ui,
                tr!("No results"),
                &tr!(
                    "{venue} has no markets under {tab}. Try another tab.",
                    venue = lookup::short_name(venue),
                    tab = self.scope.title()
                ),
            );
        } else if empty_query {
            widgets::no_results(
                ui,
                tr!("Search stocks and crypto"),
                tr!("Type a ticker, a company or a pair: BTC, AAPL, ETHUSDT."),
            );
        } else if listing.searching {
            let rect = ui.available_rect_before_wrap();
            ui.put(
                Rect::from_center_size(rect.center(), Vec2::splat(40.0)),
                LoadingIndicator::new().size(40.0),
            );
        } else if let Some((source, error)) = listing.errors.iter().find(|(source, error)| {
            // A missing key is the notice card's job, not the empty state's: on a broad
            // search it must not override the ordinary no-results message.
            !matches!(error, ProviderError::MissingKey(_))
                && SettingsPage::for_source(*source)
                    .and_then(SettingsPage::search_failure_title)
                    .is_none()
        }) {
            widgets::no_results(
                ui,
                &tr!(
                    "{venue} could not be searched",
                    venue = source.display_name()
                ),
                &error.to_string(),
            );
        } else {
            let searched = lookup
                .searched(self.scope)
                .into_iter()
                .map(lookup::short_name)
                .collect::<Vec<_>>();
            let searched = crate::i18n::join_list(&searched);
            let detail = match &lookup.unsupported {
                Some(prefix) => tr!(
                    "{prefix} isn't supported. Searched: {searched}.",
                    prefix = prefix,
                    searched = searched
                ),
                None if self.scope == Scope::Predictions => tr!(
                    "No market matches “{query}”. Searched: {searched}. Try a topic like Bitcoin or the Fed.",
                    query = query,
                    searched = searched
                ),
                None => tr!(
                    "Nothing matches “{query}”. Searched: {searched}.",
                    query = query,
                    searched = searched
                ),
            };
            widgets::no_results(ui, tr!("No results"), &detail);
        }
    }
}

/// The rows the list holds, the captions above groups of them (`(first row, caption)`) and a
/// note under them.
#[derive(Default)]
pub(super) struct Rows {
    pub(super) items: Vec<Item>,
    pub(super) heads: Vec<(usize, String)>,
    pub(super) note: Option<String>,
}

impl Rows {
    pub(super) fn of(results: Vec<SearchResult>, head: Option<&str>) -> Self {
        Self::of_items(results.into_iter().map(Item::Market).collect(), head)
    }

    pub(super) fn of_items(items: Vec<Item>, head: Option<&str>) -> Self {
        let heads = head
            .filter(|_| !items.is_empty())
            .map(|head| vec![(0, head.to_owned())])
            .unwrap_or_default();
        Self {
            items,
            heads,
            note: None,
        }
    }

    pub(super) fn head_before(&self, row: usize) -> Option<&str> {
        self.heads
            .iter()
            .find(|(first, _)| *first == row)
            .map(|(_, head)| head.as_str())
    }
}

/// A small capitalised caption above a group of rows.
pub(super) fn section_heading(ui: &mut Ui, title: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::hover());
    let galley = kit::galley(
        ui,
        &title.to_uppercase(),
        11.5,
        SEMIBOLD,
        theme::text_tertiary(),
        rect.width(),
    );
    ui.painter().galley(
        pos2(
            rect.left() + 10.0,
            rect.center().y - galley.size().y / 2.0 + 2.0,
        ),
        galley,
        theme::text_tertiary(),
    );
}

/// A line of explanation under the rows, wrapped inside the list.
pub(super) fn note_line(ui: &mut Ui, text: &str) {
    let galley = kit::paragraph(
        ui,
        text,
        13.0,
        REGULAR,
        theme::text_secondary(),
        ui.available_width() - 20.0,
    );
    let (rect, _) = ui.allocate_exact_size(
        vec2(ui.available_width(), galley.size().y + 20.0),
        Sense::hover(),
    );
    ui.painter().galley(
        pos2(rect.left() + 10.0, rect.top() + 10.0),
        galley,
        theme::text_secondary(),
    );
}

//! The list area: the rows, the captions above groups of them and what stands in for them.

use super::columns::{self, Columns};
use super::dashboards;
use super::list::{self, RowState};
use super::lookup::{self, Lookup};
use super::notice::{NOTICE_HEIGHT, SettingsPage, show_notice};
use super::offer::{self, Offer, Primary};
use super::search::{Listing, Scope};
use super::{Env, Item, PADDING, Pick, PickKind, PickMode, SymbolPicker, predictions};
use crate::i18n::tr;
use crate::model::DataSource;
use crate::service::SearchResult;
use crate::service::provider_message;
use crate::ui::chart::picker_kit::widgets;
use crate::ui::kit::loading::LoadingIndicator;
use crate::ui::kit::scroll::Beyond;
use crate::ui::kit::{self, MEDIUM, REGULAR, SEMIBOLD};
use crate::ui::theme;
use eframe::egui::{self, Rect, Sense, Ui, Vec2, pos2, vec2};

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
        let replacing = self.replacing(env);
        kit::scroll::keep_bars_visible(&mut body);
        let width = body.available_width() - body.spacing().scroll.allocated_width();
        let columns = Columns::for_width(
            width,
            columns::venue_text_width(&body),
            env.can_overlay && self.mode != PickMode::Watchlist,
        );
        if self.shows_header() && !rows.items.is_empty() {
            column_header(&mut body, &columns, width);
        }
        let scrolled = egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(&mut body, |ui| {
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                ui.set_width(width);
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
                            let names = self.caches.names.of(result);
                            list::row(ui, result, &names, state, replacing, &columns)
                        }
                        Item::Choice(result) => predictions::rows::choice(
                            ui,
                            result,
                            predictions::is_ticked(self.ticked.as_ref(), result),
                            state,
                            &columns,
                        ),
                        Item::Event(head) => {
                            predictions::rows::event_header(ui, head, state.highlighted, &columns)
                        }
                        Item::Tile(tile) => dashboards::row(ui, tile, state, &columns),
                    };
                    // A hairline between neighbours, none against the highlighted box or a
                    // caption.
                    let next = index + 1;
                    if next < rows.items.len()
                        && !state.highlighted
                        && next != self.highlight
                        && rows.head_before(next).is_none()
                    {
                        list::paint_separator(ui, response.rect);
                    }
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
                        (!(state.offer.primary == Primary::Compare && state.overlaid))
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
        let viewport = scrolled.inner_rect;
        let beyond = Beyond::of(
            scrolled.state.offset.y,
            scrolled.content_size.y,
            viewport.height(),
        );
        kit::scroll::fade_edges(ui.painter(), viewport, theme::chrome(), beyond);
        picked
    }

    /// Whether the list is of markets, whose columns the captions name; events and tiles are
    /// titles that run across them.
    fn shows_header(&self) -> bool {
        matches!(
            self.scope,
            Scope::All | Scope::Stocks | Scope::Crypto | Scope::Forex | Scope::Watchlists
        )
    }

    /// How `item`'s row is drawn: highlighted, already held, drawn over the chart.
    pub(super) fn row_state(&self, index: usize, item: &Item, env: &Env) -> RowState {
        let market = item.market().filter(|_| !matches!(item, Item::Event(_)));
        RowState {
            highlighted: index == self.highlight,
            on_chart: offer::holds(self.mode, item, env),
            overlaid: market.is_some_and(|result| offer::overlaid(env, result)),
            offer: Offer::of(self.mode, Some(item), env),
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
            error.missing_key().is_none()
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
                &provider_message::message(error),
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
            rect.left() + columns::PAD,
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

/// The captions over the list's columns, with a hairline under them. They stay put while the rows
/// scroll.
fn column_header(ui: &mut Ui, columns: &Columns, width: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, HEADER_HEIGHT), Sense::hover());
    let captions = [
        Some((columns::PAD, tr!("Symbol"))),
        columns.description.map(|left| (left, tr!("Description"))),
        Some((columns.venue, tr!("Venue"))),
    ];
    for (left, caption) in captions.into_iter().flatten() {
        let galley = kit::galley(
            ui,
            caption,
            12.5,
            MEDIUM,
            theme::text_secondary(),
            rect.right() - rect.left() - left,
        );
        ui.painter().galley(
            pos2(rect.left() + left, rect.center().y - galley.size().y / 2.0),
            galley,
            theme::text_secondary(),
        );
    }
    list::paint_separator(ui, rect);
}

/// The height of the column captions.
pub(super) const HEADER_HEIGHT: f32 = 28.0;

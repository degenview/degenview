//! The symbol picker: a large modal opened from the symbol button at the left of the toolbar, or
//! with ⌘K. It searches crypto and stocks together (or either alone, or one exchange), lists the
//! dollar markets of an asset on every exchange first, shows the highlighted market's price on
//! the right, and replaces the focused chart's market with the one picked. A pick is reported as
//! an [`Output`]; the workspace applies it.
//!
//! It opens searching for the market the focused chart shows, so a different exchange or quote
//! of the same asset is one keystroke away. See `docs/symbol-picker.md`.

mod detail;
mod list;
pub mod search;
pub mod watchlists;

use super::add_chart_sheet::{self, RecentMarket};
use super::icons::{self, Icon};
use super::kit::{self, MEDIUM, REGULAR, SEMIBOLD};
use super::theme;
use crate::model::watchlist_quote::WatchlistQuote;
use crate::model::{DataSource, InstrumentId};
use crate::service::{self, SearchResult};
use crate::store::SharedStore;
use eframe::egui::{self, Color32, Id, Key, Modifiers, Rect, Sense, Stroke, Ui, Vec2, pos2, vec2};
use list::RowState;
use search::{Backend, Listing, Scope, SymbolSearch};
use watchlists::WatchlistGroup;

const SIZE: Vec2 = vec2(1000.0, 660.0);
const PADDING: f32 = 16.0;
const TABS: f32 = 44.0;
const FOOTER: f32 = 44.0;
const DETAIL: f32 = 372.0;
/// The detail panel is dropped below this window width, so the list keeps room.
const DETAIL_MIN_WIDTH: f32 = 760.0;
/// The search field asks for focus this many frames: the first only measures the modal.
const FOCUS_FRAMES: u8 = 3;

/// What the picker asks the workspace to do.
#[derive(Clone, Debug)]
pub enum Output {
    /// Show this market on the focused chart (or add a chart when none is focused). The picker
    /// has closed itself.
    Replace(SearchResult),
    /// Add this market as a chart of its own. The picker has closed itself.
    Add(SearchResult),
    /// Open Settings on the Alpaca page; the picker stays open.
    OpenAlpacaSettings,
}

/// What the workspace tells the picker each frame.
pub struct Env<'a> {
    /// The markets already on the tab's charts, which the list marks with a check.
    pub on_tab: &'a [InstrumentId],
    /// The price of [`SymbolPicker::highlighted`], when the quote feed has one.
    pub quote: Option<&'a WatchlistQuote>,
    /// The focused chart's market, which a pick replaces; `None` when a pick adds a chart.
    pub replacing: Option<&'a str>,
}

/// The quote-demand owner of the picker in tab `tab`: clear of the sidebar's (the tab id), the
/// alert sheet's (`1 << 62`) and the alert runtime's.
pub fn quote_owner(tab: u64) -> u64 {
    (1 << 61) + tab
}

pub struct SymbolPicker {
    store: SharedStore,
    backend: Backend,
    open: bool,
    query: String,
    scope: Scope,
    venue: Option<DataSource>,
    search: SymbolSearch,
    recents: Vec<RecentMarket>,
    /// The listed row the keyboard (or the pointer) is on.
    highlight: usize,
    /// The market that row held in the last frame, for the quote feed and the detail panel.
    highlighted: Option<SearchResult>,
    /// The highlight moved by key: scroll its row into view.
    reveal: bool,
    focus_frames: u8,
    select_all: bool,
    /// The user's watchlists, fed by the workspace while the Watchlists tab shows.
    watchlists: Vec<WatchlistGroup>,
    needs_alpaca: bool,
    /// The Alpaca notice's button was pressed.
    settings_requested: bool,
}

impl SymbolPicker {
    pub fn new(store: SharedStore) -> Self {
        Self::with_backend(store, Backend::LIVE)
    }

    fn with_backend(store: SharedStore, backend: Backend) -> Self {
        Self {
            store,
            backend,
            open: false,
            query: String::new(),
            scope: Scope::All,
            venue: None,
            search: SymbolSearch::new(backend),
            recents: Vec::new(),
            highlight: 0,
            highlighted: None,
            reveal: false,
            focus_frames: 0,
            select_all: false,
            watchlists: Vec::new(),
            needs_alpaca: false,
            settings_requested: false,
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Opens a fresh picker searching for `current`, the market the focused chart shows, with the
    /// text selected so typing replaces it.
    pub fn open(&mut self, current: &str) {
        let store = self.store.clone();
        *self = Self::with_backend(store, self.backend);
        self.recents = self
            .store
            .lock()
            .ok()
            .map(|store| add_chart_sheet::load_recents(&store))
            .unwrap_or_default();
        self.query = current.to_owned();
        self.search.ask_now();
        self.select_all = !current.is_empty();
        self.focus_frames = FOCUS_FRAMES;
        self.open = true;
    }

    /// The workspace should hand over the user's watchlists (see [`Self::set_watchlists`]).
    pub fn wants_watchlists(&self) -> bool {
        self.open && self.scope == Scope::Watchlists
    }

    pub fn set_watchlists(&mut self, groups: Vec<WatchlistGroup>) {
        if self.watchlists != groups {
            self.watchlists = groups;
        }
    }

    /// What the search box holds.
    #[cfg(test)]
    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn close(&mut self) {
        self.open = false;
        self.search.forget();
        self.highlighted = None;
    }

    /// The market whose price the detail panel wants: the workspace holds a demand on it.
    pub fn highlighted(&self) -> Option<InstrumentId> {
        self.highlighted
            .as_ref()
            .map(|result| InstrumentId::new(result.source, &result.full_symbol))
    }

    pub fn show(&mut self, ctx: &egui::Context, env: &Env) -> Option<Output> {
        if !self.open {
            return None;
        }
        let search_id = Id::new("symbol_picker_search");
        self.claim_keys(ctx, search_id);
        let keys = Keys::read(ctx);

        let listing = self.listing(ctx);
        let rows = self.rows(&listing);
        let mut output = None;
        if keys.scope {
            self.set_scope(self.scope.next());
        }
        if keys.down || keys.up {
            self.step(rows.results.len(), keys.down);
        }
        self.highlight = self.highlight.min(rows.results.len().saturating_sub(1));
        self.highlighted = rows.results.get(self.highlight).cloned();
        if keys.pick
            && let Some(result) = self.highlighted.clone()
        {
            output = self.commit(result, keys.add);
        }

        let screen = ctx.content_rect().size();
        let size = vec2(
            SIZE.x.min(screen.x - 48.0).max(520.0),
            SIZE.y.min(screen.y - 48.0).max(400.0),
        );
        let related = self
            .highlighted
            .clone()
            .and_then(|result| self.search.related(ctx, &result));
        let mut picked: Option<(SearchResult, bool)> = None;
        let modal = egui::Modal::new(Id::new("symbol_picker"))
            .backdrop_color(Color32::from_black_alpha(110))
            .frame(
                egui::Frame::new()
                    .fill(theme::chrome())
                    .corner_radius(12.0)
                    .stroke(Stroke::new(1.0, theme::ink(22)))
                    .shadow(egui::Shadow {
                        offset: [0, 12],
                        blur: 40,
                        spread: 0,
                        color: Color32::from_black_alpha(130),
                    }),
            )
            .show(ctx, |ui| {
                ui.set_width(size.x);
                ui.set_height(size.y);
                let full = Rect::from_min_size(ui.min_rect().min, size);
                picked = self.layout(
                    ui,
                    full,
                    search_id,
                    &listing,
                    &rows,
                    related.as_deref(),
                    env,
                );
            });
        // A click on the dimmed window outside the sheet closes it.
        let clicked_outside = modal.backdrop_response.clicked();

        if let Some((result, add)) = picked {
            output = output.or_else(|| self.commit(result, add));
        }
        if self.select_all && ctx.memory(|memory| memory.has_focus(search_id)) {
            select_all(ctx, search_id, &self.query);
            self.select_all = false;
        }
        if std::mem::take(&mut self.settings_requested) {
            output = Some(Output::OpenAlpacaSettings);
        }
        if keys.close || clicked_outside {
            self.close();
        }
        if matches!(output, Some(Output::Replace(_) | Output::Add(_))) {
            self.close();
        }
        output
    }

    /// Keeps Tab and the arrows with the picker while the box has focus, instead of moving focus.
    fn claim_keys(&self, ctx: &egui::Context, id: Id) {
        ctx.memory_mut(|memory| {
            memory.set_focus_lock_filter(
                id,
                egui::EventFilter {
                    tab: true,
                    horizontal_arrows: false,
                    vertical_arrows: true,
                    escape: false,
                },
            );
        });
    }

    fn listing(&mut self, ctx: &egui::Context) -> Listing {
        if !self.scope.searches() {
            return Listing {
                results: Vec::new(),
                searching: false,
                errors: Vec::new(),
            };
        }
        self.search
            .listing(ctx, &self.query, self.scope, self.venue)
    }

    /// The markets in the list with the captions above groups of them: the answers to the query,
    /// the watchlists' markets, or the recent (else popular) picks while the box is empty.
    fn rows(&self, listing: &Listing) -> Rows {
        let in_scope = |result: &SearchResult| {
            self.scope.includes(result.source)
                && self.venue.is_none_or(|venue| venue == result.source)
        };
        if self.scope == Scope::Watchlists {
            let mut rows = Rows::default();
            for group in &self.watchlists {
                let before = rows.results.len();
                rows.results.extend(
                    group
                        .markets
                        .iter()
                        .filter(|market| {
                            in_scope(market) && watchlists::matches(market, &self.query)
                        })
                        .cloned(),
                );
                if rows.results.len() > before {
                    rows.heads.push((before, group.name.clone()));
                }
            }
            return rows;
        }
        if !self.query.trim().is_empty() {
            return Rows::of(listing.results.clone(), None);
        }
        let recent: Vec<SearchResult> = self
            .recents
            .iter()
            .map(RecentMarket::result)
            .filter(in_scope)
            .collect();
        if recent.is_empty() {
            Rows::of(
                search::popular().into_iter().filter(in_scope).collect(),
                Some("Popular"),
            )
        } else {
            Rows::of(recent, Some("Recent"))
        }
    }

    fn step(&mut self, rows: usize, down: bool) {
        if rows == 0 {
            return;
        }
        self.highlight = if down {
            (self.highlight + 1) % rows
        } else {
            (self.highlight + rows - 1) % rows
        };
        self.reveal = true;
    }

    fn set_scope(&mut self, scope: Scope) {
        if self.scope == scope {
            return;
        }
        self.scope = scope;
        // A venue of another scope no longer applies.
        if self.venue.is_some_and(|venue| !scope.includes(venue)) {
            self.venue = None;
        }
        self.highlight = 0;
        self.search.ask_now();
    }

    fn set_venue(&mut self, venue: Option<DataSource>) {
        if self.venue != venue {
            self.venue = venue;
            self.highlight = 0;
            self.search.ask_now();
        }
    }

    /// Validates a pick, records it as recent and hands it to the host.
    fn commit(&mut self, result: SearchResult, add: bool) -> Option<Output> {
        if result.source == DataSource::Alpaca
            && service::keychain_api_keys::alpaca_credentials().is_none()
        {
            self.needs_alpaca = true;
            return None;
        }
        if add_chart_sheet::record_recent(&mut self.recents, &result)
            && let Ok(store) = self.store.lock()
        {
            add_chart_sheet::store_recents(&store, &self.recents);
        }
        Some(if add {
            Output::Add(result)
        } else {
            Output::Replace(result)
        })
    }

    // MARK: Layout

    #[allow(clippy::too_many_arguments)]
    fn layout(
        &mut self,
        ui: &mut Ui,
        full: Rect,
        search_id: Id,
        listing: &Listing,
        rows: &Rows,
        related: Option<&[SearchResult]>,
        env: &Env,
    ) -> Option<(SearchResult, bool)> {
        let mut picked = None;
        let search_area = Rect::from_min_size(
            full.min + vec2(PADDING, PADDING),
            vec2(full.width() - 2.0 * PADDING, 44.0),
        );
        let mut field = ui.new_child(egui::UiBuilder::new().max_rect(search_area));
        let focus = self.focus_frames > 0;
        self.focus_frames = self.focus_frames.saturating_sub(1);
        let event = add_chart_sheet::search_box(
            &mut field,
            &mut self.query,
            listing.searching,
            "Search symbol…",
            search_id,
            focus,
        );
        if event.changed || event.cleared {
            if event.cleared {
                self.query.clear();
            }
            self.search.edited();
            self.highlight = 0;
            self.needs_alpaca = false;
        }

        let tabs = Rect::from_min_size(
            pos2(full.left(), search_area.bottom() + 6.0),
            vec2(full.width(), TABS),
        );
        let (scope, venue) = self.tab_row(ui, tabs);
        if let Some(scope) = scope {
            self.set_scope(scope);
        }
        if let Some(venue) = venue {
            self.set_venue(venue);
        }
        let body_top = tabs.bottom();
        ui.painter().hline(
            full.x_range(),
            body_top,
            Stroke::new(1.0, theme::separator()),
        );
        let footer_top = full.bottom() - FOOTER;
        ui.painter().hline(
            full.x_range(),
            footer_top,
            Stroke::new(1.0, theme::separator()),
        );

        let detail_width = if full.width() >= DETAIL_MIN_WIDTH {
            DETAIL
        } else {
            0.0
        };
        let list_area = Rect::from_min_max(
            pos2(full.left(), body_top + 1.0),
            pos2(full.right() - detail_width, footer_top),
        );
        if let Some(hit) = self.list(ui, list_area, listing, rows, env) {
            picked = Some(hit);
        }
        if detail_width > 0.0 {
            let panel = Rect::from_min_max(
                pos2(list_area.right(), body_top + 1.0),
                pos2(full.right(), footer_top),
            );
            ui.painter().vline(
                panel.left(),
                panel.y_range(),
                Stroke::new(1.0, theme::separator()),
            );
            if let Some(result) = &self.highlighted {
                let shown = detail::Detail {
                    result,
                    quote: env.quote,
                    related,
                };
                if let Some(other) = detail::show(ui, panel, &shown) {
                    picked = Some((other, false));
                }
            }
        }
        self.footer(
            ui,
            Rect::from_min_max(pos2(full.left(), footer_top + 1.0), full.max),
            env,
        );
        picked
    }

    /// The market tabs and the venue filter. Returns what was chosen this frame.
    fn tab_row(&self, ui: &mut Ui, area: Rect) -> (Option<Scope>, Option<Option<DataSource>>) {
        let mut scope = None;
        let mut x = area.left() + PADDING;
        for tab in Scope::TABS {
            let active = tab == self.scope;
            let label = kit::galley(
                ui,
                tab.title(),
                15.0,
                if active { SEMIBOLD } else { MEDIUM },
                if active {
                    theme::text_primary()
                } else {
                    theme::text_secondary()
                },
                f32::INFINITY,
            );
            let width = label.size().x + 28.0;
            let rect = Rect::from_min_size(pos2(x, area.top()), vec2(width, area.height()));
            let response = ui
                .interact(
                    rect,
                    Id::new(("symbol_picker_tab", tab.title())),
                    Sense::click(),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            let response = crate::ui::accessibility::label(response, tab.title());
            let color = if active || response.hovered() {
                theme::text_primary()
            } else {
                theme::text_secondary()
            };
            ui.painter().galley(
                pos2(
                    rect.center().x - label.size().x / 2.0,
                    rect.center().y - label.size().y / 2.0,
                ),
                label,
                color,
            );
            if active {
                ui.painter().rect_filled(
                    Rect::from_min_max(
                        pos2(rect.left() + 8.0, rect.bottom() - 2.0),
                        pos2(rect.right() - 8.0, rect.bottom()),
                    ),
                    1.0,
                    theme::text_primary(),
                );
            }
            if response.clicked() {
                scope = Some(tab);
            }
            x += width;
        }
        let venue = self.venue_filter(ui, area);
        (scope, venue)
    }

    /// The pull-down at the right of the tabs that narrows the list to one exchange.
    fn venue_filter(&self, ui: &mut Ui, area: Rect) -> Option<Option<DataSource>> {
        let venues = self.scope.venues();
        let label = self
            .venue
            .map_or("All venues", |venue| venue.display_name());
        let galley = kit::galley(ui, label, 14.0, MEDIUM, theme::text_secondary(), 180.0);
        let width = 16.0 + 6.0 + galley.size().x + 6.0 + 10.0 + 8.0;
        let rect = Rect::from_min_size(
            pos2(area.right() - PADDING - width, area.center().y - 16.0),
            vec2(width, 32.0),
        );
        let response = ui
            .interact(rect, Id::new("symbol_picker_venue"), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let response = crate::ui::accessibility::label(response, "Filter by exchange");
        let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&response));
        if response.hovered() || open {
            ui.painter().rect_filled(rect, 6.0, theme::ink(10));
        }
        let color = if self.venue.is_some() {
            theme::text_primary()
        } else {
            theme::text_secondary()
        };
        let mut x = rect.left() + 8.0;
        icons::paint(
            ui.painter(),
            Rect::from_center_size(pos2(x + 8.0, rect.center().y), Vec2::splat(16.0)),
            Icon::Filter,
            color,
        );
        x += 22.0;
        ui.painter().galley(
            pos2(x, rect.center().y - galley.size().y / 2.0),
            galley.clone(),
            color,
        );
        x += galley.size().x + 6.0;
        icons::paint(
            ui.painter(),
            Rect::from_center_size(pos2(x + 5.0, rect.center().y), Vec2::splat(10.0)),
            Icon::ChevronDown,
            theme::text_secondary(),
        );

        let mut labels = vec!["All venues".to_owned()];
        labels.extend(venues.iter().map(|venue| venue.display_name().to_owned()));
        let mut chosen = None;
        kit::fitted_menu(ui, &response, egui::RectAlign::BOTTOM_END, &labels, |ui| {
            kit::menu_scope(ui, 0.0);
            let rows = std::iter::once(None).chain(venues.iter().map(|venue| Some(*venue)));
            for venue in rows {
                let name = venue.map_or("All venues", |venue| venue.display_name());
                let mut row = kit::MenuRow::new(name);
                row.current = self.venue == venue;
                if kit::menu_row(ui, row).clicked() {
                    chosen = Some(venue);
                    ui.close();
                }
            }
        });
        chosen
    }

    /// The rows, or what stands in for them. Returns a pick: `(market, add as a new chart)`.
    fn list(
        &mut self,
        ui: &mut Ui,
        area: Rect,
        listing: &Listing,
        rows: &Rows,
        env: &Env,
    ) -> Option<(SearchResult, bool)> {
        let mut picked = None;
        // The Alpaca notice sits at the bottom of the list's own area, under the rows.
        let notice = self.notice(listing);
        let reserved = notice.as_ref().map_or(0.0, |_| NOTICE_HEIGHT + 20.0);
        let content = Rect::from_min_max(area.min, pos2(area.max.x, area.max.y - reserved));
        if let Some(notice) = &notice {
            let card = Rect::from_min_max(
                pos2(area.left() + PADDING - 6.0, content.bottom() + 6.0),
                pos2(area.right() - (PADDING - 6.0) - 8.0, area.bottom() - 14.0),
            );
            if show_notice(ui, card, notice) {
                self.settings_requested = true;
            }
        }
        let mut body = ui
            .new_child(egui::UiBuilder::new().max_rect(content.shrink2(vec2(PADDING - 6.0, 6.0))));
        body.set_clip_rect(content);
        let empty_query = self.query.trim().is_empty();
        if rows.results.is_empty() {
            self.empty_state(&mut body, listing, empty_query);
            return None;
        }
        let pointer_moved = ui.input(|input| input.pointer.delta() != Vec2::ZERO);
        let reveal = std::mem::take(&mut self.reveal);
        let width = body.available_width();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(&mut body, |ui| {
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                ui.set_width(width - 8.0);
                for (index, result) in rows.results.iter().enumerate() {
                    if let Some(head) = rows.head_before(index) {
                        section_heading(ui, head);
                    }
                    let state = RowState {
                        highlighted: index == self.highlight,
                        on_chart: env.on_tab.iter().any(|id| {
                            id.is_same_market(&InstrumentId::new(
                                result.source,
                                &result.full_symbol,
                            ))
                        }),
                    };
                    let (response, out) =
                        list::row(ui, result, state, env.replacing, ui.available_width());
                    if reveal && state.highlighted {
                        response.scroll_to_me(None);
                    }
                    if (out.hovered && pointer_moved) || out.opened_menu {
                        self.highlight = index;
                    }
                    if out.add {
                        picked = Some((result.clone(), true));
                    } else if out.pick {
                        picked = Some((result.clone(), false));
                    }
                }
            });
        picked
    }

    fn empty_state(&self, ui: &mut Ui, listing: &Listing, empty_query: bool) {
        if self.scope == Scope::Watchlists {
            let (title, detail) = if self.watchlists.is_empty() {
                (
                    "No watchlist markets yet",
                    "Crypto and stocks you add to a watchlist show up here.".to_owned(),
                )
            } else {
                (
                    "No results",
                    format!(
                        "Nothing in your watchlists matches “{}”.",
                        self.query.trim()
                    ),
                )
            };
            add_chart_sheet::no_results(ui, title, &detail);
        } else if empty_query {
            add_chart_sheet::no_results(
                ui,
                "Search stocks and crypto",
                "Type a ticker, a company or a pair: BTC, AAPL, ETHUSDT.",
            );
        } else if listing.searching {
            let rect = ui.available_rect_before_wrap();
            ui.put(
                Rect::from_center_size(rect.center(), Vec2::splat(22.0)),
                egui::Spinner::new()
                    .size(20.0)
                    .color(theme::text_secondary()),
            );
        } else if let Some((source, message)) = listing
            .errors
            .iter()
            .find(|(source, _)| *source != DataSource::Alpaca)
        {
            add_chart_sheet::no_results(
                ui,
                &format!("{} could not be searched", source.display_name()),
                message,
            );
        } else {
            add_chart_sheet::no_results(
                ui,
                "No results",
                &format!(
                    "Nothing matches “{}”. Try a ticker or a company name.",
                    self.query.trim()
                ),
            );
        }
    }

    /// What the card under the list says, when stocks cannot be searched or opened: Alpaca keys
    /// are missing (the search said so, or a stock was picked without them) or its search failed.
    fn notice(&self, listing: &Listing) -> Option<Notice> {
        let failure = listing
            .errors
            .iter()
            .find(|(source, _)| *source == DataSource::Alpaca)
            .map(|(_, message)| message.as_str());
        match failure {
            Some(message) if message != service::ALPACA_MISSING_KEY => Some(Notice {
                title: "Stocks could not be searched",
                detail: message.to_owned(),
                settings: false,
            }),
            Some(_) => Some(Notice::missing_key("Stocks need an Alpaca key")),
            None if self.needs_alpaca => Some(Notice::missing_key("Set up Alpaca to open a stock")),
            None => None,
        }
    }

    /// The keys that work here. A narrow picker drops the least needed hints rather than run past
    /// its edge.
    fn footer(&self, ui: &mut Ui, area: Rect, env: &Env) {
        let open = match env.replacing {
            Some(current) => format!("Replace {current}"),
            None => "Add chart".to_owned(),
        };
        let mut hints: Vec<(&[&str], String)> =
            vec![(&["↑", "↓"], "Navigate".to_owned()), (&["↵"], open)];
        if env.replacing.is_some() {
            hints.push((&["⇧", "↵"], "Add as new chart".to_owned()));
        }
        hints.push((&["Tab"], "Switch market".to_owned()));
        hints.push((&["Esc"], "Close".to_owned()));

        let room = area.width() - 2.0 * PADDING;
        let width = |hint: &(&[&str], String)| -> f32 {
            let label = kit::galley(
                ui,
                &hint.1,
                13.0,
                REGULAR,
                theme::text_secondary(),
                f32::INFINITY,
            );
            let keys: f32 = hint
                .0
                .iter()
                .map(|key| {
                    kit::galley(
                        ui,
                        key,
                        kit::size::CAPTION + 0.5,
                        MEDIUM,
                        theme::text_secondary(),
                        f32::INFINITY,
                    )
                    .size()
                    .x + 12.0
                        + 6.0
                })
                .sum();
            keys + label.size().x + 6.0 + 10.0
        };
        for least_needed in ["Add as new chart", "Switch market", "Navigate"] {
            if hints.iter().map(width).sum::<f32>() <= room {
                break;
            }
            hints.retain(|hint| hint.1 != least_needed);
        }

        let mut row = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(area.shrink2(vec2(PADDING, 0.0)))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        row.spacing_mut().item_spacing.x = 6.0;
        for (keys, label) in &hints {
            for key in *keys {
                crate::ui::replay_bar::keycap(&mut row, key);
            }
            row.label(kit::text(label, 13.0, REGULAR, theme::text_secondary()));
            row.add_space(10.0);
        }
    }
}

/// The markets the list holds and the captions above groups of them: `(first row, caption)`.
#[derive(Default)]
struct Rows {
    results: Vec<SearchResult>,
    heads: Vec<(usize, String)>,
}

impl Rows {
    fn of(results: Vec<SearchResult>, head: Option<&str>) -> Self {
        let heads = head
            .filter(|_| !results.is_empty())
            .map(|head| vec![(0, head.to_owned())])
            .unwrap_or_default();
        Self { results, heads }
    }

    fn head_before(&self, row: usize) -> Option<&str> {
        self.heads
            .iter()
            .find(|(first, _)| *first == row)
            .map(|(_, head)| head.as_str())
    }
}

const NOTICE_HEIGHT: f32 = 60.0;

/// A card under the list: a title, one line of detail and, when Settings can fix it, a button.
struct Notice {
    title: &'static str,
    detail: String,
    settings: bool,
}

impl Notice {
    fn missing_key(title: &'static str) -> Self {
        Self {
            title,
            detail: "Add your free Alpaca API keys in Settings to search and chart US stocks."
                .to_owned(),
            settings: true,
        }
    }
}

/// Draws `notice` in `card`; returns whether its button was pressed.
fn show_notice(ui: &mut Ui, card: Rect, notice: &Notice) -> bool {
    ui.painter()
        .rect_filled(card, kit::CARD_RADIUS, theme::card_fill(false));
    ui.painter().rect_stroke(
        card,
        kit::CARD_RADIUS,
        Stroke::new(1.0, theme::card_border(false)),
        egui::StrokeKind::Inside,
    );
    let badge = Rect::from_center_size(
        pos2(card.left() + 14.0 + 17.0, card.center().y),
        Vec2::splat(34.0),
    );
    kit::paint_card_badge(ui, badge, Icon::Key);

    let mut pressed = false;
    let mut right = card.right() - 14.0;
    if notice.settings {
        let button = Rect::from_center_size(pos2(right - 52.0, card.center().y), vec2(104.0, 30.0));
        let mut cell = ui.new_child(egui::UiBuilder::new().max_rect(button));
        pressed = add_chart_sheet::sheet_button(&mut cell, "Open Settings", false, true).clicked();
        right = button.left() - 12.0;
    }
    let text_left = badge.right() + 12.0;
    let width = (right - text_left).max(40.0);
    let title = kit::galley(
        ui,
        notice.title,
        14.0,
        SEMIBOLD,
        theme::text_primary(),
        width,
    );
    let detail = kit::galley(
        ui,
        &notice.detail,
        12.5,
        REGULAR,
        theme::text_secondary(),
        width,
    );
    let block = title.size().y + 2.0 + detail.size().y;
    let top = card.center().y - block / 2.0;
    let detail_top = top + title.size().y + 2.0;
    ui.painter()
        .galley(pos2(text_left, top), title, theme::text_primary());
    ui.painter()
        .galley(pos2(text_left, detail_top), detail, theme::text_secondary());
    pressed
}

/// A small capitalised caption above a group of rows.
fn section_heading(ui: &mut Ui, title: &str) {
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

/// Selects all of `text` in the text field `id`, replacing a cursor left over from an earlier sheet.
fn select_all(ctx: &egui::Context, id: Id, text: &str) {
    use egui::text::{CCursor, CCursorRange};
    let mut edit = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
    edit.cursor.set_char_range(Some(CCursorRange::two(
        CCursor::new(0),
        CCursor::new(text.chars().count()),
    )));
    edit.store(ctx, id);
}

/// What the keyboard asked for this frame.
#[derive(Default)]
struct Keys {
    up: bool,
    down: bool,
    /// Return: use the highlighted market.
    pick: bool,
    /// ⇧Return: add it as a new chart rather than replacing.
    add: bool,
    scope: bool,
    close: bool,
}

impl Keys {
    fn read(ctx: &egui::Context) -> Self {
        ctx.input_mut(|input| {
            let add = input.consume_key(Modifiers::SHIFT, Key::Enter);
            Self {
                up: input.consume_key(Modifiers::NONE, Key::ArrowUp),
                down: input.consume_key(Modifiers::NONE, Key::ArrowDown),
                pick: add || input.consume_key(Modifiers::NONE, Key::Enter),
                add,
                scope: input.consume_key(Modifiers::NONE, Key::Tab),
                close: input.consume_key(Modifiers::NONE, Key::Escape),
            }
        })
    }
}

#[cfg(test)]
mod tests;

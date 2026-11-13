//! The indicator picker: a large modal opened from the toolbar, next to the timeframe selector.
//! A sidebar (Yours, Built-in, Library, Families), a search field, a grid of cards with a small
//! generated chart each, and a detail panel with the add button. A pick is reported as an
//! [`Output`]; the workspace applies it to the focused chart.
//!
//! The list comes from `model::indicators::catalog`; see `docs/indicator-picker.md`.

mod preview;
mod sections;

use super::indicator_strip::Action;
use crate::model::indicators::catalog::{self, CatalogEntry, EntryKey, Family, Origin};
use crate::model::{DataSource, IndicatorConfig};
use crate::pine_studio::indicators;
use crate::store::SharedStore;
use crate::ui::icons::{self, Icon};
use crate::ui::kit::{self, MEDIUM, SEMIBOLD, size};
use crate::ui::pine::script_catalog;
use crate::ui::theme;
use eframe::egui::{
    self, Color32, Id, Key, Modifiers, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2,
};
use preview::Previews;
use sections::{Block, Section};
use std::collections::HashSet;

/// Where the favorites list is kept (`sync::record::policy` lists it as a portable setting).
pub const FAVORITES_KEY: &str = "indicatorPicker.favorites";

const SIZE: egui::Vec2 = vec2(1040.0, 680.0);
const HEADER: f32 = 56.0;
const FOOTER: f32 = 44.0;
const SIDEBAR: f32 = 224.0;
const DETAIL: f32 = 304.0;
/// The detail panel is dropped below this window width, so the grid keeps room.
const DETAIL_MIN_WIDTH: f32 = 900.0;
const CARD_HEIGHT: f32 = 196.0;
const PREVIEW_HEIGHT: f32 = 128.0;
const GAP: f32 = 12.0;
/// The search field asks for focus this many frames: the first only measures the modal.
const FOCUS_FRAMES: u8 = 3;

/// The chart an indicator would be added to.
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub config: IndicatorConfig,
    pub source: DataSource,
    /// The script ids already on the chart (a bundled one is `builtin:<id>`).
    pub on_chart: Vec<String>,
}

impl Target {
    fn has(&self, entry: &CatalogEntry) -> bool {
        match &entry.key {
            EntryKey::Native(kind) => kind.applied(&self.config),
            EntryKey::Bundled(id) => self
                .on_chart
                .iter()
                .any(|script| script == &format!("{}{id}", indicators::SCRIPT_ID_PREFIX)),
            EntryKey::Script(id) => self.on_chart.contains(id),
        }
    }
}

/// What the add button does for one entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Availability {
    Add,
    /// A script or bundled indicator that is already on the chart: another copy can be added.
    AddAnother,
    /// A native indicator that is on the chart: the button takes it off.
    Remove,
    Blocked(&'static str),
}

/// What the entry can do on `target`.
pub fn availability(entry: &CatalogEntry, target: Option<&Target>) -> Availability {
    let Some(target) = target else {
        return Availability::Blocked("Open a chart to add indicators to it.");
    };
    if !target.source.takes_indicators() {
        return Availability::Blocked("This chart takes no indicators.");
    }
    match &entry.key {
        EntryKey::Native(kind) if kind.applied(&target.config) => Availability::Remove,
        _ if target.has(entry) => Availability::AddAnother,
        _ => Availability::Add,
    }
}

/// The chart action behind an entry's button, or `None` when it is blocked.
pub fn action_for(entry: &CatalogEntry, availability: Availability) -> Option<Action> {
    match (availability, &entry.key) {
        (Availability::Blocked(_), _) => None,
        (Availability::Remove, EntryKey::Native(kind)) => Some(Action::Remove(*kind)),
        (_, EntryKey::Native(kind)) => Some(Action::Add(*kind)),
        (_, EntryKey::Bundled(id)) => indicators::find(id).map(Action::AddBundled),
        (_, EntryKey::Script(_)) => entry.script.clone().map(Action::AddScript),
    }
}

/// What the picker asks the workspace to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Output {
    /// Apply to the focused chart; `keep_open` leaves the picker up for the next one.
    Apply { action: Action, keep_open: bool },
    /// "Open Pine Studio" in the sidebar.
    OpenPineStudio,
}

/// A keystroke the picker takes, read once per frame before anything is drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Keys {
    down: bool,
    up: bool,
    left: bool,
    right: bool,
    enter: bool,
    shift_enter: bool,
    favorite: bool,
}

pub struct IndicatorPicker {
    open: bool,
    store: Option<SharedStore>,
    query: String,
    section: Section,
    selected: Option<EntryKey>,
    favorites: Vec<String>,
    shuffle: u64,
    focus_frames: u8,
    scroll_to_selected: bool,
    previews: Previews,
}

impl IndicatorPicker {
    pub fn new(store: Option<SharedStore>) -> Self {
        // A favorite that names an indicator this build no longer has is dropped.
        let favorites: Vec<String> = store
            .as_ref()
            .and_then(|store| store.lock().ok()?.setting::<Vec<String>>(FAVORITES_KEY))
            .unwrap_or_default()
            .into_iter()
            .filter(|stored| EntryKey::parse(stored).is_some())
            .collect();
        Self {
            open: false,
            store,
            query: String::new(),
            section: Section::Discover,
            selected: None,
            favorites,
            shuffle: 1,
            focus_frames: 0,
            scroll_to_selected: false,
            previews: Previews::default(),
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn open(&mut self) {
        self.open = true;
        self.query.clear();
        self.focus_frames = 0;
        self.selected = None;
    }

    fn favorite_set(&self) -> HashSet<String> {
        self.favorites.iter().cloned().collect()
    }

    fn is_favorite(&self, key: &EntryKey) -> bool {
        self.favorites.contains(&key.stored())
    }

    fn toggle_favorite(&mut self, key: &EntryKey) {
        let stored = key.stored();
        if let Some(position) = self.favorites.iter().position(|saved| *saved == stored) {
            self.favorites.remove(position);
        } else {
            self.favorites.push(stored);
        }
        if let Some(store) = &self.store
            && let Ok(store) = store.lock()
        {
            let _ = store.set_setting(FAVORITES_KEY, &self.favorites);
        }
    }

    /// Shows the modal while it is open and returns what was asked of it this frame.
    pub fn show(&mut self, ctx: &egui::Context, target: Option<&Target>) -> Option<Output> {
        if !self.open {
            return None;
        }
        let scripts = script_catalog::scripts(ctx);
        // Saved scripts only run on charts that take indicators, so the others list none.
        let scripts: &[_] = match target {
            Some(target) if !target.source.takes_indicators() => &[],
            _ => &scripts[..],
        };
        let all = catalog::all(scripts);
        let favorites = self.favorite_set();
        let searching = !self.query.trim().is_empty();
        let blocks = if searching {
            sections::search_blocks(&all, &self.query)
        } else {
            sections::blocks(self.section, &all, &favorites, self.shuffle)
        };

        let search_id = Id::new("indicator_picker_search");
        let search_focused = ctx.memory(|memory| memory.has_focus(search_id));
        let keys = read_keys(ctx, search_focused, self.query.is_empty());

        let screen = ctx.content_rect().size();
        let size = vec2(
            SIZE.x.min(screen.x - 32.0).max(620.0),
            SIZE.y.min(screen.y - 32.0).max(440.0),
        );
        // A click changes the selection, the section or the favorites after the panels were drawn
        // for this frame; without another frame they would show the old state until the next input.
        let state_before = (self.selected.clone(), self.section, self.favorites.clone());
        let mut output = None;
        let mut section_to = None;
        let mut query_changed = false;

        let response = egui::Modal::new(Id::new("indicator_picker"))
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
                let origin = ui.min_rect().min;
                let full = Rect::from_min_size(origin, size);
                self.header(ui, full, search_id, &mut query_changed, all.len());
                let body = Rect::from_min_max(
                    pos2(full.left(), full.top() + HEADER),
                    pos2(full.right(), full.bottom() - FOOTER),
                );
                let detail_width = if size.x >= DETAIL_MIN_WIDTH {
                    DETAIL
                } else {
                    0.0
                };
                let sidebar =
                    Rect::from_min_max(body.min, pos2(body.left() + SIDEBAR, body.bottom()));
                let detail =
                    Rect::from_min_max(pos2(body.right() - detail_width, body.top()), body.max);
                let grid = Rect::from_min_max(
                    pos2(sidebar.right(), body.top()),
                    pos2(detail.left(), body.bottom()),
                );

                self.sidebar(
                    ui,
                    sidebar,
                    &all,
                    &favorites,
                    &mut section_to,
                    &mut output,
                    searching,
                );
                let flat = sections::flatten(&blocks);
                let selected_index = self.resolve_selection(&flat, &keys, grid.width());
                self.grid(
                    ui,
                    grid,
                    &blocks,
                    &flat,
                    selected_index,
                    target,
                    &mut output,
                );
                if detail_width > 0.0 {
                    let entry = selected_index.and_then(|index| flat.get(index).copied());
                    self.detail(ui, detail, entry, target, &mut output);
                }
                self.footer(ui, full, flat.len());
                let hairline = Stroke::new(1.0, theme::separator());
                let painter = ui.painter();
                painter.hline(full.x_range(), full.top() + HEADER, hairline);
                painter.hline(full.x_range(), full.bottom() - FOOTER, hairline);
                painter.vline(sidebar.right(), body.y_range(), hairline);
                if detail_width > 0.0 {
                    painter.vline(detail.left(), body.y_range(), hairline);
                }
                selected_index.and_then(|index| flat.get(index).map(|entry| (*entry).clone()))
            });

        let close_requested = response.should_close();
        let picked = response.inner;
        if let Some(section) = section_to {
            self.section = section;
            self.query.clear();
            self.selected = None;
        }
        if query_changed {
            self.selected = None;
        }
        if let Some(entry) = &picked {
            // `resolve_selection` already kept the selection; a click this frame may have moved it.
            let status = availability(entry, target);
            if keys.favorite {
                self.toggle_favorite(&entry.key);
            }
            let wanted = if keys.shift_enter || keys.enter {
                // Return never takes an indicator off: that is the button's job.
                action_for(entry, status).filter(|_| status != Availability::Remove)
            } else {
                None
            };
            if let Some(action) = wanted {
                output = Some(Output::Apply {
                    action,
                    keep_open: keys.shift_enter,
                });
            }
        }
        if close_requested {
            self.open = false;
        }
        match &output {
            Some(Output::Apply {
                keep_open: false, ..
            })
            | Some(Output::OpenPineStudio) => {
                self.open = false;
            }
            _ => {}
        }
        if state_before != (self.selected.clone(), self.section, self.favorites.clone()) {
            ctx.request_repaint();
        }
        output
    }

    /// Moves the selection for this frame's keys and returns the selected index in `flat`.
    fn resolve_selection(
        &mut self,
        flat: &[&CatalogEntry],
        keys: &Keys,
        grid_width: f32,
    ) -> Option<usize> {
        if flat.is_empty() {
            return None;
        }
        let columns = columns_for(grid_width);
        let current = self
            .selected
            .as_ref()
            .and_then(|key| flat.iter().position(|entry| &entry.key == key))
            .unwrap_or(0);
        let last = flat.len() - 1;
        let mut next = current;
        if keys.down {
            next = (current + columns).min(last);
        }
        if keys.up {
            next = current.saturating_sub(columns);
        }
        if keys.right {
            next = (current + 1).min(last);
        }
        if keys.left {
            next = current.saturating_sub(1);
        }
        if next != current {
            self.scroll_to_selected = true;
        }
        self.selected = Some(flat[next].key.clone());
        Some(next)
    }

    fn header(
        &mut self,
        ui: &mut Ui,
        full: Rect,
        search_id: Id,
        query_changed: &mut bool,
        total: usize,
    ) {
        let bar = Rect::from_min_size(full.min, vec2(full.width(), HEADER));
        let title = kit::galley(
            ui,
            "Indicators",
            size::TITLE2,
            SEMIBOLD,
            theme::text_primary(),
            150.0,
        );
        ui.painter().galley(
            pos2(bar.left() + 24.0, bar.center().y - title.size().y / 2.0),
            title,
            theme::text_primary(),
        );
        let field = Rect::from_min_max(
            pos2(bar.left() + SIDEBAR, bar.top() + 12.0),
            pos2(bar.right() - 24.0, bar.bottom() - 12.0),
        );
        let mut field_ui = ui.new_child(egui::UiBuilder::new().max_rect(field));
        let before = self.query.clone();
        let prompt = format!("Search {total} indicators, concepts or aliases");
        let response = kit::search_field(
            &mut field_ui,
            search_id,
            &mut self.query,
            &prompt,
            field.width(),
        );
        if self.focus_frames < FOCUS_FRAMES {
            self.focus_frames += 1;
            response.request_focus();
        }
        *query_changed = before != self.query;
    }

    #[allow(clippy::too_many_arguments)]
    fn sidebar(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        all: &[CatalogEntry],
        favorites: &HashSet<String>,
        section_to: &mut Option<Section>,
        output: &mut Option<Output>,
        searching: bool,
    ) {
        let mut child =
            ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(vec2(10.0, 8.0))));
        child.set_clip_rect(rect);
        egui::ScrollArea::vertical()
            .id_salt("indicator_picker_sidebar")
            .auto_shrink([false, false])
            .min_scrolled_height(0.0)
            .show(&mut child, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                let width = ui.available_width();
                let mut group = |ui: &mut Ui, heading: &str, list: &[Section]| {
                    sidebar_heading(ui, heading);
                    for section in list {
                        let selected = !searching && self.section == *section;
                        let count = sections::count(*section, all, favorites);
                        if sidebar_row(ui, width, section.icon(), section.title(), count, selected)
                            .clicked()
                        {
                            *section_to = Some(*section);
                        }
                    }
                };
                group(ui, "Yours", &sections::YOURS);
                if sidebar_row(
                    ui,
                    width,
                    Icon::AngleBrackets,
                    "Open Pine Studio",
                    None,
                    false,
                )
                .clicked()
                {
                    *output = Some(Output::OpenPineStudio);
                }
                group(ui, "Built-in", &sections::BUILT_IN);
                group(ui, "Library", &sections::LIBRARY);
                let families: Vec<Section> = Family::ALL.into_iter().map(Section::Family).collect();
                group(ui, "Families", &families);
            });
    }

    #[allow(clippy::too_many_arguments)]
    fn grid(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        blocks: &[Block],
        flat: &[&CatalogEntry],
        selected_index: Option<usize>,
        target: Option<&Target>,
        output: &mut Option<Output>,
    ) {
        let mut child =
            ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(vec2(16.0, 12.0))));
        child.set_clip_rect(rect);
        let scroll = std::mem::take(&mut self.scroll_to_selected);
        let mut toggled = None;
        let mut clicked = None;
        egui::ScrollArea::vertical()
            .id_salt("indicator_picker_grid")
            .auto_shrink([false, false])
            .min_scrolled_height(0.0)
            .show(&mut child, |ui| {
                ui.spacing_mut().item_spacing = vec2(GAP, GAP);
                if flat.is_empty() {
                    let message = if self.query.trim().is_empty() {
                        self.section.empty_message().to_owned()
                    } else {
                        format!("No indicators match \u{201c}{}\u{201d}.", self.query.trim())
                    };
                    ui.add_space(40.0);
                    ui.vertical_centered(|ui| {
                        ui.label(kit::text(
                            message,
                            size::BODY,
                            kit::REGULAR,
                            theme::text_tertiary(),
                        ));
                    });
                    return;
                }
                let columns = columns_for(rect.width());
                let width = ui.available_width();
                let card_width = (width - GAP * (columns as f32 - 1.0)) / columns as f32;
                let mut index = 0;
                for block in blocks {
                    if block.entries.is_empty() {
                        continue;
                    }
                    if let Some(heading) = block.heading {
                        let row = block_heading(ui, heading);
                        if heading == "More to explore" && row.shuffle {
                            self.shuffle = self.shuffle.wrapping_add(1);
                        }
                    }
                    for chunk in block.entries.chunks(columns) {
                        let (row, _) =
                            ui.allocate_exact_size(vec2(width, CARD_HEIGHT), Sense::hover());
                        for (column, entry) in chunk.iter().enumerate() {
                            let card = Rect::from_min_size(
                                pos2(row.left() + column as f32 * (card_width + GAP), row.top()),
                                vec2(card_width, CARD_HEIGHT),
                            );
                            let is_selected = selected_index == Some(index);
                            let status = availability(entry, target);
                            let on_chart = target.is_some_and(|target| target.has(entry));
                            let favorite = self.is_favorite(&entry.key);
                            let preview = self.previews.get(ui.ctx(), entry);
                            let missing = self.previews.is_missing(entry);
                            let outcome = card_ui(
                                ui,
                                card,
                                entry,
                                &CardLook {
                                    selected: is_selected,
                                    on_chart,
                                    favorite,
                                    blocked: matches!(status, Availability::Blocked(_)),
                                    missing,
                                },
                                preview.as_deref(),
                            );
                            if is_selected && scroll {
                                ui.scroll_to_rect(card, None);
                            }
                            if outcome.star {
                                toggled = Some(entry.key.clone());
                            } else if outcome.clicked {
                                clicked = Some((entry.key.clone(), outcome.double));
                            }
                            index += 1;
                        }
                    }
                    ui.add_space(4.0);
                }
            });
        if let Some(key) = toggled {
            self.toggle_favorite(&key);
        }
        if let Some((key, double)) = clicked {
            self.selected = Some(key.clone());
            if double && let Some(entry) = flat.iter().find(|entry| entry.key == key) {
                let status = availability(entry, target);
                if status != Availability::Remove
                    && let Some(action) = action_for(entry, status)
                {
                    *output = Some(Output::Apply {
                        action,
                        keep_open: false,
                    });
                }
            }
        }
    }

    fn detail(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        entry: Option<&CatalogEntry>,
        target: Option<&Target>,
        output: &mut Option<Output>,
    ) {
        let inner = rect.shrink(20.0);
        let Some(entry) = entry else {
            let text = kit::galley(
                ui,
                "Select an indicator",
                size::BODY,
                kit::REGULAR,
                theme::text_tertiary(),
                inner.width(),
            );
            ui.painter().galley(
                pos2(inner.left(), inner.top()),
                text,
                theme::text_tertiary(),
            );
            return;
        };
        let status = availability(entry, target);
        let favorite = self.is_favorite(&entry.key);
        let ctx = ui.ctx().clone();
        let preview = self.previews.get(&ctx, entry);
        let missing = self.previews.is_missing(entry);

        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner));
        let ui = &mut child;
        // The buttons stay at the bottom; the text above them scrolls when it is long.
        let buttons_height = 32.0 + 20.0;
        let text_rect = Rect::from_min_max(
            inner.min,
            pos2(inner.right(), inner.bottom() - buttons_height),
        );
        let mut text_ui = ui.new_child(egui::UiBuilder::new().max_rect(text_rect));
        egui::ScrollArea::vertical()
            .id_salt("indicator_picker_detail")
            .auto_shrink([false, false])
            .min_scrolled_height(0.0)
            .show(&mut text_ui, |ui| {
                let width = ui.available_width();
                let (art, _) = ui.allocate_exact_size(vec2(width, 150.0), Sense::hover());
                kit::paint_well(ui, art, 8.0);
                if missing || matches!(entry.key, EntryKey::Script(_)) {
                    icons::paint(
                        ui.painter(),
                        Rect::from_center_size(art.center(), vec2(32.0, 32.0)),
                        glyph_for(entry),
                        theme::text_tertiary(),
                    );
                } else {
                    preview::paint(&ui.painter().with_clip_rect(art), art, preview.as_deref());
                }
                ui.add_space(12.0);
                ui.label(kit::text(
                    meta_line(entry),
                    size::SUBHEADLINE,
                    kit::REGULAR,
                    theme::text_secondary(),
                ));
                ui.add_space(2.0);
                ui.label(kit::text(
                    &entry.title,
                    size::TITLE2 + 3.0,
                    SEMIBOLD,
                    theme::text_primary(),
                ));
                ui.add_space(10.0);
                ui.add(
                    egui::Label::new(kit::text(
                        &entry.description,
                        size::BODY,
                        kit::REGULAR,
                        theme::text_secondary(),
                    ))
                    .wrap(),
                );
                if entry.origin != Origin::Script {
                    ui.add_space(10.0);
                    ui.label(kit::text(
                        format!("By {} \u{b7} {}", entry.author, entry.license),
                        size::CAPTION + 0.5,
                        kit::REGULAR,
                        theme::text_tertiary(),
                    ));
                }
                if let Availability::Blocked(reason) = status {
                    ui.add_space(10.0);
                    ui.add(
                        egui::Label::new(kit::text(
                            reason,
                            size::SUBHEADLINE,
                            kit::REGULAR,
                            theme::text_tertiary(),
                        ))
                        .wrap(),
                    );
                }
            });

        let buttons = Rect::from_min_max(pos2(inner.left(), inner.bottom() - 32.0), inner.max);
        let mut button_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(buttons)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        button_ui.spacing_mut().item_spacing.x = 8.0;
        let blocked = matches!(status, Availability::Blocked(_));
        let label = match status {
            Availability::Remove => "Remove from chart",
            Availability::AddAnother => "Add another copy",
            _ => "Add to chart",
        };
        if kit::large_prominent_button(&mut button_ui, label, !blocked, buttons.width() - 40.0)
            .clicked()
            && let Some(action) = action_for(entry, status)
        {
            *output = Some(Output::Apply {
                action,
                keep_open: false,
            });
        }
        if star_button(&mut button_ui, favorite).clicked() {
            self.toggle_favorite(&entry.key);
        }
    }

    fn footer(&mut self, ui: &mut Ui, full: Rect, shown: usize) {
        let bar = Rect::from_min_max(pos2(full.left(), full.bottom() - FOOTER), full.max);
        let mut footer = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(bar.shrink2(vec2(24.0, 0.0)))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        footer.spacing_mut().item_spacing.x = 6.0;
        let hint = |ui: &mut Ui, keys: &[&str], label: &str| {
            for key in keys {
                crate::ui::kit::keycap::keycap(ui, key);
            }
            ui.label(kit::text(
                label,
                size::SUBHEADLINE,
                kit::REGULAR,
                theme::text_secondary(),
            ));
            ui.add_space(10.0);
        };
        hint(&mut footer, &["\u{2191}", "\u{2193}"], "Browse");
        hint(&mut footer, &["\u{21b5}"], "Add");
        hint(&mut footer, &["\u{21e7}", "\u{21b5}"], "Add, keep open");
        hint(&mut footer, &["F"], "Favorite");
        hint(&mut footer, &["Esc"], "Close");
        let mut done = false;
        footer.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            done = kit::sheet_button(ui, "Done", true, true).clicked();
            ui.add_space(8.0);
            ui.label(kit::text(
                format!("{shown} shown"),
                size::SUBHEADLINE,
                kit::REGULAR,
                theme::text_tertiary(),
            ));
        });
        if done {
            self.open = false;
        }
    }
}

/// The keys the picker takes this frame. Arrows and Return are consumed before the search field
/// is drawn so it does not move its caret for them; Left/Right stay with the field while it holds
/// text, and `F` only counts when nothing is being typed.
fn read_keys(ctx: &egui::Context, search_focused: bool, query_empty: bool) -> Keys {
    ctx.input_mut(|input| {
        let arrows_sideways = !search_focused || query_empty;
        Keys {
            down: input.consume_key(Modifiers::NONE, Key::ArrowDown),
            up: input.consume_key(Modifiers::NONE, Key::ArrowUp),
            left: arrows_sideways && input.consume_key(Modifiers::NONE, Key::ArrowLeft),
            right: arrows_sideways && input.consume_key(Modifiers::NONE, Key::ArrowRight),
            shift_enter: input.consume_key(Modifiers::SHIFT, Key::Enter),
            enter: input.consume_key(Modifiers::NONE, Key::Enter),
            favorite: !search_focused && input.consume_key(Modifiers::NONE, Key::F),
        }
    })
}

/// Cards per row for a grid area `width` wide.
fn columns_for(width: f32) -> usize {
    if width >= 600.0 {
        3
    } else if width >= 360.0 {
        2
    } else {
        1
    }
}

fn glyph_for(entry: &CatalogEntry) -> Icon {
    match &entry.key {
        EntryKey::Native(kind) => kind.icon(),
        EntryKey::Bundled(_) if entry.overlay => Icon::ChartUptrend,
        EntryKey::Bundled(_) => Icon::Waveform,
        EntryKey::Script(_) => Icon::ChartLine,
    }
}

/// `Momentum · Separate pane`, `Trend · Overlay`, `Your script`.
fn meta_line(entry: &CatalogEntry) -> String {
    let placement = if entry.overlay {
        "Overlay"
    } else {
        "Separate pane"
    };
    match entry.family {
        Some(family) => format!("{} \u{b7} {placement}", family.title()),
        None => "Your script".to_owned(),
    }
}

struct CardLook {
    selected: bool,
    on_chart: bool,
    favorite: bool,
    blocked: bool,
    missing: bool,
}

#[derive(Default)]
struct CardOutcome {
    clicked: bool,
    double: bool,
    star: bool,
}

fn card_ui(
    ui: &mut Ui,
    rect: Rect,
    entry: &CatalogEntry,
    look: &CardLook,
    preview: Option<&preview::Preview>,
) -> CardOutcome {
    let id = Id::new(("indicator_card", entry.key.stored()));
    let response = ui.interact(rect, id, Sense::click());
    let star_rect = Rect::from_min_size(
        pos2(rect.right() - 34.0, rect.top() + 8.0),
        vec2(26.0, 26.0),
    );
    let star = ui.interact(star_rect, id.with("star"), Sense::click());
    let hovered = response.hovered() || star.hovered();
    let painter = ui.painter();
    let fill = if look.selected {
        theme::selected_tint(hovered)
    } else {
        theme::card_fill(hovered)
    };
    painter.rect_filled(rect, kit::CARD_RADIUS, fill);
    painter.rect_stroke(
        rect,
        kit::CARD_RADIUS,
        if look.selected {
            Stroke::new(1.5, theme::selected_stroke().color)
        } else {
            Stroke::new(1.0, theme::card_border(hovered))
        },
        StrokeKind::Inside,
    );
    let art = Rect::from_min_size(
        rect.min + vec2(6.0, 6.0),
        vec2(rect.width() - 12.0, PREVIEW_HEIGHT),
    );
    painter.rect_filled(art, 7.0, theme::well_fill());
    if look.missing || matches!(entry.key, EntryKey::Script(_)) {
        icons::paint(
            painter,
            Rect::from_center_size(art.center(), vec2(30.0, 30.0)),
            glyph_for(entry),
            theme::text_tertiary(),
        );
    } else if preview.is_some() || matches!(entry.key, EntryKey::Native(_)) {
        preview::paint(painter, art, preview);
    } else {
        // A bundled script's run is still on its way: candles only.
        preview::paint(painter, art, None);
    }
    let alpha = if look.blocked { 0.5 } else { 1.0 };
    let title = kit::galley(
        ui,
        &entry.title,
        size::BODY,
        SEMIBOLD,
        theme::with_alpha(theme::text_primary(), alpha),
        rect.width() - 24.0,
    );
    let sub = kit::galley(
        ui,
        &meta_line(entry),
        size::CAPTION + 0.5,
        kit::REGULAR,
        theme::text_secondary(),
        rect.width() - 24.0,
    );
    let painter = ui.painter();
    painter.galley(
        pos2(rect.left() + 12.0, art.bottom() + 10.0),
        title,
        theme::text_primary(),
    );
    painter.galley(
        pos2(rect.left() + 12.0, art.bottom() + 30.0),
        sub,
        theme::text_secondary(),
    );
    if look.on_chart {
        let tag = kit::galley(
            ui,
            "On chart",
            size::CAPTION,
            MEDIUM,
            theme::text_secondary(),
            80.0,
        );
        let size = tag.size() + vec2(12.0, 4.0);
        let tag_rect = Rect::from_min_size(
            pos2(rect.right() - 12.0 - size.x, art.bottom() + 12.0),
            size,
        );
        let painter = ui.painter();
        painter.rect_filled(tag_rect, 4.0, kit::primary(0.08));
        painter.galley(tag_rect.min + vec2(6.0, 2.0), tag, theme::text_secondary());
    }
    // The star sits on the preview; it brightens with the favorite state or the pointer.
    let painter = ui.painter();
    if hovered || look.favorite {
        painter.rect_filled(
            star_rect,
            6.0,
            theme::with_alpha(theme::chrome(), if star.hovered() { 0.95 } else { 0.75 }),
        );
        icons::paint(
            painter,
            star_rect.shrink(6.0),
            if look.favorite {
                Icon::StarFill
            } else {
                Icon::Star
            },
            if look.favorite {
                theme::text_primary()
            } else {
                theme::text_secondary()
            },
        );
    }
    CardOutcome {
        clicked: response.clicked() || star.clicked(),
        double: response.double_clicked(),
        star: star.clicked(),
    }
}

/// A square star button beside the add button: filled while the indicator is a favorite.
fn star_button(ui: &mut Ui, favorite: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(32.0, 32.0), Sense::click());
    let painter = ui.painter();
    painter.rect_filled(rect, 8.0, theme::card_fill(response.hovered()));
    painter.rect_stroke(
        rect,
        8.0,
        Stroke::new(1.0, theme::card_border(response.hovered())),
        StrokeKind::Inside,
    );
    icons::paint(
        painter,
        Rect::from_center_size(rect.center(), vec2(15.0, 15.0)),
        if favorite { Icon::StarFill } else { Icon::Star },
        theme::text_primary(),
    );
    response.on_hover_text(if favorite {
        "Remove from favorites"
    } else {
        "Add to favorites"
    })
}

fn sidebar_heading(ui: &mut Ui, text: &str) {
    ui.add_space(10.0);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::hover());
    let label = kit::galley(
        ui,
        &text.to_uppercase(),
        size::CAPTION,
        SEMIBOLD,
        theme::text_tertiary(),
        rect.width() - 12.0,
    );
    ui.painter().galley(
        pos2(rect.left() + 10.0, rect.center().y - label.size().y / 2.0),
        label,
        theme::text_tertiary(),
    );
}

fn sidebar_row(
    ui: &mut Ui,
    width: f32,
    icon: Icon,
    title: &str,
    count: Option<usize>,
    selected: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(width, 30.0), Sense::click());
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, 6.0, theme::selected_tint(response.hovered()));
    } else if response.hovered() {
        painter.rect_filled(rect, 6.0, kit::primary(0.06));
    }
    icons::paint(
        painter,
        Rect::from_center_size(pos2(rect.left() + 20.0, rect.center().y), vec2(14.0, 14.0)),
        icon,
        theme::text_primary(),
    );
    let label = kit::galley(
        ui,
        title,
        size::BODY,
        if selected { SEMIBOLD } else { kit::REGULAR },
        theme::text_primary(),
        rect.width() - 80.0,
    );
    ui.painter().galley(
        pos2(rect.left() + 38.0, rect.center().y - label.size().y / 2.0),
        label,
        theme::text_primary(),
    );
    if let Some(count) = count {
        let number = kit::galley(
            ui,
            &count.to_string(),
            size::SUBHEADLINE,
            kit::REGULAR,
            theme::text_tertiary(),
            40.0,
        );
        ui.painter().galley(
            pos2(
                rect.right() - 10.0 - number.size().x,
                rect.center().y - number.size().y / 2.0,
            ),
            number,
            theme::text_tertiary(),
        );
    }
    response
}

struct HeadingRow {
    shuffle: bool,
}

/// A block heading; "More to explore" carries a Shuffle button at its right edge.
fn block_heading(ui: &mut Ui, text: &str) -> HeadingRow {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
    let label = kit::galley(
        ui,
        text,
        size::HEADLINE + 1.0,
        SEMIBOLD,
        theme::text_primary(),
        rect.width() - 100.0,
    );
    ui.painter().galley(
        pos2(rect.left(), rect.center().y - label.size().y / 2.0),
        label,
        theme::text_primary(),
    );
    let mut shuffle = false;
    if text == "More to explore" {
        let button = Rect::from_min_max(pos2(rect.right() - 84.0, rect.top()), rect.max);
        let response = ui.interact(button, Id::new("indicator_picker_shuffle"), Sense::click());
        let color = if response.hovered() {
            theme::text_primary()
        } else {
            theme::text_secondary()
        };
        icons::paint(
            ui.painter(),
            Rect::from_center_size(
                pos2(button.left() + 8.0, button.center().y),
                vec2(13.0, 13.0),
            ),
            Icon::Repeat,
            color,
        );
        let label = kit::galley(ui, "Shuffle", size::BODY, kit::REGULAR, color, 60.0);
        ui.painter().galley(
            pos2(
                button.left() + 22.0,
                button.center().y - label.size().y / 2.0,
            ),
            label,
            color,
        );
        shuffle = response.clicked();
    }
    HeadingRow { shuffle }
}

#[cfg(test)]
mod tests;

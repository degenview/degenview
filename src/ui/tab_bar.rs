//! The tab strip at the top of every window: one tab per open tab, a `+`, and the events the
//! window host turns into model edits. Dragging is only *reported* here (`DragStarted`); the host
//! drives the rest from raw pointer state because a dragged tab leaves this strip.

use super::icons::{self, Icon};
use super::kit;
use super::shortcut;
use super::tab_link_menu::{self, TabLinkEdit};
use super::theme;
use super::update_indicator;
use super::window_chrome::{self, Control};
use crate::host::update_host::UpdateView;
use crate::model::InstrumentId;
use crate::model::tab;
use crate::model::tab::title::{Detail, Segments, TabChart, TabTitleStyle};
use crate::model::tab::windows::{TabId, WindowId};
use crate::model::watchlist::quote::Direction;
use crate::service::watchlist_quotes::WatchlistQuoteBook;
use eframe::egui::{
    self, Color32, CornerRadius, Id, Painter, Pos2, Rect, Sense, Shadow, Stroke, StrokeKind, Ui,
    Vec2, pos2, vec2,
};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// Strip height. The strip is the window's title bar (`window_chrome`).
pub const HEIGHT: f32 = 40.0;
/// Narrow enough to crowd a window with tabs (each shows just its logo), wide enough for the
/// logo and the close button side by side.
const MIN_TAB_WIDTH: f32 = 52.0;
/// Wide enough for the symbol, price and change and some of the layout name (which fades out at
/// the edge) when there are few tabs; the strip narrows them all together when there are many.
const MAX_TAB_WIDTH: f32 = 300.0;
const NEW_BUTTON_WIDTH: f32 = 34.0;
const SETTINGS_BUTTON_WIDTH: f32 = 34.0;
const LINK_BUTTON_WIDTH: f32 = 30.0;
/// The user guide. Linked from Settings ▸ About on every platform and, on macOS, from the Help
/// menu.
pub const DOCS_URL: &str = "https://docs.degenview.app";
/// The project's GitHub organisation and the maintainer's Buy Me a Coffee page, linked from the
/// strip's right end on every platform and, on macOS, from the Help menu too.
pub const GITHUB_URL: &str = "https://github.com/degenview";
pub const COFFEE_URL: &str = "https://buymeacoffee.com/nicoswd";
/// The hairline between two tabs at rest.
const SEPARATOR_HEIGHT: f32 = 16.0;
const TAB_GAP: f32 = 2.0;
const SIDE_PADDING: f32 = 6.0;
const CLOSE_SIZE: f32 = 18.0;
const ICON_SIZE: f32 = 13.0;
/// An instrument's logo, in a circle.
const LOGO_SIZE: f32 = 18.0;
/// The line under a linked tab, in its colour.
const UNDERLINE: f32 = 2.5;
const TEXT_SIZE: f32 = 12.5;
/// How long tabs take to slide to a new slot.
const SLIDE_SECONDS: f32 = 0.1;

/// What a tab shows.
#[derive(Clone, Debug)]
pub struct TabView {
    pub id: TabId,
    pub title: String,
    pub icon: Icon,
    /// The focused chart's market and the layout, for a chart tab that has one. Everything else
    /// (the portfolio, Pine Studio, a blank tab) shows its glyph and `title`.
    pub chart: Option<TabChart>,
    /// The timeframe shown in the hover text.
    pub timeframe: &'static str,
    /// The Tab Linking colour the tab is tagged with, shown as a dot on its logo.
    pub link: Option<crate::model::tab::link::LinkColor>,
    /// A chart tab can be linked (the portfolio and Pine Studio cannot): its menu has the section.
    pub linkable: bool,
    /// What the tab's colour group keeps in step, for the menu's switches.
    pub link_toggles: crate::model::tab::link::LinkToggles,
}

/// A window a tab can be sent to from the context menu.
#[derive(Clone, Debug)]
pub struct WindowChoice {
    pub id: WindowId,
    pub title: String,
}

/// A tab being dragged in from another window, shown in this strip as a ghost.
#[derive(Clone, Debug)]
pub struct Incoming {
    pub tab: TabView,
    /// The gap it would land in (0..=tabs.len()); the tabs from there on make room.
    pub slot: usize,
    /// Where the ghost is drawn, relative to the strip's left edge.
    pub left: f32,
}

/// What this strip shows of a drag in progress.
#[derive(Clone, Debug, Default)]
pub struct DragView {
    /// A tab of this strip being dragged by the pointer, and its left edge relative to the
    /// strip. It is drawn there instead of in its slot, which stays open for it.
    pub dragging: Option<(TabId, f32)>,
    pub incoming: Option<Incoming>,
}

pub struct TabBar<'a> {
    pub window: WindowId,
    pub tabs: &'a [TabView],
    pub active: TabId,
    pub drag: DragView,
    pub other_windows: &'a [WindowChoice],
    /// The shared quotes the price and change are read from. The strip never asks a provider.
    pub quotes: &'a WatchlistQuoteBook,
    pub style: TabTitleStyle,
    /// The app's update state; a mark shows at the strip's right end while there is news.
    pub update: Option<&'a UpdateView>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TabBarEvent {
    Select(TabId),
    Close(TabId),
    New,
    /// The settings button was clicked (Windows and Linux; macOS has Settings in its menu).
    OpenSettings,
    /// The update mark was clicked: the app decides what that means for the update's phase.
    UpdateClicked,
    /// The pointer pressed on `tab` and started dragging; `grab` is its offset inside the tab.
    DragStarted {
        tab: TabId,
        grab: Vec2,
    },
    /// Close every tab of the window but this one / the ones after it. Each goes through the
    /// ordinary close, so a Pine script with edits still asks.
    CloseOthers(TabId),
    CloseToRight(TabId),
    /// The user typed a title for a chart tab; `None` clears it.
    SetCustomTitle {
        tab: TabId,
        title: Option<String>,
    },
    SetTitleStyle(TabTitleStyle),
    /// Tag the tab with a Tab Linking colour, or `None` to unlink it. Only the tag changes: the
    /// chart is left as it is.
    SetLink {
        tab: TabId,
        color: Option<crate::model::tab::link::LinkColor>,
    },
    /// The switches of a Tab Linking colour changed.
    SetLinkToggles {
        color: crate::model::tab::link::LinkColor,
        toggles: crate::model::tab::link::LinkToggles,
    },
    MoveToNewWindow(TabId),
    MoveToWindow {
        tab: TabId,
        window: WindowId,
    },
}

/// Room the settings button takes left of the window buttons: only where the strip draws those
/// buttons, because macOS reaches Settings from its menu.
fn settings_button_width() -> f32 {
    if window_chrome::trailing_inset() > 0.0 {
        SETTINGS_BUTTON_WIDTH
    } else {
        0.0
    }
}

/// Room the GitHub and coffee buttons take at the strip's right end, left of the window buttons.
const LINKS_WIDTH: f32 = LINK_BUTTON_WIDTH * 2.0 + 6.0;

/// What one frame of the strip produced.
pub struct TabBarOutput {
    pub events: Vec<TabBarEvent>,
    /// The part of the strip that holds the tabs, in the window's own coordinates (where the
    /// dragged tab can be dropped): the window buttons' room is left out.
    pub strip: Rect,
    /// How far the tabs are scrolled (zero while they fit): the host adds it to a pointer's
    /// place in the strip to find the slot under it.
    pub scroll: f32,
}

/// Width of each tab: they share the room left of the `+`, between the min and max.
pub fn tab_width(count: usize, available: f32) -> f32 {
    if count == 0 {
        return MAX_TAB_WIDTH;
    }
    let room = (available - NEW_BUTTON_WIDTH - SIDE_PADDING * 2.0).max(0.0);
    let gaps = TAB_GAP * (count - 1) as f32;
    ((room - gaps) / count as f32).clamp(MIN_TAB_WIDTH, MAX_TAB_WIDTH)
}

/// How far the tabs run past the strip: zero while they all fit, else how far the row can be
/// scrolled so that its last tab and the `+` end at the strip's right edge.
pub fn max_scroll(count: usize, strip_width: f32) -> f32 {
    let width = tab_width(count, strip_width);
    (tab_left(count, width) + NEW_BUTTON_WIDTH - strip_width).max(0.0)
}

/// The scroll that shows tab `index` whole (with the strip's padding beside it) when it is not,
/// else `scroll` as it is.
pub fn reveal(index: usize, count: usize, strip_width: f32, scroll: f32) -> f32 {
    let width = tab_width(count, strip_width);
    let room = strip_width - NEW_BUTTON_WIDTH;
    let left = tab_left(index, width);
    let right = left + width;
    let target = if left - SIDE_PADDING < scroll {
        left - SIDE_PADDING
    } else if right + SIDE_PADDING > scroll + room {
        right + SIDE_PADDING - room
    } else {
        scroll
    };
    target.clamp(0.0, max_scroll(count, strip_width))
}

/// Left edge of tab `index`, relative to the strip's left edge.
pub fn tab_left(index: usize, width: f32) -> f32 {
    SIDE_PADDING + index as f32 * (width + TAB_GAP)
}

/// The slot under `x` (relative to the strip's left edge), for reordering an existing tab.
pub fn slot_at(x: f32, count: usize, width: f32) -> usize {
    if count == 0 {
        return 0;
    }
    let slot = ((x - SIDE_PADDING) / (width + TAB_GAP)).floor();
    (slot.max(0.0) as usize).min(count - 1)
}

/// The slot tab `index` occupies when a tab dragged in from elsewhere holds a gap at `gap`.
pub fn slot_of(index: usize, gap: Option<usize>) -> usize {
    match gap {
        Some(gap) if index >= gap => index + 1,
        _ => index,
    }
}

/// Left edge (relative to the strip) of a tab of `count` dragged by the pointer, kept between
/// the first and last slot. `grab_x` is where in the tab the pointer took hold of it.
pub fn dragged_left(pointer_x: f32, grab_x: f32, count: usize, width: f32) -> f32 {
    (pointer_x - grab_x).clamp(tab_left(0, width), tab_left(count.saturating_sub(1), width))
}

/// The slot a dragged tab sits over: the one under its centre, so it swaps with a neighbour once
/// it is half across.
pub fn dragged_slot(left: f32, count: usize, width: f32) -> usize {
    slot_at(left + width / 2.0, count, width)
}

/// The gap nearest `x`, for dropping a tab from elsewhere: 0 is before the first tab and
/// `count` after the last.
pub fn insertion_at(x: f32, count: usize, width: f32) -> usize {
    let gap = ((x - SIDE_PADDING) / (width + TAB_GAP)).round();
    (gap.max(0.0) as usize).min(count)
}

/// How a tab is painted.
#[derive(Clone, Copy, Default)]
struct Look {
    active: bool,
    /// Lifted off the strip: the tab under the pointer.
    lifted: bool,
    hovered: bool,
    close_hovered: bool,
    /// 1.0 normally; below that for the ghost of a tab from another window.
    opacity: f32,
}

impl TabBar<'_> {
    pub fn show(&self, ui: &mut Ui) -> TabBarOutput {
        let mut events = Vec::new();
        let full = ui.max_rect();
        let ctx = ui.ctx().clone();
        let painter = ui.painter().clone();
        painter.rect_filled(full, 0.0, theme::chrome());
        let (maximized, fullscreen) = ui.input(|input| {
            let viewport = input.viewport();
            (
                viewport.maximized == Some(true),
                viewport.fullscreen == Some(true),
            )
        });
        // The settings button and the update mark sit left of the window buttons, and the tabs
        // end before them.
        let update = update_indicator::layout(ui, self.update);
        let controls_left = full.right() - window_chrome::trailing_inset();
        let links_left = controls_left - LINKS_WIDTH;
        let settings_left = links_left - settings_button_width();
        let strip = Rect::from_min_max(
            pos2(
                full.left() + window_chrome::leading_inset(fullscreen),
                full.top(),
            ),
            pos2(
                settings_left
                    - update
                        .as_ref()
                        .map_or(0.0, update_indicator::Layout::reserved),
                full.bottom(),
            ),
        );

        // Empty strip space is the window's title bar. Added first so the tabs, drawn over it,
        // take the pointer where they are.
        let background = ui.interact(full, self.id("title-bar"), Sense::click_and_drag());
        if background.drag_started_by(egui::PointerButton::Primary) {
            ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
        if background.double_clicked() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
        }

        // A tab on its way in takes a slot, so the others narrow and open a gap for it.
        let gap = self.drag.incoming.as_ref().map(|incoming| incoming.slot);
        let slots = self.tabs.len() + usize::from(gap.is_some());
        let target_width = tab_width(slots, strip.width());
        let width = ctx.animate_value_with_time(self.id("width"), target_width, SLIDE_SECONDS);
        let active_index = self.tabs.iter().position(|tab| tab.id == self.active);
        let scroll = self.scroll_offset(ui, strip, slots, active_index);
        let slide = |name: &str, key: u64, slot: usize| {
            ctx.animate_value_with_time(
                self.id(name).with(key),
                tab_left(slot, target_width),
                SLIDE_SECONDS,
            )
        };

        let mut lifted = None;
        // Tabs past the strip's end (more than fit at the narrowest) are cut off, not drawn over
        // the buttons beside it.
        let tabs_clip = Rect::from_min_max(
            strip.min,
            pos2(strip.right() - NEW_BUTTON_WIDTH, strip.bottom()),
        );
        let outer_clip = ui.clip_rect();
        ui.set_clip_rect(outer_clip.intersect(tabs_clip));
        let rects: Vec<Rect> = self
            .tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| {
                let left = match self.drag.dragging {
                    Some((dragged, left)) if dragged == tab.id => {
                        // Not slid: it follows the pointer exactly. Its own slot is remembered
                        // so it settles there when let go.
                        slide("x", tab.id, slot_of(index, gap));
                        left
                    }
                    _ => slide("x", tab.id, slot_of(index, gap)),
                };
                Rect::from_min_size(
                    pos2(strip.left() + left - scroll, strip.top() + 4.0),
                    vec2(width, strip.height() - 4.0),
                )
            })
            .collect();
        for (index, tab) in self.tabs.iter().enumerate() {
            let rect = rects[index];
            if self
                .drag
                .dragging
                .is_some_and(|(dragged, _)| dragged == tab.id)
            {
                lifted = Some((rect, index, tab));
            } else {
                self.show_tab(ui, rect, (index, tab), false, &mut events);
            }
        }
        self.paint_separators(ui, &rects, gap);
        // Drawn last so it passes over its neighbours.
        if let Some((rect, index, tab)) = lifted {
            self.show_tab(ui, rect, (index, tab), true, &mut events);
        }
        ui.set_clip_rect(outer_clip);
        self.show_rename(ui, &mut events);

        let plus_left = ctx.animate_value_with_time(
            self.id("plus"),
            tab_left(slots, target_width),
            SLIDE_SECONDS,
        );
        // Stays in reach when the tabs fill the strip.
        let plus_left = (plus_left - scroll).min(strip.width() - NEW_BUTTON_WIDTH);
        if scroll > 0.5 {
            fade_edge(&painter, tabs_clip, true);
        }
        if scroll < max_scroll(slots, strip.width()) - 0.5 {
            fade_edge(&painter, tabs_clip, false);
        }
        let new_rect = Rect::from_min_size(
            pos2(strip.left() + plus_left, strip.top() + 5.0),
            vec2(NEW_BUTTON_WIDTH - 4.0, strip.height() - 8.0),
        );
        let new_tab = ui.interact(new_rect, self.id("new"), Sense::click());
        let new_tab = super::accessibility::label(new_tab, "New tab").on_hover_text("New tab");
        if new_tab.hovered() {
            painter.rect_filled(new_rect, 6.0, theme::ink(18));
        }
        icons::paint(
            &painter,
            Rect::from_center_size(new_rect.center(), Vec2::splat(13.0)),
            Icon::Plus,
            theme::text_secondary(),
        );
        if new_tab.clicked() {
            events.push(TabBarEvent::New);
        }

        if let Some(incoming) = &self.drag.incoming {
            let rect = Rect::from_min_size(
                pos2(strip.left() + incoming.left, strip.top() + 4.0),
                vec2(width, strip.height() - 4.0),
            );
            let look = Look {
                active: true,
                lifted: true,
                opacity: 0.88,
                ..Look::default()
            };
            self.paint_tab(
                ui,
                &painter.with_clip_rect(strip),
                rect,
                &incoming.tab,
                look,
            );
        }

        if settings_button_width() > 0.0 {
            let rect = Rect::from_min_size(
                pos2(settings_left + 2.0, full.top() + 5.0),
                vec2(SETTINGS_BUTTON_WIDTH - 4.0, full.height() - 8.0),
            );
            let label = shortcut::OPEN_SETTINGS.hint("Settings");
            let button = ui.interact(rect, self.id("settings"), Sense::click());
            let button = super::accessibility::label(button, &label).on_hover_text(&label);
            if button.hovered() {
                painter.rect_filled(rect, 6.0, theme::ink(18));
            }
            icons::paint(
                &painter,
                Rect::from_center_size(rect.center(), Vec2::splat(13.0)),
                Icon::Sliders,
                theme::text_secondary(),
            );
            if button.clicked() {
                events.push(TabBarEvent::OpenSettings);
            }
        }

        // Two gray links at the far right: the project on GitHub, and a coffee for its maker.
        let links = [
            (
                "View project on GitHub",
                GITHUB_URL,
                egui::include_image!("../../assets/brand/github.svg"),
                14.0,
            ),
            (
                "Support this project by buying me a coffee",
                COFFEE_URL,
                egui::include_image!("../../assets/brand/coffee.svg"),
                // The cup fills about three quarters of its 24 grid, where the GitHub mark
                // fills all of its box, so it is drawn larger to look the same size.
                19.0,
            ),
        ];
        for (index, (name, url, logo, size)) in links.into_iter().enumerate() {
            let rect = Rect::from_min_size(
                pos2(
                    links_left + 3.0 + index as f32 * LINK_BUTTON_WIDTH,
                    full.top() + 5.0,
                ),
                vec2(LINK_BUTTON_WIDTH - 2.0, full.height() - 8.0),
            );
            let button = ui.interact(rect, self.id(name), Sense::click());
            let button = super::accessibility::label(button, name)
                .on_hover_text(name)
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            let color = if button.hovered() {
                painter.rect_filled(rect, 6.0, theme::ink(18));
                theme::text_primary()
            } else {
                theme::text_secondary()
            };
            let glyph = Rect::from_center_size(rect.center(), Vec2::splat(size));
            egui::Image::new(logo).tint(color).paint_at(ui, glyph);
            if button.clicked() {
                ctx.open_url(egui::OpenUrl::new_tab(url));
            }
        }

        if let Some(update) = &update
            && update_indicator::show(ui, update, settings_left, full.top(), full.height())
        {
            events.push(TabBarEvent::UpdateClicked);
        }

        match window_chrome::window_controls(ui, full, maximized) {
            Some(Control::Minimize) => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            }
            Some(Control::ToggleMaximize) => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
            }
            // Goes through the window's close request, so unsaved scripts still ask first.
            Some(Control::Close) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            None => {}
        }

        TabBarOutput {
            events,
            strip,
            scroll,
        }
    }

    fn id(&self, name: &str) -> Id {
        Id::new(("window-tab-bar", self.window, name))
    }

    /// How far this strip's tabs are scrolled. The wheel or a trackpad over the strip moves them;
    /// the active tab is brought into view when it (or the number of tabs) changes; a tab being
    /// dragged to either end scrolls them on.
    fn scroll_offset(
        &self,
        ui: &Ui,
        strip: Rect,
        count: usize,
        active_index: Option<usize>,
    ) -> f32 {
        let ctx = ui.ctx();
        let id = self.id("scroll");
        let mut state: ScrollState = ctx.data(|data| data.get_temp(id)).unwrap_or_default();
        let max = max_scroll(count, strip.width());
        if max <= 0.0 {
            ctx.data_mut(|data| data.insert_temp(id, ScrollState::default()));
            return 0.0;
        }
        let pointer = ctx.input(|input| input.pointer.hover_pos());
        let delta = ctx.input(|input| input.smooth_scroll_delta);
        let wheel = if delta.x.abs() > delta.y.abs() {
            delta.x
        } else {
            delta.y
        };
        if wheel != 0.0 && pointer.is_some_and(|pointer| strip.contains(pointer)) {
            state.offset -= wheel;
            state.target = None;
        }
        let seen = (self.active, count);
        if state.seen != Some(seen) {
            state.seen = Some(seen);
            if let Some(index) = active_index {
                state.target = Some(reveal(index, count, strip.width(), state.offset));
            }
        }
        let dt = ctx.input(|input| input.stable_dt).min(0.1);
        if self.drag.dragging.is_some()
            && let Some(pointer) = pointer
        {
            let edge = pointer.x - strip.left() < DRAG_EDGE;
            let far = strip.right() - NEW_BUTTON_WIDTH - pointer.x < DRAG_EDGE;
            if edge != far {
                state.offset += if far { 1.0 } else { -1.0 } * DRAG_SCROLL_SPEED * dt;
                state.target = None;
                ctx.request_repaint();
            }
        }
        if let Some(target) = state.target {
            let step = target.clamp(0.0, max) - state.offset;
            if step.abs() < 0.5 {
                state.offset = target.clamp(0.0, max);
                state.target = None;
            } else {
                state.offset += step * (1.0 - (-dt * 18.0).exp());
                ctx.request_repaint();
            }
        }
        state.offset = state.offset.clamp(0.0, max);
        let offset = state.offset;
        ctx.data_mut(|data| data.insert_temp(id, state));
        offset
    }

    fn show_tab(
        &self,
        ui: &mut Ui,
        rect: Rect,
        (index, tab): (usize, &TabView),
        lifted: bool,
        events: &mut Vec<TabBarEvent>,
    ) {
        let close_rect = close_rect(rect);
        let response = ui.interact(rect, self.id("tab").with(tab.id), Sense::click_and_drag());
        let close = ui.interact(close_rect, self.id("close").with(tab.id), Sense::click());
        let response = super::accessibility::label_as(
            response,
            egui::WidgetType::SelectableLabel,
            tab.title.clone(),
        );
        let close = super::accessibility::label(close, format!("Close {}", tab.title));

        let look = Look {
            active: tab.id == self.active,
            lifted,
            hovered: response.hovered(),
            close_hovered: close.hovered(),
            opacity: 1.0,
        };
        self.paint_tab(ui, ui.painter(), rect, tab, look);

        // Everything the strip had no room for, and what the price is measured against.
        let response = match &tab.chart {
            Some(chart) if !lifted && !close.hovered() => response.on_hover_ui(|ui| {
                let quote = self
                    .quotes
                    .get(&InstrumentId::new(chart.source, &chart.symbol));
                ui.label(tab::title::tooltip(chart, quote.as_ref(), tab.timeframe));
                if let Some(link) = tab.link {
                    // Said in words too: a dot's colour alone is not enough.
                    ui.label(format!(
                        "Linked tab: {} ({})",
                        link.title(),
                        tab_link_menu::describe(tab.link_toggles)
                    ));
                }
            }),
            // A tab squeezed to its glyph is named on hover.
            None if !lifted && rect.width() < Detail::SYMBOL_WIDTH => {
                response.on_hover_text(&tab.title)
            }
            _ => response,
        };

        if close.clicked() || response.middle_clicked() {
            events.push(TabBarEvent::Close(tab.id));
        } else if response.clicked() {
            events.push(TabBarEvent::Select(tab.id));
        }
        if response.drag_started_by(egui::PointerButton::Primary)
            && let Some(pointer) = response.interact_pointer_pos()
        {
            events.push(TabBarEvent::Select(tab.id));
            events.push(TabBarEvent::DragStarted {
                tab: tab.id,
                grab: pointer - rect.min,
            });
        }

        let rows = MENU_ROWS.map(|label| (label, None));
        // The Tab Linking section at the bottom has a width of its own.
        let section = if tab.linkable {
            tab_link_menu::WIDTH
        } else {
            0.0
        };
        kit::fitted_context_menu_min(ui, &response, &rows, section, |ui| {
            self.tab_menu(ui, rect, (index, tab), events);
        });
    }

    fn tab_menu(
        &self,
        ui: &mut Ui,
        rect: Rect,
        (index, tab): (usize, &TabView),
        events: &mut Vec<TabBarEvent>,
    ) {
        if kit::menu_item(ui, Icon::Plus, "New Tab", true, false) {
            events.push(TabBarEvent::New);
            ui.close();
        }
        kit::menu_separator(ui);
        if kit::menu_item(ui, Icon::Xmark, "Close Tab", true, false) {
            events.push(TabBarEvent::Close(tab.id));
            ui.close();
        }
        if kit::menu_item(
            ui,
            Icon::Xmark,
            "Close Other Tabs",
            self.tabs.len() > 1,
            false,
        ) {
            events.push(TabBarEvent::CloseOthers(tab.id));
            ui.close();
        }
        if kit::menu_item(
            ui,
            Icon::Xmark,
            "Close Tabs to the Right",
            index + 1 < self.tabs.len(),
            false,
        ) {
            events.push(TabBarEvent::CloseToRight(tab.id));
            ui.close();
        }
        if let Some(chart) = &tab.chart {
            kit::menu_separator(ui);
            if kit::menu_item(ui, Icon::Pencil, "Rename Tab\u{2026}", true, false) {
                self.begin_rename(ui.ctx(), rect, tab.id, chart);
                ui.close();
            }
            if kit::menu_item(
                ui,
                Icon::Pencil,
                "Reset Tab Title",
                chart.custom_title.is_some(),
                false,
            ) {
                events.push(TabBarEvent::SetCustomTitle {
                    tab: tab.id,
                    title: None,
                });
                ui.close();
            }
            kit::menu_submenu(ui, kit::MenuRow::new("Tab Title"), 0.0, |ui| {
                let rows = TabTitleStyle::ALL.map(|style| (style.label(), None));
                kit::menu_fit_rows(ui, &rows);
                for style in TabTitleStyle::ALL {
                    let row = kit::menu_row(
                        ui,
                        kit::MenuRow {
                            checked: style == self.style,
                            ..kit::MenuRow::new(style.label())
                        },
                    );
                    if row.clicked() {
                        events.push(TabBarEvent::SetTitleStyle(style));
                        ui.close();
                    }
                }
            });
        }
        if self.tabs.len() > 1 || !self.other_windows.is_empty() {
            kit::menu_separator(ui);
        }
        if self.tabs.len() > 1 && kit::menu_item(ui, Icon::Plus, MOVE_NEW, true, false) {
            events.push(TabBarEvent::MoveToNewWindow(tab.id));
            ui.close();
        }
        if !self.other_windows.is_empty() {
            kit::menu_submenu(ui, kit::MenuRow::new(MOVE_TO), 0.0, |ui| {
                let titles: Vec<(&str, Option<&str>)> = self
                    .other_windows
                    .iter()
                    .map(|window| (window.title.as_str(), None))
                    .collect();
                kit::menu_fit_rows(ui, &titles);
                for window in self.other_windows {
                    if kit::menu_row(ui, kit::MenuRow::new(&window.title)).clicked() {
                        events.push(TabBarEvent::MoveToWindow {
                            tab: tab.id,
                            window: window.id,
                        });
                        ui.close();
                    }
                }
            });
        }
        if tab.linkable {
            kit::menu_separator(ui);
            match tab_link_menu::show(ui, tab.link, tab.link_toggles) {
                Some(TabLinkEdit::Color(color)) => {
                    events.push(TabBarEvent::SetLink { tab: tab.id, color });
                }
                Some(TabLinkEdit::Toggles(toggles)) => {
                    if let Some(color) = tab.link {
                        events.push(TabBarEvent::SetLinkToggles { color, toggles });
                    }
                }
                None => {}
            }
        }
    }

    // MARK: Rename

    fn rename_id(&self) -> Id {
        self.id("rename")
    }

    fn begin_rename(&self, ctx: &egui::Context, rect: Rect, tab: TabId, chart: &TabChart) {
        let text = chart
            .custom_title
            .clone()
            .unwrap_or_else(|| chart.title.clone());
        let state = Rename {
            tab,
            pos: rect.left_bottom() + vec2(0.0, 4.0),
            text,
            focused: false,
        };
        ctx.data_mut(|data| data.insert_temp(self.rename_id(), state));
    }

    /// The small field under a tab that takes a title for it: Enter keeps it (empty clears it),
    /// Escape or a click elsewhere leaves the tab as it was.
    fn show_rename(&self, ui: &mut Ui, events: &mut Vec<TabBarEvent>) {
        let ctx = ui.ctx().clone();
        let Some(mut state) = ctx.data(|data| data.get_temp::<Rename>(self.rename_id())) else {
            return;
        };
        if !self.tabs.iter().any(|tab| tab.id == state.tab) {
            ctx.data_mut(|data| data.remove::<Rename>(self.rename_id()));
            return;
        }
        let mut done = None;
        egui::Area::new(self.id("rename-area"))
            .order(egui::Order::Foreground)
            .fixed_pos(state.pos)
            .show(&ctx, |ui| {
                kit::menu_frame().show(ui, |ui| {
                    ui.set_width(240.0);
                    let edit = ui.add(
                        egui::TextEdit::singleline(&mut state.text)
                            .hint_text("Tab title")
                            .char_limit(crate::workspace::MAX_CUSTOM_TITLE_CHARS)
                            .desired_width(f32::INFINITY),
                    );
                    if !state.focused {
                        edit.request_focus();
                        state.focused = true;
                    }
                    ui.add_space(4.0);
                    ui.label(kit::text(
                        "Return to save, Esc to cancel. Empty restores the default.",
                        kit::size::CAPTION,
                        kit::REGULAR,
                        theme::text_secondary(),
                    ));
                    if edit.lost_focus() {
                        let enter = ui.input(|input| input.key_pressed(egui::Key::Enter));
                        done = Some(enter);
                    }
                });
            });
        match done {
            Some(true) => {
                events.push(TabBarEvent::SetCustomTitle {
                    tab: state.tab,
                    title: Some(state.text.clone()),
                });
                ctx.data_mut(|data| data.remove::<Rename>(self.rename_id()));
            }
            Some(false) => ctx.data_mut(|data| data.remove::<Rename>(self.rename_id())),
            None => ctx.data_mut(|data| {
                data.insert_temp(self.rename_id(), state);
            }),
        }
    }

    // MARK: Painting

    /// The tab's text for the room it has, cached until the quote, the layout, the style or the
    /// width class changes so an idle frame builds no strings.
    fn segments(
        &self,
        ctx: &egui::Context,
        tab: &TabView,
        chart: &TabChart,
        detail: Detail,
    ) -> Arc<Segments> {
        let key = {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            chart.source.hash(&mut hasher);
            chart.symbol.hash(&mut hasher);
            chart.title.hash(&mut hasher);
            chart.layout.hash(&mut hasher);
            chart.custom_title.hash(&mut hasher);
            (self.style as u8, detail as u8, self.quotes.revision()).hash(&mut hasher);
            hasher.finish()
        };
        let id = self.id("segments").with(tab.id);
        if let Some((cached, segments)) = ctx.data(|data| data.get_temp::<(u64, Arc<Segments>)>(id))
            && cached == key
        {
            return segments;
        }
        let quote = chart
            .wants_quote()
            .then(|| {
                self.quotes
                    .get(&InstrumentId::new(chart.source, &chart.symbol))
            })
            .flatten();
        let segments = Arc::new(tab::title::compose(
            self.style,
            chart,
            quote.as_ref(),
            detail,
        ));
        ctx.data_mut(|data| data.insert_temp(id, (key, Arc::clone(&segments))));
        segments
    }

    /// A hairline in the gap between two neighbouring tabs that are both at rest: neither is the
    /// showing tab, being dragged or under the pointer (those have a fill of their own), and
    /// nothing is on its way in between them. It follows the animated rects, so it slides with
    /// the tabs.
    fn paint_separators(&self, ui: &Ui, rects: &[Rect], gap: Option<usize>) {
        let pointer = ui.input(|input| input.pointer.hover_pos());
        let dragged = self.drag.dragging.map(|(tab, _)| tab);
        let at_rest = |index: usize| {
            let tab = &self.tabs[index];
            tab.id != self.active
                && Some(tab.id) != dragged
                && !pointer.is_some_and(|pointer| rects[index].contains(pointer))
        };
        let painter = ui.painter();
        for (index, pair) in rects.windows(2).enumerate() {
            if slot_of(index + 1, gap) != slot_of(index, gap) + 1
                || !at_rest(index)
                || !at_rest(index + 1)
            {
                continue;
            }
            let rect = pair[0];
            let x = (rect.right() + TAB_GAP / 2.0).round() + 0.5;
            let cy = rect.center().y - 1.0;
            painter.vline(
                x,
                (cy - SEPARATOR_HEIGHT / 2.0)..=(cy + SEPARATOR_HEIGHT / 2.0),
                Stroke::new(1.0, theme::separator()),
            );
        }
    }

    fn paint_tab(&self, ui: &Ui, painter: &Painter, rect: Rect, tab: &TabView, look: Look) {
        let fade = |color: Color32| color.gamma_multiply(look.opacity);
        let raised = look.active || look.lifted;
        let corners = CornerRadius {
            nw: 8,
            ne: 8,
            sw: 0,
            se: 0,
        };
        if look.lifted {
            painter.add(
                Shadow {
                    offset: [0, 2],
                    blur: 8,
                    spread: 0,
                    color: Color32::from_black_alpha(70),
                }
                .as_shape(rect, corners),
            );
        }
        let fill = if raised {
            theme::background()
        } else if look.hovered {
            theme::ink(14)
        } else {
            Color32::TRANSPARENT
        };
        painter.rect_filled(rect, corners, fade(fill));
        if look.lifted {
            painter.rect_stroke(
                rect,
                corners,
                Stroke::new(1.0, fade(theme::separator())),
                StrokeKind::Inside,
            );
        } else if look.active {
            // Joins the tab to the content below by hiding the strip's bottom rule under it.
            painter.hline(
                rect.x_range(),
                rect.bottom() - 0.5,
                Stroke::new(1.5, theme::background()),
            );
        }
        if let Some(link) = tab.link {
            // The only coloured thing in the strip: the Tab Linking colour the tab is tagged
            // with. A tab with no colour has no line, showing or not.
            let line = Rect::from_min_max(
                pos2(rect.left() + 10.0, rect.bottom() - UNDERLINE),
                pos2(rect.right() - 10.0, rect.bottom()),
            );
            painter.rect_filled(
                line,
                UNDERLINE / 2.0,
                fade(crate::ui::tab_link_menu::swatch(link)),
            );
        }

        let primary = fade(if raised || look.hovered {
            theme::text_primary()
        } else {
            theme::text_secondary()
        });
        let secondary = fade(theme::text_secondary());
        let weight = if raised { kit::SEMIBOLD } else { kit::MEDIUM };
        let close_rect = close_rect(rect);
        let right = close_rect.left() - 6.0;
        let cy = rect.center().y - 1.0;
        let narrow = rect.width() < Detail::SYMBOL_WIDTH;
        let mut x = rect.left() + if narrow { 9.0 } else { 12.0 };

        match &tab.chart {
            Some(chart) => {
                let detail = Detail::for_width(rect.width());
                let segments = self.segments(ui.ctx(), tab, chart, detail);
                let logo =
                    Rect::from_center_size(pos2(x + LOGO_SIZE / 2.0, cy), Vec2::splat(LOGO_SIZE));
                kit::chart_icon(ui, logo, &chart.symbol, chart.source, false);
                x = logo.right() + 8.0;
                if detail > Detail::Logo {
                    let text = kit::galley(
                        ui,
                        &segments.text,
                        TEXT_SIZE,
                        weight,
                        primary,
                        (right - x).max(0.0),
                    );
                    painter.galley(pos2(x, cy - text.size().y / 2.0), text.clone(), primary);
                    x += text.size().x;

                    if let Some(block) = &segments.price {
                        x += 9.0;
                        x = self.paint_price(
                            ui,
                            painter,
                            (x, cy, right),
                            (tab.id, chart, detail),
                            block,
                            (primary, weight, look.opacity),
                        );
                    }
                    if let Some(layout) = &segments.layout
                        && right - x > 48.0
                    {
                        x += 9.0;
                        let slash = kit::galley(
                            ui,
                            "/",
                            TEXT_SIZE,
                            kit::REGULAR,
                            secondary.gamma_multiply(0.7),
                            12.0,
                        );
                        painter.galley(
                            pos2(x, cy - slash.size().y / 2.0),
                            slash.clone(),
                            secondary,
                        );
                        x += slash.size().x + 9.0;
                        // Not cut with an ellipsis: the name runs on to the close button's edge
                        // and fades into the tab's own fill, like the strip's ends.
                        let name = kit::galley(
                            ui,
                            layout,
                            TEXT_SIZE,
                            kit::REGULAR,
                            secondary,
                            f32::INFINITY,
                        );
                        let room = (right - x).max(0.0);
                        let overflows = name.size().x > room;
                        let clipped = painter.with_clip_rect(painter.clip_rect().intersect(
                            Rect::from_min_max(pos2(x, rect.top()), pos2(right, rect.bottom())),
                        ));
                        clipped.galley(pos2(x, cy - name.size().y / 2.0), name, secondary);
                        if overflows {
                            let mut base = if raised {
                                theme::background()
                            } else {
                                theme::chrome()
                            };
                            if look.hovered && !raised {
                                base = base.blend(theme::ink(14));
                            }
                            let width = NAME_FADE.min(room);
                            fade_strip(
                                painter,
                                (right - width, right),
                                (cy - 10.0, cy + 10.0),
                                fade(base),
                            );
                        }
                    }
                }
            }
            None => {
                let icon_rect =
                    Rect::from_center_size(pos2(x + ICON_SIZE / 2.0, cy), Vec2::splat(ICON_SIZE));
                icons::paint(painter, icon_rect, tab.icon, primary);
                x = icon_rect.right() + 7.0;
                if narrow {
                    // Just the glyph: the tooltip names the tab.
                } else {
                    let text = kit::galley(
                        ui,
                        &tab.title,
                        TEXT_SIZE,
                        weight,
                        primary,
                        (right - x).max(0.0),
                    );
                    painter.galley(pos2(x, cy - text.size().y / 2.0), text, primary);
                }
            }
        }

        if look.hovered || look.close_hovered || look.active {
            if look.close_hovered {
                painter.rect_filled(close_rect, 5.0, theme::ink(26));
            }
            icons::paint(
                painter,
                Rect::from_center_size(close_rect.center(), Vec2::splat(9.0)),
                Icon::Xmark,
                primary,
            );
        }
    }

    /// `▲ 142.92 +0.96%` from `x`; returns where the next part starts. The block keeps the
    /// widest it has been for this market and width class, so the layout name after it does
    /// not shuffle as the digits change.
    fn paint_price(
        &self,
        ui: &Ui,
        painter: &Painter,
        (x, cy, right): (f32, f32, f32),
        (tab, chart, detail): (TabId, &TabChart, Detail),
        block: &tab::title::PriceBlock,
        (ink, weight, opacity): (Color32, f32, f32),
    ) -> f32 {
        let dim = if block.stale { 0.55 } else { 1.0 };
        let tone = match block.direction {
            Direction::Up => theme::UP,
            Direction::Down => theme::DOWN,
            Direction::Flat => theme::text_secondary(),
        }
        .gamma_multiply(opacity * dim);
        let ink = ink.gamma_multiply(dim);
        let clip = painter.with_clip_rect(Rect::from_min_max(
            pos2(x, cy - 12.0),
            pos2(right, cy + 12.0),
        ));
        let mut at = x;
        // The arrow's room is kept when the price is unchanged, so a tick across zero is not a jump.
        const ARROW: f32 = 7.0;
        let mid = pos2(at + ARROW / 2.0, cy);
        match block.direction {
            Direction::Up => {
                clip.add(egui::Shape::convex_polygon(
                    vec![
                        pos2(mid.x, mid.y - 3.0),
                        pos2(mid.x + 3.5, mid.y + 3.0),
                        pos2(mid.x - 3.5, mid.y + 3.0),
                    ],
                    tone,
                    Stroke::NONE,
                ));
            }
            Direction::Down => {
                clip.add(egui::Shape::convex_polygon(
                    vec![
                        pos2(mid.x, mid.y + 3.0),
                        pos2(mid.x + 3.5, mid.y - 3.0),
                        pos2(mid.x - 3.5, mid.y - 3.0),
                    ],
                    tone,
                    Stroke::NONE,
                ));
            }
            Direction::Flat => {}
        }
        at += ARROW + 4.0;
        let price = kit::digits(
            ui,
            &block.price,
            TEXT_SIZE,
            weight.min(kit::MEDIUM),
            ink,
            f32::INFINITY,
        );
        clip.galley(pos2(at, cy - price.size().y / 2.0), price.clone(), ink);
        at += price.size().x;
        if let Some(change) = &block.change {
            at += 6.0;
            let change = kit::digits(ui, change, TEXT_SIZE, kit::MEDIUM, tone, f32::INFINITY);
            clip.galley(pos2(at, cy - change.size().y / 2.0), change.clone(), tone);
            at += change.size().x;
        }
        let key = self
            .id("price-width")
            .with((tab, chart.source, &chart.symbol, detail as u8));
        let widest = ui.ctx().data_mut(|data| {
            let widest = data.get_temp_mut_or_insert_with(key, || 0.0f32);
            *widest = widest.max(at - x);
            *widest
        });
        x + widest
    }
}

/// Where a strip's tabs are scrolled to, and what moved them last.
#[derive(Clone, Copy, Default)]
struct ScrollState {
    offset: f32,
    /// Where an eased scroll is heading (to show the active tab).
    target: Option<f32>,
    /// The active tab and tab count the scroll last made room for.
    seen: Option<(TabId, usize)>,
}

/// How close to the strip's ends a dragged tab scrolls it, and how fast (points per second).
const DRAG_EDGE: f32 = 28.0;
const DRAG_SCROLL_SPEED: f32 = 520.0;

/// A soft edge on a strip whose tabs run on past it: the strip's colour fading in over them.
fn fade_edge(painter: &Painter, clip: Rect, left: bool) {
    const WIDTH: f32 = 18.0;
    let solid = theme::chrome();
    let (from, to) = if left {
        (clip.left() + WIDTH, clip.left())
    } else {
        (clip.right() - WIDTH, clip.right())
    };
    fade_strip(painter, (from, to), (clip.top(), clip.bottom()), solid);
}

/// How far a tab's layout name fades out before the close button's edge.
const NAME_FADE: f32 = 28.0;

/// A horizontal gradient from clear at `x.0` to `solid` at `x.1` (either may be the larger),
/// between `y.0` and `y.1`.
fn fade_strip(painter: &Painter, x: (f32, f32), y: (f32, f32), solid: Color32) {
    let clear = solid.gamma_multiply(0.0);
    let mut mesh = egui::Mesh::default();
    let uv = egui::epaint::WHITE_UV;
    mesh.vertices.extend([
        egui::epaint::Vertex {
            pos: pos2(x.0, y.0),
            uv,
            color: clear,
        },
        egui::epaint::Vertex {
            pos: pos2(x.1, y.0),
            uv,
            color: solid,
        },
        egui::epaint::Vertex {
            pos: pos2(x.1, y.1),
            uv,
            color: solid,
        },
        egui::epaint::Vertex {
            pos: pos2(x.0, y.1),
            uv,
            color: clear,
        },
    ]);
    mesh.indices.extend([0, 1, 2, 0, 2, 3]);
    painter.add(egui::Shape::mesh(mesh));
}

/// An open rename: the tab, where its field sits and what has been typed.
#[derive(Clone)]
struct Rename {
    tab: TabId,
    pos: Pos2,
    text: String,
    focused: bool,
}

const MOVE_NEW: &str = "Move Tab to New Window";
const MOVE_TO: &str = "Move Tab to Window";

/// Every row the tab menu can show, for sizing it.
const MENU_ROWS: [&str; 9] = [
    "New Tab",
    "Close Tab",
    "Close Other Tabs",
    "Close Tabs to the Right",
    "Rename Tab\u{2026}",
    "Reset Tab Title",
    "Tab Title",
    MOVE_NEW,
    MOVE_TO,
];

fn close_rect(tab: Rect) -> Rect {
    Rect::from_center_size(
        pos2(tab.right() - 6.0 - CLOSE_SIZE / 2.0, tab.center().y + 1.0),
        Vec2::splat(CLOSE_SIZE),
    )
}

/// Where the pointer is relative to the strip, for deciding between reordering and tearing off.
pub fn is_outside_tear_off_zone(pointer: Pos2, strip: Rect) -> bool {
    pointer.y < strip.top() - HEIGHT
        || pointer.y > strip.bottom() + HEIGHT
        || pointer.x < strip.left() - HEIGHT
        || pointer.x > strip.right() + HEIGHT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_share_the_room_between_the_limits() {
        assert_eq!(tab_width(1, 2000.0), MAX_TAB_WIDTH);
        assert_eq!(tab_width(40, 800.0), MIN_TAB_WIDTH);
        let width = tab_width(4, 800.0);
        assert!(width > MIN_TAB_WIDTH && width < MAX_TAB_WIDTH);
        // Four tabs and the + fit the strip.
        let used = tab_left(4, width) + NEW_BUTTON_WIDTH;
        assert!(used <= 800.0 + 0.01, "{used}");
    }

    /// A right-click on a chart tab opens the menu with the Tab Linking section at the bottom
    /// (after a separator), and its swatches and switches say what changed for that tab.
    #[test]
    fn the_tab_menu_ends_with_the_tab_linking_section() {
        use crate::model::tab::link::{LinkColor, LinkToggles};
        let ctx = egui::Context::default();
        let quotes = WatchlistQuoteBook::default();
        let tabs = [
            TabView {
                id: 7,
                title: "BTC".to_owned(),
                icon: Icon::Plus,
                chart: None,
                timeframe: "1D",
                link: Some(LinkColor::Orange),
                linkable: true,
                link_toggles: LinkToggles::default(),
            },
            TabView {
                id: 8,
                title: "Portfolio".to_owned(),
                icon: Icon::Plus,
                chart: None,
                timeframe: "",
                link: None,
                linkable: false,
                link_toggles: LinkToggles::default(),
            },
        ];
        let mut time = 0.0;
        let mut events = Vec::new();
        let mut frame = |ctx: &egui::Context, input: Vec<egui::Event>| {
            time += 0.05;
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 700.0))),
                time: Some(time),
                events: input,
                ..Default::default()
            };
            let mut output = ctx.run_ui(raw, |ui| {
                let shown = TabBar {
                    window: 1,
                    tabs: &tabs,
                    active: 7,
                    drag: DragView::default(),
                    other_windows: &[],
                    quotes: &quotes,
                    style: TabTitleStyle::default(),
                    update: None,
                }
                .show(ui);
                events.extend(shown.events);
            });
            output.textures_delta.clear();
        };
        let button =
            |at: Pos2, button: egui::PointerButton, pressed: bool| egui::Event::PointerButton {
                pos: at,
                button,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
        for _ in 0..3 {
            frame(&ctx, Vec::new());
        }
        let on_tab = pos2(tab_left(0, tab_width(2, 900.0)) + 120.0, 14.0);
        frame(&ctx, vec![egui::Event::PointerMoved(on_tab)]);
        frame(
            &ctx,
            vec![button(on_tab, egui::PointerButton::Secondary, true)],
        );
        frame(
            &ctx,
            vec![button(on_tab, egui::PointerButton::Secondary, false)],
        );
        for _ in 0..4 {
            frame(&ctx, Vec::new());
        }

        let swatch = |color: LinkColor| {
            ctx.read_response(Id::new(("tab-link-swatch", color.key())))
                .map(|response| response.rect)
        };
        let blue = swatch(LinkColor::Blue).expect("the section is in the menu");
        let move_row = ctx.read_response(Id::new(("tab-link-option", "Symbol")));
        assert!(move_row.is_some(), "the switches are there too");
        // It is the last thing in the menu, under the tab rows.
        assert!(blue.top() > on_tab.y + 150.0, "{blue:?}");

        let at = blue.center();
        frame(&ctx, vec![egui::Event::PointerMoved(at)]);
        frame(&ctx, vec![button(at, egui::PointerButton::Primary, true)]);
        frame(&ctx, vec![button(at, egui::PointerButton::Primary, false)]);
        frame(&ctx, Vec::new());
        assert!(
            events.contains(&TabBarEvent::SetLink {
                tab: 7,
                color: Some(LinkColor::Blue)
            }),
            "{events:?}"
        );
    }

    /// The tab menu as a picture, both themes. Run by hand with `DEGENVIEW_GALLERY_DIR` set.
    #[test]
    #[ignore = "writes images for a person to look at"]
    fn gallery_of_the_tab_menu() {
        use crate::model::tab::link::{LinkColor, LinkToggles};
        use crate::ui::render_tests::{Screen, gallery_dir};
        if gallery_dir().is_none() {
            return;
        }
        let _restore = theme::RestoreDefaultTheme;
        let quotes = WatchlistQuoteBook::default();
        for (name, choice) in [("dark", egui::Theme::Dark), ("light", egui::Theme::Light)] {
            for (state, link) in [("linked", Some(LinkColor::Orange)), ("unlinked", None)] {
                let tabs = [
                    TabView {
                        id: 7,
                        title: "BTCUSDT".to_owned(),
                        icon: Icon::Plus,
                        chart: Some(TabChart {
                            source: crate::model::DataSource::Binance,
                            symbol: "BTCUSDT".to_owned(),
                            title: "BTC/USDT".to_owned(),
                            layout: None,
                            custom_title: None,
                        }),
                        timeframe: "1D",
                        link,
                        linkable: true,
                        link_toggles: LinkToggles::default(),
                    },
                    TabView {
                        id: 8,
                        title: "Other".to_owned(),
                        icon: Icon::Plus,
                        chart: None,
                        timeframe: "1D",
                        link: Some(LinkColor::Blue),
                        linkable: true,
                        link_toggles: LinkToggles::default(),
                    },
                ];
                let mut screen = Screen::new(vec2(520.0, 760.0));
                screen.ctx.set_theme(choice);
                let on_tab = pos2(tab_left(0, tab_width(2, 520.0)) + 100.0, 14.0);
                let button = |pressed| egui::Event::PointerButton {
                    pos: on_tab,
                    button: egui::PointerButton::Secondary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                };
                let run = |screen: &mut Screen, events: Vec<egui::Event>| {
                    screen.run(events, |ui| {
                        theme::sync(ui.ctx());
                        TabBar {
                            window: 1,
                            tabs: &tabs,
                            active: 7,
                            drag: DragView::default(),
                            other_windows: &[],
                            quotes: &quotes,
                            style: TabTitleStyle::default(),
                            update: None,
                        }
                        .show(ui);
                    })
                };
                for _ in 0..3 {
                    run(&mut screen, Vec::new());
                }
                run(&mut screen, vec![egui::Event::PointerMoved(on_tab)]);
                run(&mut screen, vec![button(true)]);
                run(&mut screen, vec![button(false)]);
                let mut last = None;
                for _ in 0..5 {
                    last = Some(run(&mut screen, Vec::new()));
                }
                screen.save(&format!("tab-menu-{state}-{name}"), last.expect("a frame"));
            }
        }
    }

    /// A tab that cannot be linked (the portfolio, Pine Studio) has no such section.
    #[test]
    fn a_tab_that_cannot_be_linked_has_no_section() {
        use crate::model::tab::link::LinkColor;
        let ctx = egui::Context::default();
        let quotes = WatchlistQuoteBook::default();
        let tabs = [TabView {
            id: 8,
            title: "Portfolio".to_owned(),
            icon: Icon::Plus,
            chart: None,
            timeframe: "",
            link: None,
            linkable: false,
            link_toggles: Default::default(),
        }];
        let mut time = 0.0;
        let mut frame = |input: Vec<egui::Event>| {
            time += 0.05;
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 700.0))),
                time: Some(time),
                events: input,
                ..Default::default()
            };
            let mut output = ctx.run_ui(raw, |ui| {
                TabBar {
                    window: 1,
                    tabs: &tabs,
                    active: 8,
                    drag: DragView::default(),
                    other_windows: &[],
                    quotes: &quotes,
                    style: TabTitleStyle::default(),
                    update: None,
                }
                .show(ui);
            });
            output.textures_delta.clear();
        };
        let on_tab = pos2(tab_left(0, tab_width(1, 900.0)) + 120.0, 14.0);
        let button = |pressed| egui::Event::PointerButton {
            pos: on_tab,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        for _ in 0..3 {
            frame(Vec::new());
        }
        frame(vec![egui::Event::PointerMoved(on_tab)]);
        frame(vec![button(true)]);
        frame(vec![button(false)]);
        for _ in 0..4 {
            frame(Vec::new());
        }
        assert!(
            ctx.read_response(Id::new(("tab-link-swatch", LinkColor::Blue.key())))
                .is_none()
        );
    }

    /// The strip the tabs get for a window, with or without an update mark.
    fn strip_with(update: Option<&UpdateView>) -> Rect {
        let ctx = egui::Context::default();
        let mut strip = Rect::NOTHING;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let tabs = [TabView {
                id: 1,
                title: "BTC".to_owned(),
                icon: Icon::Plus,
                chart: None,
                timeframe: "1D",
                link: None,
                linkable: false,
                link_toggles: Default::default(),
            }];
            let quotes = WatchlistQuoteBook::default();
            strip = TabBar {
                window: 1,
                tabs: &tabs,
                active: 1,
                drag: DragView::default(),
                other_windows: &[],
                quotes: &quotes,
                style: TabTitleStyle::default(),
                update,
            }
            .show(ui)
            .strip;
        });
        output.textures_delta.clear();
        strip
    }

    /// One frame of a strip `width` wide holding `count` tabs (every other one a chart tab whose
    /// logo needs no network), with the tab at `active` showing. Returns the shapes painted.
    fn paint_strip(
        count: usize,
        active: usize,
        width: f32,
        style: TabTitleStyle,
    ) -> Vec<egui::epaint::ClippedShape> {
        paint_strip_linked(count, active, width, style, None)
    }

    /// [`paint_strip`] with the tab at `linked` tagged Orange.
    fn paint_strip_linked(
        count: usize,
        active: usize,
        width: f32,
        style: TabTitleStyle,
        linked: Option<usize>,
    ) -> Vec<egui::epaint::ClippedShape> {
        let ctx = egui::Context::default();
        let tabs: Vec<TabView> = (0..count)
            .map(|index| TabView {
                id: index as u64 + 1,
                title: format!("Tab {index}"),
                icon: Icon::Plus,
                chart: (index % 2 == 0).then(|| TabChart {
                    source: crate::model::DataSource::CoinMarketCap,
                    symbol: "A-VERY-LONG-SYMBOL-NAME/USDT".to_owned(),
                    title: "A-VERY-LONG-SYMBOL-NAME/USDT".to_owned(),
                    layout: Some("A layout with a rather long name".to_owned()),
                    custom_title: None,
                }),
                timeframe: "1D",
                link: (linked == Some(index)).then_some(crate::model::tab::link::LinkColor::Orange),
                linkable: false,
                link_toggles: Default::default(),
            })
            .collect();
        let quotes = WatchlistQuoteBook::default();
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(width, HEIGHT))),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            TabBar {
                window: 1,
                tabs: &tabs,
                active: active as u64 + 1,
                drag: DragView::default(),
                other_windows: &[],
                quotes: &quotes,
                style,
                update: None,
            }
            .show(ui);
        });
        output.textures_delta.clear();
        output.shapes
    }

    /// Rects filled with `color`: the lines under tabs.
    fn lines_of(shapes: &[egui::epaint::ClippedShape], color: Color32) -> usize {
        shapes
            .iter()
            .filter(
                |clipped| matches!(&clipped.shape, egui::Shape::Rect(rect) if rect.fill == color),
            )
            .count()
    }

    #[test]
    fn a_tab_has_no_underline_unless_it_is_linked_and_then_it_is_its_colour() {
        let orange = crate::ui::tab_link_menu::swatch(crate::model::tab::link::LinkColor::Orange);
        for style in TabTitleStyle::ALL {
            for width in [1400.0, 700.0, 300.0] {
                let plain = paint_strip(4, 1, width, style);
                assert_eq!(lines_of(&plain, theme::SYSTEM_ACCENT), 0, "no blue line");
                assert_eq!(
                    lines_of(&plain, orange),
                    0,
                    "unlinked: none, showing or not"
                );
                // Linked: its colour, whether it is the showing tab or not.
                for active in [1, 2] {
                    let linked = paint_strip_linked(4, active, width, style, Some(2));
                    assert_eq!(lines_of(&linked, orange), 1, "{style:?} at {width}");
                    assert_eq!(lines_of(&linked, theme::SYSTEM_ACCENT), 0);
                }
            }
        }
    }

    fn tab_separators(shapes: &[egui::epaint::ClippedShape]) -> usize {
        shapes
            .iter()
            .filter(|clipped| match &clipped.shape {
                egui::Shape::LineSegment { points, stroke } => {
                    stroke.color == theme::separator()
                        && points[0].x == points[1].x
                        && (points[1].y - points[0].y).abs() == SEPARATOR_HEIGHT
                }
                _ => false,
            })
            .count()
    }

    #[test]
    fn a_hairline_divides_two_tabs_only_when_neither_is_showing() {
        // Five tabs: four gaps. The showing tab takes the gaps on both its sides.
        assert_eq!(
            tab_separators(&paint_strip(5, 0, 1400.0, TabTitleStyle::Full)),
            3
        );
        assert_eq!(
            tab_separators(&paint_strip(5, 2, 1400.0, TabTitleStyle::Full)),
            2
        );
        assert_eq!(
            tab_separators(&paint_strip(5, 4, 1400.0, TabTitleStyle::Full)),
            3
        );
        // A lone tab, or one next to nothing else, has none; two tabs with one showing too.
        assert_eq!(
            tab_separators(&paint_strip(1, 0, 1400.0, TabTitleStyle::Full)),
            0
        );
        assert_eq!(
            tab_separators(&paint_strip(2, 0, 1400.0, TabTitleStyle::Full)),
            0
        );
        assert_eq!(
            tab_separators(&paint_strip(3, 1, 1400.0, TabTitleStyle::Full)),
            0
        );
    }

    #[test]
    fn crowded_and_long_titles_paint_at_every_width() {
        // Nothing panics from a roomy strip down to one that holds only a sliver, with a
        // symbol and a layout name far longer than any tab.
        for count in [1, 2, 12, 60] {
            for width in [2000.0, 900.0, 400.0, 120.0] {
                let shapes = paint_strip(count, count - 1, width, TabTitleStyle::Full);
                assert!(!shapes.is_empty());
            }
        }
    }

    #[test]
    fn tabs_squeeze_to_their_logo_before_they_run_out_of_strip() {
        // Eleven tabs used to overflow a 1000-wide strip at the old 96-wide minimum.
        let width = tab_width(11, 1000.0);
        assert!((MIN_TAB_WIDTH..96.0).contains(&width), "{width}");
        assert_eq!(Detail::for_width(width), Detail::Logo);
        // The logo and the close button fit side by side at the narrowest.
        assert!(
            close_rect(Rect::from_min_size(
                pos2(0.0, 0.0),
                vec2(MIN_TAB_WIDTH, 36.0)
            ))
            .left()
                >= 9.0 + LOGO_SIZE
        );
    }

    #[test]
    fn tabs_scroll_only_when_they_cannot_all_be_shown() {
        assert_eq!(max_scroll(0, 800.0), 0.0);
        assert_eq!(max_scroll(3, 800.0), 0.0);
        // Twenty at the narrowest do not fit 800 wide.
        let max = max_scroll(20, 800.0);
        assert!(max > 0.0);
        // Scrolled to the end, the last tab and the `+` end at the strip's edge.
        let width = tab_width(20, 800.0);
        assert_eq!(width, MIN_TAB_WIDTH);
        let end = tab_left(20, width) - max + NEW_BUTTON_WIDTH;
        assert!((end - 800.0).abs() < 0.01, "{end}");
    }

    #[test]
    fn the_active_tab_is_scrolled_into_view_and_a_visible_one_is_left_alone() {
        let (count, strip) = (20, 800.0);
        let max = max_scroll(count, strip);
        let width = tab_width(count, strip);
        let room = strip - NEW_BUTTON_WIDTH;
        // The last tab, from the start.
        let to_end = reveal(count - 1, count, strip, 0.0);
        assert_eq!(to_end, max);
        // The first, from the end.
        assert_eq!(reveal(0, count, strip, max), 0.0);
        // One already whole in view does not move the strip.
        assert_eq!(reveal(2, count, strip, 10.0), 10.0);
        // Whichever tab is revealed ends up inside the visible part.
        for index in 0..count {
            for from in [0.0, max / 2.0, max] {
                let scroll = reveal(index, count, strip, from);
                let left = tab_left(index, width) - scroll;
                assert!(
                    left >= -0.01 && left + width <= room + 0.01,
                    "{index} from {from}"
                );
            }
        }
    }

    #[test]
    fn a_scrolled_strip_still_paints_and_reports_where_it_is() {
        let ctx = egui::Context::default();
        let tabs: Vec<TabView> = (0..30)
            .map(|index| TabView {
                id: index as u64 + 1,
                title: format!("Tab {index}"),
                icon: Icon::Plus,
                chart: None,
                timeframe: "1D",
                link: None,
                linkable: false,
                link_toggles: Default::default(),
            })
            .collect();
        let quotes = WatchlistQuoteBook::default();
        let mut scrolls = Vec::new();
        for _ in 0..3 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(700.0, HEIGHT))),
                    ..Default::default()
                },
                |ui| {
                    let output = TabBar {
                        window: 1,
                        tabs: &tabs,
                        // The last tab is the active one, so the strip scrolls to it.
                        active: 30,
                        drag: DragView::default(),
                        other_windows: &[],
                        quotes: &quotes,
                        style: TabTitleStyle::default(),
                        update: None,
                    }
                    .show(ui);
                    scrolls.push(output.scroll);
                },
            );
            output.textures_delta.clear();
        }
        assert!(scrolls[0] <= *scrolls.last().unwrap());
        assert!(*scrolls.last().unwrap() > 0.0, "{scrolls:?}");
        assert!(*scrolls.last().unwrap() <= max_scroll(30, 700.0));
    }

    #[test]
    fn the_strip_gives_way_to_an_update_mark_only_while_there_is_one() {
        use crate::host::update_host::Phase;
        let view = |phase| UpdateView {
            phase,
            checking: false,
            checked_ago: None,
        };
        let none = strip_with(None);
        let quiet = strip_with(Some(&view(Phase::UpToDate)));
        let available = strip_with(Some(&view(Phase::Available {
            version: "9.9.9".to_owned(),
            url: String::new(),
            installable: true,
            note: None,
        })));
        assert_eq!(quiet, none);
        assert!(available.right() < none.right() - 40.0, "{available:?}");
        assert_eq!(available.left(), none.left());
    }

    #[test]
    fn slot_at_clamps_to_the_strip() {
        let width = 100.0;
        assert_eq!(slot_at(-50.0, 3, width), 0);
        assert_eq!(slot_at(SIDE_PADDING + 50.0, 3, width), 0);
        assert_eq!(slot_at(SIDE_PADDING + 150.0, 3, width), 1);
        assert_eq!(slot_at(9999.0, 3, width), 2);
        assert_eq!(slot_at(10.0, 0, width), 0);
    }

    #[test]
    fn insertion_picks_the_nearest_gap() {
        let width = 100.0;
        assert_eq!(insertion_at(-30.0, 3, width), 0);
        assert_eq!(insertion_at(SIDE_PADDING + 40.0, 3, width), 0);
        assert_eq!(insertion_at(SIDE_PADDING + 60.0, 3, width), 1);
        assert_eq!(insertion_at(9999.0, 3, width), 3);
    }

    #[test]
    fn tabs_after_an_incoming_gap_shift_one_slot() {
        assert_eq!(slot_of(0, None), 0);
        assert_eq!(slot_of(1, Some(2)), 1);
        assert_eq!(slot_of(2, Some(2)), 3);
        assert_eq!(slot_of(0, Some(0)), 1);
    }

    #[test]
    fn a_dragged_tab_follows_the_pointer_between_the_end_slots() {
        let width = 100.0;
        let first = tab_left(0, width);
        let last = tab_left(2, width);
        // The pointer grabbed the tab 30 in: it sits 30 left of the pointer.
        assert_eq!(dragged_left(first + 130.0, 30.0, 3, width), first + 100.0);
        assert_eq!(dragged_left(-500.0, 30.0, 3, width), first);
        assert_eq!(dragged_left(9999.0, 30.0, 3, width), last);
        assert_eq!(dragged_left(50.0, 0.0, 0, width), first);
    }

    #[test]
    fn a_dragged_tab_swaps_once_its_centre_crosses_a_neighbour() {
        let width = 100.0;
        let step = width + TAB_GAP;
        // Slot 0, dragged right: still slot 0 until its centre passes the slot-0/1 boundary.
        let left = tab_left(0, width);
        assert_eq!(dragged_slot(left, 3, width), 0);
        assert_eq!(dragged_slot(left + step * 0.49, 3, width), 0);
        assert_eq!(dragged_slot(left + step * 0.51, 3, width), 1);
        assert_eq!(dragged_slot(left + step * 5.0, 3, width), 2);
    }

    #[test]
    fn tearing_off_needs_a_clear_exit_from_the_strip() {
        let strip = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, HEIGHT));
        assert!(!is_outside_tear_off_zone(pos2(400.0, 20.0), strip));
        assert!(!is_outside_tear_off_zone(pos2(400.0, HEIGHT + 10.0), strip));
        assert!(is_outside_tear_off_zone(pos2(400.0, HEIGHT * 2.5), strip));
        assert!(is_outside_tear_off_zone(pos2(-HEIGHT - 1.0, 10.0), strip));
    }
}

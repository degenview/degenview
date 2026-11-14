//! The tab strip at the top of every window: one tab per open tab, a `+`, and the events the
//! window host turns into model edits. Dragging is only *reported* here (`DragStarted`); the host
//! drives the rest from raw pointer state because a dragged tab leaves this strip.

use super::chrome::{self, Control};
use super::tab_link_menu;
use super::update_indicator;
use crate::host::update::UpdateView;
use crate::model::InstrumentId;
use crate::model::tab;
use crate::model::tab::title::{Detail, TabChart, TabTitleStyle};
use crate::model::tab::windows::{TabId, WindowId};
use crate::service::watchlist_quotes::WatchlistQuoteBook;
use crate::ui::icons::{self, Icon};
use crate::ui::kit;
use crate::ui::shortcut;
use crate::ui::theme;
use eframe::egui::{self, Id, Rect, Sense, Ui, Vec2, pos2, vec2};
use geometry::close_rect;
use menu::MENU_ROWS;
use paint::fade_edge;

mod geometry;
mod menu;
mod paint;
mod rename;

pub use geometry::{
    dragged_left, dragged_slot, insertion_at, is_outside_tear_off_zone, max_scroll, reveal,
    slot_of, tab_left, tab_width,
};

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
    if chrome::trailing_inset() > 0.0 {
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
        let controls_left = full.right() - chrome::trailing_inset();
        let links_left = controls_left - LINKS_WIDTH;
        let settings_left = links_left - settings_button_width();
        let strip = Rect::from_min_max(
            pos2(full.left() + chrome::leading_inset(fullscreen), full.top()),
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
        let new_tab = crate::ui::accessibility::label(new_tab, "New tab").on_hover_text("New tab");
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
            let button = crate::ui::accessibility::label(button, &label).on_hover_text(&label);
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
                egui::include_image!("../../../../assets/brand/github.svg"),
                14.0,
            ),
            (
                "Support this project by buying me a coffee",
                COFFEE_URL,
                egui::include_image!("../../../../assets/brand/coffee.svg"),
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
            let button = crate::ui::accessibility::label(button, name)
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

        match chrome::window_controls(ui, full, maximized) {
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
        let response = crate::ui::accessibility::label_as(
            response,
            egui::WidgetType::SelectableLabel,
            tab.title.clone(),
        );
        let close = crate::ui::accessibility::label(close, format!("Close {}", tab.title));

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

#[cfg(test)]
mod tests;

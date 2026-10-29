//! The tab strip at the top of every window: one tab per open tab, a `+`, and the events the
//! window host turns into model edits. Dragging is only *reported* here (`DragStarted`); the host
//! drives the rest from raw pointer state because a dragged tab leaves this strip.

use super::icons::{self, Icon};
use super::shortcut;
use super::theme;
use super::update_indicator;
use super::window_chrome::{self, Control};
use crate::host::update_host::UpdateView;
use crate::model::tab_windows::{TabId, WindowId};
use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Id, Painter, Pos2, Rect, Sense, Shadow, Stroke,
    StrokeKind, Ui, Vec2, pos2, vec2,
};

/// Strip height. The strip is the window's title bar (`window_chrome`).
pub const HEIGHT: f32 = 40.0;
const MIN_TAB_WIDTH: f32 = 96.0;
const MAX_TAB_WIDTH: f32 = 220.0;
const NEW_BUTTON_WIDTH: f32 = 34.0;
const SETTINGS_BUTTON_WIDTH: f32 = 34.0;
const TAB_GAP: f32 = 2.0;
const SIDE_PADDING: f32 = 6.0;
const CLOSE_SIZE: f32 = 18.0;
const ICON_SIZE: f32 = 13.0;
/// How long tabs take to slide to a new slot.
const SLIDE_SECONDS: f32 = 0.1;

/// What a tab shows.
#[derive(Clone, Debug)]
pub struct TabView {
    pub id: TabId,
    pub title: String,
    pub icon: Icon,
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

/// What one frame of the strip produced.
pub struct TabBarOutput {
    pub events: Vec<TabBarEvent>,
    /// The part of the strip that holds the tabs, in the window's own coordinates (where the
    /// dragged tab can be dropped): the window buttons' room is left out.
    pub strip: Rect,
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
        let settings_left = controls_left - settings_button_width();
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
        let slide = |name: &str, key: u64, slot: usize| {
            ctx.animate_value_with_time(
                self.id(name).with(key),
                tab_left(slot, target_width),
                SLIDE_SECONDS,
            )
        };

        let mut lifted = None;
        for (index, tab) in self.tabs.iter().enumerate() {
            let left = match self.drag.dragging {
                Some((dragged, left)) if dragged == tab.id => {
                    // Not slid: it follows the pointer exactly. Its own slot is remembered so
                    // it settles there when let go.
                    slide("x", tab.id, slot_of(index, gap));
                    left
                }
                _ => slide("x", tab.id, slot_of(index, gap)),
            };
            let rect = Rect::from_min_size(
                pos2(strip.left() + left, strip.top() + 4.0),
                vec2(width, strip.height() - 4.0),
            );
            if self
                .drag
                .dragging
                .is_some_and(|(dragged, _)| dragged == tab.id)
            {
                lifted = Some((rect, tab));
            } else {
                self.show_tab(ui, rect, tab, false, &mut events);
            }
        }
        // Drawn last so it passes over its neighbours.
        if let Some((rect, tab)) = lifted {
            self.show_tab(ui, rect, tab, true, &mut events);
        }

        let plus_left = ctx.animate_value_with_time(
            self.id("plus"),
            tab_left(slots, target_width),
            SLIDE_SECONDS,
        );
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
            self.paint_tab(&painter.with_clip_rect(strip), rect, &incoming.tab, look);
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

        TabBarOutput { events, strip }
    }

    fn id(&self, name: &str) -> Id {
        Id::new(("window-tab-bar", self.window, name))
    }

    fn show_tab(
        &self,
        ui: &mut Ui,
        rect: Rect,
        tab: &TabView,
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
        self.paint_tab(ui.painter(), rect, tab, look);

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

        response.context_menu(|ui| {
            if ui.button("Close Tab").clicked() {
                events.push(TabBarEvent::Close(tab.id));
                ui.close();
            }
            if self.tabs.len() > 1 && ui.button("Move Tab to New Window").clicked() {
                events.push(TabBarEvent::MoveToNewWindow(tab.id));
                ui.close();
            }
            if !self.other_windows.is_empty() {
                ui.menu_button("Move Tab to Window", |ui| {
                    for window in self.other_windows {
                        if ui.button(&window.title).clicked() {
                            events.push(TabBarEvent::MoveToWindow {
                                tab: tab.id,
                                window: window.id,
                            });
                            ui.close();
                        }
                    }
                });
            }
        });
    }

    fn paint_tab(&self, painter: &Painter, rect: Rect, tab: &TabView, look: Look) {
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
        if raised {
            painter.rect_stroke(
                rect,
                corners,
                Stroke::new(1.0, fade(theme::separator())),
                StrokeKind::Inside,
            );
            if !look.lifted {
                // Joins the tab to the content below by hiding the strip's bottom rule under it.
                painter.hline(
                    rect.x_range().shrink(1.0),
                    rect.bottom() - 0.5,
                    Stroke::new(1.5, theme::background()),
                );
            }
        }

        let color = fade(if look.active || look.lifted {
            theme::text_primary()
        } else {
            theme::text_secondary()
        });
        let icon_rect = Rect::from_center_size(
            pos2(rect.left() + 12.0 + ICON_SIZE / 2.0, rect.center().y + 1.0),
            Vec2::splat(ICON_SIZE),
        );
        icons::paint(painter, icon_rect, tab.icon, color);

        let close_rect = close_rect(rect);
        let text_left = icon_rect.right() + 7.0;
        let text_clip = Rect::from_min_max(
            pos2(text_left, rect.top()),
            pos2(close_rect.left() - 2.0, rect.bottom()),
        );
        painter.with_clip_rect(text_clip).text(
            pos2(text_left, rect.center().y + 1.0),
            Align2::LEFT_CENTER,
            &tab.title,
            FontId::proportional(12.0),
            color,
        );

        if look.hovered || look.close_hovered || look.active {
            if look.close_hovered {
                painter.rect_filled(close_rect, 5.0, theme::ink(26));
            }
            icons::paint(
                painter,
                Rect::from_center_size(close_rect.center(), Vec2::splat(9.0)),
                Icon::Xmark,
                color,
            );
        }
    }
}

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

    /// The strip the tabs get for a window, with or without an update mark.
    fn strip_with(update: Option<&UpdateView>) -> Rect {
        let ctx = egui::Context::default();
        let mut strip = Rect::NOTHING;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let tabs = [TabView {
                id: 1,
                title: "BTC".to_owned(),
                icon: Icon::Plus,
            }];
            strip = TabBar {
                window: 1,
                tabs: &tabs,
                active: 1,
                drag: DragView::default(),
                other_windows: &[],
                update,
            }
            .show(ui)
            .strip;
        });
        output.textures_delta.clear();
        strip
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

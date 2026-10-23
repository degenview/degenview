//! The stateful bridge between the tab-window model and the window callbacks.
//!
//! Every OS window shows one `TabWindow`: a tab strip over the active tab. The window callbacks run
//! as deferred viewports with `'static` closures, so this is a cheap-to-clone handle on shared
//! state. Pure edits (select, reorder, move) apply here at once; anything that needs the app's
//! tab contents (new tab, close with an unsaved-script prompt) is queued as a [`TabRequest`] for
//! `DegenViewApp::process_commands`.

use crate::model::tab_windows::{TabId, TabWindow, TabWindows, WindowId};
use crate::ui::tab_bar::{self, TabBarEvent};
use crate::ui::window_chrome;
use eframe::egui::{self, Pos2, Rect, Vec2, ViewportCommand, ViewportId, vec2};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

/// Where and how big a window first opens. Constant for the window's lifetime so repeated
/// `ViewportBuilder`s never re-position or re-size it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub position: Option<Pos2>,
    pub size: Vec2,
}

impl Placement {
    pub const DEFAULT_SIZE: Vec2 = vec2(1440.0, 900.0);
}

/// What the app has to do for a window.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TabRequest {
    /// The `+`: a new empty tab in this window.
    New(WindowId),
    /// Close one tab (a Script Manager with edits asks first).
    Close(TabId),
    /// The OS close button: close every tab of the window.
    CloseWindow(WindowId),
}

/// What a strip shows of a drag, in the host's terms; the app adds the tab titles.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HostDragView {
    /// A tab of this strip following the pointer, and its left edge in the strip.
    pub dragging: Option<(TabId, f32)>,
    /// A tab from another window: which, the gap it would land in, and where to draw it.
    pub incoming: Option<(TabId, usize, f32)>,
}

/// Where a window and its tab strip are on screen, in points. Missing where the platform hides
/// window positions (Wayland): such a window can tear a tab off but not receive a drop.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Geometry {
    outer: Rect,
    inner: Rect,
    /// The strip in the window's own coordinates.
    strip: Rect,
}

impl Geometry {
    fn strip_on_screen(&self) -> Rect {
        self.strip.translate(self.inner.min.to_vec2())
    }

    /// The window's titlebar height.
    fn titlebar(&self) -> f32 {
        (self.inner.min.y - self.outer.min.y).max(0.0)
    }
}

/// A tab being dragged by the pointer. The window that received the press keeps the pointer
/// grab for the whole drag, so it alone drives the session (`drive_drag`).
#[derive(Clone, Copy, Debug)]
struct Drag {
    tab: TabId,
    source: WindowId,
    /// Pointer offset inside the tab where it was grabbed.
    grab: Vec2,
    /// Once the tab left its strip: the window that follows the cursor.
    floating: Option<WindowId>,
    /// The strip the floating tab would join if released now, and the slot in it.
    hover: Option<(WindowId, usize)>,
    cursor: Option<Pos2>,
    /// The pointer in the source window's own coordinates, as last seen.
    pointer: Option<Pos2>,
    /// The floating window is hidden because its tab shows as a ghost in another strip.
    hidden: bool,
}

#[derive(Default)]
struct State {
    geometry: HashMap<WindowId, Geometry>,
    drag: Option<Drag>,
    windows: TabWindows,
    placement: HashMap<WindowId, Placement>,
    /// Windows by most recent focus, newest first.
    focus: Vec<WindowId>,
    closed: Vec<TabId>,
    requests: Vec<TabRequest>,
}

impl State {
    fn place(&mut self, window: WindowId, position: Option<Pos2>, size: Vec2) {
        self.placement
            .entry(window)
            .or_insert(Placement { position, size });
    }

    /// Drops state of windows the model no longer has.
    fn forget_gone_windows(&mut self) {
        let windows = &self.windows;
        self.placement.retain(|id, _| windows.window(*id).is_some());
        self.focus.retain(|id| windows.window(*id).is_some());
        self.geometry.retain(|id, _| windows.window(*id).is_some());
        if self
            .drag
            .is_some_and(|drag| windows.window(drag.source).is_none())
        {
            self.drag = None;
        }
    }

    /// The strip under `cursor` (not `floating`, the window being dragged) and the slot there.
    /// With overlapping windows the most recently focused one wins.
    fn hover_target(&self, floating: WindowId, cursor: Pos2) -> Option<(WindowId, usize)> {
        let candidates: Vec<(WindowId, Rect, usize)> = self
            .windows
            .windows()
            .iter()
            .filter(|window| window.id != floating)
            .filter_map(|window| {
                let geometry = self.geometry.get(&window.id)?;
                Some((window.id, geometry.strip_on_screen(), window.tabs.len()))
            })
            .collect();
        hover_slot(&candidates, &self.focus, cursor)
    }

    fn size_of(&self, window: WindowId) -> Vec2 {
        self.placement
            .get(&window)
            .map_or(Placement::DEFAULT_SIZE, |placement| placement.size)
    }

    fn focused(&self) -> Option<WindowId> {
        self.focus
            .iter()
            .copied()
            .find(|id| self.windows.window(*id).is_some())
            .or_else(|| self.windows.windows().first().map(|window| window.id))
    }
}

#[derive(Clone, Default)]
pub struct TabWindowHost {
    state: Arc<Mutex<State>>,
}

impl TabWindowHost {
    /// Every tab in one window, in this order: how a launch restores the session.
    pub fn restore(tabs: impl IntoIterator<Item = TabId>) -> Self {
        let host = Self::default();
        {
            let mut state = host.lock();
            state.windows = TabWindows::from_tabs(tabs);
            let ids: Vec<WindowId> = state.windows.windows().iter().map(|w| w.id).collect();
            for id in ids {
                state.place(id, None, Placement::DEFAULT_SIZE);
            }
        }
        host
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    pub fn snapshot(&self) -> Vec<(TabWindow, Placement)> {
        let state = self.lock();
        state
            .windows
            .windows()
            .iter()
            .map(|window| {
                let placement = state
                    .placement
                    .get(&window.id)
                    .copied()
                    .unwrap_or(Placement {
                        position: None,
                        size: Placement::DEFAULT_SIZE,
                    });
                (window.clone(), placement)
            })
            .collect()
    }

    pub fn window(&self, id: WindowId) -> Option<TabWindow> {
        self.lock().windows.window(id).cloned()
    }

    pub fn ordered_tabs(&self) -> Vec<TabId> {
        self.lock().windows.ordered_tabs()
    }

    pub fn is_empty(&self) -> bool {
        self.lock().windows.is_empty()
    }

    pub fn focused_window(&self) -> Option<WindowId> {
        self.lock().focused()
    }

    /// Records that `window` has keyboard focus.
    pub fn note_focused(&self, window: WindowId) {
        let mut state = self.lock();
        if state.focus.first() != Some(&window) {
            state.focus.retain(|id| *id != window);
            state.focus.insert(0, window);
        }
    }

    /// A new tab joins the window in front, else the first one, else opens a window.
    pub fn add_tab(&self, tab: TabId) -> WindowId {
        let mut state = self.lock();
        let target = state.focused();
        let window = state.windows.add_tab(target, tab);
        state.place(window, None, Placement::DEFAULT_SIZE);
        window
    }

    pub fn add_tab_to(&self, window: WindowId, tab: TabId) {
        self.lock().windows.add_tab(Some(window), tab);
    }

    /// Shows `tab` in its window and returns that window.
    pub fn select_tab(&self, tab: TabId) -> Option<WindowId> {
        let mut state = self.lock();
        if state.windows.select(tab) {
            state.windows.window_of(tab)
        } else {
            None
        }
    }

    /// Removes the tab without asking anyone; the app drops its content next frame.
    pub fn close_tab(&self, tab: TabId) {
        let mut state = self.lock();
        if state.windows.close_tab(tab).is_some() {
            state.closed.push(tab);
            state.forget_gone_windows();
        }
    }

    pub fn take_requests(&self) -> Vec<TabRequest> {
        std::mem::take(&mut self.lock().requests)
    }

    pub fn take_closed(&self) -> Vec<TabId> {
        std::mem::take(&mut self.lock().closed)
    }

    pub fn request_close_window(&self, window: WindowId) {
        self.lock().requests.push(TabRequest::CloseWindow(window));
    }

    /// The tabs of `window`, for closing them one by one.
    pub fn tabs_of(&self, window: WindowId) -> Vec<TabId> {
        self.lock()
            .windows
            .window(window)
            .map(|window| window.tabs.clone())
            .unwrap_or_default()
    }

    /// Applies what a window's strip reported.
    pub fn apply(&self, window: WindowId, events: Vec<TabBarEvent>) {
        let mut state = self.lock();
        for event in events {
            match event {
                TabBarEvent::Select(tab) => {
                    state.windows.select(tab);
                }
                TabBarEvent::Close(tab) => state.requests.push(TabRequest::Close(tab)),
                TabBarEvent::New => state.requests.push(TabRequest::New(window)),
                TabBarEvent::MoveToNewWindow(tab) => {
                    Self::detach(&mut state, tab, None);
                }
                TabBarEvent::MoveToWindow { tab, window } => {
                    let end = state.windows.window(window).map_or(0, |w| w.tabs.len());
                    state.windows.move_tab(tab, window, end);
                    state.forget_gone_windows();
                }
                TabBarEvent::DragStarted { tab, grab } => {
                    state.drag = Some(Drag {
                        tab,
                        source: window,
                        grab,
                        floating: None,
                        hover: None,
                        cursor: None,
                        pointer: None,
                        hidden: false,
                    });
                }
                // Not about tabs: the window answers it with the update host.
                TabBarEvent::UpdateClicked => {}
            }
        }
    }

    fn detach(state: &mut State, tab: TabId, position: Option<Pos2>) -> Option<WindowId> {
        let source = state.windows.window_of(tab)?;
        let size = state.size_of(source);
        let window = state.windows.detach(tab)?;
        state.place(window, position, size);
        Some(window)
    }

    /// The Window menu's Move Tab to New Window, for the window in front.
    pub fn detach_active(&self) {
        let mut state = self.lock();
        if let Some(window) = state
            .focused()
            .and_then(|id| state.windows.window(id).cloned())
        {
            Self::detach(&mut state, window.active, None);
        }
    }

    /// The Window menu's Merge All Windows, into the window in front.
    pub fn merge_all(&self) {
        let mut state = self.lock();
        if let Some(target) = state.focused() {
            state.windows.merge_all(target);
            state.forget_gone_windows();
        }
    }

    /// Show Next / Previous Tab in the window in front; returns the tab now showing.
    pub fn cycle(&self, forward: bool) -> Option<TabId> {
        let mut state = self.lock();
        let window = state.focused()?;
        state.windows.cycle(window, forward)
    }

    /// Show Next / Previous Tab in a specific window.
    pub fn cycle_window(&self, window: WindowId, forward: bool) -> Option<TabId> {
        self.lock().windows.cycle(window, forward)
    }

    pub fn viewport_id(window: WindowId) -> ViewportId {
        ViewportId::from_hash_of(("degenview-window", window))
    }

    /// What a window reports every frame: where it is and where its tab strip is.
    pub fn publish_geometry(
        &self,
        window: WindowId,
        outer: Option<Rect>,
        inner: Option<Rect>,
        strip: Rect,
    ) {
        let mut state = self.lock();
        match (outer, inner) {
            (Some(outer), Some(inner)) => {
                state.geometry.insert(
                    window,
                    Geometry {
                        outer,
                        inner,
                        strip,
                    },
                );
            }
            _ => {
                state.geometry.remove(&window);
            }
        }
    }

    /// What `window`'s strip shows of a drag: a tab of its own following the pointer, or a tab
    /// from another window hovering over it.
    pub fn drag_view(&self, window: WindowId) -> HostDragView {
        let state = self.lock();
        let (Some(drag), Some(geometry)) = (state.drag, state.geometry.get(&window)) else {
            return HostDragView::default();
        };
        let count = state
            .windows
            .window(window)
            .map_or(0, |window| window.tabs.len());
        let mut view = HostDragView::default();
        if drag.source == window
            && drag.floating.is_none()
            && let Some(pointer) = drag.pointer
        {
            let width = tab_bar::tab_width(count, geometry.strip.width());
            let left =
                tab_bar::dragged_left(pointer.x - geometry.strip.left(), drag.grab.x, count, width);
            view.dragging = Some((drag.tab, left));
        }
        if let Some((target, slot)) = drag.hover
            && target == window
            && let Some(cursor) = drag.cursor
        {
            let width = tab_bar::tab_width(count + 1, geometry.strip.width());
            let left = (cursor.x - geometry.strip_on_screen().left() - drag.grab.x)
                .clamp(tab_bar::tab_left(0, width), tab_bar::tab_left(count, width));
            view.incoming = Some((drag.tab, slot, left));
        }
        view
    }

    /// Advances the drag; every window calls this each frame. The window that got the press sees
    /// the pointer and reorders inside the strip. With the OS's cursor and button state any
    /// window can carry the drag on, which it must: the floating window is hidden while its tab
    /// shows as a ghost, and a hidden window runs no frames. Past the strip the tab tears off into
    /// a window that follows the cursor, and on release the tab joins the strip under it.
    pub fn drive_drag(&self, ctx: &egui::Context, window: WindowId) {
        let egui_down = ctx.input(|input| input.pointer.primary_down());
        let down = crate::native_bridge::primary_button_down().unwrap_or(egui_down);
        let cursor = crate::native_bridge::global_cursor(ctx.pixels_per_point());
        self.drive_drag_with(ctx, window, down, cursor);
    }

    /// `drive_drag` with the button and the OS cursor given (tests have no real mouse).
    fn drive_drag_with(
        &self,
        ctx: &egui::Context,
        window: WindowId,
        down: bool,
        os_cursor: Option<Pos2>,
    ) {
        let mut commands: Vec<(ViewportId, ViewportCommand)> = Vec::new();
        let mut repaint: Vec<ViewportId> = Vec::new();
        {
            let mut state = self.lock();
            let Some(mut drag) = state.drag else {
                return;
            };
            let is_source = drag.source == window;
            // Only the press's window sees the pointer; the others need the OS cursor.
            if !is_source && os_cursor.is_none() {
                return;
            }
            let pointer = if is_source {
                ctx.input(|input| input.pointer.latest_pos())
            } else {
                None
            };
            let source = drag.source;
            let geometry = state.geometry.get(&source).copied();
            let previous_hover = drag.hover;
            drag.pointer = pointer.or(drag.pointer);

            match geometry {
                None => {
                    if is_source {
                        Self::drive_without_geometry(&mut state, &mut drag, pointer, down);
                    }
                }
                Some(geometry) => {
                    // The OS's cursor when it tells us. Window-relative pointer positions are
                    // only trustworthy while the source window stands still.
                    let source_moves = drag.floating == Some(source);
                    let cursor = os_cursor
                        .or_else(|| {
                            (!source_moves && is_source)
                                .then(|| pointer.map(|p| geometry.inner.min + p.to_vec2()))
                                .flatten()
                        })
                        .or(drag.cursor);
                    drag.cursor = cursor;
                    if !down {
                        if let Some((target, index)) = drag.hover {
                            state.windows.move_tab(drag.tab, target, index);
                            state.forget_gone_windows();
                            commands.push((Self::viewport_id(target), ViewportCommand::Focus));
                        }
                        state.drag = None;
                        repaint.push(ViewportId::ROOT);
                        drag.hover = None;
                    } else if let Some(floating) = drag.floating {
                        if source_moves && os_cursor.is_none() {
                            // No way to follow the cursor ourselves: the OS drags the window.
                            commands.push((Self::viewport_id(source), ViewportCommand::StartDrag));
                            state.drag = None;
                        } else if let Some(cursor) = cursor {
                            drag.hover = state.hover_target(floating, cursor);
                            // Over a strip the dragged tab shows there as a ghost, so its own
                            // window is hidden.
                            let hovering = drag.hover.is_some();
                            if hovering != drag.hidden {
                                drag.hidden = hovering;
                                commands.push((
                                    Self::viewport_id(floating),
                                    ViewportCommand::Visible(!hovering),
                                ));
                            }
                            if !hovering {
                                commands.push((
                                    Self::viewport_id(floating),
                                    ViewportCommand::OuterPosition(
                                        cursor - float_offset(drag.grab, geometry.titlebar()),
                                    ),
                                ));
                            }
                        }
                        // A hidden window gets no frames of its own; ask for the next one anyway
                        // so it comes back as soon as the tab leaves the strip.
                        repaint.push(Self::viewport_id(floating));
                    } else if is_source && let Some(pointer) = pointer {
                        Self::drive_reorder(
                            &mut state, &mut drag, source, geometry, pointer, cursor,
                        );
                    }
                    if state.drag.is_some() {
                        state.drag = Some(drag);
                    }
                }
            }
            // The strip under the cursor follows it with its ghost tab every frame.
            if let Some(hover) = drag.hover {
                repaint.push(Self::viewport_id(hover.0));
            }
            if let Some(previous) = previous_hover
                && previous_hover != drag.hover
            {
                repaint.push(Self::viewport_id(previous.0));
            }
        }
        for (viewport, command) in commands {
            ctx.send_viewport_cmd_to(viewport, command);
        }
        for viewport in repaint {
            ctx.request_repaint_of(viewport);
        }
        ctx.request_repaint();
    }

    fn drive_reorder(
        state: &mut State,
        drag: &mut Drag,
        window: WindowId,
        geometry: Geometry,
        pointer: Pos2,
        cursor: Option<Pos2>,
    ) {
        let Some(count) = state.windows.window(window).map(|window| window.tabs.len()) else {
            return;
        };
        if !tab_bar::is_outside_tear_off_zone(pointer, geometry.strip) {
            let width = tab_bar::tab_width(count, geometry.strip.width());
            let left =
                tab_bar::dragged_left(pointer.x - geometry.strip.left(), drag.grab.x, count, width);
            state
                .windows
                .reorder(drag.tab, tab_bar::dragged_slot(left, count, width));
            return;
        }
        // Left the strip: the tab becomes a window under the cursor (a lone tab already is one).
        let Some(cursor) = cursor else {
            return;
        };
        if count > 1 {
            let position = cursor - float_offset(drag.grab, geometry.titlebar());
            let size = geometry.inner.size();
            if let Some(new) = Self::detach_with(state, drag.tab, Some(position), size) {
                drag.floating = Some(new);
            }
        } else {
            drag.floating = Some(window);
        }
    }

    /// Without screen coordinates a tab can still leave its strip, into a window the OS places.
    fn drive_without_geometry(
        state: &mut State,
        drag: &mut Drag,
        pointer: Option<Pos2>,
        down: bool,
    ) {
        let leaving = pointer.is_some_and(|pointer| {
            pointer.y > tab_bar::HEIGHT * 2.5 || pointer.y < -tab_bar::HEIGHT
        });
        if leaving {
            let _ = Self::detach(state, drag.tab, None);
        }
        if leaving || !down {
            state.drag = None;
        }
    }

    fn detach_with(
        state: &mut State,
        tab: TabId,
        position: Option<Pos2>,
        size: Vec2,
    ) -> Option<WindowId> {
        let window = state.windows.detach(tab)?;
        state.place(window, position, size);
        Some(window)
    }

    /// The tab showing in the window in front.
    pub fn focused_tab(&self) -> Option<TabId> {
        let state = self.lock();
        let window = state.focused()?;
        state.windows.window(window).map(|window| window.active)
    }
}

/// Where the cursor sits relative to a window's top-left while it carries a torn-off tab: over
/// the tab, which is the first one in its strip.
fn float_offset(grab: Vec2, titlebar: f32) -> Vec2 {
    vec2(
        window_chrome::leading_inset(false) + tab_bar::tab_left(0, 0.0) + grab.x,
        titlebar + 4.0 + grab.y,
    )
}

/// Slack above and below a strip in which a dragged tab still counts as over it.
const DROP_SLACK: f32 = 14.0;

/// The strip (`id`, screen rect, tab count) the cursor is over and the slot there, preferring
/// the window focused most recently.
fn hover_slot(
    strips: &[(WindowId, Rect, usize)],
    focus: &[WindowId],
    cursor: Pos2,
) -> Option<(WindowId, usize)> {
    let rank = |id: WindowId| {
        focus
            .iter()
            .position(|other| *other == id)
            .unwrap_or(usize::MAX)
    };
    strips
        .iter()
        .filter(|(_, strip, _)| strip.expand2(vec2(0.0, DROP_SLACK)).contains(cursor))
        .min_by_key(|(id, _, _)| rank(*id))
        .map(|(id, strip, count)| {
            let width = tab_bar::tab_width(*count, strip.width());
            (
                *id,
                tab_bar::insertion_at(cursor.x - strip.left(), *count, width),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::pos2;

    impl TabWindowHost {
        fn window_of(&self, tab: TabId) -> Option<WindowId> {
            self.lock().windows.window_of(tab)
        }
    }

    #[test]
    fn restore_opens_one_window_with_every_tab() {
        let host = TabWindowHost::restore([4, 2, 7]);
        let windows = host.snapshot();
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].0.tabs, vec![4, 2, 7]);
        assert_eq!(windows[0].0.active, 4);
    }

    #[test]
    fn new_tabs_join_the_window_in_front() {
        let host = TabWindowHost::restore([1, 2]);
        host.apply(
            host.focused_window().unwrap(),
            vec![TabBarEvent::MoveToNewWindow(2)],
        );
        let second = host.window_of(2).unwrap();
        host.note_focused(second);
        host.add_tab(3);
        assert_eq!(host.window_of(3), Some(second));
        assert_eq!(host.ordered_tabs(), vec![1, 2, 3]);
    }

    #[test]
    fn strip_events_edit_or_queue() {
        let host = TabWindowHost::restore([1, 2, 3]);
        let window = host.focused_window().unwrap();
        host.apply(
            window,
            vec![
                TabBarEvent::Select(3),
                TabBarEvent::Close(2),
                TabBarEvent::New,
            ],
        );
        assert_eq!(host.window(window).unwrap().active, 3);
        // Closing waits for the app (an unsaved script may object); the tab is still there.
        assert_eq!(host.ordered_tabs(), vec![1, 2, 3]);
        assert_eq!(
            host.take_requests(),
            vec![TabRequest::Close(2), TabRequest::New(window)]
        );
        assert!(host.take_requests().is_empty());
    }

    #[test]
    fn closing_a_tab_reports_it_once_and_forgets_an_emptied_window() {
        let host = TabWindowHost::restore([1, 2]);
        let home = host.focused_window().unwrap();
        host.apply(home, vec![TabBarEvent::MoveToNewWindow(2)]);
        let other = host.window_of(2).unwrap();
        host.note_focused(other);
        host.close_tab(2);
        assert_eq!(host.take_closed(), vec![2]);
        assert!(host.take_closed().is_empty());
        assert_eq!(host.focused_window(), Some(home));
        assert!(host.window(other).is_none());
    }

    #[test]
    fn move_to_window_appends_and_merge_all_gathers() {
        let host = TabWindowHost::restore([1, 2, 3]);
        let home = host.focused_window().unwrap();
        host.apply(home, vec![TabBarEvent::MoveToNewWindow(3)]);
        let other = host.window_of(3).unwrap();
        host.apply(
            home,
            vec![TabBarEvent::MoveToWindow {
                tab: 1,
                window: other,
            }],
        );
        assert_eq!(host.window(other).unwrap().tabs, vec![3, 1]);
        host.note_focused(home);
        host.merge_all();
        assert_eq!(host.snapshot().len(), 1);
        assert_eq!(host.ordered_tabs(), vec![2, 3, 1]);
    }

    #[test]
    fn cycling_and_active_tab_follow_the_window_in_front() {
        let host = TabWindowHost::restore([1, 2, 3]);
        assert_eq!(host.focused_tab(), Some(1));
        assert_eq!(host.cycle(true), Some(2));
        assert_eq!(host.focused_tab(), Some(2));
        assert_eq!(host.cycle(false), Some(1));
    }

    fn strip(x: f32, y: f32) -> Rect {
        Rect::from_min_size(pos2(x, y), vec2(800.0, tab_bar::HEIGHT))
    }

    #[test]
    fn the_cursor_over_a_strip_picks_its_slot() {
        let strips = [(1, strip(0.0, 100.0), 3)];
        assert_eq!(hover_slot(&strips, &[], pos2(5.0, 110.0)), Some((1, 0)));
        assert_eq!(hover_slot(&strips, &[], pos2(795.0, 110.0)), Some((1, 3)));
        // A little above or below still counts; well away does not.
        assert!(hover_slot(&strips, &[], pos2(100.0, 100.0 - DROP_SLACK + 1.0)).is_some());
        assert_eq!(hover_slot(&strips, &[], pos2(100.0, 300.0)), None);
        assert_eq!(hover_slot(&strips, &[], pos2(900.0, 110.0)), None);
    }

    #[test]
    fn overlapping_strips_go_to_the_window_focused_most_recently() {
        let strips = [(1, strip(0.0, 100.0), 2), (2, strip(0.0, 100.0), 2)];
        assert_eq!(
            hover_slot(&strips, &[2, 1], pos2(10.0, 110.0)).unwrap().0,
            2
        );
        assert_eq!(
            hover_slot(&strips, &[1, 2], pos2(10.0, 110.0)).unwrap().0,
            1
        );
    }

    #[test]
    fn a_floating_window_sits_so_the_tab_stays_under_the_cursor() {
        let offset = float_offset(vec2(30.0, 8.0), 28.0);
        assert_eq!(
            offset,
            vec2(
                window_chrome::leading_inset(false) + tab_bar::tab_left(0, 0.0) + 30.0,
                28.0 + 4.0 + 8.0
            )
        );
    }

    /// One egui frame with the primary button down and the pointer at `x` on the strip.
    fn drag_frame(host: &TabWindowHost, window: WindowId, x: f32) {
        let ctx = egui::Context::default();
        let pos = pos2(x, 17.0);
        let input = egui::RawInput {
            events: vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::default(),
                },
            ],
            ..egui::RawInput::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            host.drive_drag_with(ui.ctx(), window, true, None)
        });
        // Nothing paints it; egui insists the font texture upload is acknowledged.
        output.textures_delta.clear();
    }

    fn started_drag(host: &TabWindowHost, tab: TabId, grab_x: f32) -> WindowId {
        let window = host.focused_window().unwrap();
        let strip = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, tab_bar::HEIGHT));
        let inner = Rect::from_min_size(pos2(100.0, 128.0), vec2(800.0, 600.0));
        let outer = Rect::from_min_size(pos2(100.0, 100.0), vec2(800.0, 628.0));
        host.publish_geometry(window, Some(outer), Some(inner), strip);
        host.apply(
            window,
            vec![TabBarEvent::DragStarted {
                tab,
                grab: vec2(grab_x, 5.0),
            }],
        );
        window
    }

    #[test]
    fn a_dragged_tab_follows_the_pointer_and_reorders_by_its_centre() {
        let host = TabWindowHost::restore([1, 2]);
        let window = started_drag(&host, 2, 10.0);
        // Nothing to show until the pointer has been seen.
        assert_eq!(host.drag_view(window), HostDragView::default());

        // 200 − 10 grabbed: the tab's left edge is 190, its centre is over slot 1 where it is.
        drag_frame(&host, window, 200.0);
        assert_eq!(host.drag_view(window).dragging, Some((2, 190.0)));
        assert_eq!(host.ordered_tabs(), vec![1, 2]);

        // Dragged far left: pinned to the first slot, and now over it.
        drag_frame(&host, window, 0.0);
        let left = tab_bar::tab_left(0, tab_bar::tab_width(2, 800.0));
        assert_eq!(host.drag_view(window).dragging, Some((2, left)));
        assert_eq!(host.ordered_tabs(), vec![2, 1]);
        assert_eq!(host.drag_view(window).incoming, None);
    }

    #[test]
    fn the_session_ends_with_its_source_window() {
        let host = TabWindowHost::restore([1, 2]);
        let window = started_drag(&host, 2, 10.0);
        host.close_tab(2);
        host.close_tab(1);
        assert_eq!(host.drag_view(window), HostDragView::default());
    }

    #[test]
    fn a_detached_window_opens_at_its_source_size() {
        let host = TabWindowHost::restore([1, 2]);
        let home = host.focused_window().unwrap();
        host.apply(home, vec![TabBarEvent::MoveToNewWindow(2)]);
        let detached = host.window_of(2).unwrap();
        let placement = host
            .snapshot()
            .into_iter()
            .find(|(window, _)| window.id == detached)
            .unwrap()
            .1;
        assert_eq!(placement.size, Placement::DEFAULT_SIZE);
    }
}

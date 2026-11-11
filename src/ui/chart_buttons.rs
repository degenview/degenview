//! The buttons a chart carries on its own canvas: the navigation group (zoom, scroll, reset) at
//! the foot of the plot, and the pane group (move, collapse, maximize, remove) on each script
//! pane. Whether each group shows is the canvas' Buttons setting; hiding a group hides only the
//! buttons, since the wheel, the trackpad, the drag and the keyboard drive the same viewport.
//!
//! These functions only draw and report a click. The chart card applies the action to its own
//! viewport and Pine instance list, so there is one zoom/scroll state and one pane order.

use super::accessibility;
use super::canvas_render::CanvasStyle;
use super::icons::{self, Icon};
use super::theme;
use crate::model::chart::canvas::ButtonVisibility;
use eframe::egui::{self, CursorIcon, Id, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, vec2};

const BUTTON: f32 = 20.0;
const GAP: f32 = 2.0;
const PADDING: f32 = 2.0;
const INSET: f32 = 10.0;
/// How long a group takes to fade in or out.
const FADE: f32 = 0.12;
/// Clicks are taken once a fading group is this opaque.
const CLICKABLE: f32 = 0.5;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NavAction {
    ZoomIn,
    ZoomOut,
    ScrollLeft,
    ScrollRight,
    Reset,
}

impl NavAction {
    pub const ALL: [Self; 5] = [
        Self::ZoomIn,
        Self::ZoomOut,
        Self::ScrollLeft,
        Self::ScrollRight,
        Self::Reset,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::ZoomIn => "Zoom in",
            Self::ZoomOut => "Zoom out",
            Self::ScrollLeft => "Scroll to the left",
            Self::ScrollRight => "Scroll to the right",
            Self::Reset => "Reset chart view",
        }
    }

    const fn icon(self) -> Icon {
        match self {
            Self::ZoomIn => Icon::Plus,
            Self::ZoomOut => Icon::Minus,
            Self::ScrollLeft => Icon::ChevronLeft,
            Self::ScrollRight => Icon::ChevronRight,
            Self::Reset => Icon::Undo,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PaneAction {
    MoveUp,
    MoveDown,
    ToggleCollapse,
    ToggleMaximize,
    Remove,
}

/// What a pane's buttons need to know about the pane.
#[derive(Clone, Copy, Debug)]
pub struct PaneState {
    pub can_move_up: bool,
    pub can_move_down: bool,
    pub collapsed: bool,
    pub maximized: bool,
}

impl PaneState {
    fn buttons(self) -> [(PaneAction, Icon, &'static str, bool); 5] {
        [
            (
                PaneAction::MoveUp,
                Icon::ChevronUp,
                "Move pane up",
                self.can_move_up,
            ),
            (
                PaneAction::MoveDown,
                Icon::ChevronDown,
                "Move pane down",
                self.can_move_down,
            ),
            (
                PaneAction::ToggleCollapse,
                if self.collapsed {
                    Icon::Plus
                } else {
                    Icon::Minus
                },
                if self.collapsed {
                    "Expand pane"
                } else {
                    "Collapse pane"
                },
                // A maximized pane cannot also be collapsed.
                !self.maximized,
            ),
            (
                PaneAction::ToggleMaximize,
                if self.maximized {
                    Icon::Restore
                } else {
                    Icon::Maximize
                },
                if self.maximized {
                    "Restore pane"
                } else {
                    "Maximize pane"
                },
                !self.collapsed,
            ),
            (
                PaneAction::Remove,
                Icon::Xmark,
                "Remove script from the chart",
                true,
            ),
        ]
    }
}

// MARK: Geometry

/// The least plot that has room for the navigation group.
pub fn navigation_fits(plot: Rect) -> bool {
    plot.width() >= group_width(NavAction::ALL.len()) + 2.0 * INSET
        && plot.height() >= 3.0 * BUTTON + 2.0 * INSET
}

fn group_width(count: usize) -> f32 {
    count as f32 * BUTTON + count.saturating_sub(1) as f32 * GAP + 2.0 * PADDING
}

/// The group's pill and the rect of each button, bottom-left of `plot`.
pub fn navigation_rects(plot: Rect) -> (Rect, [Rect; 5]) {
    let size = vec2(group_width(NavAction::ALL.len()), BUTTON + 2.0 * PADDING);
    let pill = Rect::from_min_size(
        Pos2::new(plot.left() + INSET, plot.bottom() - INSET - size.y),
        size,
    );
    let buttons = std::array::from_fn(|index| {
        Rect::from_min_size(
            Pos2::new(
                pill.left() + PADDING + index as f32 * (BUTTON + GAP),
                pill.top() + PADDING,
            ),
            Vec2::splat(BUTTON),
        )
    });
    (pill, buttons)
}

/// The group's pill and button rects, along the top-right of `pane` (left of `right`).
pub fn pane_rects(pane: Rect, right: f32) -> (Rect, [Rect; 5]) {
    let size = vec2(group_width(5), BUTTON + 2.0 * PADDING);
    let pill = Rect::from_min_size(
        Pos2::new(
            right - 6.0 - size.x,
            pane.top() + (4.0f32).min((pane.height() - size.y).max(0.0) / 2.0),
        ),
        size,
    );
    let buttons = std::array::from_fn(|index| {
        Rect::from_min_size(
            Pos2::new(
                pill.left() + PADDING + index as f32 * (BUTTON + GAP),
                pill.top() + PADDING,
            ),
            Vec2::splat(BUTTON),
        )
    });
    (pill, buttons)
}

/// Whether a pane is tall and wide enough for its buttons beside a title.
pub fn pane_fits(pane: Rect, right: f32) -> bool {
    pane.height() >= BUTTON + 2.0 * PADDING && right - pane.left() >= group_width(5) + 80.0
}

// MARK: Drawing

/// How visible a group is this frame: animated towards the wanted state, so a group fades rather
/// than pops, and stays up while the pointer is on one of its own buttons.
fn fade(ui: &Ui, id: Id, visibility: ButtonVisibility, hovered: bool) -> f32 {
    ui.ctx()
        .animate_bool_with_time(id.with("fade"), visibility.wanted(hovered), FADE)
}

fn paint_group(ui: &Ui, pill: Rect, opacity: f32) {
    let radius = 8.0;
    ui.painter().rect_filled(
        pill,
        radius,
        theme::with_alpha(theme::surface(), 0.92 * opacity),
    );
    ui.painter().rect_stroke(
        pill,
        radius,
        Stroke::new(1.0, theme::with_alpha(theme::border(), opacity)),
        StrokeKind::Inside,
    );
}

fn paint_button(ui: &Ui, rect: Rect, icon: Icon, hovered: bool, enabled: bool, opacity: f32) {
    if hovered && enabled {
        ui.painter()
            .rect_filled(rect, 6.0, theme::with_alpha(theme::ink(40), opacity));
    }
    let color = if enabled {
        theme::text_primary()
    } else {
        theme::text_tertiary()
    };
    icons::paint(
        ui.painter(),
        egui::Rect::from_center_size(rect.center(), Vec2::splat(12.0)),
        icon,
        theme::with_alpha(color, opacity),
    );
}

/// The navigation group over `plot`. `hovered` is whether the pointer is over the chart.
/// Returns the button clicked this frame.
pub fn navigation(
    ui: &mut Ui,
    id: Id,
    plot: Rect,
    style: &CanvasStyle,
    hovered: bool,
) -> Option<NavAction> {
    if !navigation_fits(plot) {
        return None;
    }
    let (pill, rects) = navigation_rects(plot);
    // The group counts as hovered while the pointer is over it too, so it never vanishes under
    // the pointer however the chart's own hover state resolves.
    let over_group = ui.rect_contains_pointer(pill.expand(4.0));
    let opacity = fade(ui, id, style.navigation, hovered || over_group);
    if opacity <= 0.01 {
        return None;
    }
    paint_group(ui, pill, opacity);
    let mut clicked = None;
    for (action, rect) in NavAction::ALL.into_iter().zip(rects) {
        let response = ui
            .interact(rect, id.with(action.label()), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand)
            .on_hover_text(action.label());
        let response = accessibility::label(response, action.label());
        paint_button(ui, rect, action.icon(), response.hovered(), true, opacity);
        if response.clicked() && opacity >= CLICKABLE {
            clicked = Some(action);
        }
    }
    clicked
}

/// One script pane's buttons along its top-right. `hovered` is whether the pointer is over the
/// pane. Returns the button clicked this frame.
pub fn pane(
    ui: &mut Ui,
    id: Id,
    pane: Rect,
    right: f32,
    style: &CanvasStyle,
    state: PaneState,
    hovered: bool,
) -> Option<PaneAction> {
    if !pane_fits(pane, right) {
        return None;
    }
    let (pill, rects) = pane_rects(pane, right);
    let over_group = ui.rect_contains_pointer(pill.expand(4.0));
    // A collapsed or maximized pane keeps its buttons: they are the only way back.
    let visibility = if state.collapsed || state.maximized {
        ButtonVisibility::Always
    } else {
        style.pane_buttons
    };
    let opacity = fade(ui, id, visibility, hovered || over_group);
    if opacity <= 0.01 {
        return None;
    }
    paint_group(ui, pill, opacity);
    let mut clicked = None;
    for ((action, icon, label, enabled), rect) in state.buttons().into_iter().zip(rects) {
        let response = ui.interact(
            rect,
            id.with(label),
            if enabled {
                Sense::click()
            } else {
                Sense::hover()
            },
        );
        let response = accessibility::label(response, label).on_hover_text(label);
        let response = if enabled {
            response.on_hover_cursor(CursorIcon::PointingHand)
        } else {
            response
        };
        paint_button(ui, rect, icon, response.hovered(), enabled, opacity);
        if enabled && response.clicked() && opacity >= CLICKABLE {
            clicked = Some(action);
        }
    }
    clicked
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plot() -> Rect {
        Rect::from_min_size(Pos2::new(20.0, 30.0), vec2(600.0, 320.0))
    }

    #[test]
    fn navigation_sits_at_the_bottom_left_inside_the_plot() {
        let (pill, buttons) = navigation_rects(plot());
        assert!(plot().contains_rect(pill));
        assert_eq!(pill.left(), plot().left() + INSET);
        assert_eq!(pill.bottom(), plot().bottom() - INSET);
        for pair in buttons.windows(2) {
            assert!((pair[1].left() - pair[0].right() - GAP).abs() < 1e-4);
            assert_eq!(pair[0].size(), Vec2::splat(BUTTON));
        }
        assert!(pill.contains_rect(buttons[0]) && pill.contains_rect(buttons[4]));
    }

    #[test]
    fn navigation_is_left_out_of_a_plot_too_small_for_it() {
        assert!(navigation_fits(plot()));
        assert!(!navigation_fits(Rect::from_min_size(
            Pos2::ZERO,
            vec2(100.0, 320.0)
        )));
        assert!(!navigation_fits(Rect::from_min_size(
            Pos2::ZERO,
            vec2(600.0, 40.0)
        )));
        assert!(!navigation_fits(Rect::from_min_size(
            Pos2::ZERO,
            Vec2::ZERO
        )));
    }

    #[test]
    fn pane_buttons_sit_in_the_panes_top_right_corner() {
        let pane_rect = Rect::from_min_size(Pos2::new(20.0, 400.0), vec2(600.0, 90.0));
        let right = 520.0;
        assert!(pane_fits(pane_rect, right));
        let (pill, buttons) = pane_rects(pane_rect, right);
        assert!(pane_rect.contains_rect(pill));
        assert!(pill.right() < right);
        assert!(pill.top() >= pane_rect.top());
        assert!(pill.contains_rect(buttons[4]));
        // A collapsed pane's strip is just tall enough.
        let strip = Rect::from_min_size(
            Pos2::new(20.0, 400.0),
            vec2(600.0, crate::model::chart::canvas::COLLAPSED_PANE_HEIGHT),
        );
        assert!(pane_fits(strip, right));
        let (strip_pill, _) = pane_rects(strip, right);
        assert!(strip.contains_rect(strip_pill) || strip_pill.height() <= strip.height() + 6.0);
        assert!(!pane_fits(
            Rect::from_min_size(Pos2::ZERO, vec2(100.0, 90.0)),
            100.0
        ));
    }

    #[test]
    fn pane_buttons_switch_with_the_panes_state() {
        let normal = PaneState {
            can_move_up: false,
            can_move_down: true,
            collapsed: false,
            maximized: false,
        };
        let buttons = normal.buttons();
        assert!(!buttons[0].3 && buttons[1].3, "the first pane cannot go up");
        assert_eq!(buttons[2].2, "Collapse pane");
        assert_eq!(buttons[3].2, "Maximize pane");
        let collapsed = PaneState {
            collapsed: true,
            ..normal
        }
        .buttons();
        assert_eq!(collapsed[2].2, "Expand pane");
        assert!(
            !collapsed[3].3,
            "a collapsed pane is expanded before it is maximized"
        );
        let maximized = PaneState {
            maximized: true,
            ..normal
        }
        .buttons();
        assert_eq!(maximized[3].2, "Restore pane");
        assert!(!maximized[2].3);
        assert!(maximized[4].3, "a script can always be removed");
    }

    #[test]
    fn every_action_has_a_label() {
        for action in NavAction::ALL {
            assert!(!action.label().is_empty());
        }
    }

    fn click_at(ctx: &egui::Context, at: Pos2, mut draw: impl FnMut(&mut Ui)) {
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(900.0, 700.0));
        let mut time = 0.0;
        let mut frame = |events: Vec<egui::Event>| {
            time += 0.2;
            let input = egui::RawInput {
                screen_rect: Some(screen),
                time: Some(time),
                events,
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| draw(ui));
            output.textures_delta.clear();
        };
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(vec![egui::Event::PointerMoved(at)]);
        frame(Vec::new());
        frame(vec![button(true)]);
        frame(vec![button(false)]);
    }

    #[test]
    fn a_visible_group_reports_the_button_that_was_clicked_and_a_hidden_one_nothing() {
        use crate::model::chart::canvas::ChartCanvas;
        for (visibility, expected) in [
            (ButtonVisibility::Always, Some(NavAction::ZoomOut)),
            (ButtonVisibility::OnHover, Some(NavAction::ZoomOut)),
            (ButtonVisibility::Never, None),
        ] {
            let mut canvas = ChartCanvas::default();
            canvas.buttons.navigation = visibility;
            let style = CanvasStyle::resolve(&canvas);
            let (_, buttons) = navigation_rects(plot());
            let ctx = egui::Context::default();
            let mut clicked = None;
            click_at(&ctx, buttons[1].center(), |ui| {
                // The pointer is over the chart, as it is when it reaches a button.
                if let Some(action) = navigation(ui, Id::new("nav"), plot(), &style, true) {
                    clicked = Some(action);
                }
            });
            assert_eq!(clicked, expected, "{visibility:?}");
        }
    }

    #[test]
    fn a_group_that_is_hover_only_stays_hidden_while_the_pointer_is_away() {
        use crate::model::chart::canvas::ChartCanvas;
        let style = CanvasStyle::resolve(&ChartCanvas::default());
        let (_, buttons) = navigation_rects(plot());
        let ctx = egui::Context::default();
        let mut clicked = None;
        click_at(&ctx, buttons[0].center(), |ui| {
            if let Some(action) = navigation(ui, Id::new("nav"), plot(), &style, false) {
                clicked = Some(action);
            }
        });
        // Over a button the group counts itself as hovered, so the click still lands: the group
        // does not vanish from under the pointer.
        assert_eq!(clicked, Some(NavAction::ZoomIn));
    }
}

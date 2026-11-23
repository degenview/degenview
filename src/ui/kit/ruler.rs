//! A ruler: a labelled dial in the shape of a pill of tick marks that slides under a fixed centre
//! marker, with the value written beside it.
//!
//! Dragging works on the ticks, not on a handle: the ticks follow the pointer, so dragging LEFT
//! brings the higher values under the marker and raises the value (how Vela's dials feel). The
//! widget keeps no value and knows no unit: it is given the value and the range, and says what
//! the user did ([`RulerEvent`]); what a step is, what a snap is and what the value means belong
//! to the caller.
//!
//! Keyboard, when focused: Right and Up raise the value one step, Left and Down lower it, Page
//! Up and Page Down do the same by a larger step, Home or 0 and a double-click put it back at its
//! home value. The focus ring is the neutral label colour, never the accent.

use crate::ui::accessibility;
use crate::ui::kit::{REGULAR, digits, galley, size};
use crate::ui::theme;
use eframe::egui::{
    self, CursorIcon, Id, Key, Modifiers, Rect, Response, Sense, Stroke, StrokeKind, Ui,
    WidgetInfo, pos2, vec2,
};

/// The pill's size.
const TRACK: egui::Vec2 = vec2(128.0, 22.0);
/// How far apart the values of two ticks are, and how many pixels a unit of value is wide.
const MINOR: f64 = 0.05;
const PIXELS_PER_UNIT: f64 = 120.0;
/// Every this many ticks one is taller.
const MAJOR_EVERY: i64 = 5;
/// The room before the track for the label (at least), and the room after it for the value.
const LABEL_WIDTH: f32 = 42.0;
const READOUT_WIDTH: f32 = 46.0;
const GAP: f32 = 8.0;

/// What a ruler is told each frame.
pub struct Ruler<'a> {
    pub id: Id,
    /// The word before the track.
    pub label: &'a str,
    /// What a screen reader calls the dial.
    pub name: &'a str,
    pub value: f64,
    pub range: (f64, f64),
    /// The value Home, 0 and a double-click go back to; its tick is a little taller.
    pub home: f64,
    /// The value as written after the track.
    pub readout: &'a str,
    pub enabled: bool,
    /// Why a disabled ruler is, shown as its tooltip.
    pub hint: &'a str,
}

/// What the user did to a ruler this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RulerEvent {
    /// The ticks are being dragged: the value they ask for, inside the range.
    Dragged(f64),
    /// A drag ended.
    Released,
    /// A key asked for the next lower or higher step; `coarse` for Page Up and Page Down.
    Nudged { up: bool, coarse: bool },
    /// Home, 0 or a double-click.
    Reset,
}

/// What a ruler reports.
pub struct RulerOutput {
    pub events: Vec<RulerEvent>,
    /// The ticks are being dragged right now.
    pub dragging: bool,
    /// The track itself, for the caller's tooltips and tests.
    pub response: Response,
}

/// The value of a ruler dragged by `dx` points from `start`: left (negative) raises it.
pub fn dragged_value(start: f64, dx: f32, (low, high): (f64, f64)) -> f64 {
    let moved = if dx.is_finite() {
        -f64::from(dx) / PIXELS_PER_UNIT
    } else {
        0.0
    };
    (start + moved).clamp(low, high)
}

/// What a tick is, which says how tall it is.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Kind {
    Minor,
    Major,
    Home,
}

/// The ticks that are on a ruler showing `value` and are inside `range`, as their distance in
/// points from the centre marker (right is a higher value) and what they are. Evenly spaced.
fn ticks(value: f64, (low, high): (f64, f64), home: f64, half_width: f32) -> Vec<(f32, Kind)> {
    let reach = f64::from(half_width) / PIXELS_PER_UNIT;
    let first = ((value - reach).max(low) / MINOR - 1e-9).ceil() as i64;
    let last = ((value + reach).min(high) / MINOR + 1e-9).floor() as i64;
    (first..=last)
        .map(|index| {
            let at = index as f64 * MINOR;
            let kind = if (at - home).abs() < MINOR / 2.0 {
                Kind::Home
            } else if index % MAJOR_EVERY == 0 {
                Kind::Major
            } else {
                Kind::Minor
            };
            (((at - value) * PIXELS_PER_UNIT) as f32, kind)
        })
        .collect()
}

/// The keys a focused ruler reads, taken off the frame's input.
fn keyed(ui: &Ui) -> Vec<RulerEvent> {
    let mut events = Vec::new();
    ui.input_mut(|input| {
        let none = Modifiers::NONE;
        for (keys, event) in [
            (
                [Key::ArrowRight, Key::ArrowUp],
                RulerEvent::Nudged {
                    up: true,
                    coarse: false,
                },
            ),
            (
                [Key::ArrowLeft, Key::ArrowDown],
                RulerEvent::Nudged {
                    up: false,
                    coarse: false,
                },
            ),
        ] {
            for key in keys {
                if input.consume_key(none, key) {
                    events.push(event);
                }
            }
        }
        if input.consume_key(none, Key::PageUp) {
            events.push(RulerEvent::Nudged {
                up: true,
                coarse: true,
            });
        }
        if input.consume_key(none, Key::PageDown) {
            events.push(RulerEvent::Nudged {
                up: false,
                coarse: true,
            });
        }
        if input.consume_key(none, Key::Home) || input.consume_key(none, Key::Num0) {
            events.push(RulerEvent::Reset);
        }
    });
    events
}

/// Draws the ruler row (label, track, value) and reports what was done to it.
pub fn ruler(ui: &mut Ui, spec: &Ruler<'_>) -> RulerOutput {
    let label = galley(
        ui,
        spec.label,
        size::SUBHEADLINE + 0.5,
        REGULAR,
        theme::text_secondary(),
        f32::INFINITY,
    );
    // The label's own width, so a longer word in another language pushes the track along.
    let label_width = label.size().x.ceil().max(LABEL_WIDTH);
    let total = vec2(label_width + GAP + TRACK.x + GAP + READOUT_WIDTH, TRACK.y);
    let (row, _) = ui.allocate_exact_size(total, Sense::hover());
    let track = Rect::from_min_size(pos2(row.left() + label_width + GAP, row.top()), TRACK);
    let sense = if spec.enabled {
        Sense::click_and_drag()
    } else {
        Sense::hover()
    };
    let response = ui.interact(track, spec.id, sense);
    let mut events = Vec::new();
    let mut shown = spec.value;
    let mut dragging = false;
    if spec.enabled {
        // A press is what puts the keyboard on the ruler: egui only drops focus elsewhere.
        if response.is_pointer_button_down_on() {
            response.request_focus();
        }
        if response.drag_started() {
            ui.data_mut(|data| data.insert_temp(spec.id, spec.value));
        }
        if response.dragged() {
            dragging = true;
            let start = ui
                .data(|data| data.get_temp::<f64>(spec.id))
                .unwrap_or(spec.value);
            let delta = response.total_drag_delta().map_or(0.0, |total| total.x);
            shown = dragged_value(start, delta, spec.range);
            events.push(RulerEvent::Dragged(shown));
        }
        if response.drag_stopped() {
            events.push(RulerEvent::Released);
        }
        if response.double_clicked() {
            events.push(RulerEvent::Reset);
        }
        if response.has_focus() {
            ui.memory_mut(|memory| {
                memory.set_focus_lock_filter(
                    spec.id,
                    egui::EventFilter {
                        tab: false,
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        escape: false,
                    },
                );
            });
            events.extend(keyed(ui));
        }
        if response.hovered() || dragging {
            ui.ctx().set_cursor_icon(if dragging {
                CursorIcon::Grabbing
            } else {
                CursorIcon::Grab
            });
        }
    }
    let response = accessibility::label_as(response, egui::WidgetType::Slider, spec.name);
    response.widget_info(|| WidgetInfo::slider(spec.enabled, shown, spec.name));
    let response = if spec.enabled {
        response
    } else {
        response.on_hover_text(spec.hint)
    };

    let fade = if spec.enabled { 1.0 } else { 0.4 };
    let painter = ui.painter();
    painter.galley(
        pos2(row.left(), row.center().y - label.size().y / 2.0),
        label,
        theme::text_secondary().gamma_multiply(fade),
    );
    paint_track(
        painter,
        track,
        spec,
        shown,
        fade,
        response.hovered() || dragging,
    );
    if spec.enabled && response.has_focus() {
        painter.rect_stroke(
            track.expand(2.0),
            TRACK.y / 2.0 + 2.0,
            Stroke::new(1.5, theme::text_secondary()),
            StrokeKind::Outside,
        );
    }
    let readout = digits(
        ui,
        spec.readout,
        size::BODY,
        REGULAR,
        theme::text_primary().gamma_multiply(fade),
        READOUT_WIDTH,
    );
    ui.painter().galley(
        pos2(track.right() + GAP, row.center().y - readout.size().y / 2.0),
        readout,
        theme::text_primary(),
    );
    RulerOutput {
        events,
        dragging,
        response,
    }
}

fn paint_track(
    painter: &egui::Painter,
    track: Rect,
    spec: &Ruler<'_>,
    shown: f64,
    fade: f32,
    lit: bool,
) {
    let radius = track.height() / 2.0;
    let fill = if lit { 0.14 } else { 0.1 };
    painter.rect_filled(track, radius, theme::ink((255.0 * fill * fade) as u8));
    painter.rect_stroke(
        track,
        radius,
        Stroke::new(1.0, theme::ink((26.0 * fade) as u8)),
        StrokeKind::Inside,
    );
    let centre = track.center();
    let inner = track.width() / 2.0 - radius * 0.7;
    let tick_painter = painter.with_clip_rect(track.shrink2(vec2(radius * 0.5, 0.0)));
    for (offset, kind) in ticks(shown, spec.range, spec.home, inner) {
        // Fades out toward the ends of the pill, so the ticks seem to come from under its edge.
        let closeness = (offset.abs() / inner).clamp(0.0, 1.0);
        let strength = (1.0 - closeness * closeness) * fade;
        let (height, alpha) = match kind {
            Kind::Minor => (5.0, 70.0),
            Kind::Major => (9.0, 120.0),
            Kind::Home => (13.0, 170.0),
        };
        tick_painter.vline(
            centre.x + offset,
            (centre.y - height / 2.0)..=(centre.y + height / 2.0),
            Stroke::new(1.0, theme::ink((alpha * strength) as u8)),
        );
    }
    painter.vline(
        centre.x,
        (centre.y - 8.0)..=(centre.y + 8.0),
        Stroke::new(2.0, theme::text_primary().gamma_multiply(fade)),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::render_tests::Screen;
    use eframe::egui::{Event, PointerButton, Pos2};

    const RANGE: (f64, f64) = (0.5, 2.0);

    /// A ruler on a screen of its own, driven one frame at a time.
    struct Rig {
        screen: Screen,
        value: f64,
        enabled: bool,
        events: Vec<RulerEvent>,
        track: Rect,
    }

    impl Rig {
        fn new(value: f64) -> Self {
            let mut rig = Self {
                screen: Screen::new(vec2(320.0, 80.0)),
                value,
                enabled: true,
                events: Vec::new(),
                track: Rect::NOTHING,
            };
            rig.frame(Vec::new());
            rig
        }

        fn frame(&mut self, input: Vec<Event>) {
            let (value, enabled) = (self.value, self.enabled);
            let mut got = None;
            self.screen.run(input, |ui| {
                ui.add_space(20.0);
                let readout = format!("{value}");
                got = Some(ruler(
                    ui,
                    &Ruler {
                        id: Id::new("ruler"),
                        label: "Width",
                        name: "Width",
                        value,
                        range: RANGE,
                        home: 1.0,
                        readout: &readout,
                        enabled,
                        hint: "Not here",
                    },
                ));
            });
            let output = got.expect("drawn");
            self.track = output.response.rect;
            self.events = output.events;
        }

        fn centre(&self) -> Pos2 {
            self.track.center()
        }

        fn button(&self, at: Pos2, pressed: bool) -> Event {
            Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            }
        }

        /// Presses at the centre and moves `dx` points in a few steps; leaves the button down.
        fn drag(&mut self, dx: f32) -> Vec<RulerEvent> {
            let start = self.centre();
            self.frame(vec![Event::PointerMoved(start)]);
            self.frame(vec![self.button(start, true)]);
            let mut last = Vec::new();
            for step in 1..=4 {
                let at = start + vec2(dx * step as f32 / 4.0, 0.0);
                self.frame(vec![Event::PointerMoved(at)]);
                if let Some(RulerEvent::Dragged(value)) = self.events.last().copied() {
                    self.value = value;
                }
                last = self.events.clone();
            }
            last
        }

        fn release(&mut self, at: Pos2) -> Vec<RulerEvent> {
            self.frame(vec![self.button(at, false)]);
            self.events.clone()
        }

        fn key(&mut self, key: Key) -> Vec<RulerEvent> {
            self.frame(vec![Event::Key {
                key,
                physical_key: Some(key),
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            }]);
            self.events.clone()
        }

        fn focus(&mut self) {
            let at = self.centre();
            self.frame(vec![Event::PointerMoved(at)]);
            self.frame(vec![self.button(at, true)]);
            self.frame(vec![self.button(at, false)]);
        }
    }

    #[test]
    fn ticks_are_evenly_spaced_and_stay_inside_the_range() {
        let ticks = ticks(1.0, RANGE, 1.0, 56.0);
        assert!(ticks.len() > 8);
        let gaps: Vec<f32> = ticks.windows(2).map(|pair| pair[1].0 - pair[0].0).collect();
        for gap in &gaps {
            assert!((gap - gaps[0]).abs() < 1e-3, "{gaps:?}");
        }
        assert!(
            ticks
                .iter()
                .any(|(offset, kind)| *offset == 0.0 && *kind == Kind::Home)
        );
        assert!(ticks.iter().all(|(offset, _)| offset.abs() <= 56.5));

        // At the bottom of the range nothing lies below it.
        let at_low = super::ticks(0.5, RANGE, 1.0, 56.0);
        assert!(
            at_low.iter().all(|(offset, _)| *offset >= -1e-3),
            "{at_low:?}"
        );
        assert_eq!(at_low.first().map(|tick| tick.0), Some(0.0));
        let at_high = super::ticks(2.0, RANGE, 1.0, 56.0);
        assert!(at_high.iter().all(|(offset, _)| *offset <= 1e-3));
    }

    #[test]
    fn every_fifth_tick_is_taller_and_one_is_home() {
        let ticks = ticks(1.0, RANGE, 1.0, 56.0);
        let kinds: Vec<Kind> = ticks.iter().map(|tick| tick.1).collect();
        assert_eq!(kinds.iter().filter(|kind| **kind == Kind::Home).count(), 1);
        assert!(kinds.contains(&Kind::Major) && kinds.contains(&Kind::Minor));
    }

    #[test]
    fn dragging_left_raises_the_value_and_right_lowers_it() {
        assert!(dragged_value(1.0, -12.0, RANGE) > 1.0);
        assert!(dragged_value(1.0, 12.0, RANGE) < 1.0);
        assert_eq!(dragged_value(1.0, -12.0, RANGE), 1.1);
        assert_eq!(dragged_value(1.0, -10_000.0, RANGE), 2.0);
        assert_eq!(dragged_value(1.0, 10_000.0, RANGE), 0.5);
        assert_eq!(dragged_value(1.0, f32::NAN, RANGE), 1.0);
    }

    #[test]
    fn dragging_the_ticks_left_asks_for_a_higher_value_and_a_release_is_reported() {
        let mut rig = Rig::new(1.0);
        let events = rig.drag(-24.0);
        let Some(RulerEvent::Dragged(value)) = events.last().copied() else {
            panic!("a drag: {events:?}");
        };
        assert!((value - 1.2).abs() < 0.02, "{value}");
        let end = rig.centre() + vec2(-24.0, 0.0);
        assert!(rig.release(end).contains(&RulerEvent::Released));
    }

    #[test]
    fn dragging_right_asks_for_a_lower_value() {
        let mut rig = Rig::new(1.0);
        let events = rig.drag(24.0);
        let Some(RulerEvent::Dragged(value)) = events.last().copied() else {
            panic!("a drag: {events:?}");
        };
        assert!(value < 1.0, "{value}");
    }

    #[test]
    fn a_drag_is_measured_from_where_it_began_and_stays_in_the_range() {
        let mut rig = Rig::new(1.9);
        let events = rig.drag(-400.0);
        assert_eq!(events.last().copied(), Some(RulerEvent::Dragged(2.0)));
    }

    #[test]
    fn keys_step_when_focused_and_are_ignored_when_not() {
        let mut rig = Rig::new(1.0);
        assert!(rig.key(Key::ArrowRight).is_empty(), "not focused yet");
        rig.focus();
        let up = RulerEvent::Nudged {
            up: true,
            coarse: false,
        };
        let down = RulerEvent::Nudged {
            up: false,
            coarse: false,
        };
        assert_eq!(rig.key(Key::ArrowRight), vec![up]);
        assert_eq!(rig.key(Key::ArrowUp), vec![up]);
        assert_eq!(rig.key(Key::ArrowLeft), vec![down]);
        assert_eq!(rig.key(Key::ArrowDown), vec![down]);
        assert_eq!(
            rig.key(Key::PageUp),
            vec![RulerEvent::Nudged {
                up: true,
                coarse: true
            }]
        );
        assert_eq!(
            rig.key(Key::PageDown),
            vec![RulerEvent::Nudged {
                up: false,
                coarse: true
            }]
        );
        assert_eq!(rig.key(Key::Home), vec![RulerEvent::Reset]);
        assert_eq!(rig.key(Key::Num0), vec![RulerEvent::Reset]);
    }

    #[test]
    fn a_double_click_resets() {
        let mut rig = Rig::new(1.4);
        let at = rig.centre();
        rig.frame(vec![Event::PointerMoved(at)]);
        for _ in 0..2 {
            rig.frame(vec![rig.button(at, true)]);
            rig.frame(vec![rig.button(at, false)]);
        }
        assert!(rig.events.contains(&RulerEvent::Reset), "{:?}", rig.events);
    }

    #[test]
    fn a_disabled_ruler_does_nothing_and_says_why() {
        let mut rig = Rig::new(1.0);
        rig.enabled = false;
        rig.frame(Vec::new());
        let events = rig.drag(-30.0);
        assert!(events.is_empty());
        rig.focus();
        assert!(rig.key(Key::ArrowRight).is_empty());
        assert!(rig.key(Key::Home).is_empty());
    }

    #[test]
    fn a_ruler_is_a_slider_to_a_screen_reader() {
        let mut screen = Screen::new(vec2(320.0, 80.0));
        screen.ctx.enable_accesskit();
        let mut update = None;
        for _ in 0..2 {
            let output = screen.run(Vec::new(), |ui| {
                ruler(
                    ui,
                    &Ruler {
                        id: Id::new("ruler"),
                        label: "Width",
                        name: "Width",
                        value: 1.28,
                        range: RANGE,
                        home: 1.0,
                        readout: "1.28×",
                        enabled: true,
                        hint: "",
                    },
                );
            });
            update = output.platform_output.accesskit_update.or(update);
        }
        let update = update.expect("an accessibility tree");
        let slider = update
            .nodes
            .iter()
            .map(|(_, node)| node)
            .find(|node| node.role() == egui::accesskit::Role::Slider)
            .expect("a slider");
        assert_eq!(slider.numeric_value(), Some(1.28));
        assert_eq!(slider.label(), Some("Width"));
    }
}

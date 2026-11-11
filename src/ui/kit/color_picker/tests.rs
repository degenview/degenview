use super::*;
use eframe::egui::{Event, Modifiers, PointerButton, RawInput, UiBuilder};

fn rect() -> Rect {
    Rect::from_min_size(pos2(10.0, 20.0), vec2(200.0, 100.0))
}

// MARK: Hex

#[test]
fn hex_parses_the_forms_people_type() {
    let color = |r, g, b| Some(Color32::from_rgb(r, g, b));
    assert_eq!(parse_hex("#FF8000", false), color(255, 128, 0));
    assert_eq!(parse_hex("ff8000", false), color(255, 128, 0));
    assert_eq!(parse_hex("  #0a0b0c ", true), color(10, 11, 12));
    assert_eq!(parse_hex("#f80", false), color(255, 136, 0));
    assert_eq!(
        parse_hex("#11223344", true),
        Some(Color32::from_rgba_unmultiplied(0x11, 0x22, 0x33, 0x44))
    );
}

#[test]
fn hex_rejects_what_is_not_a_colour() {
    for bad in [
        "",
        "#",
        "#12",
        "#1234",
        "#12345",
        "#1234567",
        "#GGGGGG",
        "red",
        "#12 34 56",
        "##123456",
    ] {
        assert_eq!(parse_hex(bad, true), None, "{bad:?}");
    }
    // Eight digits mean opacity: not offered, not accepted.
    assert_eq!(parse_hex("#11223344", false), None);
}

#[test]
fn hex_formats_and_round_trips() {
    assert_eq!(format_hex(Color32::from_rgb(255, 128, 0), true), "#FF8000");
    assert_eq!(format_hex(Color32::from_rgb(1, 2, 3), false), "#010203");
    let translucent = Color32::from_rgba_unmultiplied(255, 0, 0, 128);
    assert_eq!(format_hex(translucent, true), "#FF000080");
    assert_eq!(
        format_hex(translucent, false),
        "#FF0000",
        "opacity is not shown when not offered"
    );
    for color in [
        Color32::from_rgb(0, 0, 0),
        Color32::from_rgb(255, 255, 255),
        Color32::from_rgba_unmultiplied(12, 34, 56, 78),
    ] {
        assert_eq!(parse_hex(&format_hex(color, true), true), Some(color));
    }
}

// MARK: Geometry

#[test]
fn a_slider_value_is_the_clamped_share_along_the_rect() {
    let rect = rect();
    assert_eq!(slider_value(rect, 10.0), 0.0);
    assert_eq!(slider_value(rect, 110.0), 0.5);
    assert_eq!(slider_value(rect, 210.0), 1.0);
    assert_eq!(slider_value(rect, -500.0), 0.0);
    assert_eq!(slider_value(rect, 900.0), 1.0);
    let empty = Rect::from_min_size(pos2(5.0, 5.0), Vec2::ZERO);
    assert_eq!(slider_value(empty, 5.0), 0.0);
    assert_eq!(slider_value(rect, f32::NAN), 0.0);
}

#[test]
fn the_square_reads_saturation_across_and_brightness_up() {
    let rect = rect();
    assert_eq!(square_to_sv(rect, rect.left_bottom()), (0.0, 0.0));
    assert_eq!(square_to_sv(rect, rect.right_top()), (1.0, 1.0));
    assert_eq!(square_to_sv(rect, rect.center()), (0.5, 0.5));
    assert_eq!(square_to_sv(rect, pos2(-50.0, 500.0)), (0.0, 0.0));
    let (s, v) = square_to_sv(Rect::from_min_size(Pos2::ZERO, Vec2::ZERO), pos2(1.0, 1.0));
    assert!(s.is_finite() && v.is_finite());
}

#[test]
fn the_thumb_position_inverts_the_pointer_mapping() {
    let rect = rect();
    for (s, v) in [(0.0, 0.0), (1.0, 1.0), (0.25, 0.75), (0.9, 0.1)] {
        let (back_s, back_v) = square_to_sv(rect, sv_to_square(rect, s, v));
        assert!((back_s - s).abs() < 1e-5 && (back_v - v).abs() < 1e-5);
    }
    assert!(
        rect.contains(sv_to_square(rect, 7.0, -3.0))
            || rect.expand(0.1).contains(sv_to_square(rect, 7.0, -3.0))
    );
}

#[test]
fn arrow_nudges_are_small_big_with_shift_and_stay_in_range() {
    assert!((nudged(0.5, 1.0, false) - 0.51).abs() < 1e-6);
    assert!((nudged(0.5, -1.0, true) - 0.4).abs() < 1e-6);
    assert_eq!(nudged(0.995, 1.0, false), 1.0);
    assert_eq!(nudged(0.0, -1.0, true), 0.0);
}

#[test]
fn the_layout_stacks_without_overlap_and_inside_its_total() {
    for alpha in [false, true] {
        let origin = pos2(20.0, 30.0);
        let layout = Layout::new(origin, alpha);
        let mut parts = vec![layout.square, layout.hue];
        parts.extend(layout.opacity);
        parts.extend([layout.fields, layout.swatches, layout.recents]);
        let bounds = Rect::from_min_size(origin, layout.total);
        for (index, part) in parts.iter().enumerate() {
            assert!(
                bounds.expand(0.01).contains_rect(*part),
                "{part:?} outside {bounds:?}"
            );
            for other in &parts[index + 1..] {
                assert!(!part.intersects(*other), "{part:?} overlaps {other:?}");
            }
        }
        assert_eq!(layout.opacity.is_some(), alpha);
    }
    let with = Layout::new(Pos2::ZERO, true).total.y;
    let without = Layout::new(Pos2::ZERO, false).total.y;
    assert!((with - without - (SLIDER_HEIGHT + 8.0)).abs() < 1e-3);
}

#[test]
fn swatches_tile_the_width_in_ten_columns() {
    let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(WIDTH, 40.0));
    let first = Layout::swatch(area, 0);
    let last_of_row = Layout::swatch(area, COLUMNS - 1);
    let second_row = Layout::swatch(area, COLUMNS);
    assert_eq!(first.left(), 0.0);
    assert!((last_of_row.right() - WIDTH).abs() < 1e-3);
    assert!(second_row.top() > first.bottom());
    assert_eq!(second_row.left(), first.left());
    assert_eq!(first.size(), Vec2::splat(Layout::cell()));
}

// MARK: Palettes

#[test]
fn the_system_swatches_are_distinct_and_fill_two_rows() {
    assert_eq!(SYSTEM_SWATCHES.len(), 2 * COLUMNS);
    let mut sorted = SYSTEM_SWATCHES.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), SYSTEM_SWATCHES.len());
}

#[test]
fn recents_put_the_newest_first_without_repeats_up_to_a_limit() {
    let mut recents = Recents::default();
    for value in 0..(RECENT_LIMIT as u8 + 3) {
        recents.push([value, 0, 0, 255]);
    }
    assert_eq!(recents.0.len(), RECENT_LIMIT);
    assert_eq!(recents.0[0], [RECENT_LIMIT as u8 + 2, 0, 0, 255]);
    recents.push([5, 0, 0, 255]);
    assert_eq!(recents.0[0], [5, 0, 0, 255]);
    assert_eq!(
        recents.0.iter().filter(|c| **c == [5, 0, 0, 255]).count(),
        1
    );
    assert_eq!(recents.0.len(), RECENT_LIMIT);
    // A different opacity is a different colour.
    recents.push([5, 0, 0, 128]);
    assert_eq!(recents.0[0], [5, 0, 0, 128]);
    assert!(recents.0.contains(&[5, 0, 0, 255]));
}

#[test]
fn recents_survive_a_round_trip_through_json() {
    let mut recents = Recents::default();
    recents.push([1, 2, 3, 4]);
    let json = serde_json::to_string(&recents).expect("serialize");
    assert_eq!(
        serde_json::from_str::<Recents>(&json).expect("deserialize"),
        recents
    );
}

// MARK: The widget

const ORIGIN: Pos2 = pos2(20.0, 20.0);

struct Harness {
    ctx: egui::Context,
    color: Color32,
    alpha: Alpha,
    time: f64,
}

impl Harness {
    fn new(color: Color32, alpha: Alpha) -> Self {
        Self {
            ctx: egui::Context::default(),
            color,
            alpha,
            time: 0.0,
        }
    }

    fn layout(&self) -> Layout {
        Layout::new(ORIGIN, self.alpha != Alpha::Opaque)
    }

    fn frame(&mut self, events: Vec<Event>) -> bool {
        self.time += 0.1;
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(600.0, 900.0))),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let (mut color, alpha) = (self.color, self.alpha);
        let mut changed = false;
        let mut output = self.ctx.run_ui(input, |ui| {
            ui.scope_builder(
                UiBuilder::new().max_rect(Rect::from_min_size(ORIGIN, vec2(WIDTH, 700.0))),
                |ui| changed = show(ui, &mut color, alpha),
            );
        });
        output.textures_delta.clear();
        self.color = color;
        changed
    }

    fn click(&mut self, at: Pos2) -> bool {
        let button = |pressed| Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        self.frame(vec![Event::PointerMoved(at)]);
        let pressed = self.frame(vec![button(true)]);
        let released = self.frame(vec![button(false)]);
        pressed || released
    }

    fn settle(&mut self) {
        for _ in 0..3 {
            self.frame(Vec::new());
        }
    }
}

#[test]
fn the_picker_draws_with_and_without_opacity_and_with_every_kind_of_colour() {
    for alpha in [Alpha::Opaque, Alpha::OnlyBlend] {
        for color in [
            Color32::BLACK,
            Color32::WHITE,
            Color32::from_rgb(10, 200, 90),
            Color32::from_rgba_unmultiplied(200, 30, 30, 90),
            Color32::TRANSPARENT,
        ] {
            let mut harness = Harness::new(color, alpha);
            harness.settle();
            assert_eq!(
                harness.color,
                color,
                "drawing alone changes nothing (opaque: {})",
                alpha == Alpha::Opaque
            );
        }
    }
}

#[test]
fn clicking_the_square_picks_saturation_and_brightness() {
    let mut harness = Harness::new(Color32::from_rgb(255, 0, 0), Alpha::Opaque);
    harness.settle();
    let square = harness.layout().square;
    // The top-right corner is the fully saturated, fully bright hue: red stays red.
    harness.click(square.right_top() - vec2(0.001, -0.001));
    assert_eq!(harness.color, Color32::from_rgb(255, 0, 0));
    // The bottom is black whatever the hue.
    assert!(harness.click(square.center_bottom() - vec2(0.0, 0.001)));
    assert_eq!(harness.color, Color32::BLACK);
    // Halfway across the top is white mixed halfway with the hue.
    harness.click(square.center_top() + vec2(0.0, 0.001));
    let [r, g, b, a] = harness.color.to_srgba_unmultiplied();
    assert_eq!((r, a), (255, 255));
    assert!(
        (120..=135).contains(&g) && (120..=135).contains(&b),
        "{g} {b}"
    );
}

#[test]
fn the_hue_survives_a_trip_through_black() {
    let mut harness = Harness::new(Color32::from_rgb(0, 0, 255), Alpha::Opaque);
    harness.settle();
    let square = harness.layout().square;
    harness.click(square.center_bottom() - vec2(0.0, 0.001));
    assert_eq!(harness.color, Color32::BLACK);
    harness.settle();
    // Back up to full brightness and saturation: the blue it came from, not a default red.
    harness.click(square.right_top() - vec2(0.001, -0.001));
    assert_eq!(harness.color, Color32::from_rgb(0, 0, 255));
}

#[test]
fn the_hue_slider_changes_only_the_hue() {
    let mut harness = Harness::new(Color32::from_rgb(255, 0, 0), Alpha::Opaque);
    harness.settle();
    let hue = harness.layout().hue;
    harness.click(pos2(hue.left() + hue.width() / 3.0, hue.center().y));
    let [r, g, b, _] = harness.color.to_srgba_unmultiplied();
    assert!(r <= 3 && g >= 253 && b <= 3, "green: {r} {g} {b}");
}

#[test]
fn the_opacity_slider_sets_the_alpha_and_keeps_the_colour() {
    let mut harness = Harness::new(Color32::from_rgb(10, 200, 90), Alpha::OnlyBlend);
    harness.settle();
    let opacity = harness.layout().opacity.expect("an opacity slider");
    harness.click(pos2(opacity.center().x, opacity.center().y));
    let [r, g, b, a] = harness.color.to_srgba_unmultiplied();
    assert!((120..=135).contains(&a), "{a}");
    assert!(
        r.abs_diff(10) <= 2 && g.abs_diff(200) <= 2 && b.abs_diff(90) <= 2,
        "{r} {g} {b}"
    );
    harness.click(pos2(opacity.right() - 0.001, opacity.center().y));
    assert_eq!(harness.color.a(), 255);
}

#[test]
fn without_opacity_the_colour_is_made_opaque_the_moment_it_is_edited() {
    let mut harness = Harness::new(
        Color32::from_rgba_unmultiplied(40, 80, 120, 100),
        Alpha::Opaque,
    );
    assert!(harness.layout().opacity.is_none());
    harness.settle();
    assert_eq!(harness.color.a(), 100, "looking is not editing");
    let square = harness.layout().square;
    harness.click(square.center());
    assert_eq!(harness.color.a(), 255);
}

#[test]
fn a_system_swatch_sets_its_colour_and_keeps_the_opacity() {
    let mut harness = Harness::new(
        Color32::from_rgba_unmultiplied(1, 2, 3, 128),
        Alpha::OnlyBlend,
    );
    harness.settle();
    let swatches = harness.layout().swatches;
    harness.click(Layout::swatch(swatches, 7).center());
    let [r, g, b, a] = harness.color.to_srgba_unmultiplied();
    assert_eq!([r, g, b], SYSTEM_SWATCHES[7]);
    assert!((126..=130).contains(&a), "{a}");
    let mut opaque = Harness::new(Color32::from_rgb(1, 2, 3), Alpha::Opaque);
    opaque.settle();
    opaque.click(Layout::swatch(opaque.layout().swatches, 0).center());
    let [r, g, b] = SYSTEM_SWATCHES[0];
    assert_eq!(opaque.color, Color32::from_rgb(r, g, b));
}

#[test]
fn a_picked_colour_shows_up_in_the_recents_and_can_be_picked_again() {
    let mut harness = Harness::new(Color32::from_rgb(1, 2, 3), Alpha::Opaque);
    harness.settle();
    let swatches = harness.layout().swatches;
    harness.click(Layout::swatch(swatches, 3).center());
    let [r, g, b] = SYSTEM_SWATCHES[3];
    let picked = Color32::from_rgb(r, g, b);
    harness.settle();
    let recents: Recents = harness
        .ctx
        .data_mut(|data| data.get_persisted(recents_id()))
        .expect("remembered");
    assert_eq!(recents.0[0], picked.to_srgba_unmultiplied());
    // Another colour, then the first from the Recent row.
    harness.click(Layout::swatch(swatches, 9).center());
    harness.settle();
    let recent_row = harness.layout().recents;
    harness.click(Layout::swatch(recent_row, 1).center());
    assert_eq!(harness.color, picked);
}

#[test]
fn typing_a_hex_colour_and_pressing_return_sets_it() {
    let mut harness = Harness::new(Color32::from_rgb(1, 2, 3), Alpha::Opaque);
    harness.settle();
    let fields = harness.layout().fields;
    let field = pos2(fields.left() + PREVIEW + 8.0 + 30.0, fields.center().y);
    harness.click(field);
    harness.frame(vec![Event::Key {
        key: Key::A,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::COMMAND,
    }]);
    harness.frame(vec![Event::Text("#00ff7f".to_owned())]);
    harness.frame(vec![Event::Key {
        key: Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    }]);
    harness.settle();
    assert_eq!(harness.color, Color32::from_rgb(0, 255, 127));
}

#[test]
fn a_bad_hex_leaves_the_colour_alone() {
    let mut harness = Harness::new(Color32::from_rgb(1, 2, 3), Alpha::Opaque);
    harness.settle();
    let fields = harness.layout().fields;
    harness.click(pos2(
        fields.left() + PREVIEW + 8.0 + 30.0,
        fields.center().y,
    ));
    harness.frame(vec![Event::Key {
        key: Key::A,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::COMMAND,
    }]);
    harness.frame(vec![Event::Text("#nonsense".to_owned())]);
    harness.frame(vec![Event::Key {
        key: Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    }]);
    harness.settle();
    assert_eq!(harness.color, Color32::from_rgb(1, 2, 3));
}

#[test]
fn a_translucent_swatch_paints_a_checkerboard_and_an_opaque_one_does_not() {
    let ctx = egui::Context::default();
    let count = |color: Color32| {
        let mut meshes = 0;
        let mut output = ctx.run_ui(RawInput::default(), |ui| {
            paint_swatch(
                ui.painter(),
                Rect::from_min_size(pos2(5.0, 5.0), vec2(40.0, 24.0)),
                color,
                6.0,
            );
        });
        output.textures_delta.clear();
        for clipped in &output.shapes {
            if matches!(clipped.shape, Shape::Mesh(_)) {
                meshes += 1;
            }
        }
        meshes
    };
    assert_eq!(count(Color32::from_rgb(1, 2, 3)), 0);
    assert!(count(Color32::from_rgba_unmultiplied(1, 2, 3, 90)) >= 2);
}

#[test]
fn hsv_matches_the_colours_designers_expect_and_round_trips() {
    let bytes = |h, s, v| Hsv::new(h, s, v, 1.0).to_srgba_unmultiplied();
    assert_eq!(bytes(0.0, 1.0, 1.0), [255, 0, 0, 255]);
    assert_eq!(bytes(1.0 / 3.0, 1.0, 1.0), [0, 255, 0, 255]);
    assert_eq!(bytes(2.0 / 3.0, 1.0, 1.0), [0, 0, 255, 255]);
    assert_eq!(bytes(0.0, 0.0, 1.0), [255, 255, 255, 255]);
    assert_eq!(bytes(0.4, 0.7, 0.0), [0, 0, 0, 255]);
    // Halfway to white is halfway on screen (sRGB), not in linear light.
    assert_eq!(bytes(0.0, 0.5, 1.0), [255, 128, 128, 255]);
    for rgba in [
        [12, 200, 77, 255],
        [255, 128, 0, 90],
        [3, 3, 3, 255],
        [250, 251, 252, 17],
    ] {
        assert_eq!(
            Hsv::from_srgba_unmultiplied(rgba).to_srgba_unmultiplied(),
            rgba
        );
    }
    // Hue wraps cleanly at both ends.
    assert_eq!(bytes(1.0, 1.0, 1.0), bytes(0.0, 1.0, 1.0));
}

// MARK: The Pine editor's picker (vertical strips)

mod inline_picker {
    use super::*;
    use crate::ui::kit::color_picker::inline::{
        self, Header, Layout as InlineLayout, Output, Style, opacity_of, strip_position, strip_y,
        transparency_percent,
    };

    #[test]
    fn strips_map_a_pointer_to_a_fraction_and_back() {
        let strip = Rect::from_min_size(pos2(0.0, 100.0), vec2(16.0, 160.0));
        assert_eq!(strip_position(strip, 100.0), 0.0);
        assert_eq!(strip_position(strip, 180.0), 0.5);
        assert_eq!(strip_position(strip, 260.0), 1.0);
        assert_eq!(strip_position(strip, -50.0), 0.0, "above clamps");
        assert_eq!(strip_position(strip, 900.0), 1.0, "below clamps");
        for fraction in [0.0, 0.25, 0.5, 1.0] {
            assert!((strip_position(strip, strip_y(strip, fraction)) - fraction).abs() < 1e-6);
        }
    }

    #[test]
    fn a_strip_with_no_height_or_a_bad_pointer_never_gives_nan() {
        let flat = Rect::from_min_size(pos2(0.0, 0.0), vec2(16.0, 0.0));
        assert_eq!(strip_position(flat, 5.0), 0.0);
        assert_eq!(strip_position(rect(), f32::NAN), 0.0);
        assert_eq!(strip_position(rect(), f32::INFINITY), 0.0);
    }

    #[test]
    fn pine_transparency_and_opacity_round_trip() {
        for percent in 0..=100u8 {
            assert_eq!(transparency_percent(opacity_of(percent)), percent);
        }
        assert_eq!(opacity_of(0), 1.0, "0 is opaque");
        assert_eq!(opacity_of(100), 0.0, "100 is invisible");
        assert_eq!(opacity_of(200), 0.0, "clamped");
        assert_eq!(transparency_percent(2.0), 0);
        assert_eq!(transparency_percent(-1.0), 100);
    }

    #[test]
    fn the_transparency_strip_sits_between_the_square_and_the_hue_strip() {
        let layout = InlineLayout::new(pos2(10.0, 20.0));
        assert!(layout.square.right() < layout.transparency.left());
        assert!(layout.transparency.right() < layout.hue.left());
        assert_eq!(layout.square.top(), layout.transparency.top());
        assert_eq!(layout.square.top(), layout.hue.top());
        assert!(layout.header.bottom() < layout.square.top());
        assert_eq!(layout.total.x, layout.hue.right() - 10.0);
        assert_eq!(layout.total.y, layout.hue.bottom() - 20.0);
        assert_eq!(layout.total.x, InlineLayout::width());
    }

    struct Inline {
        ctx: egui::Context,
        hsv: Hsv,
        time: f64,
        last: Output,
        header: String,
    }

    impl Inline {
        fn new(hsv: Hsv) -> Self {
            Self {
                ctx: egui::Context::default(),
                hsv,
                time: 0.0,
                last: Output::default(),
                header: String::new(),
            }
        }

        fn frame(&mut self, events: Vec<Event>) {
            self.time += 0.1;
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(600.0, 900.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let (mut hsv, mut last, mut header) = (self.hsv, self.last, String::new());
            let mut output = self.ctx.run_ui(input, |ui| {
                ui.scope_builder(
                    UiBuilder::new().max_rect(Rect::from_min_size(ORIGIN, vec2(300.0, 400.0))),
                    |ui| {
                        last = inline::show(
                            ui,
                            ui.id().with("inline"),
                            &mut hsv,
                            &Header {
                                font: egui::FontId::monospace(12.0),
                            },
                            Style {
                                background: Color32::from_gray(30),
                                focus: Color32::WHITE,
                                text: Color32::WHITE,
                            },
                            |_, _| "color.red".to_owned(),
                        );
                    },
                );
                header = "color.red".to_owned();
            });
            output.textures_delta.clear();
            (self.hsv, self.last, self.header) = (hsv, last, header);
        }

        fn layout(&self) -> InlineLayout {
            InlineLayout::new(ORIGIN)
        }

        fn press(&mut self, position: Pos2) {
            self.frame(vec![Event::PointerMoved(position)]);
            self.frame(vec![Event::PointerButton {
                pos: position,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            }]);
        }

        fn release(&mut self, position: Pos2) {
            self.frame(vec![Event::PointerButton {
                pos: position,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            }]);
        }
    }

    fn red() -> Hsv {
        Hsv::new(0.0, 1.0, 1.0, 1.0)
    }

    #[test]
    fn the_hue_strip_runs_down_from_red_and_leaves_saturation_and_brightness() {
        let mut picker = Inline::new(Hsv::new(0.0, 0.6, 0.7, 1.0));
        let hue = picker.layout().hue;
        let at = pos2(hue.center().x, hue.top() + hue.height() / 3.0);
        picker.press(at);
        assert!(picker.last.color_changed);
        assert!(!picker.last.transparency_changed);
        assert!((picker.hsv.h - 1.0 / 3.0).abs() < 0.01, "{}", picker.hsv.h);
        assert_eq!((picker.hsv.s, picker.hsv.v), (0.6, 0.7));
        picker.release(at);
        assert!(picker.last.finished);
    }

    #[test]
    fn the_transparency_strip_is_opaque_at_the_top() {
        let mut picker = Inline::new(red());
        let strip = picker.layout().transparency;
        picker.press(pos2(strip.center().x, strip.top()));
        assert_eq!(picker.hsv.a, 1.0);
        let bottom = pos2(strip.center().x, strip.bottom());
        picker.frame(vec![Event::PointerMoved(bottom)]);
        assert_eq!(picker.hsv.a, 0.0, "invisible at the bottom");
        assert!(picker.last.transparency_changed);
        assert!(!picker.last.color_changed);
        // A percent at a time: the middle is 50.
        picker.frame(vec![Event::PointerMoved(strip.center())]);
        assert_eq!(transparency_percent(picker.hsv.a), 50);
        assert_eq!(picker.hsv.a, 0.5);
    }

    #[test]
    fn a_drag_that_leaves_the_square_keeps_its_hold_and_clamps() {
        let mut picker = Inline::new(Hsv::new(0.5, 0.5, 0.5, 1.0));
        let square = picker.layout().square;
        picker.press(square.center());
        picker.frame(vec![Event::PointerMoved(pos2(
            square.right() + 300.0,
            square.top() - 300.0,
        ))]);
        assert_eq!((picker.hsv.s, picker.hsv.v), (1.0, 1.0));
        picker.frame(vec![Event::PointerMoved(pos2(
            square.left() - 300.0,
            square.bottom() + 300.0,
        ))]);
        assert_eq!((picker.hsv.s, picker.hsv.v), (0.0, 0.0));
        assert_eq!(picker.hsv.h, 0.5, "the hue is not disturbed");
        assert_eq!(picker.hsv.a, 1.0, "nor the opacity");
    }

    #[test]
    fn nothing_changes_without_input() {
        let mut picker = Inline::new(Hsv::new(0.3, 0.4, 0.5, 0.6));
        picker.frame(vec![]);
        picker.frame(vec![]);
        assert_eq!(picker.last, Output::default());
        assert_eq!(picker.hsv, Hsv::new(0.3, 0.4, 0.5, 0.6));
    }
}

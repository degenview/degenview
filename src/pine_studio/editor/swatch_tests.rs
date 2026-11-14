//! Colour swatches and their picker, driven through `show` with synthetic pointer and key events.

use super::*;
use crate::ui::kit::color_picker::inline;
use eframe::egui::Color32;
use eframe::egui::PointerButton;

const SCRIPT: &str = "//@version=6\nindicator(\"Color Picker Test\", overlay=true)\n\nbaseColor = input.color(color.red, \"Base Color\")\n\nratioColor1 = input.color(\n    color.new(color.rgb(128, 0, 192), 40),\n    \"Ratio 1 Color\"\n)\n\nratioColor2 = input.color(\n    color.new(#8000C0, 40),\n    \"Ratio 2 Color\"\n)\n\nplot(close, color=ratioColor1)\n";

struct Harness {
    ctx: egui::Context,
    state: EditorState,
    time: f64,
    screen: Rect,
}

fn cmd() -> Modifiers {
    let mac = shortcut::is_mac();
    Modifiers {
        command: true,
        mac_cmd: mac,
        ctrl: !mac,
        ..Modifiers::default()
    }
}

fn key(key: Key, modifiers: Modifiers) -> Event {
    Event::Key {
        key,
        physical_key: Some(key),
        pressed: true,
        repeat: false,
        modifiers,
    }
}

impl Harness {
    fn new(source: &str) -> Self {
        let mut harness = Self {
            ctx: egui::Context::default(),
            state: EditorState::new("swatches", source, source),
            time: 0.0,
            screen: Rect::from_min_size(egui::pos2(0.0, 0.0), Vec2::new(900.0, 700.0)),
        };
        harness
            .ctx
            .memory_mut(|m| m.request_focus(harness.state.id));
        harness.frame(vec![]);
        harness.frame(vec![]);
        harness
    }

    fn frame(&mut self, events: Vec<Event>) {
        self.time += 0.05;
        let raw = egui::RawInput {
            time: Some(self.time),
            screen_rect: Some(self.screen),
            focused: true,
            events,
            ..Default::default()
        };
        let state = &mut self.state;
        let mut output = self.ctx.run_ui(raw, |ui| {
            show(ui, state);
        });
        output.textures_delta.clear();
    }

    /// The character index where `needle`'s `n`th occurrence starts.
    fn index_of(&self, needle: &str, n: usize) -> usize {
        let byte = self
            .state
            .source
            .match_indices(needle)
            .nth(n)
            .expect("in the script")
            .0;
        self.state.source[..byte].chars().count()
    }

    fn swatch(&self, start: usize) -> Rect {
        swatches::swatch_rect(&self.state, start).expect("a swatch is drawn there")
    }

    fn press(&mut self, pos: egui::Pos2) {
        self.frame(vec![Event::PointerMoved(pos)]);
        self.frame(vec![Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        }]);
    }

    fn release(&mut self, pos: egui::Pos2) {
        self.frame(vec![Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }]);
        self.frame(vec![]);
    }

    fn click(&mut self, pos: egui::Pos2) {
        self.press(pos);
        self.release(pos);
    }

    fn drag_to(&mut self, pos: egui::Pos2) {
        self.frame(vec![Event::PointerMoved(pos)]);
    }

    fn caret(&self) -> (usize, usize) {
        let range = TextEditState::load(&self.ctx, self.state.id)
            .and_then(|s| s.cursor.char_range())
            .expect("a cursor");
        (range.secondary.index.0, range.primary.index.0)
    }

    fn place(&mut self, index: usize) {
        let mut edit_state = TextEditState::load(&self.ctx, self.state.id).unwrap_or_default();
        edit_state
            .cursor
            .set_char_range(Some(CCursorRange::one(CCursor::new(index))));
        edit_state.store(&self.ctx, self.state.id);
        self.frame(vec![]);
    }

    fn open(&mut self, start: usize) {
        let rect = self.swatch(start);
        self.click(rect.center());
        assert!(self.state.swatches.open.is_some(), "the picker opened");
        self.frame(vec![]);
    }

    /// The picker's controls, where the area put them.
    fn layout(&self, start: usize) -> inline::Layout {
        let anchor = self.swatch(start);
        let position = swatches::picker_position_for_tests(anchor, self.screen);
        inline::Layout::new(position + Vec2::splat(swatches::picker_margin_for_tests()))
    }

    fn source(&self) -> &str {
        &self.state.source
    }
}

#[test]
fn every_constant_colour_of_the_script_has_a_swatch() {
    let h = Harness::new(SCRIPT);
    for needle in ["color.red", "color.new(color.rgb", "color.new(#"] {
        let start = h.index_of(needle, 0);
        assert!(swatches::swatch_rect(&h.state, start).is_some(), "{needle}");
    }
    // Nested colours get one swatch, on the outermost expression.
    let inner = h.index_of("color.rgb", 0);
    assert!(swatches::swatch_rect(&h.state, inner).is_none());
}

#[test]
fn a_swatch_sits_in_room_opened_before_its_colour() {
    let mut h = Harness::new("c = color.red\n");
    let start = 4;
    h.place(start - 1);
    let before = h.state.caret_rect.expect("a caret").min.x;
    h.place(start);
    let after = h.state.caret_rect.expect("a caret").min.x;
    let swatch = h.swatch(start);
    let gap = h.state.swatches.gap;
    // The colour starts a character, then the swatch's room, after the space before it.
    assert!(after - before >= gap, "{before} {after} {gap}");
    assert!(swatch.left() >= before, "the swatch starts after the space");
    assert!(swatch.right() <= after, "and ends before the colour");
    assert!((swatch.height() - swatch.width()).abs() < 0.01, "a square");
    // The text is not changed by it.
    assert_eq!(h.source(), "c = color.red\n");
}

#[test]
fn text_without_colours_is_laid_out_as_before() {
    let mut h = Harness::new("a = 1\nb = 2\n");
    assert!(h.state.swatches.rects_for_tests().is_empty());
    h.place(1);
    let one = h.state.caret_rect.unwrap().min.x;
    h.place(2);
    let two = h.state.caret_rect.unwrap().min.x;
    let width = two - one;
    h.place(3);
    assert!((h.state.caret_rect.unwrap().min.x - two - width).abs() < 0.01);
}

#[test]
fn clicking_a_swatch_opens_the_picker_and_leaves_the_caret() {
    let mut h = Harness::new(SCRIPT);
    h.place(5);
    let start = h.index_of("color.red", 0);
    let rect = h.swatch(start);
    h.click(rect.center());
    assert!(h.state.swatches.open.is_some());
    assert_eq!(h.caret(), (5, 5), "the click never reached the text");
}

#[test]
fn the_picker_closes_on_escape_a_press_elsewhere_and_a_second_click() {
    let mut h = Harness::new(SCRIPT);
    let start = h.index_of("color.red", 0);
    h.open(start);
    h.frame(vec![key(Key::Escape, Modifiers::NONE)]);
    assert!(h.state.swatches.open.is_none(), "Escape");

    h.open(start);
    let elsewhere = egui::pos2(600.0, 500.0);
    h.click(elsewhere);
    assert!(h.state.swatches.open.is_none(), "a press outside");

    h.open(start);
    let rect = h.swatch(start);
    h.click(rect.center());
    assert!(h.state.swatches.open.is_none(), "the swatch again");
}

#[test]
fn another_swatch_takes_over_the_picker() {
    let mut h = Harness::new(SCRIPT);
    let first = h.index_of("color.red", 0);
    let second = h.index_of("color.new(#", 0);
    h.open(first);
    let rect = h.swatch(second);
    h.click(rect.center());
    let open = h.state.swatches.open.as_ref().expect("still open");
    assert_eq!(open.start(), second);
}

#[test]
fn text_changed_behind_the_picker_closes_it() {
    let mut h = Harness::new(SCRIPT);
    let start = h.index_of("color.red", 0);
    h.open(start);
    h.state.source.push_str("// typed\n");
    h.frame(vec![]);
    assert!(h.state.swatches.open.is_none());
}

#[test]
fn dragging_the_hue_strip_rewrites_only_that_colour() {
    let mut h = Harness::new(SCRIPT);
    let start = h.index_of("color.red", 0);
    h.open(start);
    let layout = h.layout(start);
    h.press(layout.hue.center_top() + Vec2::new(0.0, 4.0));
    h.drag_to(layout.hue.center());
    let during = h.source().to_owned();
    h.release(layout.hue.center());
    let after = h.source().to_owned();
    assert_ne!(during, SCRIPT);
    assert_eq!(during, after, "letting go changes nothing");
    assert!(
        after.contains("input.color(color.rgb("),
        "a named colour becomes an rgb constructor: {after}"
    );
    // Everything else is untouched.
    let rewritten = &after[after.find("color.rgb(").unwrap()..after.find(", \"Base").unwrap()];
    assert_eq!(after.replacen(rewritten, "color.red", 1), SCRIPT);
    // The swatch shows the new colour at once.
    let map = h.state.colors();
    assert_ne!(map.sites[0].rgba, 0xF236_45FF);
}

#[test]
fn dragging_the_transparency_strip_writes_a_pine_transparency() {
    let mut h = Harness::new(SCRIPT);
    let start = h.index_of("color.new(#", 0);
    h.open(start);
    let layout = h.layout(start);
    // Opaque at the top, so the middle of the strip is half transparent.
    h.press(layout.transparency.center());
    h.release(layout.transparency.center());
    assert!(
        h.source().contains("color.new(#8000C0, 50)"),
        "{}",
        h.source()
    );
}

#[test]
fn the_square_moves_the_colour_and_keeps_the_transparency() {
    let mut h = Harness::new(SCRIPT);
    let start = h.index_of("color.new(color.rgb", 0);
    h.open(start);
    let layout = h.layout(start);
    let to = layout.square.center() + Vec2::new(10.0, 10.0);
    h.press(layout.square.center());
    h.drag_to(to);
    h.release(to);
    let source = h.source();
    assert!(source.contains("color.new(color.rgb("), "{source}");
    assert!(
        source.contains("), 40),\n    \"Ratio 1 Color\""),
        "{source}"
    );
    assert!(!source.contains("color.rgb(128, 0, 192)"), "{source}");
}

#[test]
fn a_gesture_is_one_undo_step_however_long_it_takes() {
    let mut h = Harness::new(SCRIPT);
    let start = h.index_of("color.red", 0);
    h.open(start);
    let layout = h.layout(start);
    h.press(layout.hue.center_top() + Vec2::new(0.0, 4.0));
    for step in 1..=6 {
        h.drag_to(layout.hue.center_top() + Vec2::new(0.0, 4.0 + 10.0 * step as f32));
        // Resting for longer than egui's undo window must not cut a second step.
        h.time += 1.5;
    }
    h.release(layout.hue.center());
    let edited = h.source().to_owned();
    assert_ne!(edited, SCRIPT);

    h.frame(vec![key(Key::Escape, Modifiers::NONE)]);
    assert!(h.state.swatches.open.is_none());
    h.frame(vec![key(Key::Z, cmd())]);
    h.frame(vec![]);
    assert_eq!(h.source(), SCRIPT, "one undo restores the original");
    let mut redo = cmd();
    redo.shift = true;
    h.frame(vec![key(Key::Z, redo)]);
    h.frame(vec![]);
    assert_eq!(h.source(), edited, "redo puts the colour back");
}

#[test]
fn undo_while_the_picker_is_open_undoes_the_colour() {
    let mut h = Harness::new(SCRIPT);
    let start = h.index_of("color.red", 0);
    h.open(start);
    let layout = h.layout(start);
    h.click(layout.hue.center());
    assert_ne!(h.source(), SCRIPT);
    h.frame(vec![key(Key::Z, cmd())]);
    h.frame(vec![]);
    assert_eq!(h.source(), SCRIPT);
    assert!(h.state.swatches.open.is_none());
}

#[test]
fn escape_during_a_drag_gives_the_original_back() {
    let mut h = Harness::new(SCRIPT);
    let start = h.index_of("color.red", 0);
    h.open(start);
    let layout = h.layout(start);
    h.press(layout.hue.center());
    h.drag_to(layout.hue.center() + Vec2::new(0.0, 20.0));
    assert_ne!(h.source(), SCRIPT);
    h.frame(vec![key(Key::Escape, Modifiers::NONE)]);
    assert_eq!(h.source(), SCRIPT);
    assert!(h.state.swatches.open.is_none());
    h.release(layout.hue.center());
    assert_eq!(h.source(), SCRIPT, "the rest of the drag does nothing");
}

#[test]
fn identical_colours_are_edited_one_at_a_time() {
    let source = "a = color.red\nb = color.red\nc = color.red\n";
    let mut h = Harness::new(source);
    let second = h.index_of("color.red", 1);
    h.open(second);
    let layout = h.layout(second);
    h.click(layout.hue.center());
    let lines: Vec<&str> = h.source().lines().collect();
    assert_eq!(lines[0], "a = color.red");
    assert_eq!(lines[2], "c = color.red");
    assert!(lines[1].starts_with("b = color.rgb("), "{}", lines[1]);
}

#[test]
fn the_caret_after_the_colour_moves_with_the_text() {
    let source = "a = color.red // note\n";
    let mut h = Harness::new(source);
    let caret = source.chars().count() - 1;
    h.place(caret);
    let start = h.index_of("color.red", 0);
    h.open(start);
    let layout = h.layout(start);
    h.click(layout.hue.center());
    let length = h.source().chars().count();
    assert_eq!(h.caret(), (length - 1, length - 1));
}

#[test]
fn unicode_before_the_colour_is_kept() {
    let source = "// héllo 😀\nc = color.red\n";
    let mut h = Harness::new(source);
    let start = h.index_of("color.red", 0);
    h.open(start);
    let layout = h.layout(start);
    h.click(layout.hue.center());
    assert!(h.source().starts_with("// héllo 😀\nc = color.rgb("));
}

#[test]
fn the_picker_stays_inside_the_window() {
    let screen = Rect::from_min_size(egui::pos2(0.0, 0.0), Vec2::new(900.0, 700.0));
    let size = swatches::picker_size_for_tests();
    let corners = [
        Rect::from_min_size(egui::pos2(2.0, 2.0), Vec2::splat(12.0)),
        Rect::from_min_size(egui::pos2(880.0, 20.0), Vec2::splat(12.0)),
        Rect::from_min_size(egui::pos2(300.0, 690.0), Vec2::splat(12.0)),
        Rect::from_min_size(egui::pos2(890.0, 690.0), Vec2::splat(12.0)),
    ];
    for anchor in corners {
        let at = swatches::picker_position_for_tests(anchor, screen);
        let rect = Rect::from_min_size(at, size);
        assert!(screen.contains_rect(rect), "{anchor:?} -> {rect:?}");
    }
    // Room below: it hangs under the swatch; no room: over it.
    let low = Rect::from_min_size(egui::pos2(300.0, 20.0), Vec2::splat(12.0));
    assert!(swatches::picker_position_for_tests(low, screen).y > low.bottom());
    let high = Rect::from_min_size(egui::pos2(300.0, 650.0), Vec2::splat(12.0));
    assert!(swatches::picker_position_for_tests(high, screen).y + size.y < high.top());
}

#[test]
fn a_sample_editor_shows_swatches_but_never_a_picker() {
    let mut h = Harness::new("c = color.red\n");
    h.state = EditorState::new("sample", "c = color.red\n", "c = color.red\n").sampled();
    h.frame(vec![]);
    h.frame(vec![]);
    let rect = swatches::swatch_rect(&h.state, 4).expect("drawn");
    h.click(rect.center());
    assert!(h.state.swatches.open.is_none());
}

#[test]
fn replacing_the_script_drops_the_picker() {
    let mut h = Harness::new(SCRIPT);
    let start = h.index_of("color.red", 0);
    h.open(start);
    h.state.replace_source("x = 1\n", false);
    assert!(h.state.swatches.open.is_none());
}

#[test]
fn room_before_counts_only_the_room_a_pointer_has_passed() {
    let mut h = Harness::new("c = color.red\n");
    h.frame(vec![]);
    let gap = h.state.swatches.gap;
    let map = h.state.colors();
    let ctx = h.ctx.clone();
    let mut room = Vec::new();
    let screen = h.screen;
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(screen),
            ..Default::default()
        },
        |ui| {
            let font = FontId::monospace(13.0);
            let galley = ui.fonts_mut(|fonts| {
                fonts.layout_job(egui::text::LayoutJob::simple_singleline(
                    "c = color.red".to_owned(),
                    font,
                    Color32::WHITE,
                ))
            });
            // This galley has no room opened, so the sums depend on the glyph positions only.
            let x_of_colour = galley.rows[0].glyphs[4].pos.x;
            for x in [
                0.0,
                x_of_colour - gap,
                x_of_colour - gap / 2.0,
                x_of_colour + 50.0,
            ] {
                room.push(swatches::room_before(&map, &galley, 0, x, gap));
            }
        },
    );
    output.textures_delta.clear();
    assert_eq!(room[0], 0.0);
    assert_eq!(room[1], 0.0);
    assert!((room[2] - gap / 2.0).abs() < 0.01, "half-way into the room");
    assert_eq!(room[3], gap, "past it: all of it");
}

/// Pictures of the swatches and the picker, for looking at by hand:
/// `DEGENVIEW_GALLERY_DIR=<dir> cargo test --lib gallery_of_the_swatches -- --ignored`.
#[test]
#[ignore = "writes pictures"]
fn gallery_of_the_swatches() {
    use crate::ui::render_tests::{Screen, gallery_dir};
    use crate::ui::theme::AppTheme;
    if gallery_dir().is_none() {
        return;
    }
    for (name, app_theme) in [("dark", AppTheme::Dark), ("light", AppTheme::Light)] {
        let mut screen = Screen::new(Vec2::new(760.0, 520.0));
        theme::apply(&screen.ctx, app_theme);
        let mut state = EditorState::new("gallery", SCRIPT, SCRIPT);
        let run = |screen: &mut Screen, state: &mut EditorState, events: Vec<Event>| {
            theme::sync(&screen.ctx);
            screen.run(events, |ui| {
                show(ui, state);
            })
        };
        for _ in 0..3 {
            run(&mut screen, &mut state, vec![]);
        }
        let output = run(&mut screen, &mut state, vec![]);
        screen.save(&format!("swatches-{name}"), output);

        let start = SCRIPT.find("color.new(color.rgb").unwrap();
        let start = SCRIPT[..start].chars().count();
        let rect = swatches::swatch_rect(&state, start).expect("drawn");
        let press = |pressed| Event::PointerButton {
            pos: rect.center(),
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        run(
            &mut screen,
            &mut state,
            vec![Event::PointerMoved(rect.center())],
        );
        run(&mut screen, &mut state, vec![press(true)]);
        run(&mut screen, &mut state, vec![press(false)]);
        run(&mut screen, &mut state, vec![]);
        let output = run(&mut screen, &mut state, vec![]);
        screen.save(&format!("picker-open-{name}"), output);

        // Drag the hue strip and the transparency strip, as a person would.
        let position = swatches::picker_position_for_tests(
            rect,
            Rect::from_min_size(egui::Pos2::ZERO, screen.size),
        );
        let layout =
            inline::Layout::new(position + Vec2::splat(swatches::picker_margin_for_tests()));
        let hue = layout.hue.center() + Vec2::new(0.0, 30.0);
        run(&mut screen, &mut state, vec![Event::PointerMoved(hue)]);
        run(
            &mut screen,
            &mut state,
            vec![Event::PointerButton {
                pos: hue,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            }],
        );
        let output = run(&mut screen, &mut state, vec![]);
        screen.save(&format!("picker-hue-{name}"), output);
        run(
            &mut screen,
            &mut state,
            vec![Event::PointerButton {
                pos: hue,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            }],
        );
    }
}

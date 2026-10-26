//! The editor driven the way a person drives it: synthetic keys and clicks through `show`.

use super::*;
use eframe::egui::PointerButton;

const SCRIPT: &str = "//@version=6\nindicator(\"T\")\nlen = 14\nplot(ta.sma(close, len))\n";

struct Harness {
    ctx: egui::Context,
    state: EditorState,
    time: f64,
    area: Rect,
    char_width: f32,
    row_height: f32,
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

fn alt() -> Modifiers {
    Modifiers {
        alt: true,
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
            state: EditorState::new("nav", source, source),
            time: 0.0,
            area: Rect::NOTHING,
            char_width: 8.0,
            row_height: 16.0,
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
            events,
            ..Default::default()
        };
        let (mut area, mut width, mut height) = (self.area, self.char_width, self.row_height);
        let state = &mut self.state;
        let mut output = self.ctx.run_ui(raw, |ui| {
            area = show(ui, state).text_area.expect("drawn");
            let font = FontId::monospace(FONT_SIZE);
            width = ui.fonts_mut(|f| f.glyph_width(&font, ' '));
            height = ui.fonts_mut(|f| f.row_height(&font));
        });
        output.textures_delta.clear();
        (self.area, self.char_width, self.row_height) = (area, width, height);
    }

    fn at(&self, line: usize, column: usize) -> egui::Pos2 {
        egui::pos2(
            self.area.min.x + (column as f32 + 0.25) * self.char_width,
            self.area.min.y + (line as f32 + 0.5) * self.row_height,
        )
    }

    fn index(&self, line: usize, column: usize) -> usize {
        self.state
            .source
            .split('\n')
            .take(line)
            .map(|l| l.chars().count() + 1)
            .sum::<usize>()
            + column
    }

    fn place(&mut self, anchor: usize, head: usize) {
        let mut edit_state = TextEditState::load(&self.ctx, self.state.id).unwrap_or_default();
        edit_state.cursor.set_char_range(Some(CCursorRange::two(
            CCursor::new(anchor),
            CCursor::new(head),
        )));
        edit_state.store(&self.ctx, self.state.id);
        self.frame(vec![]);
    }

    fn primary(&self) -> (usize, usize) {
        let range = TextEditState::load(&self.ctx, self.state.id)
            .and_then(|s| s.cursor.char_range())
            .expect("a cursor");
        (range.secondary.index.0, range.primary.index.0)
    }

    fn click(&mut self, pos: egui::Pos2, modifiers: Modifiers) {
        self.frame(vec![Event::PointerMoved(pos)]);
        self.frame(vec![Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed: true,
            modifiers,
        }]);
        self.frame(vec![Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed: false,
            modifiers,
        }]);
        self.frame(vec![]);
    }

    fn type_text(&mut self, text: &str) {
        self.frame(vec![Event::Text(text.to_owned())]);
    }
}

#[test]
fn cmd_click_on_a_use_selects_its_declaration() {
    let mut h = Harness::new(SCRIPT);
    h.place(0, 0);
    let target = h.at(3, 20);
    h.click(target, cmd());
    let declaration = h.index(2, 0);
    assert_eq!(h.primary(), (declaration, declaration + 3));
    assert!(h.state.extra_carets().is_empty());
}

#[test]
fn cmd_click_off_a_name_adds_a_caret_where_the_caret_was() {
    let mut h = Harness::new(SCRIPT);
    h.place(0, 0);
    let target = h.at(3, 3);
    h.click(target, cmd());
    assert_eq!(h.state.extra_carets(), vec![(0, 0)]);
    let at = h.index(3, 3);
    assert_eq!(h.primary(), (at, at));
}

#[test]
fn alt_click_always_adds_a_caret_even_on_a_name() {
    let mut h = Harness::new(SCRIPT);
    h.place(0, 0);
    let target = h.at(3, 20);
    h.click(target, alt());
    assert_eq!(h.state.extra_carets(), vec![(0, 0)]);
    let at = h.index(3, 20);
    assert_eq!(h.primary().1, at);
}

#[test]
fn a_plain_click_drops_the_extra_carets() {
    let mut h = Harness::new(SCRIPT);
    h.place(0, 0);
    let target = h.at(3, 3);
    h.click(target, alt());
    assert_eq!(h.state.extra_carets().len(), 1);
    let elsewhere = h.at(2, 2);
    h.click(elsewhere, Modifiers::default());
    assert!(h.state.extra_carets().is_empty());
}

#[test]
fn typing_goes_to_every_caret_and_escape_leaves_one() {
    let mut h = Harness::new("a\nb\nc\n");
    h.place(1, 1);
    // Carets after a, b and c.
    h.state.set_extra_carets(&[3, 5]);
    h.type_text("x");
    assert_eq!(h.state.source, "ax\nbx\ncx\n");
    assert_eq!(h.state.extra_carets().len(), 2);
    h.frame(vec![key(Key::Backspace, Modifiers::default())]);
    assert_eq!(h.state.source, "a\nb\nc\n");
    h.frame(vec![key(Key::Escape, Modifiers::default())]);
    assert!(h.state.extra_carets().is_empty());
    h.type_text("y");
    assert_eq!(h.state.source, "ay\nb\nc\n", "one caret again");
}

#[test]
fn undo_drops_the_extra_carets_and_reverts_the_text() {
    let mut h = Harness::new("a\nb\n");
    h.place(1, 1);
    h.state.set_extra_carets(&[3]);
    h.type_text("x");
    assert_eq!(h.state.source, "ax\nbx\n");
    h.time += 2.0;
    h.frame(vec![]);
    h.frame(vec![key(Key::Z, cmd())]);
    assert!(h.state.extra_carets().is_empty());
    assert_eq!(
        h.state.source, "a\nb\n",
        "both carets' typing is one undo step"
    );
}

#[test]
fn any_other_edit_of_the_text_drops_the_extra_carets() {
    let mut h = Harness::new("a\nb\n");
    h.place(1, 1);
    h.state.set_extra_carets(&[3]);
    h.state.source.push_str("c\n");
    h.frame(vec![]);
    assert!(h.state.extra_carets().is_empty());
}

#[test]
fn cmd_d_selects_the_word_then_each_next_match() {
    let mut h = Harness::new("foo bar foo foo\n");
    h.place(1, 1);
    h.frame(vec![key(Key::D, cmd())]);
    assert_eq!(h.primary(), (0, 3));
    h.frame(vec![key(Key::D, cmd())]);
    assert_eq!(h.primary(), (8, 11));
    assert_eq!(h.state.extra_carets(), vec![(0, 3)]);
    h.frame(vec![key(Key::D, cmd())]);
    assert_eq!(h.state.extra_carets().len(), 2);
    h.type_text("q");
    assert_eq!(h.state.source, "q bar q q\n");
}

#[test]
fn alt_f3_selects_every_match_at_once() {
    let mut h = Harness::new("foo bar foo foo\n");
    h.place(1, 1);
    h.frame(vec![key(Key::F3, alt())]);
    assert_eq!(h.state.extra_carets().len(), 2);
    h.type_text("z");
    assert_eq!(h.state.source, "z bar z z\n");
}

#[test]
fn f12_jumps_to_the_declaration_of_the_name_at_the_caret() {
    let mut h = Harness::new(SCRIPT);
    let use_at = h.index(3, 20);
    h.place(use_at, use_at);
    h.frame(vec![key(Key::F12, Modifiers::default())]);
    h.frame(vec![]);
    let declaration = h.index(2, 0);
    assert_eq!(h.primary(), (declaration, declaration + 3));
}

#[test]
fn rename_applies_to_every_use_as_one_undo_step() {
    let mut h = Harness::new(SCRIPT);
    let use_at = h.index(3, 20);
    h.place(use_at, use_at);
    h.frame(vec![key(Key::F2, Modifiers::default())]);
    h.frame(vec![]);
    assert!(h.state.rename_open());
    h.state.rename_text("period");
    h.frame(vec![]);
    h.frame(vec![key(Key::Enter, Modifiers::default())]);
    h.frame(vec![]);
    assert!(!h.state.rename_open(), "{:?}", h.state.rename_error());
    assert_eq!(
        h.state.source,
        "//@version=6\nindicator(\"T\")\nperiod = 14\nplot(ta.sma(close, period))\n"
    );
    assert!(
        h.state.completion.is_none() && !h.state.completion_wanted,
        "a rename is not typing: no suggestions"
    );
    h.frame(vec![key(Key::Z, cmd())]);
    assert_eq!(h.state.source, SCRIPT, "one undo");
}

#[test]
fn rename_to_a_clashing_name_stays_open_and_changes_nothing() {
    let source = "//@version=6\nindicator(\"T\")\nlen = 14\nother = 2\nplot(len + other)\n";
    let mut h = Harness::new(source);
    let at = h.index(2, 1);
    h.place(at, at);
    h.frame(vec![key(Key::F2, Modifiers::default())]);
    h.frame(vec![]);
    h.state.rename_text("other");
    h.frame(vec![]);
    h.frame(vec![key(Key::Enter, Modifiers::default())]);
    h.frame(vec![]);
    assert!(h.state.rename_open(), "kept for another try");
    assert_eq!(h.state.source, source);
    h.frame(vec![key(Key::Escape, Modifiers::default())]);
    h.frame(vec![]);
    assert!(!h.state.rename_open());
    assert_eq!(h.state.source, source);
}

#[test]
fn rename_is_dropped_when_the_text_changes_under_it() {
    let mut h = Harness::new(SCRIPT);
    let at = h.index(2, 1);
    h.place(at, at);
    h.frame(vec![key(Key::F2, Modifiers::default())]);
    h.frame(vec![]);
    assert!(h.state.rename_open());
    h.state.source.push_str("// more\n");
    h.frame(vec![]);
    assert!(!h.state.rename_open());
}

#[test]
fn a_middle_button_drag_selects_a_column() {
    let mut h = Harness::new("abcdef\nabcdef\nabcdef\n");
    h.place(0, 0);
    let (start, end) = (h.at(0, 1), h.at(2, 3));
    h.frame(vec![Event::PointerMoved(start)]);
    h.frame(vec![Event::PointerButton {
        pos: start,
        button: PointerButton::Middle,
        pressed: true,
        modifiers: Modifiers::default(),
    }]);
    h.frame(vec![Event::PointerMoved(end)]);
    h.frame(vec![]);
    let (a, b) = h.primary();
    assert_eq!(
        (a, b),
        (h.index(2, 1), h.index(2, 3)),
        "the last line is the primary"
    );
    assert_eq!(
        h.state.extra_carets(),
        vec![
            (h.index(0, 1), h.index(0, 3)),
            (h.index(1, 1), h.index(1, 3))
        ]
    );
    h.frame(vec![Event::PointerButton {
        pos: end,
        button: PointerButton::Middle,
        pressed: false,
        modifiers: Modifiers::default(),
    }]);
    h.type_text("-");
    assert_eq!(h.state.source, "a-def\na-def\na-def\n");
}

#[test]
fn ctrl_shift_arrows_add_carets_above_and_below() {
    let mut h = Harness::new("abc\nabc\nabc\n");
    let middle = h.index(1, 2);
    h.place(middle, middle);
    let (up, down) = if shortcut::is_mac() {
        let m = Modifiers {
            ctrl: true,
            shift: true,
            ..Modifiers::default()
        };
        (key(Key::ArrowUp, m), key(Key::ArrowDown, m))
    } else {
        let m = Modifiers {
            ctrl: true,
            command: true,
            alt: true,
            ..Modifiers::default()
        };
        (key(Key::ArrowUp, m), key(Key::ArrowDown, m))
    };
    h.frame(vec![up]);
    h.frame(vec![down]);
    assert_eq!(h.state.extra_carets().len(), 2);
    h.type_text("!");
    assert_eq!(h.state.source, "ab!c\nab!c\nab!c\n");
}

#[test]
fn a_right_click_on_a_name_opens_the_menu_for_that_name() {
    let mut h = Harness::new(SCRIPT);
    h.place(0, 0);
    let on_use = h.at(3, 20);
    h.frame(vec![Event::PointerMoved(on_use)]);
    for pressed in [true, false] {
        h.frame(vec![Event::PointerButton {
            pos: on_use,
            button: PointerButton::Secondary,
            pressed,
            modifiers: Modifiers::default(),
        }]);
    }
    h.frame(vec![]);
    let name = h.index(3, 19);
    assert!(
        h.state
            .menu_target()
            .is_some_and(|at| (name..=name + 3).contains(&at)),
        "{:?}",
        h.state.menu_target()
    );
    assert!(egui::Popup::is_any_open(&h.ctx), "the menu is up");
}

fn right_click(h: &mut Harness, pos: egui::Pos2) {
    h.frame(vec![Event::PointerMoved(pos)]);
    for pressed in [true, false] {
        h.frame(vec![Event::PointerButton {
            pos,
            button: PointerButton::Secondary,
            pressed,
            modifiers: Modifiers::default(),
        }]);
    }
    h.frame(vec![]);
}

#[test]
fn a_right_click_inside_the_selection_keeps_it_for_cut_and_copy() {
    let mut h = Harness::new(SCRIPT);
    let (start, end) = (h.index(3, 5), h.index(3, 22));
    h.place(start, end);
    let inside = h.at(3, 12);
    right_click(&mut h, inside);
    assert_eq!(h.primary(), (start, end));
    assert!(egui::Popup::is_any_open(&h.ctx));
}

#[test]
fn a_right_click_outside_the_selection_moves_the_caret_there() {
    let mut h = Harness::new(SCRIPT);
    let (start, end) = (h.index(3, 5), h.index(3, 12));
    h.place(start, end);
    let outside = h.at(2, 2);
    right_click(&mut h, outside);
    let (a, b) = h.primary();
    assert_eq!(a, b, "a caret, not the old selection");
    assert_eq!(a, h.index(2, 2));
}

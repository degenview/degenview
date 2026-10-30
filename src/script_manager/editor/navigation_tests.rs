//! The editor driven the way a person drives it: synthetic keys and clicks through `show`.

use super::*;
use eframe::egui::PointerButton;

const SCRIPT: &str = "//@version=6\nindicator(\"T\")\nlen = 14\nplot(ta.sma(close, len))\n";

struct Harness {
    ctx: egui::Context,
    state: EditorState,
    time: f64,
    focused: bool,
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
            focused: true,
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
            focused: self.focused,
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

#[test]
fn the_edit_menu_follows_the_caret_and_the_selection() {
    let mut h = Harness::new(SCRIPT);
    let on_use = h.index(3, 20);
    h.place(on_use, on_use);
    let menu = h.state.edit_menu();
    assert!(menu.go_to_definition && menu.rename && menu.paste);
    assert!(!menu.cut && !menu.copy, "nothing selected");

    h.place(h.index(3, 5), h.index(3, 12));
    let menu = h.state.edit_menu();
    assert!(menu.cut && menu.copy);

    let on_declaration = h.index(2, 1);
    h.place(on_declaration, on_declaration);
    let menu = h.state.edit_menu();
    assert!(!menu.go_to_definition, "already at the definition");
    assert!(menu.rename);

    let in_comment = h.index(0, 4);
    h.place(in_comment, in_comment);
    let menu = h.state.edit_menu();
    assert!(!menu.go_to_definition && !menu.rename);
}

#[test]
fn the_edit_menu_is_off_when_the_editor_has_not_the_keyboard() {
    let mut h = Harness::new(SCRIPT);
    h.ctx.memory_mut(|m| m.surrender_focus(h.state.id));
    h.frame(vec![]);
    h.frame(vec![]);
    assert_eq!(
        h.state.edit_menu(),
        crate::native_bridge::EditMenu::default()
    );
}

#[test]
fn edit_menu_commands_act_on_the_caret() {
    let mut h = Harness::new(SCRIPT);
    let on_use = h.index(3, 20);
    h.place(on_use, on_use);
    h.state.request_edit(EditAction::GoToDefinition);
    h.frame(vec![]);
    h.frame(vec![]);
    let declaration = h.index(2, 0);
    assert_eq!(h.primary(), (declaration, declaration + 3));

    h.place(on_use, on_use);
    h.state.request_edit(EditAction::Rename);
    h.frame(vec![]);
    assert!(h.state.rename_open());
}

// MARK: ⌘-click on a function's declaration

const CALLS: &str = "//@version=6\nindicator(\"T\")\nhalf(x) => x / 2\nquarter(x) => x\nunused(x) => x\n\
                     a = half(1)\nb = half(2)\nc = quarter(3)\nd = 1\n";

#[test]
fn cmd_click_on_a_declaration_with_several_calls_lists_the_lines() {
    let mut h = Harness::new(CALLS);
    h.place(0, 0);
    let target = h.at(2, 1);
    h.click(target, cmd());
    assert_eq!(
        h.state.call_list_rows(),
        ["   6  a = half(1)", "   7  b = half(2)"]
    );
    assert!(h.state.extra_carets().is_empty(), "no second caret either");
}

#[test]
fn cmd_click_on_a_declaration_with_one_call_goes_there() {
    let mut h = Harness::new(CALLS);
    h.place(0, 0);
    let target = h.at(3, 1);
    h.click(target, cmd());
    let call = h.index(7, 4);
    assert_eq!(h.primary(), (call, call + 7), "the call's name is selected");
    assert!(h.state.call_list_rows().is_empty());
    assert!(h.state.extra_carets().is_empty());
}

#[test]
fn cmd_click_on_a_declaration_nothing_calls_still_adds_a_caret() {
    let mut h = Harness::new(CALLS);
    h.place(0, 0);
    let target = h.at(4, 2);
    h.click(target, cmd());
    assert!(h.state.call_list_rows().is_empty());
    assert_eq!(h.state.extra_carets(), vec![(0, 0)]);
}

#[test]
fn a_plain_click_on_a_declaration_only_places_the_caret() {
    let mut h = Harness::new(CALLS);
    h.place(0, 0);
    let target = h.at(2, 1);
    h.click(target, Modifiers::NONE);
    assert!(h.state.call_list_rows().is_empty());
    let at = h.index(2, 1);
    assert_eq!(h.primary(), (at, at));
}

#[test]
fn cmd_click_on_a_call_still_goes_to_its_declaration() {
    let mut h = Harness::new(CALLS);
    h.place(0, 0);
    let target = h.at(5, 5);
    h.click(target, cmd());
    let declaration = h.index(2, 0);
    assert_eq!(h.primary(), (declaration, declaration + 4));
    assert!(h.state.call_list_rows().is_empty());
}

#[test]
fn the_list_closes_on_an_edit_and_on_escape() {
    let mut h = Harness::new(CALLS);
    h.place(0, 0);
    let target = h.at(2, 1);
    h.click(target, cmd());
    assert_eq!(h.state.call_list_rows().len(), 2);
    h.type_text("z");
    h.frame(vec![]);
    assert!(h.state.call_list_rows().is_empty(), "an edit drops it");

    let mut h = Harness::new(CALLS);
    h.place(0, 0);
    let target = h.at(2, 1);
    h.click(target, cmd());
    assert_eq!(h.state.call_list_rows().len(), 2);
    h.frame(vec![key(Key::Escape, Modifiers::NONE)]);
    h.frame(vec![]);
    assert!(h.state.call_list_rows().is_empty(), "Esc closes it");
}

fn open_list(h: &mut Harness) {
    h.place(0, 0);
    let target = h.at(2, 1);
    h.click(target, cmd());
    assert_eq!(h.state.call_list_rows().len(), 2, "the list is open");
    // One more frame so the list has been drawn and knows where it is.
    h.frame(vec![]);
}

#[test]
fn a_click_anywhere_else_closes_the_list_and_still_lands() {
    let mut h = Harness::new(CALLS);
    open_list(&mut h);
    let elsewhere = h.at(0, 5);
    h.click(elsewhere, Modifiers::NONE);
    assert!(h.state.call_list_rows().is_empty(), "closed by the press");
    let at = h.index(0, 5);
    assert_eq!(h.primary(), (at, at), "and the click moved the caret");
}

#[test]
fn a_press_with_any_button_outside_closes_it() {
    let mut h = Harness::new(CALLS);
    open_list(&mut h);
    let elsewhere = h.at(0, 3);
    h.frame(vec![Event::PointerMoved(elsewhere)]);
    h.frame(vec![Event::PointerButton {
        pos: elsewhere,
        button: PointerButton::Secondary,
        pressed: true,
        modifiers: Modifiers::NONE,
    }]);
    assert!(h.state.call_list_rows().is_empty());
}

#[test]
fn the_arrow_keys_move_and_enter_goes_to_the_selected_call() {
    let mut h = Harness::new(CALLS);
    open_list(&mut h);
    let before = h.state.source.clone();
    h.frame(vec![key(Key::ArrowDown, Modifiers::NONE)]);
    h.frame(vec![key(Key::Enter, Modifiers::NONE)]);
    // The second call of `half`, on the line below the first.
    let second = h.index(6, 4);
    assert_eq!(h.primary(), (second, second + 4));
    assert!(h.state.call_list_rows().is_empty());
    assert_eq!(
        h.state.source, before,
        "Enter chose a row, it typed nothing"
    );
}

#[test]
fn up_stays_on_the_first_row_and_end_and_home_jump() {
    let mut h = Harness::new(CALLS);
    open_list(&mut h);
    h.frame(vec![key(Key::ArrowUp, Modifiers::NONE)]);
    h.frame(vec![key(Key::End, Modifiers::NONE)]);
    h.frame(vec![key(Key::Enter, Modifiers::NONE)]);
    let last = h.index(6, 4);
    assert_eq!(h.primary(), (last, last + 4), "End selects the last call");

    let mut h = Harness::new(CALLS);
    open_list(&mut h);
    h.frame(vec![key(Key::End, Modifiers::NONE)]);
    h.frame(vec![key(Key::Home, Modifiers::NONE)]);
    h.frame(vec![key(Key::Enter, Modifiers::NONE)]);
    let first = h.index(5, 4);
    assert_eq!(h.primary(), (first, first + 4), "Home selects the first");
}

#[test]
fn losing_the_windows_focus_closes_the_list() {
    let mut h = Harness::new(CALLS);
    open_list(&mut h);
    h.focused = false;
    h.frame(vec![]);
    assert!(h.state.call_list_rows().is_empty());
}

// MARK: Pictures

const GALLERY: &str = "//@version=6
indicator(\"Reaction\", overlay = true)

type ReactionState
    float priorCore = na
    float priorTop = na
    int reclaimDirection = 0
    int breakBar = na
    int outsideCloses = 0

method forget(ReactionState self) =>
    self.reclaimDirection := 0
    self.breakBar := na
    self.outsideCloses := 0

method inspect(ReactionState self, float core, float halfWidth, bool reset, bool enabled) =>
    if reset
        self.forget()
    float top = core + halfWidth
    if not enabled
        self.forget()
    top

var ReactionState state = ReactionState.new()
float level = ta.sma(close, 20)
if barstate.isfirst
    state.forget()
if close > open and volume > ta.sma(volume, 50) and not na(level) and ta.crossover(close, level)
    state.forget()
state.inspect(level, 1.5, false, true)
plot(level)
";

#[test]
#[ignore = "writes images for a person to look at"]
fn gallery_of_the_call_list() {
    use crate::ui::render_tests::{Screen, gallery_dir};
    use crate::ui::theme::AppTheme;
    if gallery_dir().is_none() {
        return;
    }
    for (name, app_theme) in [("dark", AppTheme::Dark), ("light", AppTheme::Light)] {
        let mut screen = Screen::new(egui::vec2(980.0, 640.0));
        theme::apply(&screen.ctx, app_theme);
        let mut state = EditorState::new("gallery", GALLERY, GALLERY);
        screen.ctx.memory_mut(|m| m.request_focus(state.id));
        let run = |screen: &mut Screen, state: &mut EditorState, events: Vec<Event>| {
            theme::sync(&screen.ctx);
            screen.run(events, |ui| {
                show(ui, state);
            })
        };
        for _ in 0..4 {
            run(&mut screen, &mut state, vec![]);
        }
        // Aim at the name through the caret: put it inside `forget`, and click where it is drawn.
        let inside = GALLERY.find("forget").expect("the declaration");
        let inside = GALLERY[..inside].chars().count() + 2;
        let mut edit_state = TextEditState::load(&screen.ctx, state.id).unwrap_or_default();
        edit_state
            .cursor
            .set_char_range(Some(CCursorRange::one(CCursor::new(inside))));
        edit_state.store(&screen.ctx, state.id);
        run(&mut screen, &mut state, vec![]);
        let caret = state.caret_rect.expect("a caret");
        let target = caret.center();
        run(&mut screen, &mut state, vec![Event::PointerMoved(target)]);
        let press = |pressed| Event::PointerButton {
            pos: target,
            button: PointerButton::Primary,
            pressed,
            modifiers: cmd(),
        };
        run(&mut screen, &mut state, vec![press(true)]);
        run(&mut screen, &mut state, vec![press(false)]);
        let output = run(&mut screen, &mut state, vec![]);
        let output = {
            drop(output);
            run(&mut screen, &mut state, vec![])
        };
        screen.save(&format!("call-list-{name}"), output);
        run(
            &mut screen,
            &mut state,
            vec![key(Key::ArrowDown, Modifiers::NONE)],
        );
        run(
            &mut screen,
            &mut state,
            vec![key(Key::ArrowDown, Modifiers::NONE)],
        );
        let output = run(&mut screen, &mut state, vec![]);
        screen.save(&format!("call-list-second-{name}"), output);
    }
}

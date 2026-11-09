use super::*;
use crate::model::Tool;
use crate::model::object_tree::IndicatorKey;
use eframe::egui::{Event, Modifiers, PointerButton, Pos2, RawInput};

fn row(key: Key, label: &str, shown: Option<bool>, locked: Option<bool>) -> Row {
    Row {
        key,
        label: label.to_owned(),
        detail: String::new(),
        shown,
        locked,
        tool: None,
        count: 0,
    }
}

fn heading_row(section: Section, count: usize) -> Row {
    Row {
        count,
        ..row(Key::Heading(section), "Heading", None, None)
    }
}

fn rows() -> Vec<Row> {
    vec![
        heading_row(Section::Indicators, 1),
        Row {
            detail: "Script".to_owned(),
            ..row(
                Key::Indicator(IndicatorKey::Script(3)),
                "My script",
                Some(true),
                None,
            )
        },
        heading_row(Section::Drawings, 2),
        Row {
            tool: Some(Tool::TrendLine),
            ..row(Key::Drawing(7), "Trend line 1", Some(true), Some(false))
        },
        Row {
            tool: Some(Tool::Rectangle),
            ..row(Key::Drawing(8), "Rectangle 1", Some(true), Some(true))
        },
    ]
}

/// The tree on a throwaway context, driven frame by frame.
struct Rig {
    ctx: egui::Context,
    state: TreeState,
    rows: Vec<Row>,
    selected: Vec<u64>,
    actions: Vec<TreeAction>,
}

impl Rig {
    fn new() -> Self {
        Self::with(rows())
    }

    fn with(rows: Vec<Row>) -> Self {
        probe::clear();
        let rig = Self {
            ctx: egui::Context::default(),
            state: TreeState::default(),
            rows,
            selected: Vec::new(),
            actions: Vec::new(),
        };
        rig.settled()
    }

    fn settled(mut self) -> Self {
        for _ in 0..3 {
            self.frame(Vec::new());
        }
        self.actions.clear();
        self
    }

    fn frame(&mut self, events: Vec<Event>) {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(320.0, 700.0))),
            events,
            ..RawInput::default()
        };
        let rows = self.rows.clone();
        let selected = self.selected.clone();
        let state = &mut self.state;
        let mut found = Vec::new();
        let mut output = self.ctx.run_ui(input, |ui| {
            found = show(ui, state, "BTCUSDT", &rows, &selected);
        });
        output.textures_delta.clear();
        self.actions.extend(found);
    }

    fn row(&self, key: Key) -> Rect {
        probe::rect_of(key).expect("row drawn")
    }

    fn drawn(&self, key: Key) -> bool {
        probe::rect_of(key).is_some()
    }

    fn move_to(&mut self, point: Pos2) {
        self.frame(vec![Event::PointerMoved(point)]);
        // The buttons are hit-tested from the frame before: one more at rest.
        self.frame(Vec::new());
    }

    fn click(&mut self, point: Pos2) {
        self.move_to(point);
        for pressed in [true, false] {
            self.frame(vec![Event::PointerButton {
                pos: point,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            }]);
        }
    }
}

// MARK: Buttons

#[test]
fn the_buttons_stay_while_the_pointer_is_on_them_and_can_be_clicked() {
    let mut rig = Rig::new();
    let row = rig.row(Key::Drawing(7));
    // Hover the row, then slide onto the eye: the row's own hover ends there, the buttons
    // must not.
    rig.move_to(row.center());
    rig.move_to(button_slot(row, 2).center());
    rig.click(button_slot(row, 2).center());
    assert_eq!(rig.actions, [TreeAction::ToggleShown(Key::Drawing(7))]);
}

#[test]
fn delete_lock_and_show_each_do_their_own_thing() {
    let mut rig = Rig::new();
    let row = rig.row(Key::Drawing(7));
    rig.click(button_slot(row, 0).center());
    assert_eq!(rig.actions, [TreeAction::Delete(Key::Drawing(7))]);
    rig.actions.clear();
    rig.click(button_slot(row, 1).center());
    assert_eq!(rig.actions, [TreeAction::ToggleLocked(7)]);
    rig.actions.clear();
    rig.click(button_slot(row, 2).center());
    assert_eq!(rig.actions, [TreeAction::ToggleShown(Key::Drawing(7))]);
}

#[test]
fn a_script_row_can_be_hidden_and_removed() {
    let mut rig = Rig::new();
    let key = Key::Indicator(IndicatorKey::Script(3));
    let row = rig.row(key);
    // No lock on a script: the eye sits right next to the delete, with no empty slot.
    rig.click(button_slot(row, 1).center());
    assert_eq!(rig.actions, [TreeAction::ToggleShown(key)]);
    rig.actions.clear();
    rig.click(button_slot(row, 0).center());
    assert_eq!(rig.actions, [TreeAction::Delete(key)]);
}

#[test]
fn a_locked_drawing_cannot_be_deleted_from_the_tree() {
    let mut rig = Rig::new();
    let row = rig.row(Key::Drawing(8));
    rig.move_to(row.center());
    // Its delete is greyed in place: nothing there to click.
    rig.click(button_slot(row, 0).center());
    assert!(
        rig.actions
            .iter()
            .all(|action| !matches!(action, TreeAction::Delete(_)))
    );
    // Unlocking it is possible.
    rig.actions.clear();
    rig.click(button_slot(row, 1).center());
    assert_eq!(rig.actions, [TreeAction::ToggleLocked(8)]);
}

#[test]
fn clicking_the_row_selects_it_and_a_button_does_not() {
    let mut rig = Rig::new();
    let row = rig.row(Key::Drawing(7));
    rig.click(row.left_center() + vec2(60.0, 0.0));
    assert_eq!(rig.actions, [TreeAction::Select(Key::Drawing(7))]);
    rig.actions.clear();
    rig.click(button_slot(row, 2).center());
    assert!(
        rig.actions
            .iter()
            .all(|action| !matches!(action, TreeAction::Select(_))),
        "{:?}",
        rig.actions
    );
}

#[test]
fn a_hidden_row_keeps_its_show_button_without_hover() {
    let mut rig = Rig::new();
    rig.rows[3].shown = Some(false);
    let mut rig = rig.settled();
    let row = rig.row(Key::Drawing(7));
    rig.frame(vec![Event::PointerMoved(button_slot(row, 2).center())]);
    rig.frame(Vec::new());
    rig.click(button_slot(row, 2).center());
    assert_eq!(rig.actions, [TreeAction::ToggleShown(Key::Drawing(7))]);
}

#[test]
fn a_hidden_script_shows_its_eye_next_to_the_edge_without_hover() {
    let mut rig = Rig::new();
    rig.rows[1].shown = Some(false);
    let mut rig = rig.settled();
    let key = Key::Indicator(IndicatorKey::Script(3));
    let row = rig.row(key);
    // At rest the eye is the only button: it sits in the second slot, delete's is empty.
    rig.frame(vec![Event::PointerMoved(button_slot(row, 1).center())]);
    rig.frame(Vec::new());
    rig.click(button_slot(row, 1).center());
    assert_eq!(rig.actions, [TreeAction::ToggleShown(key)]);
}

#[test]
fn a_row_without_a_lock_has_no_gap_where_the_lock_would_be() {
    let rows = rows();
    assert_eq!(row_buttons(&rows[3]), [Btn::Delete, Btn::Lock, Btn::Show]);
    assert_eq!(row_buttons(&rows[1]), [Btn::Delete, Btn::Show]);
    // The two slots a script uses are neighbours.
    let row = Rect::from_min_size(pos2(0.0, 0.0), vec2(300.0, ROW_HEIGHT));
    let gap = button_slot(row, 0).left() - button_slot(row, 1).right();
    assert!(gap <= 2.0 + f32::EPSILON, "{gap}");
}

#[test]
fn the_symbol_and_headings_have_no_buttons() {
    let symbol = row(Key::Symbol, "BTCUSDT", None, None);
    assert!(row_buttons(&symbol).is_empty());
    assert!(row_buttons(&heading_row(Section::Drawings, 2)).is_empty());
}

#[test]
fn button_slots_are_fixed_and_do_not_overlap() {
    let row = Rect::from_min_size(pos2(0.0, 0.0), vec2(300.0, ROW_HEIGHT));
    let slots: Vec<Rect> = (0..3).map(|index| button_slot(row, index)).collect();
    assert!(slots[0].left() > slots[1].right());
    assert!(slots[1].left() > slots[2].right());
    assert!(slots.iter().all(|slot| row.contains_rect(*slot)));
}

// MARK: Sections

#[test]
fn a_folded_section_hides_its_rows_but_keeps_its_heading() {
    let rows = rows();
    let none = HashSet::new();
    assert_eq!(visible(&rows, &none, false).len(), rows.len());
    let folded = HashSet::from([Section::Drawings]);
    let shown = visible(&rows, &folded, false);
    assert_eq!(shown.len(), 3, "both headings and the script");
    assert!(
        shown
            .iter()
            .any(|row| row.key == Key::Heading(Section::Drawings))
    );
    assert!(!shown.iter().any(|row| matches!(row.key, Key::Drawing(_))));
}

#[test]
fn a_filter_lists_its_matches_even_in_a_folded_section() {
    let rows = rows();
    let folded = HashSet::from([Section::Drawings, Section::Indicators]);
    assert_eq!(visible(&rows, &folded, true).len(), rows.len());
}

#[test]
fn clicking_a_heading_folds_and_unfolds_its_section() {
    let mut rig = Rig::new();
    let heading = rig.row(Key::Heading(Section::Drawings));
    rig.click(heading.center());
    assert!(rig.state.collapsed.contains(&Section::Drawings));
    assert!(!rig.drawn(Key::Drawing(7)) || rig.state.collapsed.contains(&Section::Drawings));
    rig.frame(Vec::new());
    let heading = rig.row(Key::Heading(Section::Drawings));
    rig.click(heading.center());
    assert!(!rig.state.collapsed.contains(&Section::Drawings));
    // Folding one section leaves the other alone.
    assert!(rig.state.collapsed.is_empty());
}

#[test]
fn a_filter_stops_headings_from_folding() {
    let mut rig = Rig::new();
    rig.state.filter = "line".to_owned();
    let mut rig = rig.settled();
    let heading = rig.row(Key::Heading(Section::Drawings));
    rig.click(heading.center());
    assert!(rig.state.collapsed.is_empty());
}

#[test]
fn only_a_symbol_and_nothing_else_asks_for_objects() {
    let symbol_only = vec![
        heading_row(Section::Symbol, 1),
        row(Key::Symbol, "BTCUSDT", None, None),
    ];
    assert!(only_the_symbol(&symbol_only));
    assert!(!only_the_symbol(&rows()));
}

// MARK: Glyphs

#[test]
fn a_compared_market_row_can_be_hidden_and_removed_but_not_locked() {
    use crate::model::overlay::OverlayId;
    let key = Key::Overlay(OverlayId(5));
    let rows = vec![
        heading_row(Section::Overlays, 1),
        row(key, "ETHUSDT", Some(true), None),
    ];
    assert_eq!(row_buttons(&rows[1]), [Btn::Delete, Btn::Show]);
    assert_eq!(row_icon(&rows[1]), Icon::Compare);
    let mut rig = Rig::with(rows);
    let drawn = rig.row(key);
    rig.click(button_slot(drawn, 1).center());
    assert_eq!(rig.actions, [TreeAction::ToggleShown(key)]);
    rig.actions.clear();
    rig.click(button_slot(drawn, 0).center());
    assert_eq!(rig.actions, [TreeAction::Delete(key)]);
    // Double-clicking it opens its settings, like any row but the symbol.
    rig.actions.clear();
    let centre = drawn.center() - vec2(40.0, 0.0);
    rig.click(centre);
    rig.click(centre);
    assert!(rig.actions.contains(&TreeAction::Select(key)));
}

#[test]
fn every_row_leads_with_the_glyph_of_what_it_is() {
    let drawing = Row {
        tool: Some(Tool::Rectangle),
        ..row(Key::Drawing(1), "Rectangle 1", Some(true), Some(false))
    };
    assert_eq!(row_icon(&drawing), tool_icon(Tool::Rectangle));
    let unknown = row(Key::Drawing(2), "Drawing", Some(true), Some(false));
    assert_eq!(row_icon(&unknown), Icon::Pencil);
    assert_eq!(
        row_icon(&row(
            Key::Indicator(IndicatorKey::Script(1)),
            "S",
            None,
            None
        )),
        Icon::Curlybraces
    );
    let builtin = |indicator| {
        row_icon(&row(
            Key::Indicator(IndicatorKey::BuiltIn(indicator)),
            "I",
            None,
            None,
        ))
    };
    assert_eq!(builtin(BuiltIn::Rsi), Icon::Waveform);
    assert_eq!(builtin(BuiltIn::Volume), Icon::BarChart);
    assert_eq!(builtin(BuiltIn::Ema), Icon::ChartUptrend);
    assert_eq!(
        row_icon(&row(Key::Symbol, "BTC", None, None)),
        Icon::ChartLine
    );
}

// MARK: States

#[test]
fn an_empty_list_and_a_list_of_only_the_symbol_draw_without_trouble() {
    let mut rig = Rig::with(Vec::new());
    rig.state.filter = "nothing".to_owned();
    rig.frame(Vec::new());
    rig.frame(Vec::new());
    let mut rig = Rig::with(vec![
        heading_row(Section::Symbol, 1),
        row(Key::Symbol, "BTCUSDT", None, None),
    ]);
    rig.frame(Vec::new());
    assert!(rig.drawn(Key::Symbol));
}

#[test]
fn a_selected_drawing_is_drawn_selected() {
    let mut rig = Rig::new();
    rig.selected = vec![7];
    rig.frame(Vec::new());
    // Selection is the workspace's: the tree reports a click, it never keeps its own copy.
    let row = rig.row(Key::Drawing(8));
    rig.click(row.left_center() + vec2(60.0, 0.0));
    assert_eq!(rig.actions, [TreeAction::Select(Key::Drawing(8))]);
}

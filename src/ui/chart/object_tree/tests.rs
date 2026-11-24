use super::*;
use crate::model::Tool;
use crate::model::chart::object_tree::IndicatorKey;
use eframe::egui::{
    Event, Modifiers, MouseWheelUnit, PointerButton, Pos2, RawInput, TouchDeviceId, TouchId,
    TouchPhase,
};

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
        Icon::AngleBrackets
    );
    let builtin = |indicator| {
        row_icon(&row(
            Key::Indicator(IndicatorKey::BuiltIn(indicator)),
            "I",
            None,
            None,
        ))
    };
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

/// The tree in the pseudo-locale: its toolbar, search, section headings, an empty answer to a
/// filter and the hint under the symbol alone. `DEGENVIEW_GALLERY_DIR=<dir>` writes the pictures.
#[test]
fn the_tree_draws_in_the_pseudo_locale() {
    use crate::i18n::{Locale, with_locale};
    use crate::ui::chart::painted_text;
    use crate::ui::render_tests::{Screen, gallery_dir};

    let _theme = crate::ui::theme::RestoreDefaultTheme;
    let symbol_only = vec![Row {
        count: 1,
        ..row(Key::Symbol, "BTCUSDT", Some(true), None)
    }];
    for (theme_name, app_theme) in [
        ("dark", crate::ui::theme::AppTheme::Dark),
        ("light", crate::ui::theme::AppTheme::Light),
    ] {
        for (name, rows, filter, wanted) in [
            ("rows", rows(), "", "Object tree"),
            (
                "no-matches",
                Vec::new(),
                "zzz",
                "Nothing on this chart matches the filter.",
            ),
            (
                "symbol-only",
                symbol_only.clone(),
                "",
                "Draw with the tools on the left or add an indicator, and it is listed here.",
            ),
        ] {
            let mut screen = Screen::new(vec2(300.0, 420.0));
            crate::ui::theme::apply(&screen.ctx, app_theme);
            let mut state = TreeState {
                filter: filter.to_owned(),
                ..TreeState::default()
            };
            let mut last = None;
            with_locale(Locale::PSEUDO, || {
                for _ in 0..4 {
                    last = Some(screen.run(Vec::new(), |ui| {
                        let full = ui.max_rect();
                        ui.painter()
                            .rect_filled(full, 0.0, crate::ui::theme::chrome());
                        show(ui, &mut state, "BTCUSDT", &rows, &[]);
                    }));
                }
            });
            let last = last.take().expect("a frame");
            let texts = painted_text::of(&last);
            painted_text::assert_painted(&texts, wanted);
            if name == "rows" {
                painted_text::assert_painted(&texts, "INDICATORS");
                painted_text::assert_painted(&texts, "DRAWINGS");
            }
            if gallery_dir().is_some() {
                eprintln!("{name}: {:?}", painted_text::unbracketed(&texts));
                screen.save(&format!("pseudo-object-tree-{name}-{theme_name}"), last);
            }
        }
    }
}

// MARK: Reordering

impl Rig {
    /// Presses on `from`, carries the pointer to `to` and lets go.
    fn drag(&mut self, from: Pos2, to: Pos2) {
        self.move_to(from);
        let button = |pos, pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        self.frame(vec![button(from, true)]);
        for point in [from.lerp(to, 0.5), to] {
            self.frame(vec![Event::PointerMoved(point)]);
        }
        self.frame(Vec::new());
        self.frame(vec![button(to, false)]);
        self.frame(Vec::new());
    }

    /// A point on a row that is clear of its buttons, `fraction` of the way down it.
    fn on(&self, key: Key, fraction: f32) -> Pos2 {
        let row = self.row(key);
        pos2(row.left() + 60.0, row.top() + row.height() * fraction)
    }

    fn drops(&self) -> Vec<&TreeAction> {
        self.actions
            .iter()
            .filter(|action| matches!(action, TreeAction::Drop { .. }))
            .collect()
    }
}

#[test]
fn dragging_a_drawing_above_another_drops_it_into_that_ones_place() {
    let mut rig = Rig::new();
    let (from, to) = (rig.on(Key::Drawing(8), 0.5), rig.on(Key::Drawing(7), 0.2));
    rig.drag(from, to);
    assert_eq!(
        rig.actions,
        [TreeAction::Drop {
            key: Key::Drawing(8),
            over: Key::Drawing(7)
        }]
    );
    assert_eq!(rig.state.dragging, None, "let go");
}

#[test]
fn dragging_a_drawing_below_the_last_one_drops_it_into_the_last_place() {
    let mut rig = Rig::new();
    let (from, to) = (rig.on(Key::Drawing(7), 0.5), rig.on(Key::Drawing(8), 0.9));
    rig.drag(from, to);
    assert_eq!(
        rig.actions,
        [TreeAction::Drop {
            key: Key::Drawing(7),
            over: Key::Drawing(8)
        }]
    );
}

#[test]
fn a_drop_back_where_the_row_was_is_nothing() {
    let mut rig = Rig::new();
    // Into the half of the neighbour that is nearer to the row itself: its own gap.
    let (from, to) = (rig.on(Key::Drawing(7), 0.5), rig.on(Key::Drawing(8), 0.1));
    rig.drag(from, to);
    assert!(rig.drops().is_empty(), "{:?}", rig.actions);
    assert_eq!(rig.state.dragging, None);
}

#[test]
fn a_row_dragged_past_the_end_of_its_section_lands_at_its_end() {
    let mut rig = Rig::new();
    // Dragged on up to the toolbar, the heading and the script above its section.
    let (from, to) = (
        rig.on(Key::Drawing(8), 0.5),
        rig.on(Key::Heading(Section::Indicators), 0.5),
    );
    rig.drag(from, to);
    assert_eq!(
        rig.actions,
        [TreeAction::Drop {
            key: Key::Drawing(8),
            over: Key::Drawing(7)
        }]
    );
}

#[test]
fn letting_go_beside_the_list_puts_the_row_back() {
    let mut rig = Rig::new();
    let from = rig.on(Key::Drawing(8), 0.5);
    rig.drag(from, pos2(900.0, from.y - 40.0));
    assert!(rig.actions.is_empty(), "{:?}", rig.actions);
    assert_eq!(rig.state.dragging, None);
}

#[test]
fn only_what_can_be_moved_can_be_picked_up() {
    // A heading is not a row to move, and nothing moves in a filtered list.
    for (key, filter) in [
        (Key::Heading(Section::Drawings), ""),
        (Key::Drawing(8), "line"),
    ] {
        let mut rig = Rig::new();
        rig.state.filter = filter.to_owned();
        let mut rig = rig.settled();
        if !rig.drawn(key) {
            continue;
        }
        let from = rig.on(key, 0.5);
        rig.drag(from, pos2(from.x, from.y + 20.0));
        assert!(
            rig.drops().is_empty(),
            "{key:?} {filter:?}: {:?}",
            rig.actions
        );
        assert_eq!(rig.state.dragging, None, "{key:?} {filter:?}");
    }
    // A drawing from a newer version has no tool, so it stays where it is.
    let mut rows = rows();
    rows[3].tool = None;
    let mut rig = Rig::with(rows);
    let from = rig.on(Key::Drawing(7), 0.5);
    rig.drag(from, rig.on(Key::Drawing(8), 0.9));
    assert!(rig.drops().is_empty(), "{:?}", rig.actions);
}

#[test]
fn a_drag_is_not_a_click_and_a_click_is_not_a_drag() {
    let mut rig = Rig::new();
    let (from, to) = (rig.on(Key::Drawing(8), 0.5), rig.on(Key::Drawing(7), 0.2));
    rig.drag(from, to);
    assert!(
        !rig.actions
            .iter()
            .any(|action| matches!(action, TreeAction::Select(_))),
        "{:?}",
        rig.actions
    );
    let mut rig = Rig::new();
    let point = rig.on(Key::Drawing(8), 0.5);
    rig.click(point);
    assert_eq!(rig.actions, [TreeAction::Select(Key::Drawing(8))]);
    assert_eq!(rig.state.dragging, None);
}

#[test]
fn nothing_happens_until_the_carried_row_is_let_go() {
    let mut rig = Rig::new();
    rig.move_to(rig.on(Key::Drawing(8), 0.5));
    let from = rig.on(Key::Drawing(8), 0.5);
    rig.frame(vec![Event::PointerButton {
        pos: from,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    }]);
    let to = rig.on(Key::Drawing(7), 0.2);
    for point in [from.lerp(to, 0.5), to] {
        rig.frame(vec![Event::PointerMoved(point)]);
    }
    rig.frame(Vec::new());
    assert_eq!(rig.state.dragging, Some(Key::Drawing(8)));
    assert!(rig.actions.is_empty(), "nothing happens until it is let go");
}

// MARK: Geometry, bounds and touch

/// A drawings section of `count` trend lines, topmost first, long enough to scroll.
fn many_drawings(count: u64) -> Vec<Row> {
    let mut rows = vec![heading_row(Section::Drawings, count as usize)];
    rows.extend((1..=count).rev().map(|id| Row {
        tool: Some(Tool::TrendLine),
        ..row(
            Key::Drawing(id),
            &format!("Trend line {id}"),
            Some(true),
            Some(false),
        )
    }));
    rows
}

impl Rig {
    /// Scrolls the list with the wheel, then lets the animation settle and forgets where rows were.
    fn scroll(&mut self, points: f32) {
        self.move_to(pos2(160.0, 300.0));
        self.frame(vec![Event::MouseWheel {
            unit: MouseWheelUnit::Point,
            delta: vec2(0.0, -points),
            phase: TouchPhase::Move,
            modifiers: Modifiers::NONE,
        }]);
        for _ in 0..40 {
            self.frame(Vec::new());
        }
        probe::clear();
        self.frame(Vec::new());
    }

    /// The drawings drawn now, as ids, lowest in the list first.
    fn visible_drawings(&self, count: u64) -> Vec<u64> {
        (1..=count)
            .filter(|id| self.drawn(Key::Drawing(*id)))
            .collect()
    }

    /// One touch event, with the pointer events a touch screen's backend sends beside it.
    fn touch(&mut self, phase: TouchPhase, pos: Pos2) {
        let button = |pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        let mut events = vec![Event::Touch {
            device_id: TouchDeviceId(0),
            id: TouchId(0),
            phase,
            pos,
            force: None,
        }];
        match phase {
            TouchPhase::Start => events.extend([Event::PointerMoved(pos), button(true)]),
            TouchPhase::Move => events.push(Event::PointerMoved(pos)),
            TouchPhase::End | TouchPhase::Cancel => {
                events.extend([button(false), Event::PointerGone]);
            }
        }
        self.frame(events);
    }

    /// A finger swipe from `from` to `to`, in small steps, then lifted.
    fn swipe(&mut self, from: Pos2, to: Pos2) {
        self.touch(TouchPhase::Start, from);
        for step in 1..=10 {
            self.touch(TouchPhase::Move, from.lerp(to, step as f32 / 10.0));
        }
        self.touch(TouchPhase::End, to);
        self.frame(Vec::new());
    }
}

#[test]
fn a_scrolled_list_drops_on_the_row_under_the_pointer() {
    let mut rig = Rig::with(many_drawings(40));
    rig.scroll(600.0);
    let visible = rig.visible_drawings(40);
    assert!(
        visible.len() > 8 && !visible.contains(&40),
        "scrolled well down the list: {visible:?}"
    );
    // Rows near the middle of the viewport, away from the edges that scroll.
    let middle = visible.len() / 2;
    let (below, over, above) = (visible[middle - 2], visible[middle], visible[middle + 2]);
    // A row lower in the list, let go on the upper half of `over`, takes `over`'s place...
    rig.drag(
        rig.on(Key::Drawing(below), 0.5),
        rig.on(Key::Drawing(over), 0.2),
    );
    assert_eq!(
        rig.actions,
        [TreeAction::Drop {
            key: Key::Drawing(below),
            over: Key::Drawing(over)
        }]
    );
    // ...and so does one higher in the list, let go on its lower half.
    rig.actions.clear();
    rig.drag(
        rig.on(Key::Drawing(above), 0.5),
        rig.on(Key::Drawing(over), 0.8),
    );
    assert_eq!(
        rig.actions,
        [TreeAction::Drop {
            key: Key::Drawing(above),
            over: Key::Drawing(over)
        }]
    );
}

#[test]
fn letting_go_above_or_below_the_list_puts_the_row_back() {
    let mut rig = Rig::new();
    let from = rig.on(Key::Drawing(8), 0.5);
    // Over the filter field, above the list.
    let list_top = rig.row(Key::Heading(Section::Indicators)).top();
    rig.drag(from, pos2(from.x, list_top - 20.0));
    assert!(rig.drops().is_empty(), "above: {:?}", rig.actions);
    assert_eq!(rig.state.dragging, None);
    // Below a list that is shorter than the panel.
    let bottom = rig.row(Key::Drawing(8)).bottom();
    let from = rig.on(Key::Drawing(7), 0.5);
    rig.drag(from, pos2(from.x, bottom + 80.0));
    assert!(rig.drops().is_empty(), "below: {:?}", rig.actions);
    assert_eq!(rig.state.dragging, None);
}

/// How far down the list is scrolled: how far the first drawing row has moved up.
fn scrolled_by(rig: &Rig, was: f32) -> f32 {
    was - rig.row(Key::Drawing(40)).top()
}

#[test]
fn a_touch_swipe_scrolls_the_list_over_any_row() {
    // A drawing that could be dragged, then rows that cannot: a built-in indicator, and a row of a
    // filtered list. A finger moving over them is scrolling, and nothing is picked up.
    let mut cases: Vec<(&str, Vec<Row>, &str, Key)> = Vec::new();
    cases.push(("a drawing", many_drawings(40), "", Key::Drawing(39)));
    let mut with_built_in = vec![
        heading_row(Section::Indicators, 1),
        row(
            Key::Indicator(IndicatorKey::BuiltIn(crate::model::BuiltIn::Ema)),
            "EMA 20",
            Some(true),
            None,
        ),
    ];
    with_built_in.extend(many_drawings(40));
    cases.push((
        "a built-in indicator",
        with_built_in,
        "",
        Key::Indicator(IndicatorKey::BuiltIn(crate::model::BuiltIn::Ema)),
    ));
    cases.push((
        "a filtered row",
        many_drawings(40),
        "line",
        Key::Drawing(39),
    ));
    for (name, rows, filter, key) in cases {
        let mut rig = Rig::with(rows);
        rig.state.filter = filter.to_owned();
        let mut rig = rig.settled();
        // The first touch tells egui it has a touch screen.
        rig.touch(TouchPhase::Start, pos2(5.0, 5.0));
        rig.touch(TouchPhase::End, pos2(5.0, 5.0));
        let was = rig.row(Key::Drawing(40)).top();
        let from = rig.on(key, 0.5);
        rig.swipe(from, pos2(from.x, from.y - 120.0));
        // Scrolling by a delta is animated: let it settle.
        for _ in 0..40 {
            rig.frame(Vec::new());
        }
        assert!(
            scrolled_by(&rig, was) > 30.0,
            "{name}: the list scrolled {}",
            scrolled_by(&rig, was)
        );
        assert!(rig.drops().is_empty(), "{name}: {:?}", rig.actions);
        assert_eq!(rig.state.dragging, None, "{name}");
    }
}

#[test]
fn a_touch_long_press_picks_a_row_up_and_letting_go_drops_it() {
    let mut rig = Rig::new();
    let (from, to) = (rig.on(Key::Drawing(8), 0.5), rig.on(Key::Drawing(7), 0.2));
    rig.touch(TouchPhase::Start, from);
    // Held still for about half a second.
    for _ in 0..40 {
        rig.frame(Vec::new());
    }
    assert_eq!(rig.state.dragging, Some(Key::Drawing(8)), "picked up");
    for step in 1..=5 {
        rig.touch(TouchPhase::Move, from.lerp(to, step as f32 / 5.0));
    }
    // A finger lifted has no pointer position left: the drop is where it was last.
    rig.touch(TouchPhase::End, to);
    rig.frame(Vec::new());
    assert_eq!(
        rig.actions,
        [TreeAction::Drop {
            key: Key::Drawing(8),
            over: Key::Drawing(7)
        }]
    );
    assert_eq!(rig.state.dragging, None);
}

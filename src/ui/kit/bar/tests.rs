use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Act {
    One,
    Two,
    Menu,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Key {
    Only,
}

type TestItem = Item<Act, Key>;
type TestButton = Button<Act, Key>;

fn left() -> Vec<TestItem> {
    vec![
        Item::Button(
            TestButton::new("one", "One")
                .icon(Icon::Gear)
                .label("First action")
                .on_click(Act::One),
        ),
        Item::Separator,
        Item::Button(
            TestButton::new("pick", "Pick")
                .label("Pick one")
                .with_menu(Key::Only),
        ),
    ]
}

fn right() -> Vec<TestItem> {
    vec![
        Item::Button(
            TestButton::new("two", "Two")
                .icon(Icon::Plus)
                .label("Second action")
                .on_click(Act::Two),
        ),
        Item::Button(
            TestButton::new("off", "Off")
                .icon(Icon::Eye)
                .enabled(false)
                .on_click(Act::Menu),
        ),
    ]
}

fn screen(width: f32) -> egui::RawInput {
    egui::RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(width, HEIGHT))),
        ..Default::default()
    }
}

fn geometry_at(width: f32) -> (bool, Vec<Placed<Act>>) {
    let ctx = egui::Context::default();
    let mut out = None;
    let mut frame = ctx.run_ui(screen(width), |ui| {
        out = Some(geometry(ui, &left(), &right(), width));
    });
    frame.textures_delta.clear();
    out.expect("a frame")
}

#[test]
fn labels_fall_back_to_icons_before_the_right_run_drops_anything() {
    let (wide, wide_buttons) = geometry_at(1200.0);
    assert!(wide, "a wide bar has room for the labels");
    assert_eq!(wide_buttons.len(), 4);
    let (narrow, narrow_buttons) = geometry_at(330.0);
    assert!(!narrow, "no room for the labels");
    assert!(
        narrow_buttons.len() >= 2,
        "the icons and the label-only button stay"
    );
    let (_, tiny) = geometry_at(130.0);
    assert!(
        tiny.len() < 4,
        "the right run drops buttons, never overlaps"
    );
}

#[test]
fn no_two_buttons_overlap_or_leave_the_bar() {
    for width in [1200.0_f32, 700.0, 330.0] {
        let (_, mut buttons) = geometry_at(width);
        buttons.sort_by(|a, b| a.from.total_cmp(&b.from));
        for pair in buttons.windows(2) {
            assert!(pair[0].to <= pair[1].from, "overlap at {width}");
        }
        for button in &buttons {
            assert!(button.from >= EDGE - 0.5, "before the edge at {width}");
            assert!(button.to <= width - EDGE + 0.5, "past the edge at {width}");
        }
    }
}

/// Clicks the middle of the `n`th button and returns what the bar asked for.
fn click_button(width: f32, n: usize) -> Vec<Act> {
    let (_, buttons) = geometry_at(width);
    let at = pos2((buttons[n].from + buttons[n].to) / 2.0, HEIGHT / 2.0);
    let ctx = egui::Context::default();
    let press = |pressed| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    let mut asked = Vec::new();
    let mut time = 0.0;
    for events in [
        vec![],
        vec![egui::Event::PointerMoved(at)],
        vec![press(true)],
        vec![press(false)],
        vec![],
    ] {
        time += 0.1;
        let mut raw = screen(width);
        raw.time = Some(time);
        raw.events = events;
        let mut frame = ctx.run_ui(raw, |ui| {
            asked.extend(show(ui, &left(), &right(), |_, _, _, _| {}));
        });
        frame.textures_delta.clear();
    }
    asked
}

#[test]
fn a_click_asks_for_the_buttons_action() {
    // `geometry` lists the left run in order, then the right run from the edge inwards: First
    // action, Pick one, then Off (last on the bar) and Second action.
    assert_eq!(click_button(1200.0, 0), vec![Act::One]);
    assert_eq!(click_button(1200.0, 3), vec![Act::Two]);
}

#[test]
fn a_disabled_button_never_asks_for_anything() {
    assert!(click_button(1200.0, 2).is_empty());
}

#[test]
fn a_button_with_a_menu_hands_its_key_to_the_popup() {
    let ctx = egui::Context::default();
    let mut keys = Vec::new();
    let mut frame = ctx.run_ui(screen(1200.0), |ui| {
        show(ui, &left(), &right(), |_, key, _, _| keys.push(key));
    });
    frame.textures_delta.clear();
    assert_eq!(keys, vec![Key::Only]);
}

#[test]
fn a_disabled_menu_button_draws_no_popup() {
    let ctx = egui::Context::default();
    let mut keys = Vec::new();
    let items = vec![Item::Button(
        TestButton::new("pick", "Pick")
            .label("Pick one")
            .enabled(false)
            .with_menu(Key::Only),
    )];
    let mut frame = ctx.run_ui(screen(600.0), |ui| {
        show(ui, &items, &[], |_, key, _, _| keys.push(key));
    });
    frame.textures_delta.clear();
    assert!(keys.is_empty());
}

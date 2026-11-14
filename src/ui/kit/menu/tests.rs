use super::fit::{MENU_MAX_WIDTH, MENU_ROW_INSET};
use super::{
    MENU_MAX_HEIGHT, MenuRow, menu_picker, menu_popup, menu_row, menu_scope, menu_submenu,
    menu_width_for_rows,
};
use crate::ui::kit::{REGULAR, galley, size};
use eframe::egui::{self, Color32, Rect, Sense, pos2, vec2};

/// A menu is as wide as its widest row and no wider: never padded to a guess, never below the
/// floor or above the cap, and a shortcut hint widens the row it sits on.
#[test]
fn a_menu_is_as_wide_as_its_rows_need() {
    let ctx = egui::Context::default();
    crate::ui::theme::install_offscreen(&ctx);
    let mut widths = Vec::new();
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        let long = "x".repeat(200);
        for rows in [
            vec![("I", None)],
            vec![("I", None), ("Export Trading Data\u{2026}", None)],
            vec![("Export Trading Data\u{2026}", Some("\u{2318}\u{21e7}E"))],
            vec![(long.as_str(), None)],
        ] {
            widths.push(menu_width_for_rows(ui, &rows, 0.0));
        }
        // A wider trigger is the floor.
        widths.push(menu_width_for_rows(ui, &[("I", None)], 150.0));
    });
    output.textures_delta.clear();
    assert_eq!(widths[0], 72.0, "short menus stop at the floor");
    assert!(widths[1] > widths[0] && widths[1] < 200.0, "{widths:?}");
    let plain = menu_width_for_rows_plain(&ctx, "Export Trading Data\u{2026}");
    assert!(
        widths[2] > plain,
        "a hint widens its row: {widths:?} vs {plain}"
    );
    assert_eq!(widths[3], MENU_MAX_WIDTH, "long labels hit the cap");
    assert_eq!(widths[4], 150.0, "a wide trigger is the floor");
}

fn menu_width_for_rows_plain(ctx: &egui::Context, label: &str) -> f32 {
    let mut width = 0.0;
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        width = menu_width_for_rows(ui, &[(label, None)], 0.0);
    });
    output.textures_delta.clear();
    width
}

/// Opens a `menu_picker` with a 95-point trigger over `labels` and returns the popup's
/// on-screen rect.
fn open_menu(labels: &[String], selected: usize) -> Option<Rect> {
    let ctx = egui::Context::default();
    let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 700.0));
    let mut time = 0.0;
    let mut frame = |events: Vec<egui::Event>| {
        time += 0.1;
        let input = egui::RawInput {
            screen_rect: Some(screen),
            time: Some(time),
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            menu_picker(ui, 95.0, labels, selected, true);
        });
        output.textures_delta.clear();
    };
    let at = pos2(40.0, 12.0);
    frame(vec![egui::Event::PointerMoved(at)]);
    let button = |pressed| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    frame(vec![button(true)]);
    frame(vec![button(false)]);
    for _ in 0..3 {
        frame(Vec::new());
    }
    ctx.memory(|memory| {
        memory
            .layer_ids()
            .filter(|layer| layer.order == egui::Order::Foreground)
            .find_map(|layer| memory.area_rect(layer.id))
    })
}

#[test]
fn a_menu_picker_opens_wide_enough_for_its_longest_choice() {
    let long = "A choice with a rather long title that overflows".to_owned();
    let labels = vec!["Short".to_owned(), long.clone()];
    let menu = open_menu(&labels, 0).expect("the menu opens");
    let ctx = egui::Context::default();
    let widest = {
        let mut width = 0.0;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            width = galley(
                ui,
                &long,
                size::BODY,
                REGULAR,
                Color32::WHITE,
                f32::INFINITY,
            )
            .size()
            .x;
        });
        output.textures_delta.clear();
        width
    };
    assert!(
        menu.width() >= widest + MENU_ROW_INSET,
        "menu {} wide for a {} wide row",
        menu.width(),
        widest
    );
    // A choice list that fits keeps to the trigger.
    let narrow = open_menu(&["A".to_owned(), "B".to_owned()], 0).expect("the menu opens");
    assert!(narrow.width() <= 100.0, "{}", narrow.width());
}

#[test]
fn a_menu_never_outgrows_its_cap_and_scrolls_when_tall() {
    let labels: Vec<String> = (0..60)
        .map(|i| format!("{i} {}", "x".repeat(120)))
        .collect();
    let menu = open_menu(&labels, 30).expect("the menu opens");
    // The frame's one-point border sits outside the width the menu is given.
    assert!(menu.width() <= MENU_MAX_WIDTH + 2.5, "{}", menu.width());
    assert!(menu.height() <= MENU_MAX_HEIGHT + 20.0, "{}", menu.height());
}

/// A menu with one submenu row and one child, driven by pointer events.
struct SubmenuRig {
    ctx: egui::Context,
    time: f64,
    picked: usize,
    close_on_pick: bool,
}

impl SubmenuRig {
    fn new(close_on_pick: bool) -> Self {
        Self {
            ctx: egui::Context::default(),
            time: 0.0,
            picked: 0,
            close_on_pick,
        }
    }

    fn frame(&mut self, events: Vec<egui::Event>) {
        self.time += 0.1;
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, 600.0))),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let (close_on_pick, picked) = (self.close_on_pick, &mut self.picked);
        let mut output = self.ctx.run_ui(raw, |ui| {
            let (rect, response) = ui.allocate_exact_size(vec2(100.0, 24.0), Sense::click());
            let _ = rect;
            menu_popup(&response).width(160.0).show(|ui| {
                menu_scope(ui, 150.0);
                menu_submenu(ui, MenuRow::new("Parent"), 120.0, |ui| {
                    if menu_row(ui, MenuRow::new("Child")).clicked() {
                        *picked += 1;
                        if close_on_pick {
                            ui.close();
                        }
                    }
                });
            });
        });
        output.textures_delta.clear();
    }

    fn click(&mut self, at: egui::Pos2) {
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        self.frame(vec![egui::Event::PointerMoved(at)]);
        self.frame(vec![button(true)]);
        self.frame(vec![button(false)]);
        for _ in 0..3 {
            self.frame(Vec::new());
        }
    }

    fn open(&self) -> bool {
        egui::Popup::is_any_open(&self.ctx)
    }

    /// Opens the menu and its submenu; leaves the pointer on the child.
    fn open_submenu(&mut self) -> egui::Pos2 {
        self.frame(Vec::new());
        self.click(pos2(20.0, 12.0));
        assert!(self.open(), "the button opens the menu");
        // The first row sits under the button inside the frame's margin.
        let parent = pos2(40.0, 24.0 + 5.0 + 12.0);
        self.click(parent);
        // The submenu opens beside the menu, level with its row.
        pos2(24.0 + 160.0 + 40.0, parent.y)
    }
}

#[test]
fn a_submenu_row_opens_its_rows_beside_the_menu() {
    let mut rig = SubmenuRig::new(true);
    let child = rig.open_submenu();
    rig.click(child);
    assert_eq!(rig.picked, 1, "the child is where the submenu opened");
    assert!(!rig.open(), "a child that closes ends the whole menu");
}

#[test]
fn a_submenu_row_that_does_not_close_leaves_the_menu_open() {
    let mut rig = SubmenuRig::new(false);
    let child = rig.open_submenu();
    rig.click(child);
    rig.click(child);
    assert_eq!(rig.picked, 2);
    assert!(rig.open(), "toggling rows can be ticked again and again");
}

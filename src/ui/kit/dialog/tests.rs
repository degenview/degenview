use super::*;
use crate::ui::kit;
use eframe::egui::{Event, Key, Modifiers, PointerButton, Pos2, RawInput};

const ID: &str = "dialog-under-test";

fn spec<'a>() -> Dialog<'a> {
    Dialog::new(
        Icon::Trash,
        kit::RED,
        "Remove all indicators?",
        "4 indicators will be removed from this chart. Saved scripts are kept, and you can undo it.",
        "Remove All",
    )
}

fn destructive<'a>() -> Dialog<'a> {
    Dialog {
        destructive: true,
        ..spec()
    }
}

/// A dialog on a throwaway context, driven frame by frame.
struct Rig {
    ctx: egui::Context,
    time: f64,
}

impl Rig {
    fn new() -> Self {
        Self {
            ctx: egui::Context::default(),
            time: 0.0,
        }
    }

    fn frame(&mut self, events: Vec<Event>, dialog: Option<&Dialog>) -> Option<Outcome> {
        self.time += 0.1;
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(900.0, 700.0))),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let mut outcome = None;
        let mut output = self.ctx.run_ui(raw, |ui| {
            if let Some(dialog) = dialog {
                outcome = show(ui.ctx(), Id::new(ID), dialog, |_| {});
            }
        });
        output.textures_delta.clear();
        outcome
    }

    /// Runs a few frames so layout and focus settle.
    fn settle(&mut self, dialog: &Dialog) {
        for _ in 0..4 {
            self.frame(Vec::new(), Some(dialog));
        }
    }

    fn rect(&self, name: &str) -> Rect {
        self.ctx
            .read_response(Id::new(ID).with(name))
            .unwrap_or_else(|| panic!("{name} was drawn"))
            .rect
    }

    fn focused(&self, name: &str) -> bool {
        self.ctx
            .memory(|memory| memory.has_focus(Id::new(ID).with(name)))
    }

    fn key(&mut self, key: Key, dialog: &Dialog) -> Option<Outcome> {
        let event = |pressed| Event::Key {
            key,
            physical_key: Some(key),
            pressed,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        let first = self.frame(vec![event(true)], Some(dialog));
        let second = self.frame(vec![event(false)], Some(dialog));
        first.or(second)
    }

    fn click(&mut self, at: Pos2, dialog: &Dialog) -> Option<Outcome> {
        self.frame(vec![Event::PointerMoved(at)], Some(dialog));
        let mut outcome = None;
        for pressed in [true, false] {
            let event = Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            };
            outcome = outcome.or(self.frame(vec![event], Some(dialog)));
        }
        outcome
    }
}

#[test]
fn cancel_and_the_action_sit_side_by_side_with_the_action_on_the_right() {
    let mut rig = Rig::new();
    let dialog = destructive();
    rig.settle(&dialog);
    let (cancel, accept) = (rig.rect("dismiss"), rig.rect("accept"));
    assert!(cancel.right() < accept.left(), "{cancel:?} {accept:?}");
    assert_eq!(cancel.top(), accept.top(), "one row");
    assert_eq!(
        (cancel.height(), accept.height()),
        (BUTTON_HEIGHT, BUTTON_HEIGHT)
    );
    assert!(cancel.width() >= BUTTON_MIN_WIDTH && accept.width() >= BUTTON_MIN_WIDTH);
    assert!(
        (accept.left() - cancel.right() - BUTTON_GAP).abs() < 0.5,
        "the gap between them"
    );
    let close = rig.rect("close");
    assert!(
        close.center().y < cancel.top(),
        "the close button is on top"
    );
}

#[test]
fn the_card_keeps_its_width_and_settles_in_height() {
    let area = |rig: &Rig| {
        rig.ctx
            .memory(|memory| memory.area_rect(Id::new(ID).with("Modal")))
            .or_else(|| rig.ctx.memory(|memory| memory.area_rect(Id::new(ID))))
            .expect("the dialog was laid out")
    };
    let short = Dialog {
        message: "Short.",
        ..destructive()
    };
    let mut rig = Rig::new();
    rig.settle(&short);
    let small = area(&rig);

    let message = "A much longer explanation that has to wrap over several lines. ".repeat(6);
    let long = Dialog {
        message: &message,
        ..destructive()
    };
    let mut rig_long = Rig::new();
    let mut heights = Vec::new();
    for _ in 0..6 {
        rig_long.frame(Vec::new(), Some(&long));
        heights.push(area(&rig_long).height());
    }
    let tall = area(&rig_long);
    assert!((small.width() - tall.width()).abs() < 0.5, "same width");
    assert!(
        (small.width() - (WIDTH + 2.0 * f32::from(MARGIN))).abs() < 3.0,
        "{}",
        small.width()
    );
    assert!(
        tall.height() > small.height(),
        "a long message grows it downwards"
    );
    assert!(
        heights
            .windows(2)
            .skip(2)
            .all(|pair| (pair[0] - pair[1]).abs() < 0.5),
        "the height settles: {heights:?}"
    );
}

#[test]
fn a_destructive_dialog_starts_on_cancel_so_enter_destroys_nothing() {
    let mut rig = Rig::new();
    let dialog = destructive();
    rig.settle(&dialog);
    assert!(rig.focused("dismiss"));
    assert_eq!(rig.key(Key::Enter, &dialog), Some(Outcome::Dismissed));
}

#[test]
fn tab_moves_to_the_action_and_enter_then_confirms() {
    let mut rig = Rig::new();
    let dialog = destructive();
    rig.settle(&dialog);
    for _ in 0..4 {
        if rig.focused("accept") {
            break;
        }
        rig.key(Key::Tab, &dialog);
    }
    assert!(rig.focused("accept"), "Tab reaches the action");
    assert_eq!(rig.key(Key::Enter, &dialog), Some(Outcome::Accepted));
}

#[test]
fn a_dialog_that_is_not_destructive_starts_on_its_action_and_a_cautious_one_on_nothing() {
    let mut rig = Rig::new();
    let plain = spec();
    rig.settle(&plain);
    assert!(rig.focused("accept"));
    assert_eq!(rig.key(Key::Enter, &plain), Some(Outcome::Accepted));

    let mut careful = Rig::new();
    let cautious = Dialog {
        cautious: true,
        ..spec()
    };
    careful.settle(&cautious);
    assert!(!careful.focused("dismiss") && !careful.focused("accept"));
    assert_eq!(
        careful.key(Key::Enter, &cautious),
        None,
        "Enter does nothing yet"
    );
    // Tab goes left to right: Cancel, then the action.
    careful.key(Key::Tab, &cautious);
    assert!(careful.focused("dismiss"));
    careful.key(Key::Tab, &cautious);
    assert_eq!(careful.key(Key::Enter, &cautious), Some(Outcome::Accepted));
}

#[test]
fn escape_and_the_close_button_dismiss() {
    let dialog = destructive();
    let mut rig = Rig::new();
    rig.settle(&dialog);
    assert_eq!(rig.key(Key::Escape, &dialog), Some(Outcome::Dismissed));

    let mut rig = Rig::new();
    rig.settle(&dialog);
    let at = rig.rect("close").center();
    assert_eq!(rig.click(at, &dialog), Some(Outcome::Dismissed));
}

#[test]
fn clicking_the_action_accepts_and_a_disabled_action_does_not() {
    let dialog = destructive();
    let mut rig = Rig::new();
    rig.settle(&dialog);
    let at = rig.rect("accept").center();
    assert_eq!(rig.click(at, &dialog), Some(Outcome::Accepted));

    let disabled = Dialog {
        accept_enabled: false,
        ..destructive()
    };
    let mut rig = Rig::new();
    rig.settle(&disabled);
    let at = rig.rect("accept").center();
    assert_eq!(rig.click(at, &disabled), None, "a disabled button is inert");
}

#[test]
fn a_leading_button_is_on_the_left_and_answers_for_itself() {
    let dialog = Dialog {
        leading: Some("Don't Save"),
        accept: "Save",
        ..spec()
    };
    let mut rig = Rig::new();
    rig.settle(&dialog);
    let (leading, cancel, accept) = (rig.rect("leading"), rig.rect("dismiss"), rig.rect("accept"));
    assert!(leading.right() < cancel.left() && cancel.right() < accept.left());
    let at = leading.center();
    assert_eq!(rig.click(at, &dialog), Some(Outcome::Leading));
}

#[test]
fn a_notice_has_one_button_and_enter_dismisses_it() {
    let dialog = Dialog {
        dismiss: "",
        ..Dialog::new(
            Icon::Triangle,
            theme::ORANGE,
            "Already in Watchlist",
            "It is here.",
            "OK",
        )
    };
    let mut rig = Rig::new();
    rig.settle(&dialog);
    assert!(rig.ctx.read_response(Id::new(ID).with("dismiss")).is_none());
    assert_eq!(rig.key(Key::Enter, &dialog), Some(Outcome::Accepted));
}

#[test]
fn reopening_a_dialog_asks_for_the_focus_again() {
    let dialog = destructive();
    let mut rig = Rig::new();
    rig.settle(&dialog);
    rig.key(Key::Tab, &dialog);
    for _ in 0..3 {
        rig.frame(Vec::new(), None);
    }
    rig.settle(&dialog);
    assert!(rig.focused("dismiss"), "it starts on Cancel again");
}

/// Pictures of the dialogs: destructive short and long, an action that asks a question, a notice,
/// a disabled action and the three-button save prompt, in both themes.
#[test]
#[ignore = "writes images for a person to look at"]
fn gallery_of_the_dialogs() {
    use crate::ui::render_tests::{Screen, gallery_dir};
    if gallery_dir().is_none() {
        return;
    }
    let _restore = theme::RestoreDefaultTheme;
    let cases: Vec<(&str, Dialog)> = vec![
        (
            "remove-indicators",
            Dialog {
                message: "4 indicators, the 2 compared symbols included, will be removed from this chart. Saved scripts are kept, and you can undo it.",
                ..destructive()
            },
        ),
        (
            "short",
            Dialog {
                title: "Delete this alert?",
                message: "The BTCUSD alert is removed. Its trigger history is kept.",
                accept: "Delete",
                ..destructive()
            },
        ),
        (
            "long",
            Dialog {
                title: "Turn off approvals?",
                message: "Any program that holds a credential will be able to place and change paper orders and edit your charts without asking. This is only safe if you trust every client you have connected, and you can turn approvals back on at any time.",
                accept: "Turn Off",
                icon: Icon::LockFill,
                ..destructive()
            },
        ),
        (
            "question",
            Dialog {
                icon: Icon::Cycle,
                tint: theme::ORANGE,
                title: "Restart to finish updating?",
                message: "DegenView 0.6.0 is ready. DegenView will close and reopen.",
                accept: "Restart Now",
                dismiss: "Later",
                ..spec()
            },
        ),
        (
            "notice",
            Dialog {
                icon: Icon::Triangle,
                tint: theme::ORANGE,
                title: "Already in Watchlist",
                message: "\u{201c}BTC\u{201d} is already in this watchlist.",
                accept: "OK",
                dismiss: "",
                ..spec()
            },
        ),
        (
            "save",
            Dialog {
                icon: Icon::Clipboard,
                tint: theme::ORANGE,
                title: "Save changes to \u{201c}Morning\u{201d}?",
                message: "Your changes are lost if you don't save them.",
                accept: "Save",
                leading: Some("Don't Save"),
                ..spec()
            },
        ),
        (
            "disabled",
            Dialog {
                title: "Delete iCloud data?",
                message: "Type DELETE to confirm.",
                accept: "Delete Everything",
                accept_enabled: false,
                ..destructive()
            },
        ),
    ];
    for (name, dialog) in &cases {
        for light in [false, true] {
            let mut screen = Screen::new(vec2(560.0, 300.0));
            screen.ctx.set_theme(if light {
                egui::ThemePreference::Light
            } else {
                egui::ThemePreference::Dark
            });
            let mut last = None;
            for _ in 0..5 {
                last = Some(screen.run(Vec::new(), |ui| {
                    theme::sync(ui.ctx());
                    show(ui.ctx(), Id::new(ID), dialog, |_| {});
                }));
            }
            screen.save(
                &format!("dialog-{name}-{}", if light { "light" } else { "dark" }),
                last.expect("ran"),
            );
        }
    }
}

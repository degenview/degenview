//! The app's keyboard shortcuts, one table, resolved per platform.
//!
//! egui's `Modifiers::command` is ⌘ on macOS but Ctrl everywhere else, and Ctrl also sets `ctrl`
//! on Windows/Linux — so hand-tested `command && !ctrl` conditions are wrong off macOS. Every
//! shortcut is declared here with its macOS and Windows/Linux chords; call sites ask
//! [`Shortcut::pressed`] and show [`Shortcut::label`] / [`Shortcut::hint`], so the key that works
//! and the key the UI names can never drift apart.

use eframe::egui::{Event, InputState, Key, Modifiers};

/// One key combination. On macOS `primary` is ⌘ and `control` is ⌃; elsewhere either one means Ctrl.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Chord {
    primary: bool,
    control: bool,
    alt: bool,
    shift: bool,
    key: Key,
}

impl Chord {
    const fn new(key: Key) -> Self {
        Self {
            primary: false,
            control: false,
            alt: false,
            shift: false,
            key,
        }
    }

    /// ⌘ on macOS, Ctrl elsewhere.
    const fn primary(mut self) -> Self {
        self.primary = true;
        self
    }

    /// A literal Control key (⌃ on macOS).
    const fn control(mut self) -> Self {
        self.control = true;
        self
    }

    const fn alt(mut self) -> Self {
        self.alt = true;
        self
    }

    const fn shift(mut self) -> Self {
        self.shift = true;
        self
    }

    /// Whether `modifiers` + `key` is exactly this chord. Extra modifiers do not match.
    fn matches(self, is_mac: bool, key: Key, modifiers: Modifiers) -> bool {
        if key != self.key || modifiers.alt != self.alt || modifiers.shift != self.shift {
            return false;
        }
        if is_mac {
            modifiers.mac_cmd == self.primary && modifiers.ctrl == self.control
        } else {
            modifiers.ctrl == (self.primary || self.control)
        }
    }

    fn label(self, is_mac: bool) -> String {
        let key = key_name(self.key);
        if is_mac {
            // The system's own order: ⌃ ⌥ ⇧ ⌘.
            let mut text = String::new();
            for (on, glyph) in [
                (self.control, '⌃'),
                (self.alt, '⌥'),
                (self.shift, '⇧'),
                (self.primary, '⌘'),
            ] {
                if on {
                    text.push(glyph);
                }
            }
            text.push_str(key);
            text
        } else {
            let mut parts = Vec::new();
            if self.primary || self.control {
                parts.push("Ctrl");
            }
            if self.shift {
                parts.push("Shift");
            }
            if self.alt {
                parts.push("Alt");
            }
            parts.push(key);
            parts.join("+")
        }
    }
}

fn key_name(key: Key) -> &'static str {
    match key {
        Key::ArrowUp => "↑",
        Key::ArrowDown => "↓",
        Key::Slash => "/",
        Key::Space => "Space",
        Key::Escape => "Esc",
        Key::Period => ".",
        Key::R => "R",
        Key::B => "B",
        Key::C => "C",
        Key::D => "D",
        Key::F => "F",
        Key::G => "G",
        Key::H => "H",
        Key::J => "J",
        Key::L => "L",
        Key::P => "P",
        Key::S => "S",
        Key::T => "T",
        Key::Tab => "Tab",
        Key::V => "V",
        Key::W => "W",
        Key::Y => "Y",
        Key::Z => "Z",
        Key::F3 => "F3",
        Key::F5 => "F5",
        Key::F6 => "F6",
        Key::F7 => "F7",
        Key::F8 => "F8",
        Key::F9 => "F9",
        Key::F10 => "F10",
        Key::F11 => "F11",
        _ => "?",
    }
}

/// An action's chords per platform. The first chord is the one shown; the rest are aliases.
#[derive(Clone, Copy, Debug)]
pub struct Shortcut {
    mac: &'static [Chord],
    other: &'static [Chord],
}

impl Shortcut {
    const fn new(mac: &'static [Chord], other: &'static [Chord]) -> Self {
        Self { mac, other }
    }

    fn chords(&self, is_mac: bool) -> &'static [Chord] {
        if is_mac { self.mac } else { self.other }
    }

    pub fn matches_for(&self, is_mac: bool, key: Key, modifiers: Modifiers) -> bool {
        self.chords(is_mac)
            .iter()
            .any(|chord| chord.matches(is_mac, key, modifiers))
    }

    /// Whether this shortcut's key event is among `input`'s this frame. Leaves the event in place.
    pub fn peek(&self, input: &InputState) -> bool {
        self.matches_event(input.events.iter())
    }

    /// Like [`Self::peek`], and takes the event so no other widget sees it.
    pub fn consume(&self, input: &mut InputState) -> bool {
        let mut pressed = false;
        input.events.retain(|event| {
            let hit = self.matches_event(std::iter::once(event));
            pressed |= hit;
            !hit
        });
        pressed
    }

    fn matches_event<'a>(&self, mut events: impl Iterator<Item = &'a Event>) -> bool {
        events.any(|event| {
            matches!(
                event,
                Event::Key { key, pressed: true, modifiers, .. }
                    if self.matches_for(is_mac(), *key, *modifiers)
            )
        })
    }

    pub fn label_for(&self, is_mac: bool) -> String {
        self.chords(is_mac)
            .first()
            .map(|chord| chord.label(is_mac))
            .unwrap_or_default()
    }

    /// The chord to show on this platform, like `⌘S` or `Ctrl+S`.
    pub fn label(&self) -> String {
        self.label_for(is_mac())
    }

    /// `text` with the chord appended: "Save the script (⌘S)".
    pub fn hint(&self, text: &str) -> String {
        format!("{text} ({})", self.label())
    }
}

/// Whether the platform uses ⌘-style shortcuts. Matches egui's own `is_mac`.
pub const fn is_mac() -> bool {
    cfg!(target_os = "macos")
}

const S: Chord = Chord::new(Key::S).primary();

/// Save the script or the layout.
pub const SAVE: Shortcut = Shortcut::new(&[S], &[S]);

/// Open a new tab. macOS gets ⌘T from File ▸ New Tab; this is for every other platform.
pub const NEW_TAB: Shortcut = Shortcut::new(
    &[Chord::new(Key::T).primary()],
    &[Chord::new(Key::T).primary()],
);

/// Close the tab in front. macOS gets ⌘W from File ▸ Close Tab.
pub const CLOSE_TAB: Shortcut = Shortcut::new(
    &[Chord::new(Key::W).primary()],
    &[Chord::new(Key::W).primary()],
);

/// Show the next tab of the window.
pub const NEXT_TAB: Shortcut = Shortcut::new(
    &[Chord::new(Key::Tab).control()],
    &[Chord::new(Key::Tab).primary()],
);

/// Show the previous tab of the window.
pub const PREVIOUS_TAB: Shortcut = Shortcut::new(
    &[Chord::new(Key::Tab).control().shift()],
    &[Chord::new(Key::Tab).primary().shift()],
);

/// Show or hide the Script Manager's script list.
pub const TOGGLE_SCRIPT_LIST: Shortcut = Shortcut::new(
    &[Chord::new(Key::S).control().primary()],
    &[Chord::new(Key::B).primary()],
);

/// Show or hide the Script Manager's preview chart.
pub const TOGGLE_PREVIEW_CHART: Shortcut = Shortcut::new(
    &[Chord::new(Key::P).alt().primary()],
    &[Chord::new(Key::P).primary().shift()],
);

pub const FIND: Shortcut = Shortcut::new(
    &[Chord::new(Key::F).primary()],
    &[Chord::new(Key::F).primary()],
);

pub const FIND_NEXT: Shortcut = Shortcut::new(
    &[Chord::new(Key::G).primary()],
    &[Chord::new(Key::F3), Chord::new(Key::G).primary()],
);

pub const FIND_PREVIOUS: Shortcut = Shortcut::new(
    &[Chord::new(Key::G).primary().shift()],
    &[
        Chord::new(Key::F3).shift(),
        Chord::new(Key::G).primary().shift(),
    ],
);

pub const TOGGLE_COMMENT: Shortcut = Shortcut::new(
    &[Chord::new(Key::Slash).primary()],
    &[Chord::new(Key::Slash).primary()],
);

pub const MOVE_LINE_UP: Shortcut = Shortcut::new(
    &[Chord::new(Key::ArrowUp).alt()],
    &[Chord::new(Key::ArrowUp).alt()],
);
pub const MOVE_LINE_DOWN: Shortcut = Shortcut::new(
    &[Chord::new(Key::ArrowDown).alt()],
    &[Chord::new(Key::ArrowDown).alt()],
);
pub const DUPLICATE_LINE_UP: Shortcut = Shortcut::new(
    &[Chord::new(Key::ArrowUp).alt().shift()],
    &[Chord::new(Key::ArrowUp).alt().shift()],
);
pub const DUPLICATE_LINE_DOWN: Shortcut = Shortcut::new(
    &[Chord::new(Key::ArrowDown).alt().shift()],
    &[Chord::new(Key::ArrowDown).alt().shift()],
);

/// Open the completion list on demand. Alt+Esc is the Windows window switcher, so it is Mac-only.
/// F5 was an alias until the debugger took it: it starts and continues a debugging session.
pub const COMPLETE: Shortcut = Shortcut::new(
    &[
        Chord::new(Key::Space).control(),
        Chord::new(Key::Escape).alt(),
    ],
    &[Chord::new(Key::Space).control()],
);

// The debugger. Off the Mac the keys are Visual Studio Code's; on the Mac, Xcode's, because F5 and
// F10/F11 belong to the system there (and F11 to Show Desktop). F9 sets a breakpoint on both.

/// Starts debugging, or continues a stopped script.
pub const DEBUG_CONTINUE: Shortcut = Shortcut::new(
    &[Chord::new(Key::Y).control().primary()],
    &[Chord::new(Key::F5)],
);
/// Stops a running script at its next statement.
pub const DEBUG_PAUSE: Shortcut = Shortcut::new(
    &[Chord::new(Key::P).control().primary()],
    &[Chord::new(Key::F6)],
);
pub const DEBUG_STEP_OVER: Shortcut =
    Shortcut::new(&[Chord::new(Key::F6)], &[Chord::new(Key::F10)]);
pub const DEBUG_STEP_INTO: Shortcut =
    Shortcut::new(&[Chord::new(Key::F7)], &[Chord::new(Key::F11)]);
pub const DEBUG_STEP_OUT: Shortcut =
    Shortcut::new(&[Chord::new(Key::F8)], &[Chord::new(Key::F11).shift()]);
/// Runs to the first statement of the next bar.
pub const DEBUG_STEP_BAR: Shortcut = Shortcut::new(
    &[Chord::new(Key::B).control().primary()],
    &[Chord::new(Key::F7)],
);
pub const DEBUG_STOP: Shortcut = Shortcut::new(
    &[Chord::new(Key::Period).primary()],
    &[Chord::new(Key::F5).shift()],
);
pub const DEBUG_RESTART: Shortcut = Shortcut::new(
    &[Chord::new(Key::R).primary().shift()],
    &[Chord::new(Key::F5).primary().shift()],
);
/// Sets or clears a breakpoint on the caret's line.
pub const DEBUG_TOGGLE_BREAKPOINT: Shortcut =
    Shortcut::new(&[Chord::new(Key::F9)], &[Chord::new(Key::F9)]);

/// Duplicates the selected drawings.
pub const DUPLICATE_DRAWING: Shortcut = Shortcut::new(
    &[Chord::new(Key::D).primary()],
    &[Chord::new(Key::D).primary()],
);

/// Copies the selected drawings. The platform reports this as an `Event::Copy`, not a key, so
/// the chart answers that event; the chord is here so the menus print the real binding.
pub const COPY: Shortcut = Shortcut::new(
    &[Chord::new(Key::C).primary()],
    &[Chord::new(Key::C).primary()],
);

/// Pastes copied drawings (an `Event::Paste`, see [`COPY`]).
pub const PASTE: Shortcut = Shortcut::new(
    &[Chord::new(Key::V).primary()],
    &[Chord::new(Key::V).primary()],
);

/// Arms a line tool: Alt (Option on a Mac) and a letter, as on TradingView. The J of the
/// horizontal ray is TradingView's default, which none of its help pages states.
pub const TOOL_TREND_LINE: Shortcut =
    Shortcut::new(&[Chord::new(Key::T).alt()], &[Chord::new(Key::T).alt()]);
pub const TOOL_HORIZONTAL_LINE: Shortcut =
    Shortcut::new(&[Chord::new(Key::H).alt()], &[Chord::new(Key::H).alt()]);
pub const TOOL_HORIZONTAL_RAY: Shortcut =
    Shortcut::new(&[Chord::new(Key::J).alt()], &[Chord::new(Key::J).alt()]);
pub const TOOL_VERTICAL_LINE: Shortcut =
    Shortcut::new(&[Chord::new(Key::V).alt()], &[Chord::new(Key::V).alt()]);
pub const TOOL_CROSSLINE: Shortcut =
    Shortcut::new(&[Chord::new(Key::C).alt()], &[Chord::new(Key::C).alt()]);
pub const TOOL_FIB_RETRACEMENT: Shortcut =
    Shortcut::new(&[Chord::new(Key::F).alt()], &[Chord::new(Key::F).alt()]);

/// The shortcut that arms `tool`, for the tools that have one.
pub fn tool(tool: crate::model::Tool) -> Option<Shortcut> {
    match tool.hotkey()? {
        'T' => Some(TOOL_TREND_LINE),
        'H' => Some(TOOL_HORIZONTAL_LINE),
        'J' => Some(TOOL_HORIZONTAL_RAY),
        'V' => Some(TOOL_VERTICAL_LINE),
        'C' => Some(TOOL_CROSSLINE),
        'F' => Some(TOOL_FIB_RETRACEMENT),
        _ => None,
    }
}

/// Hides or shows every drawing on the chart.
pub const HIDE_DRAWINGS: Shortcut = Shortcut::new(
    &[Chord::new(Key::H).control().alt()],
    &[Chord::new(Key::H).control().alt()],
);

/// Saves the focused chart's snapshot as an image file.
pub const SNAPSHOT_DOWNLOAD: Shortcut = Shortcut::new(
    &[Chord::new(Key::S).primary().alt()],
    &[Chord::new(Key::S).primary().alt()],
);

/// Puts the focused chart's snapshot on the clipboard as an image.
pub const SNAPSHOT_COPY_IMAGE: Shortcut = Shortcut::new(
    &[Chord::new(Key::S).primary().shift()],
    &[Chord::new(Key::S).primary().shift()],
);

/// Uploads the focused chart's snapshot and copies its link.
pub const SNAPSHOT_COPY_LINK: Shortcut =
    Shortcut::new(&[Chord::new(Key::S).alt()], &[Chord::new(Key::S).alt()]);

/// The shortcut that takes `action`'s snapshot, for the rows that have one.
pub fn snapshot(action: crate::model::snapshot::SnapshotAction) -> Option<Shortcut> {
    use crate::model::snapshot::SnapshotAction;
    match action {
        SnapshotAction::Download => Some(SNAPSHOT_DOWNLOAD),
        SnapshotAction::CopyImage => Some(SNAPSHOT_COPY_IMAGE),
        SnapshotAction::CopyLink => Some(SNAPSHOT_COPY_LINK),
        SnapshotAction::OpenInTab | SnapshotAction::ShareOnX => None,
    }
}

/// Switches the focused chart's price scale to Percent, or back to Regular when it already is.
/// TradingView's Alt+P.
pub const SCALE_PERCENT: Shortcut =
    Shortcut::new(&[Chord::new(Key::P).alt()], &[Chord::new(Key::P).alt()]);

/// Switches the focused chart's price scale to Logarithmic, or back to Regular when it already
/// is. TradingView's Alt+L.
pub const SCALE_LOGARITHMIC: Shortcut =
    Shortcut::new(&[Chord::new(Key::L).alt()], &[Chord::new(Key::L).alt()]);

/// The shortcut that switches to `mode`, for the modes that have one. TradingView documents none
/// for Indexed to 100, so none is invented.
pub fn scale_mode(mode: crate::model::price_scale::PriceScaleMode) -> Option<Shortcut> {
    use crate::model::price_scale::PriceScaleMode;
    match mode {
        PriceScaleMode::Percent => Some(SCALE_PERCENT),
        PriceScaleMode::Logarithmic => Some(SCALE_LOGARITHMIC),
        PriceScaleMode::Regular | PriceScaleMode::IndexedTo100 => None,
    }
}

pub const UNDO: Shortcut = Shortcut::new(
    &[Chord::new(Key::Z).primary()],
    &[Chord::new(Key::Z).primary()],
);

pub const REDO: Shortcut = Shortcut::new(
    &[Chord::new(Key::Z).primary().shift()],
    &[
        Chord::new(Key::Y).primary(),
        Chord::new(Key::Z).primary().shift(),
    ],
);

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [(&str, Shortcut); 42] = [
        ("debug continue", DEBUG_CONTINUE),
        ("debug pause", DEBUG_PAUSE),
        ("debug step over", DEBUG_STEP_OVER),
        ("debug step into", DEBUG_STEP_INTO),
        ("debug step out", DEBUG_STEP_OUT),
        ("debug step bar", DEBUG_STEP_BAR),
        ("debug stop", DEBUG_STOP),
        ("debug restart", DEBUG_RESTART),
        ("debug breakpoint", DEBUG_TOGGLE_BREAKPOINT),
        ("save", SAVE),
        ("new tab", NEW_TAB),
        ("close tab", CLOSE_TAB),
        ("next tab", NEXT_TAB),
        ("previous tab", PREVIOUS_TAB),
        ("script list", TOGGLE_SCRIPT_LIST),
        ("preview chart", TOGGLE_PREVIEW_CHART),
        ("find", FIND),
        ("find next", FIND_NEXT),
        ("find previous", FIND_PREVIOUS),
        ("comment", TOGGLE_COMMENT),
        ("move up", MOVE_LINE_UP),
        ("move down", MOVE_LINE_DOWN),
        ("duplicate up", DUPLICATE_LINE_UP),
        ("duplicate down", DUPLICATE_LINE_DOWN),
        ("complete", COMPLETE),
        ("duplicate drawing", DUPLICATE_DRAWING),
        ("copy", COPY),
        ("paste", PASTE),
        ("hide drawings", HIDE_DRAWINGS),
        ("snapshot download", SNAPSHOT_DOWNLOAD),
        ("snapshot copy image", SNAPSHOT_COPY_IMAGE),
        ("snapshot copy link", SNAPSHOT_COPY_LINK),
        ("percent scale", SCALE_PERCENT),
        ("logarithmic scale", SCALE_LOGARITHMIC),
        ("undo", UNDO),
        ("redo", REDO),
        ("trend line", TOOL_TREND_LINE),
        ("horizontal line", TOOL_HORIZONTAL_LINE),
        ("horizontal ray", TOOL_HORIZONTAL_RAY),
        ("vertical line", TOOL_VERTICAL_LINE),
        ("crossline", TOOL_CROSSLINE),
        ("fib retracement", TOOL_FIB_RETRACEMENT),
    ];

    #[test]
    fn every_tool_with_a_hotkey_has_a_shortcut_and_the_two_agree() {
        use crate::model::Tool;
        for tool in Tool::GROUPED {
            let shortcut = super::tool(tool);
            assert_eq!(shortcut.is_some(), tool.hotkey().is_some(), "{tool:?}");
            if let (Some(shortcut), Some(letter)) = (shortcut, tool.hotkey()) {
                // Alt (Option) and the letter, spelled the same on both platforms.
                assert!(shortcut.label_for(true).ends_with(letter), "{tool:?}");
                assert!(shortcut.label_for(true).starts_with('⌥'), "{tool:?}");
                assert_eq!(
                    shortcut.label_for(false),
                    format!("Alt+{letter}"),
                    "{tool:?}"
                );
            }
        }
    }

    /// What a Mac reports for ⌘ held (`command` mirrors `mac_cmd`).
    fn cmd() -> Modifiers {
        Modifiers {
            mac_cmd: true,
            command: true,
            ..Modifiers::default()
        }
    }

    /// What Windows/Linux report for Ctrl held (`command` mirrors `ctrl`).
    fn ctrl() -> Modifiers {
        Modifiers {
            ctrl: true,
            command: true,
            ..Modifiers::default()
        }
    }

    #[test]
    fn labels_follow_each_platform() {
        assert_eq!(SAVE.label_for(true), "⌘S");
        assert_eq!(SAVE.label_for(false), "Ctrl+S");
        assert_eq!(TOGGLE_SCRIPT_LIST.label_for(true), "⌃⌘S");
        assert_eq!(TOGGLE_SCRIPT_LIST.label_for(false), "Ctrl+B");
        assert_eq!(TOGGLE_PREVIEW_CHART.label_for(true), "⌥⌘P");
        assert_eq!(TOGGLE_PREVIEW_CHART.label_for(false), "Ctrl+Shift+P");
        assert_eq!(FIND_PREVIOUS.label_for(true), "⇧⌘G");
        assert_eq!(FIND_PREVIOUS.label_for(false), "Shift+F3");
        assert_eq!(DUPLICATE_LINE_DOWN.label_for(true), "⌥⇧↓");
        assert_eq!(DUPLICATE_LINE_DOWN.label_for(false), "Shift+Alt+↓");
        assert_eq!(COMPLETE.label_for(true), "⌃Space");
        assert_eq!(COMPLETE.label_for(false), "Ctrl+Space");
        assert_eq!(NEXT_TAB.label_for(true), "⌃Tab");
        assert_eq!(PREVIOUS_TAB.label_for(false), "Ctrl+Shift+Tab");
        assert_eq!(CLOSE_TAB.label_for(true), "⌘W");
        assert_eq!(
            SAVE.hint("Save the script"),
            format!("Save the script ({})", SAVE.label())
        );
    }

    #[test]
    fn every_action_has_a_chord_and_a_label_on_each_platform() {
        for (name, shortcut) in ALL {
            for is_mac in [true, false] {
                assert!(!shortcut.chords(is_mac).is_empty(), "{name} unbound");
                assert!(
                    !shortcut.label_for(is_mac).contains('?'),
                    "{name} unnamed key"
                );
            }
        }
    }

    #[test]
    fn no_two_actions_share_a_chord() {
        for is_mac in [true, false] {
            let mut seen: Vec<(Chord, &str)> = Vec::new();
            for (name, shortcut) in &ALL {
                for chord in shortcut.chords(is_mac) {
                    if let Some((_, other)) = seen.iter().find(|(c, _)| c == chord) {
                        panic!(
                            "{name} and {other} share {} (mac: {is_mac})",
                            chord.label(is_mac)
                        );
                    }
                    seen.push((*chord, name));
                }
            }
        }
    }

    #[test]
    fn save_is_cmd_s_on_mac_and_ctrl_s_elsewhere() {
        assert!(SAVE.matches_for(true, Key::S, cmd()));
        assert!(!SAVE.matches_for(true, Key::S, ctrl_only_mac()));
        assert!(SAVE.matches_for(false, Key::S, ctrl()));
        assert!(!SAVE.matches_for(false, Key::S, Modifiers::default()));
    }

    #[test]
    fn the_snapshot_shortcuts_are_their_own_and_leave_save_alone() {
        use crate::model::snapshot::SnapshotAction;
        assert_eq!(SNAPSHOT_DOWNLOAD.label_for(true), "⌥⌘S");
        assert_eq!(SNAPSHOT_COPY_IMAGE.label_for(true), "⇧⌘S");
        assert_eq!(SNAPSHOT_COPY_LINK.label_for(true), "⌥S");
        assert_eq!(SNAPSHOT_DOWNLOAD.label_for(false), "Ctrl+Alt+S");
        assert_eq!(SNAPSHOT_COPY_IMAGE.label_for(false), "Ctrl+Shift+S");
        assert_eq!(SNAPSHOT_COPY_LINK.label_for(false), "Alt+S");
        let with = |base: Modifiers, alt: bool, shift: bool| Modifiers { alt, shift, ..base };
        for (is_mac, base) in [(true, cmd()), (false, ctrl())] {
            let pressed = [
                (with(base, true, false), Some(SNAPSHOT_DOWNLOAD)),
                (with(base, false, true), Some(SNAPSHOT_COPY_IMAGE)),
                (with(base, false, false), None),
                (
                    with(Modifiers::default(), true, false),
                    Some(SNAPSHOT_COPY_LINK),
                ),
            ];
            for (modifiers, wanted) in pressed {
                for (name, shortcut) in [
                    ("download", SNAPSHOT_DOWNLOAD),
                    ("copy image", SNAPSHOT_COPY_IMAGE),
                    ("copy link", SNAPSHOT_COPY_LINK),
                ] {
                    let hit = shortcut.matches_for(is_mac, Key::S, modifiers);
                    let expected =
                        wanted.is_some_and(|w| w.label_for(is_mac) == shortcut.label_for(is_mac));
                    assert_eq!(hit, expected, "{name} on {modifiers:?} (mac: {is_mac})");
                }
            }
            // Plain Save, with the same key, is not any of them.
            assert!(SAVE.matches_for(is_mac, Key::S, base));
        }
        assert_eq!(
            snapshot(SnapshotAction::OpenInTab).map(|s| s.label_for(true)),
            None
        );
        assert_eq!(
            snapshot(SnapshotAction::CopyLink).map(|s| s.label_for(true)),
            Some("⌥S".to_owned())
        );
    }

    /// A Mac's literal Control key (no ⌘).
    fn ctrl_only_mac() -> Modifiers {
        Modifiers {
            ctrl: true,
            ..Modifiers::default()
        }
    }

    #[test]
    fn script_list_chord_does_not_also_save() {
        let control_cmd = Modifiers {
            ctrl: true,
            ..cmd()
        };
        assert!(TOGGLE_SCRIPT_LIST.matches_for(true, Key::S, control_cmd));
        assert!(!SAVE.matches_for(true, Key::S, control_cmd));
        // On Windows Ctrl+S is only ever Save; the list moved to Ctrl+B.
        assert!(!TOGGLE_SCRIPT_LIST.matches_for(false, Key::S, ctrl()));
        assert!(TOGGLE_SCRIPT_LIST.matches_for(false, Key::B, ctrl()));
    }

    #[test]
    fn extra_modifiers_do_not_trigger() {
        let ctrl_alt = Modifiers {
            alt: true,
            ..ctrl()
        };
        assert!(!SAVE.matches_for(false, Key::S, ctrl_alt));
        assert!(!UNDO.matches_for(
            false,
            Key::Z,
            Modifiers {
                shift: true,
                ..ctrl()
            }
        ));
        assert!(REDO.matches_for(
            false,
            Key::Z,
            Modifiers {
                shift: true,
                ..ctrl()
            }
        ));
        assert!(REDO.matches_for(false, Key::Y, ctrl()));
        assert!(!REDO.matches_for(true, Key::Y, cmd()));
    }

    #[test]
    fn completion_accepts_alt_escape_only_on_mac() {
        let alt = Modifiers {
            alt: true,
            ..Modifiers::default()
        };
        assert!(COMPLETE.matches_for(true, Key::Escape, alt));
        assert!(!COMPLETE.matches_for(false, Key::Escape, alt));
        // F5 now continues a debugging session, so it no longer opens the list.
        assert!(!COMPLETE.matches_for(false, Key::F5, Modifiers::default()));
        assert!(!COMPLETE.matches_for(true, Key::F5, Modifiers::default()));
        assert!(COMPLETE.matches_for(false, Key::Space, ctrl()));
    }

    #[test]
    fn the_debugger_uses_vs_code_keys_off_the_mac_and_xcode_keys_on_it() {
        let none = Modifiers::default();
        let shift = Modifiers {
            shift: true,
            ..none
        };
        assert!(DEBUG_CONTINUE.matches_for(false, Key::F5, none));
        assert!(DEBUG_STOP.matches_for(false, Key::F5, shift));
        assert!(DEBUG_STEP_OVER.matches_for(false, Key::F10, none));
        assert!(DEBUG_STEP_INTO.matches_for(false, Key::F11, none));
        assert!(DEBUG_STEP_OUT.matches_for(false, Key::F11, shift));
        assert!(DEBUG_TOGGLE_BREAKPOINT.matches_for(false, Key::F9, none));
        assert!(DEBUG_TOGGLE_BREAKPOINT.matches_for(true, Key::F9, none));
        assert!(DEBUG_STEP_OVER.matches_for(true, Key::F6, none));
        assert!(DEBUG_STEP_INTO.matches_for(true, Key::F7, none));
        assert!(DEBUG_STEP_OUT.matches_for(true, Key::F8, none));
        assert_eq!(DEBUG_CONTINUE.label_for(true), "⌃⌘Y");
        assert_eq!(DEBUG_CONTINUE.label_for(false), "F5");
        assert_eq!(DEBUG_STOP.label_for(true), "⌘.");
        assert_eq!(DEBUG_STOP.label_for(false), "Shift+F5");
        assert_eq!(DEBUG_RESTART.label_for(false), "Ctrl+Shift+F5");
        assert_eq!(DEBUG_STEP_OUT.label_for(false), "Shift+F11");
        // F11 is the system's on a Mac: Step Into is F7 there.
        assert!(!DEBUG_STEP_INTO.matches_for(true, Key::F11, none));
    }

    #[test]
    fn find_steps_with_f3_off_mac() {
        let shift = Modifiers {
            shift: true,
            ..Modifiers::default()
        };
        assert!(FIND_NEXT.matches_for(false, Key::F3, Modifiers::default()));
        assert!(FIND_PREVIOUS.matches_for(false, Key::F3, shift));
        assert!(FIND_NEXT.matches_for(false, Key::G, ctrl()));
        assert!(!FIND_NEXT.matches_for(true, Key::F3, Modifiers::default()));
    }
}

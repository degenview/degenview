//! "Save changes?" before a Pine Studio tab closes or the app quits over edited scripts.
//!
//! The data side (`UnsavedScript`, `lines_changed`) is pure; `show` is the sheet. The Pine
//! Studio owns the state and acts on the choice, so a failed save can keep the window open.

use crate::pine_studio::model::{ScriptId, ScriptType};
use crate::ui::dialog;
use crate::ui::icons::{self, Icon};
use crate::ui::kit::{self, MEDIUM, REGULAR, size};
use crate::ui::theme;
use eframe::egui::{self, Align, Color32, Id, Key, Layout, Modifiers, Sense, Stroke, Ui, vec2};

const WIDTH: f32 = 440.0;
/// Rows listed before the rest fold into "and N more".
const MAX_ROWS: usize = 5;

/// What is being closed over the edits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExitReason {
    CloseTab,
    Quit,
}

impl ExitReason {
    fn consequence(self) -> &'static str {
        match self {
            Self::CloseTab => "Your changes will be lost if you close this tab without saving.",
            Self::Quit => "Your changes will be lost if you quit DegenView without saving.",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnsavedChoice {
    Save,
    Discard,
    Cancel,
}

/// How the question ended, for the app that asked it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExitDecision {
    pub reason: ExitReason,
    /// True once the edits are saved or given up, so the close or quit may go ahead.
    pub proceed: bool,
}

/// A script whose text differs from what is saved, and by how much.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnsavedScript {
    pub id: ScriptId,
    pub name: String,
    pub script_type: ScriptType,
    pub added: usize,
    pub removed: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExitPrompt {
    pub reason: ExitReason,
    pub scripts: Vec<UnsavedScript>,
}

/// Lines added and removed going from `saved` to `current`: what the two share at the start and
/// end is skipped, the middle counts as replaced. A coarse summary for the sheet, not a diff.
pub fn lines_changed(saved: &str, current: &str) -> (usize, usize) {
    let before: Vec<&str> = saved.lines().collect();
    let after: Vec<&str> = current.lines().collect();
    let shared_start = before
        .iter()
        .zip(&after)
        .take_while(|(old, new)| old == new)
        .count();
    let shared_end = before[shared_start..]
        .iter()
        .rev()
        .zip(after[shared_start..].iter().rev())
        .take_while(|(old, new)| old == new)
        .count();
    (
        after.len() - shared_start - shared_end,
        before.len() - shared_start - shared_end,
    )
}

fn title(scripts: &[UnsavedScript]) -> String {
    match scripts {
        [one] => format!("Save changes to “{}”?", one.name),
        many => format!("Save changes to {} scripts?", many.len()),
    }
}

/// The dialog: Don't Save on the left, Cancel and Save on the right. Enter saves, Escape cancels,
/// ⌘D is “Don’t Save” as in any macOS save sheet.
pub fn show(ctx: &egui::Context, prompt: &ExitPrompt) -> Option<UnsavedChoice> {
    let title = title(&prompt.scripts);
    let mut spec = dialog::Dialog::new(
        Icon::Curlybraces,
        theme::ORANGE,
        &title,
        prompt.reason.consequence(),
        if prompt.scripts.len() > 1 {
            "Save All"
        } else {
            "Save"
        },
    );
    spec.leading = Some("Don’t Save");
    let mut choice = dialog::show(ctx, Id::new("script-unsaved"), &spec, |ui| {
        ui.add_space(18.0);
        list(ui, &prompt.scripts);
    })
    .map(|outcome| match outcome {
        dialog::Outcome::Accepted => UnsavedChoice::Save,
        dialog::Outcome::Dismissed => UnsavedChoice::Cancel,
        dialog::Outcome::Leading => UnsavedChoice::Discard,
    });
    if choice.is_none() {
        choice = ctx.input_mut(|input| {
            input
                .consume_key(Modifiers::COMMAND, Key::D)
                .then_some(UnsavedChoice::Discard)
        });
    }
    choice
}

/// The edited scripts in a rounded well: type glyph, name, and the size of the edit.
fn list(ui: &mut Ui, scripts: &[UnsavedScript]) {
    crate::ui::kit::card_frame(false)
        .inner_margin(egui::Margin::symmetric(12, 4))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            let shown = &scripts[..scripts.len().min(MAX_ROWS)];
            for (index, script) in shown.iter().enumerate() {
                if index > 0 {
                    ui.painter().hline(
                        ui.max_rect().x_range(),
                        ui.cursor().top(),
                        Stroke::new(1.0, theme::ink(10)),
                    );
                }
                row(ui, script);
            }
            if scripts.len() > shown.len() {
                let rest = scripts.len() - shown.len();
                ui.allocate_ui_with_layout(
                    vec2(ui.available_width(), 30.0),
                    Layout::left_to_right(Align::Center),
                    |ui| {
                        ui.add_space(26.0);
                        ui.label(kit::text(
                            format!("and {rest} more"),
                            size::SUBHEADLINE + 1.0,
                            REGULAR,
                            theme::text_secondary(),
                        ));
                    },
                );
            }
        });
}

fn row(ui: &mut Ui, script: &UnsavedScript) {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::hover());
    let painter = ui.painter();
    let glyph = egui::Rect::from_center_size(
        egui::pos2(rect.left() + 7.0, rect.center().y),
        vec2(14.0, 14.0),
    );
    icons::paint(painter, glyph, Icon::Curlybraces, theme::ORANGE);

    // The edit's size sits on the right; the name takes what is left and is cut with an ellipsis.
    let mut right = rect.right();
    if script.added == 0 && script.removed == 0 {
        let galley = painter.layout_no_wrap(
            "whitespace only".to_owned(),
            egui::FontId::proportional(size::SUBHEADLINE),
            theme::text_tertiary(),
        );
        right -= galley.size().x;
        painter.galley(
            egui::pos2(right, rect.center().y - galley.size().y / 2.0),
            galley,
            theme::text_tertiary(),
        );
        right -= 8.0;
    }
    let counts = [
        (script.removed, "−", theme::DOWN),
        (script.added, "+", theme::UP),
    ];
    for (count, sign, color) in counts {
        if count == 0 {
            continue;
        }
        let galley = painter.layout_no_wrap(
            format!("{sign}{count}"),
            egui::FontId::monospace(size::SUBHEADLINE),
            color,
        );
        right -= galley.size().x;
        painter.galley(
            egui::pos2(right, rect.center().y - galley.size().y / 2.0),
            galley,
            color,
        );
        right -= 8.0;
    }
    let name = kit::galley(
        ui,
        &script.name,
        size::BODY,
        MEDIUM,
        theme::text_primary(),
        (right - rect.left() - 26.0 - 4.0).max(40.0),
    );
    ui.painter().galley(
        egui::pos2(rect.left() + 26.0, rect.center().y - name.size().y / 2.0),
        name,
        Color32::PLACEHOLDER,
    );
    crate::ui::accessibility::label(response, &script.name);
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Rect, pos2};

    fn script(name: &str) -> UnsavedScript {
        UnsavedScript {
            id: name.into(),
            name: name.into(),
            script_type: ScriptType::Indicator,
            added: 3,
            removed: 1,
        }
    }

    #[test]
    fn a_line_summary_skips_what_both_texts_share() {
        assert_eq!(lines_changed("a\nb\nc\n", "a\nb\nc\n"), (0, 0));
        assert_eq!(lines_changed("a\nb\nc\n", "a\nb\nc\nd\n"), (1, 0));
        assert_eq!(lines_changed("a\nb\nc\n", "a\nX\nY\nc\n"), (2, 1));
        assert_eq!(lines_changed("", "a\nb\n"), (2, 0));
        assert_eq!(lines_changed("a\nb\n", ""), (0, 2));
        // The same line in two places is not mistaken for shared ends.
        assert_eq!(lines_changed("x\ny\nx\n", "x\nz\nx\n"), (1, 1));
        // A trailing newline alone is not a changed line.
        assert_eq!(lines_changed("a\nb", "a\nb\n"), (0, 0));
    }

    #[test]
    fn the_title_names_one_script_and_counts_several() {
        assert_eq!(
            title(&[script("RSI Zones")]),
            "Save changes to “RSI Zones”?"
        );
        assert_eq!(
            title(&[script("a"), script("b"), script("c")]),
            "Save changes to 3 scripts?"
        );
    }

    /// The tallest the sheet got over `frames` frames: a sheet that grows every frame has a
    /// layout loop.
    fn heights(prompt: &ExitPrompt, frames: usize) -> Vec<f32> {
        let ctx = egui::Context::default();
        (0..frames)
            .map(|_| {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 700.0))),
                        ..Default::default()
                    },
                    |ui| {
                        show(ui.ctx(), prompt);
                    },
                );
                // Nothing paints these frames; dropping uploaded textures unhandled trips a debug assert.
                output.textures_delta.clear();
                ctx.memory(|memory| memory.area_rect(Id::new("script-unsaved")))
                    .map_or(0.0, |rect| rect.height())
            })
            .collect()
    }

    #[test]
    fn the_sheet_settles_at_a_fixed_height_for_one_script_and_for_many() {
        for count in [1, 3, 9] {
            let prompt = ExitPrompt {
                reason: ExitReason::Quit,
                scripts: (0..count).map(|i| script(&format!("Script {i}"))).collect(),
            };
            let heights = heights(&prompt, 6);
            assert!(heights[5] > 120.0, "{count}: {heights:?}");
            assert_eq!(heights[4], heights[5], "{count} grows: {heights:?}");
        }
    }

    #[test]
    fn a_long_list_is_capped_with_a_summary_row() {
        let few = ExitPrompt {
            reason: ExitReason::CloseTab,
            scripts: (0..MAX_ROWS).map(|i| script(&format!("S{i}"))).collect(),
        };
        let many = ExitPrompt {
            scripts: (0..MAX_ROWS + 6)
                .map(|i| script(&format!("S{i}")))
                .collect(),
            ..few.clone()
        };
        let (few, many) = (heights(&few, 6)[5], heights(&many, 6)[5]);
        assert!(
            (many - few - 30.0).abs() < 2.0,
            "only the “and N more” row is added: {few} vs {many}"
        );
    }
}

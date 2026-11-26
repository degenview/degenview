//! The row of key hints along the foot of a picker ("↵ chart it", "↑↓ move"): keycaps, then what
//! the keys do. A row too narrow for them all drops the least needed ones.

use crate::ui::kit::keycap::{keycap, keycap_width};
use crate::ui::kit::{self, REGULAR};
use crate::ui::theme;
use eframe::egui::{self, Rect, Ui};

/// Between a hint's keycaps and its label, and between its keycaps.
const KEY_GAP: f32 = 4.0;
/// After each hint.
const HINT_GAP: f32 = 8.0;

/// One hint: the keys to press, and what they do.
pub struct Hint<'a> {
    pub keys: &'a [&'a str],
    pub label: String,
}

impl<'a> Hint<'a> {
    pub fn new(keys: &'a [&'a str], label: impl Into<String>) -> Self {
        Self {
            keys,
            label: label.into(),
        }
    }
}

/// How wide the hints are together, with the gap after each.
pub fn hints_width(ui: &Ui, hints: &[Hint], text_size: f32) -> f32 {
    hints.iter().map(|hint| width(ui, hint, text_size)).sum()
}

fn width(ui: &Ui, hint: &Hint, text_size: f32) -> f32 {
    let label = kit::galley(
        ui,
        &hint.label,
        text_size,
        REGULAR,
        theme::text_secondary(),
        f32::INFINITY,
    );
    let keys: f32 = hint
        .keys
        .iter()
        .map(|key| keycap_width(ui, key) + KEY_GAP)
        .sum();
    keys + label.size().x + KEY_GAP + HINT_GAP
}

/// Draws `hints` left to right in `row`, labels `text_size` large. While they do not fit, the
/// hint whose label is first in `least_needed` (and is present) is dropped, then the next.
pub fn hint_row(
    ui: &mut Ui,
    row: Rect,
    mut hints: Vec<Hint>,
    text_size: f32,
    least_needed: &[&str],
) {
    for label in least_needed {
        if hints_width(ui, &hints, text_size) <= row.width() {
            break;
        }
        hints.retain(|hint| hint.label != *label);
    }

    let mut cell = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(row)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    cell.spacing_mut().item_spacing.x = KEY_GAP;
    for hint in &hints {
        for key in hint.keys {
            keycap(&mut cell, key);
        }
        cell.label(kit::text(
            &hint.label,
            text_size,
            REGULAR,
            theme::text_secondary(),
        ));
        cell.add_space(HINT_GAP);
    }
}

//! The keys the open picker answers to, taken from the input before the field can see them.

use super::grid::Step;
use eframe::egui::{self, Key, Modifiers};

#[derive(Default)]
pub(super) struct Keys {
    /// Return: chart the highlighted timeframe.
    pub(super) chart: bool,
    /// ⇧Return: chart it and pin it.
    pub(super) pin: bool,
    pub(super) step: Option<Step>,
    pub(super) close: bool,
}

impl Keys {
    /// Left and Right move the highlight only while the field is empty; with text in it they move
    /// the caret.
    pub(super) fn read(ctx: &egui::Context, field_empty: bool) -> Self {
        ctx.input_mut(|input| {
            let pin = input.consume_key(Modifiers::SHIFT, Key::Enter);
            let chart = input.consume_key(Modifiers::NONE, Key::Enter);
            let mut step = None;
            for (key, direction) in [
                (Key::ArrowUp, Step::Up),
                (Key::ArrowDown, Step::Down),
                (Key::ArrowLeft, Step::Left),
                (Key::ArrowRight, Step::Right),
            ] {
                let sideways = matches!(direction, Step::Left | Step::Right);
                if (!sideways || field_empty) && input.consume_key(Modifiers::NONE, key) {
                    step = Some(direction);
                }
            }
            Self {
                chart: chart && !pin,
                pin,
                step,
                close: input.consume_key(Modifiers::NONE, Key::Escape),
            }
        })
    }
}

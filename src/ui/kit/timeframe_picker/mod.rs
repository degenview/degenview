//! The timeframe picker behind the toolbar's timeframe button (and Pine Studio's): a field that
//! reads any timeframe ("22m", "3h", "2D"), the presets and the pinned custom timeframes as chips
//! by section, a line that warns when a chart's source cannot draw the highlighted one, and the
//! keys that work. It only reports what was picked; the caller applies it.

mod grid;
mod keys;
mod state;
#[cfg(test)]
mod tests;
mod unpin;
mod warning;

use super::hints::{Hint, hint_row, hints_width};
use super::search_box::compact_search_box;
use super::{
    HEADING_INSET, MEDIUM, MENU_FRAME_MARGIN, REGULAR, galley, menu_frame, menu_heading,
    menu_scope, menu_separator, size, tile_chip_starred, tile_chip_text_width,
};
use crate::i18n::tr;
use crate::model::{DataSource, Timeframe};
use crate::ui::icons::Icon;
use crate::ui::theme;
use eframe::egui::{self, Id, Response, Sense, Ui, Vec2, vec2};
use grid::{COLUMNS, Grid, title};
use keys::Keys;
use state::{State, Typed};

/// A chip's height.
const CHIP_HEIGHT: f32 = 28.0;
/// Space either side of a chip's label.
const CHIP_PADDING: f32 = 20.0;
/// Between two chips, across and down.
const CHIP_GAP: f32 = 7.0;
const ROW_HEIGHT: f32 = 28.0;
/// A line of caption text (an error, a warning).
const NOTE_HEIGHT: f32 = 24.0;
const FOOTER_HEIGHT: f32 = 30.0;
const FOOTER_TEXT: f32 = size::SUBHEADLINE;

/// What the picker needs from its caller.
pub struct TimeframePickerInput<'a> {
    /// The timeframe the tab (or the preview chart) is on: lit, and what an empty field points at.
    pub current: Timeframe,
    /// The custom timeframes the person pinned, in any order.
    pub pinned: &'a [Timeframe],
    /// The favorite timeframes starred for quick access on the toolbar.
    pub favorites: &'a [Timeframe],
    /// The market sources of the charts the pick would reach, for the line that warns of one that
    /// cannot draw it.
    pub sources: &'a [DataSource],
}

/// What the person picked.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimeframePick {
    Chart(Timeframe),
    /// Chart it and keep it among the chips.
    ChartAndPin(Timeframe),
    Unpin(Timeframe),
    /// Star or unstar a timeframe for quick access on the toolbar.
    ToggleFavorite(Timeframe),
}

/// The popover under `button`, open while the button's popup is. Returns what was picked this
/// frame; a pick closes it.
pub fn show(ui: &Ui, button: &Response, input: &TimeframePickerInput) -> Option<TimeframePick> {
    let popup_id = egui::Popup::default_response_id(button);
    let open = egui::Popup::is_id_open(ui.ctx(), popup_id);
    let state_id = popup_id.with("timeframe-picker");
    if !open {
        ui.data_mut(|data| data.remove::<State>(state_id));
        // Measuring a menu nobody opened would cost every frame.
        if !button.clicked() {
            return None;
        }
    }
    let mut state = ui
        .data(|data| data.get_temp::<State>(state_id))
        .unwrap_or_default();
    let grid = Grid::new(input.pinned);
    let chip = chip_size(ui, &grid);
    let content = chip.x * COLUMNS as f32 + CHIP_GAP * (COLUMNS - 1) as f32 + 2.0 * HEADING_INSET;
    let width = content + MENU_FRAME_MARGIN;
    // A click on the Unpin menu is one outside the picker, to egui.
    let close_behavior = if state.unpin.is_some() {
        egui::PopupCloseBehavior::IgnoreClicks
    } else {
        egui::PopupCloseBehavior::CloseOnClickOutside
    };
    let pick = egui::Popup::menu(button)
        .frame(menu_frame())
        .close_behavior(close_behavior)
        .align(egui::RectAlign::BOTTOM_START)
        .gap(4.0)
        .width(width)
        .show(|ui| {
            menu_scope(ui, 0.0);
            ui.set_width(content);
            Body {
                input,
                grid: &grid,
                chip,
            }
            .show(ui, &mut state, state_id.with("field"))
        })
        .and_then(|shown| shown.inner);
    ui.data_mut(|data| data.insert_temp(state_id, state));
    pick
}

/// Every chip as wide as the longest label needs, and wide enough that the four of a row span
/// what the footer's hints need: the popover is the chips' width and no wider.
fn chip_size(ui: &Ui, grid: &Grid) -> Vec2 {
    let label = grid
        .all()
        .map(|timeframe| tile_chip_text_width(ui, &timeframe.label()))
        .fold(0.0, f32::max);
    let footer = HEADING_INSET + hints_width(ui, &footer_hints(), FOOTER_TEXT);
    let row = footer - 2.0 * HEADING_INSET - CHIP_GAP * (COLUMNS - 1) as f32;
    let width = (label + 2.0 * CHIP_PADDING).max(row / COLUMNS as f32);
    vec2(width.ceil(), CHIP_HEIGHT)
}

/// One frame of the open popover.
struct Body<'a> {
    input: &'a TimeframePickerInput<'a>,
    grid: &'a Grid,
    chip: Vec2,
}

impl Body<'_> {
    fn show(&self, ui: &mut Ui, state: &mut State, field_id: Id) -> Option<TimeframePick> {
        let current = self.input.current;
        let keys = Keys::read(ui.ctx(), state.query.is_empty());
        if let Some(step) = keys.step
            && let Some(from) = state.highlighted(current)
            && let Some(to) = self.grid.step(from, step)
        {
            state.moved = Some(to);
        }

        let field = compact_search_box(
            ui,
            Icon::Clock,
            &mut state.query,
            false,
            tr!("Type any timeframe: 22m, 3h, 2D"),
            field_id,
            !state.focused_once,
        );
        state.focused_once = true;
        if field.cleared {
            state.query.clear();
        }
        if field.changed || field.cleared {
            state.moved = None;
        }
        menu_separator(ui);

        let typed = Typed::read(&state.query);
        let highlighted = state.highlighted(current);
        let mut pick = None;
        match typed {
            Typed::Valid(timeframe) if !self.grid.contains(timeframe) => {
                if typed_row(ui, timeframe, highlighted == Some(timeframe)) {
                    pick = Some(TimeframePick::Chart(timeframe));
                }
            }
            Typed::Invalid(error) => note(ui, &error.to_string()),
            Typed::Empty | Typed::Valid(_) => {}
        }
        pick = self.chips(ui, state, highlighted).or(pick);
        if let Some(warning) =
            highlighted.and_then(|timeframe| warning::cannot_chart(self.input.sources, timeframe))
        {
            note(ui, &warning);
        }
        menu_separator(ui);
        footer(ui);

        if let Some(timeframe) = highlighted {
            if keys.pin {
                pick = Some(TimeframePick::ChartAndPin(timeframe));
            } else if keys.chart {
                pick = Some(TimeframePick::Chart(timeframe));
            }
        }
        // Unpinning leaves the picker open: the chip is simply gone.
        if matches!(
            pick,
            Some(TimeframePick::Chart(_) | TimeframePick::ChartAndPin(_))
        ) || keys.close
        {
            ui.close();
        }
        pick
    }

    /// The sections, each a heading over its rows of chips.
    fn chips(
        &self,
        ui: &mut Ui,
        state: &mut State,
        highlighted: Option<Timeframe>,
    ) -> Option<TimeframePick> {
        let mut pick = None;
        // Each chip that still has its menu open puts it back.
        let menu_at = state.unpin.take();
        for (section, timeframes) in self.grid.sections() {
            menu_heading(ui, title(*section));
            for row in timeframes.chunks(COLUMNS) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = CHIP_GAP;
                    ui.add_space(HEADING_INSET);
                    for timeframe in row {
                        let label = timeframe.label();
                        let is_favorite = self.input.favorites.contains(timeframe);
                        let (response, star_clicked) = tile_chip_starred(
                            ui,
                            self.chip,
                            &label,
                            *timeframe == self.input.current,
                            highlighted == Some(*timeframe),
                            is_favorite,
                        );
                        let response = response.on_hover_text(timeframe.description());
                        if star_clicked {
                            pick = Some(TimeframePick::ToggleFavorite(*timeframe));
                        } else if response.clicked() {
                            pick = Some(TimeframePick::Chart(*timeframe));
                        }
                        if !Timeframe::PRESETS.contains(timeframe) {
                            let mut at = menu_at
                                .filter(|(opened_on, _)| opened_on == timeframe)
                                .map(|(_, at)| at);
                            if unpin::menu(ui, &response, &mut at) {
                                pick = Some(TimeframePick::Unpin(*timeframe));
                            }
                            if let Some(at) = at {
                                state.unpin = Some((*timeframe, at));
                            }
                        }
                    }
                });
                ui.add_space(CHIP_GAP);
            }
        }
        pick
    }
}

/// The row that offers a typed timeframe none of the chips is: "Chart 22m", and what it is.
fn typed_row(ui: &mut Ui, timeframe: Timeframe, highlighted: bool) -> bool {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    if highlighted || response.hovered() {
        ui.painter()
            .rect_filled(rect, 5.0, theme::selected_tint(true));
    }
    let label = galley(
        ui,
        &tr!("Chart {timeframe}", timeframe = timeframe),
        size::BODY,
        MEDIUM,
        theme::text_primary(),
        rect.width(),
    );
    let description = galley(
        ui,
        &timeframe.description(),
        size::SUBHEADLINE,
        REGULAR,
        theme::text_secondary(),
        rect.width(),
    );
    let middle = rect.center().y;
    ui.painter().galley(
        egui::pos2(rect.left() + HEADING_INSET, middle - label.size().y / 2.0),
        label,
        theme::text_primary(),
    );
    ui.painter().galley(
        egui::pos2(
            rect.right() - HEADING_INSET - description.size().x,
            middle - description.size().y / 2.0,
        ),
        description,
        theme::text_secondary(),
    );
    response.clicked()
}

/// One line of caption text, cut at the popover's edge.
fn note(ui: &mut Ui, message: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), NOTE_HEIGHT), Sense::hover());
    let line = galley(
        ui,
        message,
        size::CAPTION,
        REGULAR,
        theme::text_secondary(),
        rect.width() - 2.0 * HEADING_INSET,
    );
    ui.painter().galley(
        egui::pos2(
            rect.left() + HEADING_INSET,
            rect.center().y - line.size().y / 2.0,
        ),
        line,
        theme::text_secondary(),
    );
}

fn footer_hints() -> Vec<Hint<'static>> {
    vec![
        Hint::new(&["\u{21b5}"], tr!("chart it")),
        Hint::new(&["\u{21e7}", "\u{21b5}"], tr!("chart and pin")),
        Hint::new(&["\u{2191}", "\u{2193}"], tr!("move")),
    ]
}

/// The keys that work here.
fn footer(ui: &mut Ui) {
    let (rect, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), FOOTER_HEIGHT), Sense::hover());
    let row = rect.with_min_x(rect.left() + HEADING_INSET);
    hint_row(ui, row, footer_hints(), FOOTER_TEXT, &[tr!("move")]);
}

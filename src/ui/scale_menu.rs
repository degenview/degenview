//! The price-scale button in the corner where a chart's price axis meets its time axis, and the
//! menu it opens: Regular, Percent, Indexed to 100 and Logarithmic, one of them checked.
//!
//! The button belongs to one chart card and reports the mode picked. The workspace sets it
//! (`ChartWorkspace::set_scale_mode`), the same way the shortcuts and the price-axis menu do, so
//! there is one path and no state in the menu itself. The corner is already a dead zone for the
//! chart's gestures, so the button takes no drag from the axes.

use super::icons::{self, Icon};
use super::{accessibility, kit, shortcut, theme};
use crate::model::price_scale::PriceScaleMode;
use eframe::egui::{
    self, Color32, CursorIcon, Id, Key, Modifiers, Pos2, Rect, RectAlign, Sense, Ui, Vec2,
};

const BUTTON: f32 = 20.0;
/// Space kept between the button and the axes' lines.
const MARGIN: f32 = 2.0;
/// The corner is left empty below this: the button would crowd the axes.
const MIN_CELL: f32 = 18.0;
const MENU_WIDTH: f32 = 224.0;

/// The id of a chart's button, which the menu it opens hangs from.
pub fn id(chart_id: u64) -> Id {
    Id::new(("scale-menu", chart_id))
}

/// The menu's own id: what egui derives from the button's response (`Popup::default_response_id`).
fn popup_id(button: Id) -> Id {
    button.with("popup")
}

/// Whether the menu of chart `chart_id` is open.
pub fn is_open(ctx: &egui::Context, chart_id: u64) -> bool {
    egui::Popup::is_id_open(ctx, popup_id(id(chart_id)))
}

/// Closes the open menu on Escape and spends the key, so that it does not also deselect a
/// drawing, disarm a tool or end a draft behind the menu. egui's popup closes on any Escape
/// without taking it.
pub fn close_on_escape(ctx: &egui::Context, chart_id: u64) {
    if is_open(ctx, chart_id)
        && ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape))
    {
        egui::Popup::close_id(ctx, popup_id(id(chart_id)));
    }
}

/// The corner cell: right of the plot and under it, `main` being the chart above its panes.
pub fn corner(plot: Rect, main: Rect) -> Option<Rect> {
    let cell = Rect::from_min_max(Pos2::new(plot.right(), plot.bottom()), main.max);
    (cell.width() >= MIN_CELL && cell.height() >= MIN_CELL && plot.width() > 0.0).then_some(cell)
}

/// Where the button sits in `cell`: centred, and shrunk with a cell that is smaller than it.
fn button_rect(cell: Rect) -> Rect {
    let side = BUTTON
        .min(cell.width() - 2.0 * MARGIN)
        .min(cell.height() - 2.0 * MARGIN);
    Rect::from_center_size(cell.center(), Vec2::splat(side.max(0.0)))
}

/// What the button says about the scale it is for.
pub struct ScaleButton {
    /// The mode chosen for the chart.
    pub mode: PriceScaleMode,
    /// Whether the frame is drawn on a logarithmic map. A chosen Logarithmic that is not could
    /// not be: the view holds a price at or below zero, which has no logarithm.
    pub logarithmic_drawn: bool,
}

impl ScaleButton {
    fn waiting_for_prices(&self) -> bool {
        self.mode.is_logarithmic() && !self.logarithmic_drawn
    }

    fn tooltip(&self) -> String {
        if self.waiting_for_prices() {
            "Price scale: Logarithmic, shown linear while a price is at or below zero".to_owned()
        } else {
            format!("Price scale: {}", self.mode.label())
        }
    }
}

/// The button in `cell`. Returns the mode picked this frame.
pub fn show(ui: &mut Ui, id: Id, cell: Rect, state: &ScaleButton) -> Option<PriceScaleMode> {
    let rect = button_rect(cell);
    if rect.width() < 12.0 {
        return None;
    }
    let response = ui
        .interact(rect, id, Sense::click())
        .on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text(state.tooltip());
    let response = accessibility::label(response, state.tooltip());
    let popup_id = egui::Popup::default_response_id(&response);
    let open = egui::Popup::is_id_open(ui.ctx(), popup_id);
    let focus_id = id.with("focus");
    if !open {
        ui.data_mut(|data| data.remove_temp::<Option<usize>>(focus_id));
    }
    let fill = if open || response.is_pointer_button_down_on() {
        theme::ink(26)
    } else if response.hovered() {
        theme::ink(14)
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, 5.0, fill);
    // A scale other than the plain one is lit, so a chart reading in percent is told at a glance.
    let ink = if state.mode == PriceScaleMode::Regular {
        theme::text_secondary()
    } else {
        kit::ACCENT
    };
    icons::paint(
        ui.painter(),
        Rect::from_center_size(rect.center(), Vec2::splat(rect.width() * 0.66)),
        Icon::PriceScale,
        ink,
    );

    let mut picked = None;
    kit::menu_popup(&response)
        .align(RectAlign::TOP_END)
        .gap(4.0)
        .width(MENU_WIDTH)
        .show(|ui| picked = menu(ui, focus_id, state));
    picked
}

/// The rows, with the keyboard's highlight: none until the arrows or the pointer set it, then the
/// arrows move it (the first Down lands on the first row, the first Up on the last, and both
/// wrap), Enter or Space picks it, and the pointer moves it as it moves.
fn menu(ui: &mut Ui, focus_id: Id, state: &ScaleButton) -> Option<PriceScaleMode> {
    kit::menu_scope(ui, MENU_WIDTH - 10.0);
    let mut focus = ui
        .data(|data| data.get_temp::<Option<usize>>(focus_id))
        .flatten();
    let (down, up, confirm) = ui.input_mut(|input| {
        (
            input.consume_key(Modifiers::NONE, Key::ArrowDown),
            input.consume_key(Modifiers::NONE, Key::ArrowUp),
            input.consume_key(Modifiers::NONE, Key::Enter)
                || input.consume_key(Modifiers::NONE, Key::Space),
        )
    });
    if down || up {
        focus = Some(step(focus, PriceScaleMode::ALL.len(), down, up));
    }
    let pointer_moved = ui.input(|input| input.pointer.delta() != Vec2::ZERO);

    let mut picked = None;
    for (index, mode) in PriceScaleMode::ALL.into_iter().enumerate() {
        let label = shortcut::scale_mode(mode).map(|shortcut| shortcut.label());
        let row = kit::menu_row(
            ui,
            kit::MenuRow {
                checked: state.mode == mode,
                shortcut: label.as_deref(),
                focused: focus == Some(index),
                ..kit::MenuRow::new(mode.label())
            },
        );
        if row.hovered() && pointer_moved {
            focus = Some(index);
        }
        let row = if mode.is_logarithmic() && state.waiting_for_prices() {
            row.on_hover_text("Needs prices above zero; drawn linear until then.")
        } else {
            row
        };
        if row.clicked() || (confirm && focus == Some(index)) {
            picked = Some(mode);
            ui.close();
        }
    }
    ui.data_mut(|data| data.insert_temp(focus_id, focus));
    picked
}

/// Where the highlight goes from `from` on Down or Up (Down wins if both): the next row, or the
/// first for Down and the last for Up when there is none yet, wrapping at both ends.
fn step(from: Option<usize>, count: usize, down: bool, up: bool) -> usize {
    let by = i32::from(down) - i32::from(up);
    match from {
        Some(index) => (index as i32 + by).rem_euclid(count as i32) as usize,
        None if down => 0,
        None => count - 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::vec2;

    fn plot() -> Rect {
        Rect::from_min_size(Pos2::new(0.0, 0.0), vec2(700.0, 400.0))
    }

    fn main() -> Rect {
        Rect::from_min_max(Pos2::ZERO, Pos2::new(760.0, 428.0))
    }

    #[test]
    fn the_corner_is_where_the_price_axis_meets_the_time_axis() {
        let cell = corner(plot(), main()).unwrap();
        assert_eq!(cell.min, Pos2::new(700.0, 400.0));
        assert_eq!(cell.max, Pos2::new(760.0, 428.0));
        // The button is inside it, centred and no bigger than its size.
        let button = button_rect(cell);
        assert!(cell.contains_rect(button));
        assert_eq!(button.center(), cell.center());
        assert_eq!(button.size(), Vec2::splat(BUTTON));
    }

    #[test]
    fn a_corner_too_small_for_the_button_is_left_empty_or_shrinks_it() {
        // A hairline of axis, a chart without a plot, a time axis squeezed to nothing.
        let narrow = Rect::from_min_max(Pos2::ZERO, Pos2::new(708.0, 428.0));
        assert!(corner(plot(), narrow).is_none());
        let flat = Rect::from_min_max(Pos2::ZERO, Pos2::new(760.0, 404.0));
        assert!(corner(plot(), flat).is_none());
        assert!(corner(Rect::from_min_size(Pos2::ZERO, Vec2::ZERO), main()).is_none());
        // Just fitting: the button shrinks with the cell, never past it.
        let tight = Rect::from_min_max(Pos2::new(700.0, 400.0), Pos2::new(730.0, 420.0));
        let button = button_rect(tight);
        assert!(tight.contains_rect(button));
        assert!(button.width() < BUTTON);
    }

    #[test]
    fn the_keyboard_highlight_wraps_both_ways() {
        assert_eq!(step(Some(0), 4, false, true), 3);
        assert_eq!(step(Some(3), 4, true, false), 0);
        assert_eq!(step(Some(1), 4, true, false), 2);
        // With nothing highlighted yet, Down starts at the top and Up at the bottom.
        assert_eq!(step(None, 4, true, false), 0);
        assert_eq!(step(None, 4, false, true), 3);
    }

    #[test]
    fn a_logarithmic_choice_that_cannot_be_drawn_says_so() {
        let ok = ScaleButton {
            mode: PriceScaleMode::Logarithmic,
            logarithmic_drawn: true,
        };
        assert!(!ok.waiting_for_prices());
        assert_eq!(ok.tooltip(), "Price scale: Logarithmic");
        let blocked = ScaleButton {
            logarithmic_drawn: false,
            ..ok
        };
        assert!(blocked.waiting_for_prices());
        assert!(blocked.tooltip().contains("linear"));
        // Only a logarithm can wait for prices.
        let percent = ScaleButton {
            mode: PriceScaleMode::Percent,
            logarithmic_drawn: false,
        };
        assert!(!percent.waiting_for_prices());
    }

    const SCREEN: Vec2 = vec2(900.0, 700.0);

    struct Frames {
        ctx: egui::Context,
        time: f64,
    }

    impl Frames {
        fn new() -> Self {
            Self {
                ctx: egui::Context::default(),
                time: 0.0,
            }
        }

        fn run(&mut self, events: Vec<egui::Event>, state: &ScaleButton) -> Option<PriceScaleMode> {
            self.time += 0.2;
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, SCREEN)),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let mut picked = None;
            let cell = corner(plot(), main()).unwrap();
            let mut output = self.ctx.run_ui(input, |ui| {
                picked = picked.or(show(ui, Id::new("scale"), cell, state));
            });
            output.textures_delta.clear();
            picked
        }

        fn click(&mut self, at: Pos2, state: &ScaleButton) {
            let button = |pressed| egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            };
            self.run(vec![egui::Event::PointerMoved(at)], state);
            self.run(Vec::new(), state);
            self.run(vec![button(true)], state);
            self.run(vec![button(false)], state);
            self.run(Vec::new(), state);
        }

        fn key(&mut self, key: Key, state: &ScaleButton) -> Option<PriceScaleMode> {
            let event = |pressed| egui::Event::Key {
                key,
                physical_key: Some(key),
                pressed,
                repeat: false,
                modifiers: Modifiers::NONE,
            };
            let first = self.run(vec![event(true), event(false)], state);
            first.or(self.run(Vec::new(), state))
        }

        fn menu_open(&self) -> bool {
            egui::Popup::is_any_open(&self.ctx)
        }
    }

    fn button_center() -> Pos2 {
        button_rect(corner(plot(), main()).unwrap()).center()
    }

    #[test]
    fn a_click_opens_the_menu_and_the_keys_pick_a_row() {
        let state = ScaleButton {
            mode: PriceScaleMode::Regular,
            logarithmic_drawn: false,
        };
        let mut frames = Frames::new();
        assert!(!frames.menu_open());
        frames.click(button_center(), &state);
        assert!(frames.menu_open(), "a click on the button opens the menu");

        // Nothing is highlighted until a key says so, so Enter alone picks nothing; then three
        // steps down from the top is Indexed to 100.
        assert_eq!(frames.key(Key::Enter, &state), None);
        assert!(frames.menu_open());
        for _ in 0..3 {
            assert_eq!(frames.key(Key::ArrowDown, &state), None);
        }
        assert_eq!(
            frames.key(Key::Enter, &state),
            Some(PriceScaleMode::IndexedTo100)
        );
        frames.run(Vec::new(), &state);
        assert!(!frames.menu_open(), "choosing a row closes the menu");
    }

    #[test]
    fn escape_closes_the_menu_without_choosing() {
        let state = ScaleButton {
            mode: PriceScaleMode::Percent,
            logarithmic_drawn: true,
        };
        let mut frames = Frames::new();
        frames.click(button_center(), &state);
        assert!(frames.menu_open());
        assert_eq!(frames.key(Key::Escape, &state), None);
        frames.run(Vec::new(), &state);
        assert!(!frames.menu_open());
    }

    #[test]
    fn a_click_outside_closes_the_menu() {
        let state = ScaleButton {
            mode: PriceScaleMode::Regular,
            logarithmic_drawn: true,
        };
        let mut frames = Frames::new();
        frames.click(button_center(), &state);
        assert!(frames.menu_open());
        frames.click(Pos2::new(40.0, 40.0), &state);
        assert!(!frames.menu_open());
    }
}

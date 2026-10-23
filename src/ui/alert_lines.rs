//! Price alerts on the chart: a dashed line at each alert's level, ending in a small tag against
//! the price axis. Right-clicking a tag opens the alert's menu (edit, pause, delete); dragging it
//! up or down moves the level, as for a paper order. The line itself only marks the level.
//!
//! Lines sit on the chart's own price scale, like the paper-trading markers, so they stay on
//! their level through zooms and axis drags. Pure view: it reports the click and the drop, the
//! workspace acts.

use super::chart_card::PriceScale;
use crate::model::PriceFormat;
use crate::model::alert::AlertId;
use crate::model::alert_line::{AlertLine, above_after_drag, dropped_level};
use crate::ui::{accessibility, theme};
use eframe::egui::{
    self, Align2, Color32, CursorIcon, FontId, Id, Key, Pos2, Rect, Sense, Stroke, Vec2,
};
use std::time::Duration;

const HEIGHT: f32 = 20.0;
const MARGIN: f32 = 8.0;
/// The longest a dropped tag waits at its new level for the alert list to show it. The app drains
/// saves about once a second, and a background agent that owns the alerts takes longer; a save
/// that never lands sends the tag back after this.
const HOLD_SECONDS: f64 = 10.0;

/// A tag that was right-clicked, and where.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineMenu {
    pub id: AlertId,
    pub at: Pos2,
}

/// A tag that was dropped on a new level. `price` is in the chart's prices (USD), on the tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AlertMove {
    pub id: AlertId,
    pub price: f64,
}

/// What the alert tags reported this frame.
#[derive(Default)]
pub(super) struct Shown {
    pub menu: Option<LineMenu>,
    pub moves: Vec<AlertMove>,
}

/// The market a tag is dragged on: the last price decides which way the alert then watches, and
/// the tick is where a drop lands.
#[derive(Clone, Copy, Debug, Default)]
pub struct Market {
    pub last: Option<f64>,
    pub tick: f64,
    /// The chart's price places: a drop lands on a price the chart can write, not on the
    /// hundred-millionth the pointer happened to be over.
    pub format: PriceFormat,
}

impl Market {
    /// `price` on the tick and on the chart's places; `None` unless an alert can have it.
    fn snap(self, price: f64) -> Option<f64> {
        let level = dropped_level(price, self.tick)?;
        let written = self.format.plain(level).parse::<f64>().ok()?;
        (written > 0.0).then_some(written)
    }
}

/// A dropped tag held at its new level until the alert list catches up with the save. `from` is
/// the level the alert had, so a different level in the list means the save arrived.
#[derive(Clone, Copy, Debug)]
struct Hold {
    price: f64,
    above: bool,
    from: f64,
    until: f64,
}

/// The tag's words: what the alert waits for, at the chart's price format.
fn caption(line: &AlertLine, format: PriceFormat) -> String {
    let price = format.format(line.price);
    let direction = if line.above { "↑" } else { "↓" };
    if line.paused {
        format!("Alert {direction} {price} · paused")
    } else {
        format!("Alert {direction} {price}")
    }
}

/// A line's tag, as drawn.
struct Tag {
    rect: Rect,
    caption: String,
}

/// Paints one alert's dashed line and its tag at `y`, and says where the tag is. Nothing when
/// the level is off the plot.
fn paint_line(
    painter: &egui::Painter,
    plot: Rect,
    y: f32,
    line: &AlertLine,
    format: PriceFormat,
) -> Option<Tag> {
    if !y.is_finite() || !plot.y_range().contains(y) {
        return None;
    }
    let tint = if line.paused {
        theme::text_tertiary()
    } else {
        theme::YELLOW
    };
    let caption = caption(line, format);
    let galley = painter.layout_no_wrap(caption.clone(), FontId::monospace(10.5), Color32::WHITE);
    let width = galley.size().x + 22.0;
    let tag = Rect::from_min_size(
        Pos2::new(plot.right() - MARGIN - width, y - HEIGHT / 2.0),
        Vec2::new(width, HEIGHT),
    );
    let stroke = Stroke::new(1.0, tint.gamma_multiply(0.7));
    for dash in egui::Shape::dashed_line(
        &[Pos2::new(plot.left(), y), Pos2::new(tag.left(), y)],
        stroke,
        2.0,
        4.0,
    ) {
        painter.add(dash);
    }
    painter.rect_filled(tag, 6.0, theme::chrome().gamma_multiply(0.94));
    painter.rect_stroke(
        tag,
        6.0,
        Stroke::new(1.0, tint.gamma_multiply(0.8)),
        egui::StrokeKind::Inside,
    );
    painter.text(
        Pos2::new(tag.left() + 8.0, tag.center().y),
        Align2::LEFT_CENTER,
        "●",
        FontId::proportional(7.0),
        tint,
    );
    painter.galley(
        Pos2::new(tag.left() + 16.0, tag.center().y - galley.size().y / 2.0),
        galley,
        if line.paused {
            theme::text_tertiary()
        } else {
            Color32::WHITE
        },
    );
    Some(Tag { rect: tag, caption })
}

/// Draws the alert lines over `plot` and nothing else: no tag answers a pointer. What a
/// snapshot shows.
pub(super) fn paint(
    painter: &egui::Painter,
    plot: Rect,
    scale: &PriceScale,
    lines: &[AlertLine],
    format: PriceFormat,
) {
    for line in lines {
        paint_line(painter, plot, scale.y(line.price), line, format);
    }
}

/// Keeps a dragged tag's centre where the whole tag stays on the plot.
fn clamped_y(plot: Rect, y: f32) -> f32 {
    y.clamp(plot.top() + HEIGHT / 2.0, plot.bottom() - HEIGHT / 2.0)
}

/// The line as it is when dropped at `y`: the level under the tag, on the tick, and the side that
/// level is on. `None` when that is not a price an alert can have.
fn landed(
    line: &AlertLine,
    scale: &PriceScale,
    plot: Rect,
    y: f32,
    market: Market,
) -> Option<AlertLine> {
    let price = market.snap(scale.price(clamped_y(plot, y)))?;
    Some(AlertLine {
        price,
        above: above_after_drag(line.above, price, market.last),
        ..line.clone()
    })
}

/// Draws the alert lines over `plot` and returns what the user did to the tags this frame: the
/// one right-clicked and the ones dropped on a new level.
pub(super) fn show(
    ui: &mut egui::Ui,
    plot: Rect,
    scale: &PriceScale,
    lines: &[AlertLine],
    format: PriceFormat,
    market: Market,
) -> Shown {
    let painter = ui.painter().with_clip_rect(plot);
    let market = Market { format, ..market };
    let mut shown = Shown::default();
    let now = ui.input(|input| input.time);
    for line in lines {
        let id = Id::new(("alert-line", line.id));
        let (drag_key, hold_key) = (id.with("y"), id.with("hold"));
        let dragging_y = ui.ctx().data(|data| data.get_temp::<f32>(drag_key));
        let hold = ui
            .ctx()
            .data(|data| data.get_temp::<Hold>(hold_key))
            .filter(|hold| hold.from == line.price && now < hold.until);
        if hold.is_none() {
            ui.ctx().data_mut(|data| data.remove::<Hold>(hold_key));
        }

        // What the tag shows: under the pointer while dragged, at the dropped level while the
        // save is on its way, else where the alert is.
        let at_rest = (line.clone(), scale.y(line.price));
        let (drawn, y) = match (dragging_y, hold) {
            (Some(y), _) => landed(line, scale, plot, y, market)
                .map_or(at_rest, |dragged| (dragged, clamped_y(plot, y))),
            (None, Some(hold)) => (
                AlertLine {
                    price: hold.price,
                    above: hold.above,
                    ..line.clone()
                },
                scale.y(hold.price),
            ),
            (None, None) => at_rest,
        };
        let Some(tag) = paint_line(&painter, plot, y, &drawn, format) else {
            continue;
        };
        if hold.is_some() {
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }

        let sense = if line.editable {
            Sense::click_and_drag()
        } else {
            Sense::click()
        };
        let response = ui.interact(tag.rect, id, sense);
        let mut response = accessibility::label(response, tag.caption);
        if line.editable {
            if response.hovered() || response.dragged() {
                ui.ctx().set_cursor_icon(CursorIcon::ResizeVertical);
            }
            if !response.dragged() {
                response =
                    response.on_hover_text("Drag to change the price · right-click for options");
            }
            follow_drag(ui, &response, line, scale, plot, market, &mut shown, now);
        } else {
            response = response.on_hover_text("Right-click for alert options");
        }

        if response.secondary_clicked()
            && let Some(at) = response.interact_pointer_pos()
        {
            shown.menu = Some(LineMenu { id: line.id, at });
            // The card reads it next frame.
            ui.ctx().request_repaint();
        }
    }
    shown
}

/// Follows the pointer while `line`'s tag is dragged, and reports the drop. Esc puts the tag
/// back: egui ends the drag on it and reports the stop like a release, which must not save.
#[allow(clippy::too_many_arguments)]
fn follow_drag(
    ui: &egui::Ui,
    response: &egui::Response,
    line: &AlertLine,
    scale: &PriceScale,
    plot: Rect,
    market: Market,
    shown: &mut Shown,
    now: f64,
) {
    let id = response.id;
    let (drag_key, hold_key) = (id.with("y"), id.with("hold"));
    let ctx = ui.ctx();
    if response.dragged() {
        if let Some(pos) = response.interact_pointer_pos() {
            ctx.data_mut(|data| data.insert_temp(drag_key, pos.y));
        }
    } else if response.drag_stopped() {
        ctx.data_mut(|data| data.remove::<f32>(drag_key));
        if ui.input(|input| input.key_pressed(Key::Escape)) {
            return;
        }
        let Some(pos) = response.interact_pointer_pos() else {
            return;
        };
        let Some(drop) = landed(line, scale, plot, pos.y, market) else {
            return;
        };
        // Let go where it started: nothing to save.
        if Some(drop.price) == market.snap(line.price) {
            return;
        }
        ctx.data_mut(|data| {
            data.insert_temp(
                hold_key,
                Hold {
                    price: drop.price,
                    above: drop.above,
                    from: line.price,
                    until: now + HOLD_SECONDS,
                },
            );
        });
        shown.moves.push(AlertMove {
            id: line.id,
            price: drop.price,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(above: bool) -> AlertLine {
        AlertLine {
            id: 1,
            price: 110.0,
            paused: false,
            above,
            editable: true,
        }
    }

    /// 80 at the bottom of a 400pt plot, 120 at the top: 10pt a unit.
    fn scale() -> PriceScale {
        PriceScale {
            minimum: 80.0,
            maximum: 120.0,
            top: 0.0,
            height: 400.0,
            logarithmic: false,
        }
    }

    fn plot() -> Rect {
        Rect::from_min_size(Pos2::ZERO, Vec2::new(600.0, 400.0))
    }

    #[test]
    fn the_tag_says_which_way_and_whether_it_is_paused() {
        let mut line = AlertLine {
            id: 1,
            price: 118_150.99,
            paused: false,
            above: true,
            editable: true,
        };
        assert_eq!(caption(&line, PriceFormat::AUTO), "Alert ↑ 118,151");
        line.above = false;
        line.paused = true;
        assert_eq!(
            caption(&line, PriceFormat::AUTO),
            "Alert ↓ 118,151 · paused"
        );
    }

    #[test]
    fn a_drop_lands_on_the_price_under_the_tag_on_the_tick() {
        let market = Market {
            last: Some(100.0),
            tick: 0.5,
            ..Market::default()
        };
        // 100pt from the top is 110 (120 - 100/10).
        let up = landed(&line(false), &scale(), plot(), 100.0, market).unwrap();
        assert_eq!(up.price, 110.0);
        assert!(up.above, "above the market it waits for a rise");
        // 37pt down is 116.3, which the 0.5 tick takes to 116.5.
        let near = landed(&line(false), &scale(), plot(), 37.0, market).unwrap();
        assert_eq!(near.price, 116.5);
    }

    #[test]
    fn dragging_across_the_market_turns_the_alert_around() {
        let market = Market {
            last: Some(100.0),
            tick: 0.01,
            ..Market::default()
        };
        // 300pt from the top is 90.
        let below = landed(&line(true), &scale(), plot(), 300.0, market).unwrap();
        assert_eq!(below.price, 90.0);
        assert!(!below.above);
        // Nothing known of the market: the side it had stays.
        let unknown = Market {
            last: None,
            tick: 0.01,
            ..Market::default()
        };
        assert!(
            landed(&line(true), &scale(), plot(), 300.0, unknown)
                .unwrap()
                .above
        );
    }

    #[test]
    fn a_tag_dragged_past_the_plot_stays_on_it() {
        let market = Market {
            last: Some(100.0),
            tick: 0.01,
            ..Market::default()
        };
        let top = landed(&line(true), &scale(), plot(), -500.0, market).unwrap();
        // The tag's centre stops half a tag below the top: 120 - 10/10.
        assert_eq!(top.price, 119.0);
        assert_eq!(clamped_y(plot(), 900.0), 400.0 - HEIGHT / 2.0);
    }

    #[test]
    fn a_level_that_is_not_a_price_is_refused() {
        let flat = PriceScale {
            minimum: 0.0,
            maximum: 4.0,
            top: 0.0,
            height: 400.0,
            logarithmic: false,
        };
        let market = Market {
            last: Some(2.0),
            tick: 1.0,
            ..Market::default()
        };
        // The bottom of this scale rounds to 0 on a whole-number tick.
        assert!(landed(&line(true), &flat, plot(), 400.0, market).is_none());
    }

    #[test]
    fn a_drop_lands_on_a_price_the_chart_can_write() {
        // The paper tick of a crypto market is a hundred-millionth, finer than any label.
        let market = Market {
            last: Some(118_000.0),
            tick: 0.000_000_01,
            ..Market::default()
        };
        let btc = PriceScale {
            minimum: 117_000.0,
            maximum: 119_000.0,
            top: 0.0,
            height: 400.0,
            logarithmic: false,
        };
        // 123.456pt from the top is 118,382.72 on this scale: two places, not the pointer's eight.
        let level = landed(&line(true), &btc, plot(), 123.456, market)
            .unwrap()
            .price;
        assert_eq!(level, 118_382.72);
        // A pinned precision rules: whole numbers only.
        let whole = Market {
            format: PriceFormat {
                decimals: Some(0),
                ..PriceFormat::default()
            },
            ..market
        };
        assert_eq!(
            landed(&line(true), &btc, plot(), 123.456, whole)
                .unwrap()
                .price,
            118_383.0
        );
    }

    #[test]
    fn a_token_priced_in_millionths_can_still_be_dragged() {
        let market = Market {
            last: Some(0.000_015),
            tick: 0.000_000_01,
            ..Market::default()
        };
        let token = PriceScale {
            minimum: 0.000_01,
            maximum: 0.000_02,
            top: 0.0,
            height: 400.0,
            logarithmic: false,
        };
        let level = landed(&line(true), &token, plot(), 200.0, market)
            .unwrap()
            .price;
        assert!((level - 0.000_015).abs() < 1e-8, "{level}");
    }

    // MARK: Driven with a pointer

    /// Runs `show` over a headless context one frame at a time, with the events of that frame.
    struct Pointer {
        ctx: egui::Context,
        time: f64,
        line: AlertLine,
    }

    impl Pointer {
        fn new(line: AlertLine) -> Self {
            Self {
                ctx: egui::Context::default(),
                time: 0.0,
                line,
            }
        }

        /// Over the tag of a line at 110, which sits at y = 100.
        fn tag() -> Pos2 {
            Pos2::new(plot().right() - MARGIN - 12.0, 100.0)
        }

        fn frame(&mut self, events: Vec<egui::Event>) -> (Shown, CursorIcon) {
            self.time += 0.05;
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let mut shown = Shown::default();
            let line = std::slice::from_ref(&self.line);
            let market = Market {
                last: Some(100.0),
                tick: 0.01,
                ..Market::default()
            };
            let mut output = self.ctx.run_ui(raw, |ui| {
                shown = show(ui, plot(), &scale(), line, PriceFormat::AUTO, market);
            });
            output.textures_delta.clear();
            (shown, output.platform_output.cursor_icon)
        }

        fn moved(&mut self, to: Pos2) -> (Shown, CursorIcon) {
            self.frame(vec![egui::Event::PointerMoved(to)])
        }

        fn button(&mut self, at: Pos2, pressed: bool) -> (Shown, CursorIcon) {
            self.frame(vec![egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            }])
        }

        /// Hovers the tag until the context knows it is there.
        fn hover(&mut self) -> CursorIcon {
            self.moved(Self::tag());
            self.moved(Self::tag());
            self.frame(Vec::new()).1
        }

        /// Presses on the tag and drags it to `y`, a few frames as a hand would.
        fn drag_to(&mut self, y: f32) {
            self.hover();
            self.button(Self::tag(), true);
            for step in 1..=4 {
                let to = Pos2::new(Self::tag().x, 100.0 + (y - 100.0) * step as f32 / 4.0);
                self.moved(to);
            }
        }

        /// Lets go at `y` and returns every drop the release and the frame after it reported.
        fn release(&mut self, y: f32) -> Shown {
            let at = Pos2::new(Self::tag().x, y);
            let mut shown = self.button(at, false).0;
            shown.moves.extend(self.frame(Vec::new()).0.moves);
            shown
        }
    }

    fn escape() -> egui::Event {
        egui::Event::Key {
            key: Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    #[test]
    fn hovering_a_tag_shows_the_up_down_cursor() {
        let mut pointer = Pointer::new(line(false));
        assert_eq!(pointer.hover(), CursorIcon::ResizeVertical);
    }

    #[test]
    fn a_tag_that_cannot_move_keeps_the_ordinary_cursor_and_drops_nothing() {
        let mut pointer = Pointer::new(AlertLine {
            editable: false,
            ..line(false)
        });
        assert_eq!(pointer.hover(), CursorIcon::Default);
        pointer.drag_to(160.0);
        assert!(pointer.release(160.0).moves.is_empty());
    }

    #[test]
    fn dragging_a_tag_and_letting_go_reports_the_level_under_it() {
        let mut pointer = Pointer::new(line(false));
        pointer.drag_to(160.0);
        // Mid-drag the cursor stays the arrow and nothing is reported yet.
        let (shown, cursor) = pointer.moved(Pos2::new(Pointer::tag().x, 160.0));
        assert_eq!(cursor, CursorIcon::ResizeVertical);
        assert!(shown.moves.is_empty());
        // 160pt from the top is 104.
        let dropped = pointer.release(160.0);
        assert_eq!(
            dropped.moves,
            [AlertMove {
                id: 1,
                price: 104.0
            }]
        );
        // The tag is held at 104 (y = 160) while the save travels, not snapped back to 110.
        let mut pointer = pointer;
        let (shown, _) = pointer.moved(Pos2::new(10.0, 300.0));
        assert!(shown.moves.is_empty());
        assert!(
            pointer
                .ctx
                .data(|data| data.get_temp::<Hold>(Id::new(("alert-line", 1)).with("hold")))
                .is_some_and(|hold| hold.price == 104.0)
        );
    }

    #[test]
    fn a_tag_let_go_where_it_started_saves_nothing() {
        let mut pointer = Pointer::new(line(false));
        pointer.drag_to(160.0);
        pointer.moved(Pointer::tag());
        assert!(pointer.release(100.0).moves.is_empty());
    }

    #[test]
    fn escape_mid_drag_puts_the_tag_back_and_saves_nothing() {
        let mut pointer = Pointer::new(line(false));
        pointer.drag_to(160.0);
        // egui ends the drag on Esc and reports it as a stop; that is not a drop.
        let (esc_frame, _) = pointer.frame(vec![escape()]);
        assert!(esc_frame.moves.is_empty());
        let key = Id::new(("alert-line", 1)).with("y");
        assert!(pointer.ctx.data(|data| data.get_temp::<f32>(key)).is_none());
        pointer.moved(Pos2::new(Pointer::tag().x, 200.0));
        assert!(pointer.release(200.0).moves.is_empty());
        // The next drag works again.
        pointer.drag_to(160.0);
        assert_eq!(pointer.release(160.0).moves.len(), 1);
    }
}

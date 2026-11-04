//! Price alerts on the chart: a dashed line at each alert's level, ending in a small arrow against
//! the right edge of the plot. The arrow is all a line shows until it is clicked; the click opens a
//! pill beside it (a bell, the level, and a bin that deletes the alert) and the arrow, now turned
//! the other way, closes it again. Right-clicking the arrow or the pill opens the alert's menu
//! (edit, pause, delete); dragging either up or down moves the level, as for a paper order. The
//! line itself only marks the level.
//!
//! Lines sit on the chart's own price scale, like the paper-trading markers, so they stay on
//! their level through zooms and axis drags. Pure view: it reports the clicks and the drop, the
//! workspace acts. Which pills are open is the card's (`ChartCard::alert_open`), so a snapshot
//! shows what the chart shows.

use super::chart_card::PriceScale;
use crate::model::PriceFormat;
use crate::model::alert::AlertId;
use crate::model::alert_line::{AlertLine, above_after_drag, dropped_level};
use crate::ui::icons::{self, Icon};
use crate::ui::kit::RED;
use crate::ui::{accessibility, theme};
use eframe::egui::{
    self, Color32, CornerRadius, CursorIcon, FontId, Id, Key, Pos2, Rect, Sense, Shape, Stroke,
    Vec2,
};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

/// How tall the arrow's tab and the pill are.
const HEIGHT: f32 = 22.0;
/// The gap between the arrow's tab and the edge of the plot.
const MARGIN: f32 = 6.0;
const HANDLE_WIDTH: f32 = 16.0;
/// The gap between the pill and the arrow.
const PILL_GAP: f32 = 4.0;
const PAD: f32 = 9.0;
const ICON: f32 = 16.0;
const ICON_GAP: f32 = 6.0;
/// The arrow that says what the alert waits for, between the bell and the price.
const DIRECTION: f32 = 12.0;
const DIRECTION_GAP: f32 = 5.0;
const BIN_WIDTH: f32 = 28.0;
const PILL_RADIUS: f32 = 6.0;
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
    /// Arrows clicked: the pill of each opens if it was shut and shuts if it was open.
    pub toggled: Vec<AlertId>,
    /// Bins clicked.
    pub deleted: Vec<AlertId>,
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

/// What a screen reader says of the line: what the alert waits for, at the chart's price format.
/// The pill itself shows a bell and the level, never the word.
fn caption(line: &AlertLine, format: PriceFormat) -> String {
    let price = format.format(line.price);
    let direction = match (line.percent, line.above) {
        (false, true) => "↑",
        (false, false) => "↓",
        (true, true) => "↗",
        (true, false) => "↘",
    };
    if line.paused {
        format!("Alert {direction} {price} · paused")
    } else {
        format!("Alert {direction} {price}")
    }
}

/// The arrow of what the alert waits for: straight up or down for a price that is crossed, slanted
/// for a rise or fall by a percentage.
fn direction_icon(line: &AlertLine) -> Icon {
    match (line.percent, line.above) {
        (false, true) => Icon::ArrowUp,
        (false, false) => Icon::ArrowDown,
        (true, true) => Icon::ArrowUpRight,
        (true, false) => Icon::ArrowDownRight,
    }
}

/// An open pill: its outline, and the bin at its right end.
#[derive(Clone)]
struct PillPlace {
    body: Rect,
    bin: Rect,
    price: Arc<egui::Galley>,
}

/// Where a line's arrow and pill sit.
#[derive(Clone)]
struct Placed {
    handle: Rect,
    pill: Option<PillPlace>,
}

impl Placed {
    /// Where the dashed line stops: at the pill, else at the arrow.
    fn line_end(&self) -> f32 {
        self.pill
            .as_ref()
            .map_or(self.handle.left(), |pill| pill.body.left())
    }
}

/// Which parts of a line the pointer is on.
#[derive(Clone, Copy, Default)]
struct Hover {
    handle: bool,
    body: bool,
    bin: bool,
}

/// The alert's colour: the app's yellow on dark, a deeper amber on light, where yellow is too
/// pale to read as a thin dashed line.
fn alert_color() -> Color32 {
    theme::pick(theme::YELLOW, Color32::from_rgb(196, 118, 0))
}

fn tint_of(line: &AlertLine) -> Color32 {
    if line.paused {
        theme::text_tertiary()
    } else {
        alert_color()
    }
}

/// Where the arrow and (when `open`) the pill of a line at `y` sit. Nothing when the level is off
/// the plot.
fn place(
    painter: &egui::Painter,
    plot: Rect,
    y: f32,
    line: &AlertLine,
    format: PriceFormat,
    open: bool,
) -> Option<Placed> {
    if !y.is_finite() || !plot.y_range().contains(y) {
        return None;
    }
    let handle = Rect::from_center_size(
        Pos2::new(plot.right() - MARGIN - HANDLE_WIDTH / 2.0, y),
        Vec2::new(HANDLE_WIDTH, HEIGHT),
    );
    let pill = open.then(|| {
        let color = if line.paused {
            theme::text_tertiary()
        } else {
            theme::text_primary()
        };
        let price =
            painter.layout_no_wrap(format.format(line.price), FontId::proportional(11.5), color);
        let width =
            PAD + ICON + ICON_GAP + DIRECTION + DIRECTION_GAP + price.size().x + PAD + BIN_WIDTH;
        let body = Rect::from_min_size(
            Pos2::new(handle.left() - PILL_GAP - width, y - HEIGHT / 2.0),
            Vec2::new(width, HEIGHT),
        );
        let bin = Rect::from_min_max(Pos2::new(body.right() - BIN_WIDTH, body.top()), body.max);
        PillPlace { body, bin, price }
    });
    Some(Placed { handle, pill })
}

/// Paints a line's dashes, arrow and pill where `placed` says, lit as `hover` says.
fn draw(painter: &egui::Painter, plot: Rect, line: &AlertLine, placed: &Placed, hover: Hover) {
    let tint = tint_of(line);
    let y = placed.handle.center().y;
    let stroke = Stroke::new(
        theme::pick_value(1.0, 1.25),
        tint.gamma_multiply(theme::pick_value(0.75, 0.95)),
    );
    for dash in Shape::dashed_line(
        &[Pos2::new(plot.left(), y), Pos2::new(placed.line_end(), y)],
        stroke,
        2.0,
        4.0,
    ) {
        painter.add(dash);
    }

    let open = placed.pill.is_some();
    let tab = placed.handle.shrink2(Vec2::new(0.5, 2.0));
    painter.rect_filled(tab, 4.0, theme::chrome().gamma_multiply(0.94));
    painter.rect_stroke(
        tab,
        4.0,
        Stroke::new(
            1.0,
            tint.gamma_multiply(if hover.handle { 0.95 } else { 0.4 }),
        ),
        egui::StrokeKind::Inside,
    );
    arrow(
        painter,
        tab.center(),
        open,
        if line.paused && !hover.handle {
            theme::text_tertiary()
        } else {
            theme::text_primary()
        },
    );

    let Some(pill) = &placed.pill else {
        return;
    };
    let lit = hover.body || hover.bin;
    painter.rect_filled(pill.body, PILL_RADIUS, theme::chrome().gamma_multiply(0.96));
    if hover.bin {
        painter.rect_filled(
            pill.bin.shrink(1.0),
            CornerRadius {
                nw: 0,
                sw: 0,
                ne: PILL_RADIUS as u8 - 1,
                se: PILL_RADIUS as u8 - 1,
            },
            theme::with_alpha(RED, 0.16),
        );
    }
    painter.rect_stroke(
        pill.body,
        PILL_RADIUS,
        Stroke::new(1.0, tint.gamma_multiply(if lit { 0.95 } else { 0.7 })),
        egui::StrokeKind::Inside,
    );
    painter.line_segment(
        [
            Pos2::new(pill.bin.left(), pill.body.top() + 1.0),
            Pos2::new(pill.bin.left(), pill.body.bottom() - 1.0),
        ],
        Stroke::new(1.0, theme::separator()),
    );
    let centre = pill.body.center().y;
    icons::paint(
        painter,
        Rect::from_center_size(
            Pos2::new(pill.body.left() + PAD + ICON / 2.0, centre),
            Vec2::splat(ICON),
        ),
        if line.paused {
            Icon::BellSlash
        } else {
            Icon::BellFill
        },
        tint,
    );
    let direction = pill.body.left() + PAD + ICON + ICON_GAP;
    icons::paint(
        painter,
        Rect::from_center_size(
            Pos2::new(direction + DIRECTION / 2.0, centre),
            Vec2::splat(DIRECTION),
        ),
        direction_icon(line),
        if line.paused {
            theme::text_tertiary()
        } else {
            theme::text_secondary()
        },
    );
    painter.galley(
        Pos2::new(
            direction + DIRECTION + DIRECTION_GAP,
            centre - pill.price.size().y / 2.0,
        ),
        pill.price.clone(),
        theme::text_primary(),
    );
    icons::paint(
        painter,
        Rect::from_center_size(pill.bin.center(), Vec2::splat(ICON)),
        Icon::Trash,
        if hover.bin {
            RED
        } else {
            theme::text_secondary()
        },
    );
}

/// The small filled triangle in the arrow's tab: pointing left (at the chart) while the pill is
/// shut, right (at the axis) while it is open.
fn arrow(painter: &egui::Painter, centre: Pos2, open: bool, color: Color32) {
    let (half_width, half_height) = (2.6, 4.2);
    let (tip, base) = if open {
        (half_width, -half_width)
    } else {
        (-half_width, half_width)
    };
    painter.add(Shape::convex_polygon(
        vec![
            Pos2::new(centre.x + tip, centre.y),
            Pos2::new(centre.x + base, centre.y - half_height),
            Pos2::new(centre.x + base, centre.y + half_height),
        ],
        color,
        Stroke::NONE,
    ));
}

/// Draws the alert lines over `plot` and nothing else: no tag answers a pointer. What a
/// snapshot shows.
pub(super) fn paint(
    painter: &egui::Painter,
    plot: Rect,
    scale: &PriceScale,
    lines: &[AlertLine],
    format: PriceFormat,
    open: &HashSet<AlertId>,
) {
    for line in lines {
        if let Some(placed) = place(
            painter,
            plot,
            scale.y(line.price),
            line,
            format,
            open.contains(&line.id),
        ) {
            draw(painter, plot, line, &placed, Hover::default());
        }
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

/// Draws the alert lines over `plot` and returns what the user did to them this frame: the arrows
/// clicked, the bins clicked, the tag right-clicked and the ones dropped on a new level.
pub(super) fn show(
    ui: &mut egui::Ui,
    plot: Rect,
    scale: &PriceScale,
    lines: &[AlertLine],
    format: PriceFormat,
    market: Market,
    open: &HashSet<AlertId>,
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
        let Some(placed) = place(&painter, plot, y, &drawn, format, open.contains(&line.id)) else {
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
        let caption = caption(line, format);
        let handle = accessibility::label(
            ui.interact(placed.handle, id.with("handle"), sense),
            format!(
                "{} details, {caption}",
                if placed.pill.is_some() {
                    "Hide"
                } else {
                    "Show"
                }
            ),
        );
        let body = placed
            .pill
            .as_ref()
            .map(|pill| accessibility::label(ui.interact(pill.body, id, sense), caption.clone()));
        let bin = placed.pill.as_ref().map(|pill| {
            accessibility::label(
                ui.interact(pill.bin, id.with("bin"), Sense::click()),
                format!("Delete alert at {}", format.format(line.price)),
            )
        });

        let hover = Hover {
            handle: handle.hovered() || handle.dragged(),
            body: body.as_ref().is_some_and(|r| r.hovered() || r.dragged()),
            bin: bin.as_ref().is_some_and(|r| r.hovered()),
        };
        draw(&painter, plot, &drawn, &placed, hover);

        let tips = if line.editable {
            (
                "Drag to change the price · right-click for options",
                " · drag to change the price",
            )
        } else {
            ("Right-click for options", "")
        };
        let mut handle = handle;
        let mut body = body;
        if handle.dragged() || body.as_ref().is_some_and(|r| r.dragged()) {
            ui.ctx().set_cursor_icon(CursorIcon::ResizeVertical);
        } else if handle.hovered() || bin.as_ref().is_some_and(|r| r.hovered()) {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        } else if line.editable && hover.body {
            ui.ctx().set_cursor_icon(CursorIcon::ResizeVertical);
        }
        if !handle.dragged() {
            handle = handle.on_hover_text(format!(
                "{}{}",
                if placed.pill.is_some() {
                    "Hide details"
                } else {
                    "Show details"
                },
                tips.1
            ));
        }
        if let Some(response) = body.take() {
            body = Some(if response.dragged() {
                response
            } else {
                response.on_hover_text(tips.0)
            });
        }
        let bin = bin.map(|response| response.on_hover_text("Delete alert"));

        if line.editable {
            follow_drag(ui, &handle, id, line, scale, plot, market, &mut shown, now);
            if let Some(response) = &body {
                follow_drag(ui, response, id, line, scale, plot, market, &mut shown, now);
            }
        }
        if handle.clicked() {
            shown.toggled.push(line.id);
            // The card reads it next frame.
            ui.ctx().request_repaint();
        }
        if bin.is_some_and(|response| response.clicked()) {
            shown.deleted.push(line.id);
            ui.ctx().request_repaint();
        }
        let right_clicked = [Some(&handle), body.as_ref()]
            .into_iter()
            .flatten()
            .find(|response| response.secondary_clicked());
        if let Some(response) = right_clicked
            && let Some(at) = response.interact_pointer_pos()
        {
            shown.menu = Some(LineMenu { id: line.id, at });
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
    id: Id,
    line: &AlertLine,
    scale: &PriceScale,
    plot: Rect,
    market: Market,
    shown: &mut Shown,
    now: f64,
) {
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
            percent: false,
            editable: true,
        }
    }

    /// 80 at the bottom of a 400pt plot, 120 at the top: 10pt a unit.
    fn scale() -> PriceScale {
        PriceScale::linear(80.0, 120.0, 0.0, 400.0)
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
            percent: false,
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
    fn the_pill_arrow_is_one_of_four_by_side_and_by_percent() {
        let of = |above, percent| {
            direction_icon(&AlertLine {
                above,
                percent,
                ..line(above)
            })
        };
        assert_eq!(of(true, false), Icon::ArrowUp);
        assert_eq!(of(false, false), Icon::ArrowDown);
        assert_eq!(of(true, true), Icon::ArrowUpRight);
        assert_eq!(of(false, true), Icon::ArrowDownRight);
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
        // The tag's centre stops half a tag (11pt) below the top: 120 - 11/10.
        assert_eq!(top.price, 118.9);
        assert_eq!(clamped_y(plot(), 900.0), 400.0 - HEIGHT / 2.0);
    }

    #[test]
    fn a_level_that_is_not_a_price_is_refused() {
        let flat = PriceScale::linear(0.0, 4.0, 0.0, 400.0);
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
        let btc = PriceScale::linear(117_000.0, 119_000.0, 0.0, 400.0);
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
        let token = PriceScale::linear(0.000_01, 0.000_02, 0.0, 400.0);
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
        open: HashSet<AlertId>,
    }

    impl Pointer {
        fn new(line: AlertLine) -> Self {
            Self {
                ctx: egui::Context::default(),
                time: 0.0,
                line,
                open: HashSet::new(),
            }
        }

        /// A pointer over a line whose pill is open.
        fn opened(line: AlertLine) -> Self {
            let mut pointer = Self::new(line);
            pointer.open.insert(pointer.line.id);
            pointer
        }

        /// Over the arrow of a line at 110, which sits at y = 100.
        fn tag() -> Pos2 {
            Pos2::new(plot().right() - MARGIN - HANDLE_WIDTH / 2.0, 100.0)
        }

        /// Over the price of an open pill.
        fn body() -> Pos2 {
            Pos2::new(
                plot().right() - MARGIN - HANDLE_WIDTH - PILL_GAP - BIN_WIDTH - 20.0,
                100.0,
            )
        }

        /// Over the bin of an open pill.
        fn bin() -> Pos2 {
            Pos2::new(
                plot().right() - MARGIN - HANDLE_WIDTH - PILL_GAP - BIN_WIDTH / 2.0,
                100.0,
            )
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
                shown = show(
                    ui,
                    plot(),
                    &scale(),
                    line,
                    PriceFormat::AUTO,
                    market,
                    &self.open,
                );
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

        /// Hovers `at` until the context knows it is there.
        fn hover_at(&mut self, at: Pos2) -> CursorIcon {
            self.moved(at);
            self.moved(at);
            self.frame(Vec::new()).1
        }

        /// Hovers the arrow.
        fn hover(&mut self) -> CursorIcon {
            self.hover_at(Self::tag())
        }

        /// Presses and releases `button` at `at` without moving, and returns everything the
        /// frames of it reported.
        fn click(&mut self, at: Pos2, button: egui::PointerButton) -> Shown {
            self.hover_at(at);
            let mut all = Shown::default();
            let mut press = |pointer: &mut Self, events: Vec<egui::Event>| {
                let shown = pointer.frame(events).0;
                all.menu = all.menu.or(shown.menu);
                all.moves.extend(shown.moves);
                all.toggled.extend(shown.toggled);
                all.deleted.extend(shown.deleted);
            };
            for pressed in [true, false] {
                press(
                    self,
                    vec![egui::Event::PointerButton {
                        pos: at,
                        button,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    }],
                );
            }
            press(self, Vec::new());
            all
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
    fn the_arrow_is_a_button_and_the_pill_shows_the_up_down_cursor() {
        let mut pointer = Pointer::opened(line(false));
        assert_eq!(pointer.hover(), CursorIcon::PointingHand);
        assert_eq!(pointer.hover_at(Pointer::bin()), CursorIcon::PointingHand);
        assert_eq!(
            pointer.hover_at(Pointer::body()),
            CursorIcon::ResizeVertical
        );
    }

    #[test]
    fn a_tag_that_cannot_move_keeps_the_ordinary_cursor_and_drops_nothing() {
        let mut pointer = Pointer::opened(AlertLine {
            editable: false,
            ..line(false)
        });
        assert_eq!(pointer.hover_at(Pointer::body()), CursorIcon::Default);
        pointer.drag_to(160.0);
        assert!(pointer.release(160.0).moves.is_empty());
    }

    #[test]
    fn clicking_the_arrow_asks_to_open_or_shut_the_pill_and_moves_nothing() {
        let mut pointer = Pointer::new(line(false));
        let shown = pointer.click(Pointer::tag(), egui::PointerButton::Primary);
        assert_eq!(shown.toggled, [1]);
        assert!(shown.moves.is_empty() && shown.deleted.is_empty());
        // Open, the same arrow is how it shuts.
        let mut pointer = Pointer::opened(line(false));
        let shown = pointer.click(Pointer::tag(), egui::PointerButton::Primary);
        assert_eq!(shown.toggled, [1]);
    }

    #[test]
    fn the_bin_deletes_and_the_rest_of_the_pill_does_not() {
        let mut pointer = Pointer::opened(line(false));
        let shown = pointer.click(Pointer::body(), egui::PointerButton::Primary);
        assert!(shown.deleted.is_empty() && shown.toggled.is_empty());
        let shown = pointer.click(Pointer::bin(), egui::PointerButton::Primary);
        assert_eq!(shown.deleted, [1]);
        assert!(shown.toggled.is_empty() && shown.moves.is_empty());
        // A shut pill has no bin to hit.
        let mut pointer = Pointer::new(line(false));
        let shown = pointer.click(Pointer::bin(), egui::PointerButton::Primary);
        assert!(shown.deleted.is_empty());
    }

    #[test]
    fn right_clicking_the_arrow_or_the_pill_opens_the_menu() {
        for (pointer, at) in [
            (Pointer::new(line(false)), Pointer::tag()),
            (Pointer::opened(line(false)), Pointer::body()),
        ] {
            let mut pointer = pointer;
            let shown = pointer.click(at, egui::PointerButton::Secondary);
            assert_eq!(shown.menu.map(|menu| menu.id), Some(1));
            assert!(shown.toggled.is_empty());
        }
    }

    #[test]
    fn an_open_pill_is_dragged_like_the_arrow() {
        let mut pointer = Pointer::opened(line(false));
        pointer.hover_at(Pointer::body());
        pointer.button(Pointer::body(), true);
        for step in 1..=4 {
            let to = Pos2::new(Pointer::body().x, 100.0 + 60.0 * step as f32 / 4.0);
            pointer.moved(to);
        }
        let at = Pos2::new(Pointer::body().x, 160.0);
        let mut shown = pointer.button(at, false).0;
        shown.moves.extend(pointer.frame(Vec::new()).0.moves);
        assert_eq!(
            shown.moves,
            [AlertMove {
                id: 1,
                price: 104.0
            }]
        );
        assert!(shown.toggled.is_empty(), "a drag is not a click");
    }

    #[test]
    fn a_shut_line_draws_no_words_and_an_open_one_only_its_price() {
        fn texts(open: bool) -> Vec<String> {
            let ctx = egui::Context::default();
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0))),
                ..Default::default()
            };
            let lines = [line(false)];
            let open = if open {
                HashSet::from([1])
            } else {
                HashSet::new()
            };
            let mut output = ctx.run_ui(raw, |ui| {
                let painter = ui.painter().clone();
                paint(&painter, plot(), &scale(), &lines, PriceFormat::AUTO, &open);
            });
            output.textures_delta.clear();
            let mut found = Vec::new();
            fn walk(shape: &Shape, found: &mut Vec<String>) {
                match shape {
                    Shape::Text(text) => found.push(text.galley.text().to_owned()),
                    Shape::Vec(shapes) => shapes.iter().for_each(|shape| walk(shape, found)),
                    _ => {}
                }
            }
            for clipped in &output.shapes {
                walk(&clipped.shape, &mut found);
            }
            found
        }
        assert!(texts(false).is_empty());
        assert_eq!(texts(true), ["110.00"]);
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

    /// Written by hand: `DEGENVIEW_GALLERY_DIR=<dir> cargo test --lib alert_lines::tests::gallery -- --ignored`.
    #[test]
    #[ignore = "writes pictures"]
    fn gallery_of_the_alert_lines() {
        use crate::ui::render_tests::Screen;
        let lines: Vec<AlertLine> = [
            (1, 110.0, false, true, false),
            (2, 105.0, false, true, true),
            (3, 100.0, false, false, false),
            (4, 95.0, true, false, true),
            (5, 90.0, true, true, false),
        ]
        .into_iter()
        .map(|(id, price, paused, above, percent)| AlertLine {
            id,
            price,
            paused,
            above,
            percent,
            editable: true,
        })
        .collect();
        let open = HashSet::from([2, 3, 4]);
        let market = Market {
            last: Some(100.0),
            tick: 0.01,
            ..Market::default()
        };
        let y_of = |id: u64| 400.0 - (lines[id as usize - 1].price as f32 - 80.0) * 10.0 - 0.0;
        let right = plot().right();
        let bin_x = right - MARGIN - HANDLE_WIDTH - PILL_GAP - BIN_WIDTH / 2.0;
        let handle_x = right - MARGIN - HANDLE_WIDTH / 2.0;
        let _restore = theme::RestoreDefaultTheme;
        for (name, hover, light) in [
            ("alerts-rest", None, false),
            ("alerts-hover-bin", Some(Pos2::new(bin_x, y_of(3))), false),
            (
                "alerts-hover-arrow",
                Some(Pos2::new(handle_x, y_of(1))),
                false,
            ),
            ("alerts-rest-light", None, true),
            (
                "alerts-hover-bin-light",
                Some(Pos2::new(bin_x, y_of(3))),
                true,
            ),
        ] {
            let mut screen = Screen::new(Vec2::new(600.0, 400.0));
            if light {
                screen.ctx.set_theme(egui::ThemePreference::Light);
            }
            let mut last = None;
            for _ in 0..4 {
                let events = hover.map(egui::Event::PointerMoved).into_iter().collect();
                last = Some(screen.run(events, |ui| {
                    theme::sync(ui.ctx());
                    let rect = plot();
                    ui.painter().rect_filled(rect, 0.0, theme::background());
                    // A few bars to sit behind the pills.
                    for index in 0..30 {
                        let x = 20.0 + index as f32 * 17.0;
                        let wave = (index as f32 * 0.7).sin() * 60.0;
                        ui.painter().rect_filled(
                            Rect::from_center_size(
                                Pos2::new(x, 200.0 + wave),
                                Vec2::new(8.0, 50.0),
                            ),
                            1.0,
                            if index % 3 == 0 {
                                theme::DOWN
                            } else {
                                theme::UP
                            },
                        );
                    }
                    show(ui, rect, &scale(), &lines, PriceFormat::AUTO, market, &open);
                }));
            }
            screen.save(name, last.unwrap());
        }
    }
}

//! Live orders on a chart card: the SELL / BUY pills in the card header (with the broker's own best
//! bid and ask), and a line and pill on the plot for each working order.
//!
//! The lines sit on the chart's own price scale, like paper trading's, and look the same on
//! purpose where they can: but they are amber-tagged "LIVE" wherever they could be mistaken for
//! paper, and they behave more carefully:
//!
//! * A line is the order **as the broker has it**. Dragging one shows where it would land, and on
//!   release it proposes a change; the line stays at its price until the broker confirms the edit.
//!   Escape while dragging drops the proposal.
//! * ✕ asks to cancel; the line stays, marked "cancel requested", until the broker confirms.
//! * Nothing here places or changes anything: the card returns actions and the workspace opens the
//!   ticket, the edit sheet or the cancel question, each of which shows what will be sent.
//! * Live orders are never drawn into a shared snapshot of the chart.

use degenview_brokers::{Decimal, OrderId, Side};
use eframe::egui::{self, Color32, FontId, Pos2, Rect, Sense, Vec2};

use super::chart_card::PriceScale;
use super::paper_chart_trading::{Drag, Pill, marker};
use crate::model::broker_chart::{Unavailable, snap_to_increment};
use crate::model::broker_ticket::display;
use crate::ui::accessibility;
use crate::ui::broker_badge::LIVE;
use crate::ui::theme;

/// One working order, as a line.
#[derive(Clone, Debug, PartialEq)]
pub struct OrderLine {
    pub id: OrderId,
    pub side: Side,
    /// `LMT`, `STP-LMT`...
    pub tag: String,
    /// Where the line sits (a pixel position; the figures shown come from `price_text`).
    pub price: f64,
    pub price_text: String,
    /// The size still to fill, with its unit.
    pub remaining_text: String,
    /// The line's tint: buy or sell.
    pub tint: Color32,
    /// "Cancel requested" and the like.
    pub note: Option<&'static str>,
    pub can_edit: bool,
    pub can_cancel: bool,
}

/// What the user did to the lines this frame.
#[derive(Clone, Debug, PartialEq)]
pub enum BrokerOverlayAction {
    /// ✕: ask to cancel the order.
    Cancel(OrderId),
    /// A line was dropped on a new price: propose the change (nothing is sent yet).
    ProposeEdit { id: OrderId, price: Decimal },
    /// A line was clicked: show the order.
    Open(OrderId),
}

/// Everything a chart card needs for live trading, and what the user did to it. The workspace
/// fills the inputs each frame and drains the outputs; the default draws nothing.
#[derive(Default)]
pub struct BrokerCardState {
    /// In: a live venue is active and this chart's market resolves to the broker's: draw its lines.
    pub show_lines: bool,
    /// In: draw the SELL / BUY pills. `Err` says why not and is their tooltip.
    pub order_entry: Option<Result<(), Unavailable>>,
    pub broker_name: String,
    pub bid: Option<String>,
    pub ask: Option<String>,
    pub lines: Vec<OrderLine>,
    /// In: the market's price step, to put a dropped line on it.
    pub price_increment: Option<Decimal>,
    /// Out: what the user did to the lines.
    pub actions: Vec<BrokerOverlayAction>,
}

impl BrokerCardState {
    /// Whether the header has pills to show (a live venue with a market to trade).
    pub fn pills_visible(&self) -> bool {
        self.order_entry.is_some()
    }

    pub(super) fn show_markers(
        &self,
        ui: &mut egui::Ui,
        plot: Rect,
        scale: &PriceScale,
    ) -> Vec<BrokerOverlayAction> {
        if !self.show_lines || self.lines.is_empty() {
            return Vec::new();
        }
        let mut actions = Vec::new();
        let esc = ui.input(|input| input.key_pressed(egui::Key::Escape));
        for line in &self.lines {
            let id = egui::Id::new(("broker-order", line.id.as_str()));
            let dragging = ui.ctx().data(|data| data.get_temp::<f32>(id.with("y")));
            let dropped_flag = id.with("dropped-by-escape");
            if dragging.is_some() && esc {
                // Escape drops the proposal: the line goes back, nothing is asked.
                ui.ctx().data_mut(|data| {
                    data.remove::<f32>(id.with("y"));
                    data.insert_temp(dropped_flag, true);
                });
            }
            let dragging = ui.ctx().data(|data| data.get_temp::<f32>(id.with("y")));
            let landing = dragging.and_then(|y| self.landing_price(scale, y));
            let y = dragging.unwrap_or_else(|| scale.y(line.price));
            let shown = landing.map_or_else(|| line.price_text.clone(), display);
            let pill = line_pill(line, &shown);
            let result = marker(ui, plot, y, &pill, id, line.can_edit);
            if result.closed && line.can_cancel {
                ui.ctx().data_mut(|data| data.remove::<f32>(id.with("y")));
                actions.push(BrokerOverlayAction::Cancel(line.id.clone()));
                continue;
            }
            match result.drag {
                Drag::Moving(y) => {
                    let cancelled = ui
                        .ctx()
                        .data(|data| data.get_temp::<bool>(dropped_flag))
                        .unwrap_or(false);
                    if !cancelled {
                        ui.ctx().data_mut(|data| data.insert_temp(id.with("y"), y));
                    }
                }
                Drag::Dropped(y) => {
                    let cancelled = ui.ctx().data_mut(|data| {
                        data.remove::<f32>(id.with("y"));
                        let flag = data.get_temp::<bool>(dropped_flag).unwrap_or(false);
                        data.remove::<bool>(dropped_flag);
                        flag
                    });
                    if !cancelled
                        && let Some(price) = self.landing_price(scale, y)
                        && Some(price) != price_of(line)
                    {
                        actions.push(BrokerOverlayAction::ProposeEdit {
                            id: line.id.clone(),
                            price,
                        });
                    }
                }
                Drag::None => {}
            }
            if result.clicked && !result.closed && line.can_edit && dragging.is_none() {
                actions.push(BrokerOverlayAction::Open(line.id.clone()));
            }
        }
        actions
    }

    /// Where a dragged line would land: the pointer's price on the chart's scale, on the market's
    /// increment. `None` without a price step (so nothing is proposed that could not be valid).
    fn landing_price(&self, scale: &PriceScale, y: f32) -> Option<Decimal> {
        snap_to_increment(scale.price(y), self.price_increment?)
    }
}

fn price_of(line: &OrderLine) -> Option<Decimal> {
    snap_to_increment(line.price, Decimal::new(1, 8))
}

/// What a line's pill says.
pub fn line_pill(line: &OrderLine, price_text: &str) -> Pill {
    let side = match line.side {
        Side::Buy => "BUY",
        Side::Sell => "SELL",
    };
    let mut text = format!("{side} {} @ {price_text}", line.remaining_text);
    if let Some(note) = line.note {
        text.push_str(&format!(" · {note}"));
    }
    Pill {
        tag: format!("LIVE {}", line.tag),
        tint: line.tint,
        text,
        pnl: None,
        close_label: format!("Cancel live {} order at {price_text}", side.to_lowercase()),
    }
}

/// The room SELL and BUY take in the header, from the prices they show. A long price (74,396.74
/// against 0.5412) is wider than any fixed guess, and a header that reserved too little pushed the
/// pills out past its own edge, where they could not be clicked.
pub fn quick_trade_width(ui: &egui::Ui, bid: Option<&str>, ask: Option<&str>) -> f32 {
    let pill = |word: &str, price: Option<&str>| {
        let text = match price {
            Some(price) => format!("{word}  {price}"),
            None => word.to_owned(),
        };
        ui.painter()
            .layout_no_wrap(text, FontId::proportional(11.5), theme::DOWN)
            .size()
            .x
            + 18.0
    };
    pill("SELL", bid) + pill("BUY", ask) + 4.0
}

/// SELL and BUY in the card header with the broker's own best bid and ask (not the chart's last
/// price: they are not the same price and only the broker's can be traded). They open the ticket;
/// nothing is placed from here. Returns the side pressed.
pub fn quick_trade_buttons(
    ui: &mut egui::Ui,
    broker: &str,
    bid: Option<&str>,
    ask: Option<&str>,
    entry: Result<(), Unavailable>,
) -> Option<Side> {
    let mut pressed = None;
    ui.spacing_mut().item_spacing.x = 4.0;
    for (side, word, price, tint) in [
        (Side::Sell, "SELL", bid, theme::DOWN),
        (Side::Buy, "BUY", ask, theme::UP),
    ] {
        let text = match price {
            Some(price) => format!("{word}  {price}"),
            None => word.to_owned(),
        };
        let galley = ui
            .painter()
            .layout_no_wrap(text.clone(), FontId::proportional(11.5), tint);
        let size = Vec2::new(galley.size().x + 18.0, 24.0);
        let (rect, response) = ui.allocate_exact_size(size, Sense::click());
        let enabled = entry.is_ok();
        let alpha = match (enabled, response.hovered()) {
            (false, _) => 0.06,
            (true, true) => 0.26,
            (true, false) => 0.14,
        };
        ui.painter()
            .rect_filled(rect, 6.0, tint.gamma_multiply(alpha));
        ui.painter().rect_stroke(
            rect,
            6.0,
            egui::Stroke::new(1.0, LIVE.gamma_multiply(if enabled { 0.55 } else { 0.2 })),
            egui::StrokeKind::Inside,
        );
        ui.painter().galley(
            Pos2::new(rect.left() + 9.0, rect.center().y - galley.size().y / 2.0),
            galley,
            if enabled {
                tint
            } else {
                tint.gamma_multiply(0.45)
            },
        );
        let help = match entry {
            Ok(()) => format!(
                "Live: {} at {broker}. Opens the order ticket; nothing is sent from here.",
                word.to_lowercase()
            ),
            Err(reason) => reason.text().to_owned(),
        };
        let response =
            accessibility::label(response, format!("Live {word} at {broker}")).on_hover_text(help);
        if enabled && response.clicked() {
            pressed = Some(side);
        }
    }
    pressed
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn d(text: &str) -> Decimal {
        Decimal::from_str(text).unwrap()
    }

    fn line() -> OrderLine {
        OrderLine {
            id: OrderId::new("o1"),
            side: Side::Buy,
            tag: "LMT".to_owned(),
            price: 100.0,
            price_text: "100.00".to_owned(),
            remaining_text: "0.5".to_owned(),
            tint: theme::UP,
            note: None,
            can_edit: true,
            can_cancel: true,
        }
    }

    #[test]
    fn a_pill_says_it_is_live_what_it_is_and_what_cancelling_it_does() {
        let pill = line_pill(&line(), "100.00");
        assert_eq!(pill.tag, "LIVE LMT");
        assert_eq!(pill.text, "BUY 0.5 @ 100.00");
        assert!(pill.close_label.contains("live buy order"));
        let mut asked = line();
        asked.note = Some("cancel requested");
        assert!(
            line_pill(&asked, "100.00")
                .text
                .ends_with("cancel requested")
        );
    }

    #[test]
    fn a_dropped_line_lands_on_the_markets_price_step_and_never_without_one() {
        let mut state = BrokerCardState {
            price_increment: Some(d("0.5")),
            ..BrokerCardState::default()
        };
        let scale = PriceScale::linear(0.0, 200.0, 0.0, 200.0);
        // y = 100 is price 100; y = 99.8 is 100.2, which rounds to the step 100.0.
        assert_eq!(state.landing_price(&scale, 99.8), Some(d("100")));
        assert_eq!(state.landing_price(&scale, 99.0), Some(d("101")));
        state.price_increment = None;
        assert_eq!(
            state.landing_price(&scale, 99.0),
            None,
            "no step, no proposal"
        );
    }

    #[test]
    fn nothing_is_drawn_or_offered_until_the_workspace_says_so() {
        let state = BrokerCardState::default();
        assert!(!state.pills_visible());
        assert!(!state.show_lines);
        let with_entry = BrokerCardState {
            order_entry: Some(Err(Unavailable::InReplay)),
            ..BrokerCardState::default()
        };
        assert!(
            with_entry.pills_visible(),
            "the pills show, greyed, with the reason"
        );
    }

    // ---- Drag, drop, Escape and cancel, through the real overlay ------------------------------

    use crate::ui::render_tests::Screen;

    fn button(at: Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    fn escape() -> Vec<egui::Event> {
        [true, false]
            .into_iter()
            .map(|pressed| egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: Some(egui::Key::Escape),
                pressed,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            })
            .collect()
    }

    /// Runs the overlay for a sequence of frames, each with its events; returns the actions asked.
    fn drive(state: &BrokerCardState, frames: Vec<Vec<egui::Event>>) -> Vec<BrokerOverlayAction> {
        let mut screen = Screen::new(Vec2::new(800.0, 400.0));
        let plot = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 400.0));
        // Price 0..200 over 400 points: price 100 sits at y = 200, and a point is half a price.
        let scale = PriceScale::linear(0.0, 200.0, 0.0, 400.0);
        let mut asked = Vec::new();
        for events in frames {
            let _ = screen.run(events, |ui| {
                asked.extend(state.show_markers(ui, plot, &scale));
            });
        }
        asked
    }

    fn state() -> BrokerCardState {
        BrokerCardState {
            show_lines: true,
            lines: vec![line()],
            price_increment: Some(d("0.5")),
            ..BrokerCardState::default()
        }
    }

    const BODY: Pos2 = Pos2::new(700.0, 200.0);

    #[test]
    fn dropping_a_line_proposes_its_landing_price_and_changes_nothing_by_itself() {
        let asked = drive(
            &state(),
            vec![
                vec![egui::Event::PointerMoved(BODY)],
                vec![button(BODY, true)],
                vec![egui::Event::PointerMoved(Pos2::new(700.0, 180.0))],
                vec![egui::Event::PointerMoved(Pos2::new(700.0, 150.0))],
                vec![button(Pos2::new(700.0, 150.0), false)],
                vec![],
            ],
        );
        // y = 150 is price 125.
        assert_eq!(
            asked,
            vec![BrokerOverlayAction::ProposeEdit {
                id: OrderId::new("o1"),
                price: d("125")
            }]
        );
    }

    #[test]
    fn escape_while_dragging_drops_the_proposal() {
        let mut frames = vec![
            vec![egui::Event::PointerMoved(BODY)],
            vec![button(BODY, true)],
            vec![egui::Event::PointerMoved(Pos2::new(700.0, 150.0))],
            vec![egui::Event::PointerMoved(Pos2::new(700.0, 140.0))],
            escape(),
            vec![egui::Event::PointerMoved(Pos2::new(700.0, 130.0))],
        ];
        frames.push(vec![button(Pos2::new(700.0, 130.0), false)]);
        frames.push(vec![]);
        assert_eq!(drive(&state(), frames), Vec::new(), "nothing is asked");
    }

    #[test]
    fn a_line_that_is_not_editable_does_not_move_and_one_that_is_clicked_opens() {
        let mut fixed = state();
        fixed.lines[0].can_edit = false;
        let drag = vec![
            vec![egui::Event::PointerMoved(BODY)],
            vec![button(BODY, true)],
            vec![egui::Event::PointerMoved(Pos2::new(700.0, 150.0))],
            vec![button(Pos2::new(700.0, 150.0), false)],
            vec![],
        ];
        assert_eq!(
            drive(&fixed, drag),
            Vec::new(),
            "a line that cannot be edited cannot be dragged"
        );

        let click = vec![
            vec![egui::Event::PointerMoved(BODY)],
            vec![button(BODY, true)],
            vec![button(BODY, false)],
            vec![],
        ];
        assert_eq!(
            drive(&state(), click),
            vec![BrokerOverlayAction::Open(OrderId::new("o1"))]
        );
    }

    #[test]
    fn the_x_asks_to_cancel_and_a_line_already_being_cancelled_cannot_be_asked_again() {
        // The close mark is at the pill's right end, inside the plot's right margin.
        let x_at = Pos2::new(800.0 - 8.0 - 11.0, 200.0);
        let click = vec![
            vec![egui::Event::PointerMoved(x_at)],
            vec![button(x_at, true)],
            vec![button(x_at, false)],
            vec![],
        ];
        assert_eq!(
            drive(&state(), click.clone()),
            vec![BrokerOverlayAction::Cancel(OrderId::new("o1"))]
        );
        let mut cancelling = state();
        cancelling.lines[0].can_cancel = false;
        assert_eq!(drive(&cancelling, click), Vec::new());
    }

    #[test]
    #[ignore = "writes pictures"]
    fn gallery_of_chart_trading() {
        let _restore = theme::RestoreDefaultTheme;
        for light in [false, true] {
            let mut screen = Screen::new(Vec2::new(760.0, 300.0));
            screen.ctx.set_theme(if light {
                egui::ThemePreference::Light
            } else {
                egui::ThemePreference::Dark
            });
            let mut state = state();
            state.lines = vec![
                OrderLine {
                    price: 150.0,
                    price_text: "150.00".into(),
                    ..line()
                },
                OrderLine {
                    id: OrderId::new("o2"),
                    side: Side::Sell,
                    tint: theme::DOWN,
                    price: 60.0,
                    price_text: "60.00".into(),
                    note: Some("cancel requested"),
                    can_cancel: false,
                    can_edit: false,
                    ..line()
                },
            ];
            let plot = Rect::from_min_size(Pos2::new(0.0, 40.0), Vec2::new(760.0, 260.0));
            let scale = PriceScale::linear(0.0, 200.0, 40.0, 260.0);
            let mut output = None;
            for _ in 0..3 {
                output = Some(screen.run(vec![], |ui| {
                    theme::sync(ui.ctx());
                    ui.painter().rect_filled(
                        Rect::from_min_size(Pos2::ZERO, Vec2::new(760.0, 300.0)),
                        0.0,
                        crate::ui::kit::window(),
                    );
                    let _ = state.show_markers(ui, plot, &scale);
                    let mut header = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(Rect::from_min_size(
                                Pos2::new(12.0, 8.0),
                                Vec2::new(740.0, 28.0),
                            ))
                            .layout(egui::Layout::left_to_right(egui::Align::Center)),
                    );
                    let _ = quick_trade_buttons(
                        &mut header,
                        "Acme",
                        Some("89,999.50"),
                        Some("90,000.50"),
                        Ok(()),
                    );
                    let _ = quick_trade_buttons(
                        &mut header,
                        "Acme",
                        None,
                        None,
                        Err(Unavailable::InReplay),
                    );
                }));
            }
            screen.save(
                &format!("live-chart-{}", if light { "light" } else { "dark" }),
                output.unwrap(),
            );
        }
    }
}

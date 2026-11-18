//! The values tooltip on long press: hold the button still on the plot and a floating box under the
//! pointer names the bar there (time, open, high, low, close, change, volume), the built-in
//! indicators and the scripts' plots. It follows the pointer until the button is let go, and the
//! press is the tooltip's meanwhile: the chart does not pan under it.
//!
//! Every value is one the frame already computed (`CrosshairBars`): nothing is recomputed to show
//! a tooltip. It is drawn with the crosshair, so a snapshot never has it.

use super::*;
use crate::pine::outputs::DISPLAY_DATA_WINDOW;
use crate::ui::pine::geometry::series_index;
use std::time::Duration;

/// How long the button is held still before the tooltip appears (**ours**: TradingView documents
/// no figure). Under egui's own long-touch time (0.8 s), so a touch shows it first.
const LONG_PRESS: f64 = 0.5;
/// How far (px) the pointer may stray while it counts as still: egui's own click distance.
const SLOP: f32 = 6.0;
/// The most rows one tooltip lists; the plots of a script with dozens do not fill the screen.
const MAX_ROWS: usize = 28;
/// How far the box sits from the pointer.
const OFFSET: f32 = 16.0;
const PAD: Vec2 = Vec2::new(10.0, 8.0);
const ROW_GAP: f32 = 2.0;
const COLUMN_GAP: f32 = 14.0;

/// Where a press stands on its way to the tooltip.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) enum Hold {
    /// No press, or one that is not ours.
    #[default]
    Idle,
    /// The button went down on the plot at `origin` at `since` (seconds) and has stayed there.
    Waiting { origin: Pos2, since: f64 },
    /// Long enough: the tooltip shows.
    Showing,
    /// The press moved, or something else took it: no tooltip until the button is let go.
    Spent,
}

impl ChartCard {
    /// Turns the tooltip on or off, pushed each frame from the workspace's option.
    pub fn set_values_tooltip(&mut self, on: bool) {
        self.values_tooltip = on;
        if !on {
            self.hold = Hold::Idle;
        }
    }

    /// One frame of the press: starts waiting when the button goes down on the plot and not on a
    /// drawing, gives up when the pointer strays, shows the tooltip after `LONG_PRESS` and drops it
    /// with the button. `blocked` is something else holding the pointer (a drag on a drawing, the
    /// replay pick).
    pub(super) fn handle_hold(
        &mut self,
        ui: &egui::Ui,
        response: &Response,
        plot: Rect,
        blocked: bool,
    ) {
        let (now, down, pressed, pointer) = ui.input(|input| {
            (
                input.time,
                input.pointer.primary_down(),
                input.pointer.primary_pressed(),
                input.pointer.latest_pos(),
            )
        });
        let enabled = self.values_tooltip && self.cursor.caps().values_on_hold;
        if !down || !enabled || blocked {
            self.hold = if down && enabled {
                Hold::Spent
            } else {
                Hold::Idle
            };
            return;
        }
        self.hold = match self.hold {
            Hold::Idle => match pointer.filter(|point| plot.contains(*point)) {
                Some(origin)
                    if pressed
                        && response.is_pointer_button_down_on()
                        && !self.layer.is_over_drawing() =>
                {
                    Hold::Waiting { origin, since: now }
                }
                _ => Hold::Spent,
            },
            Hold::Waiting { origin, since } => {
                if pointer.is_none_or(|point| (point - origin).length() > SLOP) {
                    Hold::Spent
                } else if now - since >= LONG_PRESS {
                    Hold::Showing
                } else {
                    ui.ctx()
                        .request_repaint_after(Duration::from_secs_f64(LONG_PRESS - (now - since)));
                    self.hold
                }
            }
            held @ (Hold::Showing | Hold::Spent) => held,
        };
    }

    /// The press is the tooltip's: the chart must not pan under it.
    pub(super) fn values_hold_claims_drag(&self) -> bool {
        self.hold == Hold::Showing
    }

    pub(super) fn shows_values_tooltip(&self) -> bool {
        self.hold == Hold::Showing
    }

    /// The tooltip's `(label, value)` rows for the bar at `index`, and how many of the first ones
    /// name the bar itself (they take its colour). Built from what the frame already holds.
    pub(super) fn values_rows(
        &self,
        info: &CrosshairBars,
        index: usize,
        candle: &Kline,
        previous_close: Option<f64>,
        show_volume: bool,
    ) -> (Vec<(String, String)>, usize) {
        let price_format = self.appearance.price_format();
        let mut rows = self.readout_rows(info, index, candle, show_volume);
        if let Some(previous) = previous_close.filter(|previous| *previous != 0.0) {
            let change = candle.close - previous;
            let sign = if change < 0.0 { "−" } else { "+" };
            rows.push((
                "Chg".to_owned(),
                format!(
                    "{sign}{} ({sign}{:.2}%)",
                    price_format.format(change.abs()),
                    (change / previous).abs() * 100.0
                ),
            ));
        }
        let bar_rows = rows.len();
        let series = info.indicators;
        let at = |values: &[Option<f64>]| values.get(index).copied().flatten();
        if let Some(value) = at(&series.ema) {
            rows.push((
                format!("EMA {}", info.indicator_config.ema_period),
                price_format.format(value),
            ));
        }
        if let Some(bands) = &series.bollinger {
            for (name, values) in [
                ("BB upper", &bands.upper),
                ("BB basis", &bands.middle),
                ("BB lower", &bands.lower),
            ] {
                if let Some(value) = at(values) {
                    rows.push((name.to_owned(), price_format.format(value)));
                }
            }
        }
        if let Some(value) = at(&series.rsi) {
            rows.push(("RSI".to_owned(), format!("{value:.2}")));
        }
        if let Some(flip) = series.trend_flips.get(index).copied().flatten() {
            let name = if flip { "Bullish flip" } else { "Bearish flip" };
            rows.push(("Supertrend".to_owned(), name.to_owned()));
        }
        // The scripts' plots that ask for the data window, on the chart and in their panes, at the
        // same bar the replay-cut candles are at.
        let hovered_from_end = info.shown_len + info.tail;
        for output in info.pine.iter().chain(&info.pane_pine) {
            for plot in output
                .plots
                .iter()
                .filter(|plot| plot.display & DISPLAY_DATA_WINDOW != 0)
            {
                let Some(value) = series_index(plot.values.len(), hovered_from_end, index)
                    .and_then(|slot| plot.values[slot])
                else {
                    continue;
                };
                let name = plot
                    .title
                    .clone()
                    .filter(|title| !title.is_empty())
                    .unwrap_or_else(|| "Plot".to_owned());
                let row = (name, price_format.format(value));
                if !rows.contains(&row) {
                    rows.push(row);
                }
            }
        }
        rows.truncate(MAX_ROWS);
        (rows, bar_rows.min(MAX_ROWS))
    }

    /// Paints the tooltip beside the live pointer, kept inside the plot.
    pub(super) fn draw_values_tooltip(
        &self,
        painter: &egui::Painter,
        plot: Rect,
        info: &CrosshairBars,
        (index, candle, previous_close): (usize, &Kline, Option<f64>),
        show_volume: bool,
    ) {
        let Some(pointer) = painter.ctx().input(|input| input.pointer.latest_pos()) else {
            return;
        };
        let (rows, bar_rows) = self.values_rows(info, index, candle, previous_close, show_volume);
        let font = FontId::monospace(11.0);
        let header = painter.layout_no_wrap(
            format::timestamp(candle.open_time),
            font.clone(),
            theme::text_secondary(),
        );
        let bar_color = self.bar_color(candle, previous_close);
        let cells: Vec<_> = rows
            .iter()
            .enumerate()
            .map(|(row, (label, value))| {
                let ink = if row < bar_rows {
                    bar_color
                } else {
                    theme::text_primary()
                };
                (
                    painter.layout_no_wrap(label.clone(), font.clone(), theme::text_secondary()),
                    painter.layout_no_wrap(value.clone(), font.clone(), ink),
                )
            })
            .collect();
        let label_width = cells
            .iter()
            .map(|(label, _)| label.size().x)
            .fold(0.0, f32::max);
        let value_width = cells
            .iter()
            .map(|(_, value)| value.size().x)
            .fold(0.0, f32::max);
        let row_height = cells
            .iter()
            .map(|(label, value)| label.size().y.max(value.size().y))
            .fold(header.size().y, f32::max);
        let width = header.size().x.max(label_width + COLUMN_GAP + value_width);
        let height = row_height * (cells.len() + 1) as f32 + ROW_GAP * cells.len() as f32;
        let size = Vec2::new(width, height) + PAD * 2.0;

        // Right and below the pointer, or the other side where there is no room.
        let room = self.below_header(plot).shrink(4.0);
        let mut min = pointer + Vec2::splat(OFFSET);
        if min.x + size.x > room.right() {
            min.x = pointer.x - OFFSET - size.x;
        }
        if min.y + size.y > room.bottom() {
            min.y = pointer.y - OFFSET - size.y;
        }
        min.x = min
            .x
            .clamp(room.left(), (room.right() - size.x).max(room.left()));
        min.y = min
            .y
            .clamp(room.top(), (room.bottom() - size.y).max(room.top()));
        let boxed = Rect::from_min_size(min, size);

        painter.add(
            egui::Shadow {
                offset: [0, 3],
                blur: 12,
                spread: 0,
                color: Color32::from_black_alpha(40),
            }
            .as_shape(boxed, 6.0),
        );
        painter.rect(
            boxed,
            6.0,
            theme::chrome().gamma_multiply(0.97),
            Stroke::new(1.0, theme::border()),
            StrokeKind::Inside,
        );
        let mut y = boxed.top() + PAD.y;
        let left = boxed.left() + PAD.x;
        painter.galley(Pos2::new(left, y), header, theme::text_secondary());
        y += row_height + ROW_GAP;
        for (label, value) in cells {
            painter.galley(Pos2::new(left, y), label, theme::text_secondary());
            let value_x = boxed.right() - PAD.x - value.size().x;
            painter.galley(Pos2::new(value_x, y), value, theme::text_primary());
            y += row_height + ROW_GAP;
        }
    }
}

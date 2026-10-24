//! "Stop at bar": pause at the start of a chosen bar, whatever the script is running there.
//!
//! The bar is chosen by its index, by clicking a candle on the preview chart, or by its date and
//! time, picked from the bars that exist. Whichever way it was chosen, the other two show it.

use eframe::egui::{self, Align, Layout, Rect, Sense, Ui, Vec2, vec2};

use super::widgets::text_field;
use super::{Context, bar_time_text};
use crate::pine::debug::BarBreak;
use crate::script_manager::debug::{DebugPanel, PanelAction, parse_bar_break};
use crate::ui::icons::{self, Icon};
use crate::ui::kit;
use crate::ui::theme;

/// The width of the labels in front of the fields.
const LABEL: f32 = 46.0;

/// Where the armed stop lands among the chart's bars.
enum Armed {
    /// The bar it stops at.
    Bar(usize),
    /// It names a bar the chart does not have.
    Missing(String),
}

fn armed(panel: &DebugPanel) -> Option<Armed> {
    let text = panel.bar_break.as_deref()?;
    let times = &panel.bar_times;
    Some(match parse_bar_break(text)? {
        BarBreak::Index(index) => match usize::try_from(index).ok().filter(|i| *i < times.len()) {
            Some(index) => Armed::Bar(index),
            None => Armed::Missing(format!(
                "Bar {index} is past the last bar{}",
                times
                    .len()
                    .checked_sub(1)
                    .map_or(String::new(), |last| format!(" ({last})"))
            )),
        },
        BarBreak::Time(time) => {
            let index = times.partition_point(|open| *open < time);
            if index < times.len() {
                Armed::Bar(index)
            } else {
                Armed::Missing(format!("No bar opens at or after {}", bar_time_text(time)))
            }
        }
    })
}

pub(super) fn show(ui: &mut Ui, context: &mut Context<'_>) {
    let panel = context.panel;
    let bars = panel.bar_times.len();
    let armed = armed(panel);
    let armed_bar = match &armed {
        Some(Armed::Bar(index)) => Some(*index),
        _ => None,
    };

    ui.add_space(6.0);
    let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(line, 0.0, kit::hairline());
    ui.add_space(10.0);

    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(12, 0))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            ui.label(kit::text(
                "Stop at bar",
                kit::size::SUBHEADLINE,
                kit::SEMIBOLD,
                theme::text_secondary(),
            ));
            ui.label(kit::text(
                "Pauses at the start of a bar, wherever the script is in it.",
                kit::size::SUBHEADLINE,
                kit::REGULAR,
                theme::text_tertiary(),
            ));
            ui.add_space(2.0);
            index_row(ui, context, bars, armed_bar);
            time_row(ui, context, armed_bar);
            if let Some(armed) = &armed {
                ui.add_space(2.0);
                status(ui, context, armed);
            }
        });
    ui.add_space(12.0);
}

/// A label in front of a row of controls.
fn row_label(ui: &mut Ui, text: &str) {
    ui.allocate_ui_with_layout(
        vec2(LABEL, 24.0),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.label(kit::text(
                text,
                kit::size::BODY,
                kit::REGULAR,
                theme::text_secondary(),
            ));
        },
    );
}

/// The bar's index, typed or taken from a click on the chart.
fn index_row(ui: &mut Ui, context: &mut Context<'_>, bars: usize, armed_bar: Option<usize>) {
    let ready = bars > 0;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        row_label(ui, "Index");
        let id = ui.id().with("bar-index");
        let editing = ui.memory(|memory| memory.has_focus(id));
        if !editing {
            // Not being typed in, the field shows the bar the stop is on.
            context.state.bar_input = armed_bar.map_or_else(String::new, |index| index.to_string());
        }
        let hint = match bars {
            0 => "No bars yet".to_owned(),
            1 => "0".to_owned(),
            _ => format!("0 – {}", bars - 1),
        };
        let response = ui
            .add_enabled_ui(ready, |ui| {
                text_field(ui, id, &mut context.state.bar_input, &hint, 96.0, true)
            })
            .inner;
        context.state.bar_input.retain(|c| c.is_ascii_digit());
        if response.lost_focus() && !ui.input(|input| input.key_pressed(egui::Key::Escape)) {
            let text = context.state.bar_input.trim();
            if text.is_empty() {
                if armed_bar.is_some() {
                    context.actions.push(PanelAction::ClearBarBreak);
                }
            } else if let Some(index) = text.parse::<usize>().ok().filter(|i| *i < bars)
                && armed_bar != Some(index)
            {
                context
                    .actions
                    .push(PanelAction::BreakOnBar(index.to_string()));
            }
        }
        select_button(ui, context);
    });
    let typed = context.state.bar_input.trim();
    if ready
        && !typed.is_empty()
        && typed.parse::<usize>().is_ok_and(|index| index >= bars)
        && ui.memory(|memory| memory.has_focus(ui.id().with("bar-index")))
    {
        ui.horizontal(|ui| {
            ui.add_space(LABEL + 6.0);
            ui.label(kit::text(
                format!("The bars are numbered 0 to {}.", bars - 1),
                kit::size::SUBHEADLINE,
                kit::REGULAR,
                theme::syntax::string(),
            ));
        });
    }
}

/// "Select bar": wait for a click on a candle of the preview chart.
fn select_button(ui: &mut Ui, context: &mut Context<'_>) {
    let picking = context.panel.picking_bar;
    let button = kit::push_button(
        ui,
        Some(Icon::ScrollToBar),
        if picking {
            "Click a candle…"
        } else {
            "Select bar"
        },
        true,
    )
    .on_hover_text(if picking {
        "Click a candle on the preview chart, or press Esc to cancel"
    } else {
        "Click a candle on the preview chart to stop at its bar"
    });
    if picking {
        ui.painter().rect_stroke(
            button.rect,
            6.0,
            egui::Stroke::new(1.5, kit::ACCENT),
            egui::StrokeKind::Inside,
        );
    }
    if button.clicked() {
        context.actions.push(if picking {
            PanelAction::CancelPickBar
        } else {
            PanelAction::PickBar
        });
    }
}

/// The date and the time of day, each a list of the ones that have a bar.
fn time_row(ui: &mut Ui, context: &mut Context<'_>, armed_bar: Option<usize>) {
    let panel = context.panel;
    let times = &panel.bar_times;
    let days = &panel.bar_days;
    let day = armed_bar.and_then(|bar| {
        days.iter()
            .position(|day| (day.first..day.end).contains(&bar))
    });
    // A chart of daily bars has one a day; the time of day would only ever say 00:00.
    let intraday = days.iter().any(|day| day.end - day.first > 1);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        row_label(ui, if intraday { "Time" } else { "Date" });
        let chosen = kit::menu_picker_hinted(
            ui,
            if intraday { 118.0 } else { 150.0 },
            &panel.day_labels,
            day.unwrap_or(usize::MAX),
            !days.is_empty(),
            "Date (UTC)",
        );
        if let Some(chosen) = chosen {
            // Keep the time of day when the new date has a bar at it, else take the first.
            let wanted = armed_bar.map(|bar| times[bar].rem_euclid(86_400_000));
            let range = days[chosen].first..days[chosen].end;
            let bar = wanted
                .and_then(|wanted| {
                    range
                        .clone()
                        .find(|bar| times[*bar].rem_euclid(86_400_000) == wanted)
                })
                .unwrap_or(range.start);
            context
                .actions
                .push(PanelAction::BreakOnBar(bar_time_text(times[bar])));
        }
        if intraday {
            let range = day.map(|day| days[day].first..days[day].end);
            let labels: Vec<String> = range
                .clone()
                .into_iter()
                .flatten()
                .map(|bar| bar_time_text(times[bar])[11..].to_owned())
                .collect();
            let selected = armed_bar
                .zip(range.as_ref())
                .map_or(usize::MAX, |(bar, range)| bar - range.start);
            if let Some(chosen) =
                kit::menu_picker_hinted(ui, 86.0, &labels, selected, day.is_some(), "Time")
            {
                let range = range.expect("a time is chosen from a day's list");
                context.actions.push(PanelAction::BreakOnBar(bar_time_text(
                    times[range.start + chosen],
                )));
            }
        }
    });
}

/// What is armed, in words, and a way to take it back.
fn status(ui: &mut Ui, context: &mut Context<'_>, armed: &Armed) {
    let (accent, headline, detail) = match armed {
        Armed::Bar(index) => (
            true,
            format!("Stops at bar {index}"),
            format!("{} UTC", bar_time_text(context.panel.bar_times[*index])),
        ),
        Armed::Missing(reason) => (false, reason.clone(), String::new()),
    };
    let tint = if accent {
        kit::ACCENT
    } else {
        theme::syntax::string()
    };
    egui::Frame::new()
        .fill(theme::with_alpha(tint, 0.12))
        .corner_radius(6.0)
        .inner_margin(egui::Margin::symmetric(8, 4))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.horizontal(|ui| {
                let (slot, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
                icons::paint(
                    ui.painter(),
                    Rect::from_center_size(slot.center(), Vec2::splat(12.0)),
                    if accent {
                        Icon::ScrollToBar
                    } else {
                        Icon::XmarkCircleFill
                    },
                    tint,
                );
                ui.label(kit::text(
                    headline,
                    kit::size::BODY,
                    kit::SEMIBOLD,
                    theme::text_primary(),
                ));
                if !detail.is_empty() {
                    ui.label(kit::text(
                        detail,
                        kit::size::SUBHEADLINE,
                        kit::REGULAR,
                        theme::text_secondary(),
                    ));
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if super::icon_button(ui, Icon::Xmark, true, "Don't stop at this bar") {
                        context.actions.push(PanelAction::ClearBarBreak);
                    }
                });
            });
        });
}

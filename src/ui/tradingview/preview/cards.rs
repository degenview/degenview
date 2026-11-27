//! A card for each thing to be added: a layout as the grid it will open as, a watchlist, a script,
//! each with its notes.

use super::window;
use crate::i18n::tr;
use crate::model::tradingview::plan::{LeftOut, PlannedLayout, PlannedScript, PlannedWatchlist};
use crate::model::{InstrumentId, Timeframe};
use crate::ui::icons::{self, Icon};
use crate::ui::kit::{self, ticker_icon};
use crate::ui::theme;
use crate::ui::tradingview::{Action, pictures, words};
use eframe::egui::{self, Align, Color32, Id, Layout, Sense, Stroke, Ui, Vec2, vec2};
use std::collections::HashSet;

const PICTURE: Vec2 = Vec2::new(68.0, 44.0);

fn card(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    egui::Frame::new()
        .fill(theme::quaternary(0.4))
        .stroke(Stroke::new(1.0, theme::separator().gamma_multiply(0.6)))
        .corner_radius(12.0)
        .inner_margin(egui::Margin::same(14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            add(ui);
        });
}

/// The item's name, and what it holds at the trailing edge.
fn title_row(ui: &mut Ui, name: &str, facts: &str, dimmed: bool) {
    let color = if dimmed {
        theme::text_secondary()
    } else {
        theme::text_primary()
    };
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(kit::text(
                facts,
                kit::size::CAPTION + 0.5,
                kit::REGULAR,
                theme::text_secondary(),
            ));
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.add(
                    egui::Label::new(kit::text(name, kit::size::HEADLINE, kit::SEMIBOLD, color))
                        .truncate(),
                );
            });
        });
    });
}

pub(super) fn layout(ui: &mut Ui, layout: &PlannedLayout, actions: &mut Vec<Action>) {
    card(ui, |ui| {
        let imported = !layout.charts.is_empty();
        let mut facts = vec![words::charts(layout.charts.len())];
        if imported {
            facts.push(words::drawings(layout.drawings));
            facts.push(layout.timeframe.label());
        }
        title_row(ui, &layout.name, &facts.join(" \u{00B7} "), !imported);
        if imported {
            ui.add_space(12.0);
            window::show(ui, layout);
        }
        if layout.timeframes.len() > 1 {
            ui.add_space(12.0);
            timeframe_choice(ui, layout, actions);
        }
        notes(ui, &layout.left_out);
    });
}

/// TradingView gives each chart its own interval; a DegenView layout has one.
fn timeframe_choice(ui: &mut Ui, layout: &PlannedLayout, actions: &mut Vec<Action>) {
    ui.add(
        egui::Label::new(kit::text(
            tr!("Its charts had different intervals. A DegenView layout has one:"),
            kit::size::CAPTION + 0.5,
            kit::REGULAR,
            theme::text_secondary(),
        ))
        .wrap(),
    );
    ui.add_space(6.0);
    let labels: Vec<String> = layout.timeframes.iter().map(|t| t.label()).collect();
    let options: Vec<(Timeframe, &str)> = layout
        .timeframes
        .iter()
        .copied()
        .zip(labels.iter().map(String::as_str))
        .collect();
    let mut chosen = layout.timeframe;
    kit::segmented(
        ui,
        Id::new(("tradingview-timeframe", &layout.source_id)),
        &options,
        &mut chosen,
    );
    if chosen != layout.timeframe {
        actions.push(Action::SetTimeframe(layout.source_id.clone(), chosen));
    }
}

/// What is not brought over, then what comes over changed, set apart from the item itself.
fn notes(ui: &mut Ui, entries: &[LeftOut]) {
    if entries.is_empty() {
        return;
    }
    ui.add_space(12.0);
    egui::Frame::new()
        .fill(theme::chrome().gamma_multiply(0.6))
        .corner_radius(8.0)
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 4.0;
            let (changed, dropped): (Vec<_>, Vec<_>) =
                entries.iter().partition(|entry| entry.is_change());
            for entry in dropped.into_iter().chain(changed) {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let (icon, tint) = if entry.is_change() {
                        (Icon::InfoCircle, theme::text_secondary())
                    } else {
                        (Icon::TriangleFill, theme::REPLAY)
                    };
                    let (mark, _) = ui.allocate_exact_size(vec2(12.0, 14.0), Sense::hover());
                    icons::paint(
                        ui.painter(),
                        egui::Rect::from_center_size(mark.center(), Vec2::splat(11.0)),
                        icon,
                        tint,
                    );
                    ui.add(
                        egui::Label::new(kit::text(
                            words::left_out(entry),
                            kit::size::CAPTION + 0.5,
                            kit::REGULAR,
                            theme::text_secondary(),
                        ))
                        .wrap(),
                    );
                });
            }
        });
}

/// A picture beside a name over a line of detail.
fn pictured(
    ui: &mut Ui,
    name: &str,
    dimmed: bool,
    picture: impl FnOnce(&mut Ui, egui::Rect),
    detail: &str,
    aside: &str,
) {
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), PICTURE.y), Sense::hover());
    let frame = egui::Rect::from_min_size(row.min, PICTURE);
    let mut art = ui.new_child(egui::UiBuilder::new().max_rect(frame));
    picture(&mut art, frame);
    let left = frame.right() + 14.0;
    let name = kit::galley(
        ui,
        name,
        kit::size::HEADLINE,
        kit::SEMIBOLD,
        if dimmed {
            theme::text_secondary()
        } else {
            theme::text_primary()
        },
        row.right() - left,
    );
    ui.painter().galley(
        egui::pos2(left, row.center().y - 1.0 - name.size().y),
        name,
        Color32::PLACEHOLDER,
    );
    let detail = kit::galley(
        ui,
        detail,
        kit::size::CAPTION + 0.5,
        kit::REGULAR,
        theme::text_secondary(),
        row.right() - left,
    );
    let end = left + detail.size().x;
    let below = row.center().y + 4.0;
    ui.painter()
        .galley(egui::pos2(left, below), detail, Color32::PLACEHOLDER);
    if !aside.is_empty() {
        let aside = kit::galley(
            ui,
            aside,
            kit::size::CAPTION + 0.5,
            kit::REGULAR,
            theme::text_tertiary(),
            (row.right() - end - 12.0).max(24.0),
        );
        ui.painter()
            .galley(egui::pos2(end + 12.0, below), aside, Color32::PLACEHOLDER);
    }
}

pub(super) fn watchlist(ui: &mut Ui, planned: &PlannedWatchlist) {
    let instruments: Vec<&InstrumentId> = planned
        .list
        .instruments()
        .map(|entry| &entry.instrument)
        .collect();
    let mut seen = HashSet::new();
    let marks: Vec<&InstrumentId> = instruments
        .iter()
        .copied()
        .filter(|id| seen.insert(id.source.base_asset(&id.symbol).to_string()))
        .collect();
    let names: Vec<&str> = instruments
        .iter()
        .take(8)
        .map(|id| id.symbol.as_str())
        .collect();
    card(ui, |ui| {
        pictured(
            ui,
            &planned.list.name,
            planned.markets == 0,
            |ui, rect| {
                pictures::logos(ui, rect, marks.len(), |ui, at, index| {
                    ticker_icon(ui, at, &marks[index].symbol, marks[index].source);
                });
            },
            &words::markets(planned.markets),
            &names.join(", "),
        );
        notes(ui, &planned.left_out);
    });
}

pub(super) fn script(ui: &mut Ui, planned: &PlannedScript) {
    card(ui, |ui| {
        pictured(
            ui,
            &planned.name,
            false,
            |ui, rect| pictures::source(ui, rect, &planned.source),
            &words::lines(planned.source.lines().count()),
            "",
        );
    });
}

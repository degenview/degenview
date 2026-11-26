//! The second step: what will be added, item by item, with what is left out of each.

use super::{Action, Content, scrolling, words};
use crate::i18n::tr;
use crate::model::tradingview::plan::{LeftOut, Plan, PlannedLayout, PlannedWatchlist};
use crate::model::{PersistedChart, Timeframe};
use crate::ui::icons::{self, Icon};
use crate::ui::kit::sheet_parts::{card_background, status_dot};
use crate::ui::kit::{self, sheet, source_logo};
use crate::ui::theme;
use eframe::egui::{self, Color32, Id, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};

const TOTALS_HEIGHT: f32 = 68.0;
const CHIP_HEIGHT: f32 = 26.0;

pub(super) fn show(
    ui: &mut Ui,
    height: f32,
    content: &Content<'_>,
    plan: &Plan,
    actions: &mut Vec<Action>,
) {
    scrolling(ui, height, ("tradingview-preview", content.opening), |ui| {
        totals(ui, plan);
        if !plan.layouts.is_empty() {
            heading(ui, tr!("Layouts"));
            for layout in &plan.layouts {
                layout_card(ui, layout, actions);
                ui.add_space(8.0);
            }
        }
        if !plan.watchlists.is_empty() {
            heading(ui, tr!("Watchlists"));
            for list in &plan.watchlists {
                watchlist_card(ui, list);
                ui.add_space(8.0);
            }
        }
        if !plan.scripts.is_empty() {
            heading(ui, tr!("Pine scripts"));
            card(ui, |ui| {
                for (index, script) in plan.scripts.iter().enumerate() {
                    if index > 0 {
                        ui.add_space(6.0);
                    }
                    ui.add(
                        egui::Label::new(kit::text(
                            &script.name,
                            kit::size::BODY,
                            kit::REGULAR,
                            theme::text_primary(),
                        ))
                        .truncate(),
                    );
                }
            });
        }
    });
    footer(ui, content, plan, actions);
}

/// What a plan adds, as `(layouts, charts, drawings, watchlists, scripts)`.
pub(super) fn counts(plan: &Plan) -> [usize; 5] {
    let layouts: Vec<&PlannedLayout> = plan
        .layouts
        .iter()
        .filter(|layout| !layout.charts.is_empty())
        .collect();
    [
        layouts.len(),
        layouts.iter().map(|layout| layout.charts.len()).sum(),
        plan.drawing_count(),
        plan.watchlists
            .iter()
            .filter(|list| list.markets > 0)
            .count(),
        plan.scripts.len(),
    ]
}

/// The big figures.
fn totals(ui: &mut Ui, plan: &Plan) {
    let (rect, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), TOTALS_HEIGHT), Sense::hover());
    card_background(ui, rect, None);
    let labels = [
        tr!("Layouts"),
        tr!("Charts"),
        tr!("Drawings"),
        tr!("Watchlists"),
        tr!("Scripts"),
    ];
    let width = rect.width() / labels.len() as f32;
    for (index, (count, label)) in counts(plan).into_iter().zip(labels).enumerate() {
        let centre = rect.left() + width * (index as f32 + 0.5);
        let color = if count > 0 {
            theme::text_primary()
        } else {
            theme::text_tertiary()
        };
        let figure = kit::galley(
            ui,
            &count.to_string(),
            22.0,
            kit::SEMIBOLD,
            color,
            width - 8.0,
        );
        let caption = kit::galley(
            ui,
            label,
            kit::size::CAPTION + 0.5,
            kit::REGULAR,
            theme::text_secondary(),
            width - 8.0,
        );
        let top = rect.center().y - (figure.size().y + 1.0 + caption.size().y) / 2.0;
        let figure_height = figure.size().y;
        ui.painter().galley(
            pos2(centre - figure.size().x / 2.0, top),
            figure,
            Color32::PLACEHOLDER,
        );
        ui.painter().galley(
            pos2(centre - caption.size().x / 2.0, top + figure_height + 1.0),
            caption,
            Color32::PLACEHOLDER,
        );
        if index > 0 {
            ui.painter().vline(
                rect.left() + width * index as f32,
                rect.top() + 16.0..=rect.bottom() - 16.0,
                Stroke::new(1.0, theme::separator().gamma_multiply(0.7)),
            );
        }
    }
}

fn heading(ui: &mut Ui, title: &str) {
    ui.add_space(18.0);
    ui.label(kit::text(
        title,
        kit::size::HEADLINE,
        kit::SEMIBOLD,
        theme::text_primary(),
    ));
    ui.add_space(8.0);
}

fn card(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    egui::Frame::new()
        .fill(theme::quaternary(0.4))
        .stroke(Stroke::new(1.0, theme::separator().gamma_multiply(0.6)))
        .corner_radius(12.0)
        .inner_margin(egui::Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            add(ui);
        });
}

fn title_row(ui: &mut Ui, name: &str, detail: &str, dimmed: bool) {
    let color = if dimmed {
        theme::text_secondary()
    } else {
        theme::text_primary()
    };
    ui.add(
        egui::Label::new(kit::text(name, kit::size::BODY + 1.0, kit::SEMIBOLD, color)).truncate(),
    );
    ui.add_space(2.0);
    ui.add(
        egui::Label::new(kit::text(
            detail,
            kit::size::CAPTION + 0.5,
            kit::REGULAR,
            theme::text_secondary(),
        ))
        .wrap(),
    );
}

fn layout_card(ui: &mut Ui, layout: &PlannedLayout, actions: &mut Vec<Action>) {
    card(ui, |ui| {
        let imported = !layout.charts.is_empty();
        let mut detail = words::charts(layout.charts.len());
        if imported {
            detail = format!(
                "{detail} \u{00B7} {} \u{00B7} {}",
                words::drawings(layout.drawings),
                layout.timeframe.label()
            );
        }
        title_row(ui, &layout.name, &detail, !imported);
        if imported {
            ui.add_space(10.0);
            chips(ui, &layout.charts);
        }
        if layout.timeframes.len() > 1 {
            ui.add_space(10.0);
            timeframe_choice(ui, layout, actions);
        }
        left_out(ui, &layout.left_out);
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

/// The charts as chips: the exchange's mark and the market, wrapping.
fn chips(ui: &mut Ui, charts: &[PersistedChart]) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
        for chart in charts {
            let name = chart
                .title
                .clone()
                .unwrap_or_else(|| chart.source.display_symbol(&chart.symbol));
            let text = kit::galley(
                ui,
                &name,
                kit::size::CAPTION + 1.0,
                kit::MEDIUM,
                theme::text_primary(),
                220.0,
            );
            let size = vec2(text.size().x + 36.0, CHIP_HEIGHT);
            let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
            response.on_hover_text(chart.source.display_name());
            ui.painter().rect_stroke(
                rect,
                8.0,
                Stroke::new(1.0, theme::separator()),
                StrokeKind::Inside,
            );
            source_logo::paint(
                ui,
                chart.source,
                Rect::from_center_size(
                    pos2(rect.left() + 15.0, rect.center().y),
                    Vec2::splat(16.0),
                ),
            );
            ui.painter().galley(
                pos2(rect.left() + 28.0, rect.center().y - text.size().y / 2.0),
                text,
                Color32::PLACEHOLDER,
            );
        }
    });
}

/// What is not brought over, one line each.
fn left_out(ui: &mut Ui, entries: &[LeftOut]) {
    if entries.is_empty() {
        return;
    }
    ui.add_space(10.0);
    for entry in entries {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 7.0;
            icons::inline(ui, Icon::TriangleFill, 10.0, theme::REPLAY);
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
        ui.add_space(3.0);
    }
}

fn watchlist_card(ui: &mut Ui, list: &PlannedWatchlist) {
    card(ui, |ui| {
        title_row(ui, &list.list.name, &words::markets(list.markets), false);
        left_out(ui, &list.left_out);
    });
}

fn footer(ui: &mut Ui, content: &Content<'_>, plan: &Plan, actions: &mut Vec<Action>) {
    let [layouts, _, _, watchlists, scripts] = counts(plan);
    let anything = layouts + watchlists + scripts > 0;
    let searching = content.placing
        || content
            .selection
            .symbols(content.library)
            .iter()
            .any(|(symbol, _)| {
                !content.mappings.has_answer(symbol) && !content.mappings.is_decided(symbol)
            });
    sheet::footer_split(
        ui,
        |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let (color, text) = if searching {
                (theme::REPLAY, tr!("Looking up markets and prices\u{2026}"))
            } else if anything {
                (theme::UP, tr!("Ready to import"))
            } else {
                (theme::REPLAY, tr!("Nothing to import"))
            };
            status_dot(ui, color, text, kit::MEDIUM, theme::text_primary());
        },
        |ui| {
            let ready = anything && !searching;
            if kit::neutral_button(ui, tr!("Import"), true, ready, false, 110.0).clicked() {
                actions.push(Action::Import);
            }
            if kit::neutral_button(ui, tr!("Back"), false, true, false, 90.0).clicked() {
                actions.push(Action::Back);
            }
        },
    );
}

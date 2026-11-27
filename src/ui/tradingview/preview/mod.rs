//! The second step: what will be added, item by item, drawn as it will look, with what is left out
//! of each or comes over changed.

mod cards;
mod window;

use super::{Action, Content, scrolling, section_title, words};
use crate::i18n::tr;
use crate::model::tradingview::plan::{Plan, PlannedLayout};
use crate::ui::icons::{self, Icon};
use crate::ui::kit::loading::LoadingIndicator;
use crate::ui::kit::sheet_parts::card_background;
use crate::ui::kit::{self, sheet};
use crate::ui::theme;
use eframe::egui::{Color32, Rect, Sense, Stroke, Ui, Vec2, pos2, vec2};

const TOTALS_HEIGHT: f32 = 76.0;
const CARD_GAP: f32 = 10.0;

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
            heading(ui, Icon::BarChart, tr!("Layouts"));
            for layout in &plan.layouts {
                cards::layout(ui, layout, actions);
                ui.add_space(CARD_GAP);
            }
        }
        if !plan.watchlists.is_empty() {
            heading(ui, Icon::List, tr!("Watchlists"));
            for list in &plan.watchlists {
                cards::watchlist(ui, list);
                ui.add_space(CARD_GAP);
            }
        }
        if !plan.scripts.is_empty() {
            heading(ui, Icon::Code, tr!("Pine scripts"));
            for script in &plan.scripts {
                cards::script(ui, script);
                ui.add_space(CARD_GAP);
            }
        }
    });
    footer(ui, content, plan, actions);
}

fn heading(ui: &mut Ui, icon: Icon, title: &str) {
    ui.add_space(14.0);
    section_title(ui, icon, title, |_, _, _| {});
    ui.add_space(10.0);
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

/// The big figures, each under its glyph.
fn totals(ui: &mut Ui, plan: &Plan) {
    let (rect, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), TOTALS_HEIGHT), Sense::hover());
    card_background(ui, rect, None);
    let labels = [
        (Icon::BarChart, tr!("Layouts")),
        (Icon::ChartLine, tr!("Charts")),
        (Icon::LineTrend, tr!("Drawings")),
        (Icon::List, tr!("Watchlists")),
        (Icon::Code, tr!("Scripts")),
    ];
    let width = rect.width() / labels.len() as f32;
    for (index, (count, (icon, label))) in counts(plan).into_iter().zip(labels).enumerate() {
        let centre = rect.left() + width * (index as f32 + 0.5);
        let color = if count > 0 {
            theme::text_primary()
        } else {
            theme::text_tertiary()
        };
        let figure = kit::galley(
            ui,
            &count.to_string(),
            kit::size::TITLE2 + 7.0,
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
            width - 26.0,
        );
        let top = rect.center().y - (figure.size().y + 3.0 + caption.size().y) / 2.0;
        let figure_height = figure.size().y;
        ui.painter().galley(
            pos2(centre - figure.size().x / 2.0, top),
            figure,
            Color32::PLACEHOLDER,
        );
        let under = top + figure_height + 3.0;
        let left = centre - (caption.size().x + 18.0) / 2.0;
        icons::paint(
            ui.painter(),
            Rect::from_center_size(
                pos2(left + 6.0, under + caption.size().y / 2.0),
                Vec2::splat(12.0),
            ),
            icon,
            theme::text_secondary(),
        );
        ui.painter()
            .galley(pos2(left + 18.0, under), caption, Color32::PLACEHOLDER);
        if index > 0 {
            ui.painter().vline(
                rect.left() + width * index as f32,
                rect.top() + 18.0..=rect.bottom() - 18.0,
                Stroke::new(1.0, theme::separator().gamma_multiply(0.7)),
            );
        }
    }
}

/// What is added, and whether it can be, beside the two ways on.
fn footer(ui: &mut Ui, content: &Content<'_>, plan: &Plan, actions: &mut Vec<Action>) {
    let [layouts, _, _, watchlists, scripts] = counts(plan);
    let summary = words::selected(layouts, watchlists, scripts);
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
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.label(kit::text(
                    summary,
                    kit::size::BODY,
                    kit::SEMIBOLD,
                    theme::text_primary(),
                ));
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let text = if searching {
                        let (busy, _) = ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
                        LoadingIndicator::new().size(12.0).paint_at(ui, busy);
                        tr!("Looking up markets and prices\u{2026}")
                    } else {
                        let (color, text) = if anything {
                            (theme::UP, tr!("Ready to import"))
                        } else {
                            (theme::REPLAY, tr!("Nothing to import"))
                        };
                        let (dot, _) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
                        ui.painter().circle_filled(dot.center(), 3.5, color);
                        text
                    };
                    ui.label(kit::text(
                        text,
                        kit::size::CAPTION + 0.5,
                        kit::REGULAR,
                        theme::text_secondary(),
                    ));
                });
            });
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

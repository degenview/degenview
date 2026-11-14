//! The value history card and its range control.

use super::{OverviewInput, OverviewState};
use crate::model::portfolio::statistics::PortfolioHistoryRange;
use crate::ui::icons::Icon;
use crate::ui::kit::{MEDIUM, REGULAR};
use crate::ui::portfolio::blocks::{panel, text_at};
use crate::ui::portfolio::charts::{self, HistoryInput};
use crate::ui::theme;
use eframe::egui::{self, Align2, CursorIcon, Pos2, Rect, Sense, Stroke, Vec2};

pub(super) fn history_card(
    ui: &mut egui::Ui,
    state: &mut OverviewState,
    input: &OverviewInput,
    height: f32,
) {
    panel(
        ui,
        height,
        Icon::ChartUptrend,
        "Portfolio Value",
        |header, ui| {
            history_range_control(header, &mut state.range);
            let loading =
                input.is_loading_initial || (input.is_loading_history && input.history.is_empty());
            charts::show_history(
                ui,
                &HistoryInput {
                    snapshots: input.history,
                    current_value: input.total_value,
                    currency: input.formatter.currency,
                    range: state.range,
                    privacy: input.formatter.privacy,
                    is_loading: loading,
                    loading_message: if input.is_loading_initial {
                        "Loading market data\u{2026}"
                    } else {
                        "Building history\u{2026}"
                    },
                    now_ms: input.now_ms,
                    utc_offset_secs: input.utc_offset_secs,
                },
                height,
            );
        },
    );
}

fn history_range_control(ui: &mut egui::Ui, selected: &mut PortfolioHistoryRange) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(195.0, 27.0), Sense::hover());
    let painter = ui.painter().clone();
    painter.rect_filled(rect, 7.0, theme::quaternary(0.6));
    painter.rect_stroke(
        rect,
        7.0,
        Stroke::new(1.0, theme::separator().gamma_multiply(0.5)),
        egui::StrokeKind::Inside,
    );
    for (index, range) in PortfolioHistoryRange::ALL.into_iter().enumerate() {
        let segment = Rect::from_min_size(
            Pos2::new(rect.left() + 2.0 + index as f32 * 38.2, rect.top() + 2.0),
            Vec2::new(38.2, 23.0),
        );
        let response = ui
            .interact(
                segment,
                egui::Id::new(("portfolio-history-range", range.title())),
                Sense::click(),
            )
            .on_hover_cursor(CursorIcon::PointingHand);
        if response.clicked() {
            *selected = range;
        }
        let is_selected = *selected == range;
        if is_selected {
            painter.rect_filled(segment, 5.0, theme::chrome());
            painter.rect_stroke(
                segment,
                5.0,
                Stroke::new(1.0, theme::separator().gamma_multiply(0.6)),
                egui::StrokeKind::Inside,
            );
        } else if response.hovered() {
            painter.rect_filled(segment, 5.0, theme::ink(12));
        }
        text_at(
            ui,
            segment.center(),
            Align2::CENTER_CENTER,
            range.title(),
            11.5,
            if is_selected { MEDIUM } else { REGULAR },
            if is_selected {
                theme::text_primary()
            } else {
                theme::text_secondary()
            },
        );
    }
}

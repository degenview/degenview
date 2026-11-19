//! The taker-flow mode of the panel: the focused chart's asset, what its takers bought minus what
//! they sold over the last minute, five minutes and hour, a bar for each of the last half hour's
//! minutes, and the same by venue.

use super::{FlowBody, MARGIN, footnote, footnote_height, usd};
use crate::model::taker_flow::{MINUTES, Snapshot, Spans, Volumes};
use crate::ui::icons::Icon;
use crate::ui::kit::{self, MEDIUM, REGULAR, SEMIBOLD, size};
use crate::ui::theme;
use eframe::egui::{self, Align, Color32, Layout, Rect, Sense, Ui, pos2, vec2};

const BAR_HEIGHT: f32 = 4.0;
const CHART_HEIGHT: f32 = 72.0;
const ROW_HEIGHT: f32 = 28.0;
const NUMBER_WIDTH: f32 = 64.0;

pub(super) fn show(ui: &mut Ui, body: &FlowBody<'_>) {
    let area = ui.available_rect_before_wrap();
    let Some(asset) = body.asset else {
        kit::empty_state(
            ui,
            area,
            Icon::Flame,
            "No flow for this market",
            "Taker flow follows the focused chart when it shows a crypto market.",
        );
        return;
    };
    let Some(snapshot) = body.snapshot.filter(|snapshot| snapshot.has_trades) else {
        kit::empty_state(
            ui,
            area,
            Icon::Flame,
            &format!("Waiting for {asset} trades"),
            &format!(
                "Binance and Bybit spot and perpetual {asset}USDT. A market they do not list stays empty."
            ),
        );
        return;
    };
    let note = footnote_text(asset);
    let reserved = footnote_height(ui, &note);
    let height = (ui.available_height() - reserved).max(60.0);
    let (body_rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let mut body_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(body_rect)
            .layout(Layout::top_down(Align::Min)),
    );
    egui::ScrollArea::vertical()
        .id_salt("taker-flow")
        .auto_shrink([false, false])
        .min_scrolled_height(0.0)
        .show(&mut body_ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            title(ui, asset);
            totals(ui, &snapshot.all);
            kit::divider(ui);
            minutes(ui, snapshot);
            kit::divider(ui);
            venues(ui, snapshot);
        });
    footnote(ui, &note);
}

fn footnote_text(asset: &str) -> String {
    format!(
        "{asset}USDT, spot and perpetual, on Binance and Bybit. Net flow is what takers bought minus what they sold: it says who was aggressive, not who was right."
    )
}

/// Net flow reads green when buyers were the aggressors, red when sellers were. Colour is data.
fn net_color(net: f64) -> Color32 {
    if net > 0.0 {
        theme::UP
    } else if net < 0.0 {
        theme::DOWN
    } else {
        theme::text_secondary()
    }
}

/// `+$1.2M` or `-$340K`.
fn signed_usd(value: f64) -> String {
    format!("{}{}", if value < 0.0 { "-" } else { "+" }, usd(value))
}

fn title(ui: &mut Ui, asset: &str) {
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        ui.add_space(MARGIN);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(kit::text(
                asset,
                size::HEADLINE,
                SEMIBOLD,
                theme::text_primary(),
            ));
            ui.label(kit::text(
                "Net taker flow, all venues",
                size::CAPTION,
                REGULAR,
                theme::text_secondary(),
            ));
        });
    });
    ui.add_space(12.0);
}

/// The last minute, five minutes and hour: the net, and how it splits between buying and selling.
fn totals(ui: &mut Ui, all: &Spans) {
    let periods = [
        ("Last minute", all.minute),
        ("5 minutes", all.five_minutes),
        ("Hour", all.hour),
    ];
    let gap = 12.0;
    let column = (ui.available_width() - 2.0 * MARGIN - 2.0 * gap) / 3.0;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        ui.add_space(MARGIN);
        for (index, (name, volumes)) in periods.into_iter().enumerate() {
            if index > 0 {
                ui.add_space(gap);
            }
            ui.allocate_ui_with_layout(vec2(column, 0.0), Layout::top_down(Align::Min), |ui| {
                ui.spacing_mut().item_spacing.y = 3.0;
                ui.label(kit::text(
                    name,
                    size::CAPTION,
                    REGULAR,
                    theme::text_secondary(),
                ));
                ui.label(kit::text(
                    signed_usd(volumes.net()),
                    size::HEADLINE,
                    SEMIBOLD,
                    net_color(volumes.net()),
                ));
                split_bar(ui, volumes, column);
            });
        }
    });
    ui.add_space(12.0);
}

/// Buying (green) and selling (red) as shares of what was traded.
fn split_bar(ui: &mut Ui, volumes: Volumes, width: f32) {
    let (rect, response) = ui.allocate_exact_size(vec2(width, BAR_HEIGHT), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, BAR_HEIGHT / 2.0, theme::quaternary(0.5));
    if volumes.total() > 0.0 {
        let share = (volumes.buy / volumes.total()) as f32;
        let split = rect.left() + rect.width() * share;
        let round = (BAR_HEIGHT / 2.0) as u8;
        painter.rect_filled(
            Rect::from_min_max(rect.min, pos2(split, rect.max.y)),
            egui::CornerRadius {
                nw: round,
                sw: round,
                ..Default::default()
            },
            theme::UP,
        );
        painter.rect_filled(
            Rect::from_min_max(pos2(split, rect.min.y), rect.max),
            egui::CornerRadius {
                ne: round,
                se: round,
                ..Default::default()
            },
            theme::DOWN,
        );
    }
    response.on_hover_text(format!(
        "Takers bought {} and sold {}",
        usd(volumes.buy),
        usd(volumes.sell)
    ));
}

/// A bar for each of the last half hour's minutes, the newest at the right: up when takers bought
/// more than they sold, down when they sold more.
fn minutes(ui: &mut Ui, snapshot: &Snapshot) {
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        ui.add_space(MARGIN);
        ui.label(kit::text(
            "Net flow a minute, last 30 minutes",
            size::CAPTION,
            MEDIUM,
            theme::text_secondary(),
        ));
    });
    ui.add_space(8.0);
    let width = ui.available_width() - 2.0 * MARGIN;
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), CHART_HEIGHT), Sense::hover());
    let area = Rect::from_min_max(
        pos2(rect.left() + MARGIN, rect.top()),
        pos2(rect.left() + MARGIN + width, rect.bottom()),
    );
    let painter = ui.painter();
    let middle = area.center().y;
    painter.hline(
        area.x_range(),
        middle,
        egui::Stroke::new(1.0, theme::separator()),
    );
    let largest = snapshot
        .minutes
        .iter()
        .map(|minute| minute.net().abs())
        .fold(1.0, f64::max);
    let slot = area.width() / MINUTES as f32;
    let reach = area.height() / 2.0 - 2.0;
    for (index, minute) in snapshot.minutes.iter().enumerate() {
        let net = minute.net();
        if net == 0.0 {
            continue;
        }
        let length = ((net.abs() / largest) as f32 * reach).max(1.0);
        let left = area.left() + slot * index as f32 + 1.0;
        let bar = if net > 0.0 {
            Rect::from_min_max(pos2(left, middle - length), pos2(left + slot - 2.0, middle))
        } else {
            Rect::from_min_max(pos2(left, middle), pos2(left + slot - 2.0, middle + length))
        };
        painter.rect_filled(bar, 1.0, net_color(net));
    }
    if let Some(pointer) = response.hover_pos().filter(|at| area.contains(*at)) {
        let index = (((pointer.x - area.left()) / slot) as usize).min(MINUTES - 1);
        let minute = snapshot.minutes[index];
        let ago = MINUTES - 1 - index;
        let when = if ago == 0 {
            "Last minute".to_owned()
        } else {
            format!("{ago} to {} minutes ago", ago + 1)
        };
        response.on_hover_text(format!(
            "{when}: net {} (bought {}, sold {})",
            signed_usd(minute.net()),
            usd(minute.buy),
            usd(minute.sell)
        ));
    }
    ui.add_space(12.0);
}

/// Each venue's net over the last minute, five minutes and hour.
fn venues(ui: &mut Ui, snapshot: &Snapshot) {
    ui.add_space(4.0);
    let (header, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
    let painter = ui.painter();
    let columns = [("1m", 2), ("5m", 1), ("1h", 0)];
    painter.text(
        pos2(header.left() + MARGIN, header.center().y),
        egui::Align2::LEFT_CENTER,
        "Venue",
        egui::FontId::proportional(size::CAPTION),
        theme::text_tertiary(),
    );
    for (title, from_right) in columns {
        painter.text(
            pos2(
                header.right() - MARGIN - NUMBER_WIDTH * from_right as f32,
                header.center().y,
            ),
            egui::Align2::RIGHT_CENTER,
            title,
            egui::FontId::proportional(size::CAPTION),
            theme::text_tertiary(),
        );
    }
    for (index, (venue, spans)) in snapshot.venues.iter().enumerate() {
        let (rect, response) =
            ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::hover());
        if response.hovered() {
            ui.painter().rect_filled(rect, 0.0, kit::primary(0.06));
        } else if index % 2 == 1 {
            ui.painter().rect_filled(rect, 0.0, theme::quaternary(0.28));
        }
        let label = kit::galley(
            ui,
            &venue.label(),
            12.0,
            MEDIUM,
            theme::text_primary(),
            (rect.width() - 2.0 * MARGIN - 3.0 * NUMBER_WIDTH).max(20.0),
        );
        let painter = ui.painter();
        painter.galley(
            pos2(rect.left() + MARGIN, rect.center().y - label.size().y / 2.0),
            label,
            Color32::PLACEHOLDER,
        );
        for (volumes, from_right) in [(spans.minute, 2), (spans.five_minutes, 1), (spans.hour, 0)] {
            let net = volumes.net();
            let text = if volumes.total() > 0.0 {
                signed_usd(net)
            } else {
                "\u{2014}".to_owned()
            };
            let galley = kit::galley(ui, &text, 12.0, REGULAR, net_color(net), NUMBER_WIDTH);
            let origin = pos2(
                rect.right() - MARGIN - NUMBER_WIDTH * from_right as f32 - galley.size().x,
                rect.center().y - galley.size().y / 2.0,
            );
            ui.painter().galley(origin, galley, Color32::PLACEHOLDER);
        }
        response.on_hover_text(format!(
            "{}: in the last hour takers bought {} and sold {}",
            venue.label(),
            usd(spans.hour.buy),
            usd(spans.hour.sell)
        ));
    }
    ui.add_space(8.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn net_flow_reads_by_its_sign() {
        assert_eq!(net_color(5.0), theme::UP);
        assert_eq!(net_color(-5.0), theme::DOWN);
        assert_eq!(net_color(0.0), theme::text_secondary());
        assert_eq!(signed_usd(1_234_567.0), "+$1.23M");
        assert_eq!(signed_usd(-340_000.0), "-$340K");
        assert_eq!(signed_usd(0.0), "+$0");
    }
}

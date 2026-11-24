//! The taker-flow mode of the panel: the focused chart's asset, what its takers bought minus what
//! they sold over the last minute, five minutes and hour, a bar for each of the last half hour's
//! minutes, and the same by venue.

use super::{FlowBody, MARGIN, footnote, footnote_height, usd};
use crate::i18n::{tr, tr_c};
use crate::model::taker_flow::{
    HOUR_MS, MINUTE_MS, MINUTES, Observed, Snapshot, Spans, VENUES, Volumes, full,
};
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
            tr!("No flow for this market"),
            tr!("Taker flow follows the focused chart when it shows a crypto market."),
        );
        return;
    };
    let Some(snapshot) = body.snapshot.filter(|snapshot| snapshot.has_trades) else {
        kit::empty_state(
            ui,
            area,
            Icon::Flame,
            &tr!("Waiting for {asset} trades", asset = asset),
            &tr!(
                "Binance and Bybit spot and perpetual {asset}USDT. A market they do not list stays empty.",
                asset = asset
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
    tr!(
        "{asset}USDT, spot and perpetual, on Binance and Bybit. Net flow is what takers bought minus what they sold: it says who was aggressive, not who was right. Figures marked partial, muted figures and faded bars cover only the time the sockets were connected: a trade from before is not in them.",
        asset = asset
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

/// `40 s` or `3 min 5 s`.
fn duration(ms: i64) -> String {
    let seconds = ms.max(0) / 1_000;
    match (seconds / 60, seconds % 60) {
        (0, seconds) => tr!("{seconds} s", seconds = seconds),
        (minutes, 0) => tr!("{minutes} min", minutes = minutes),
        (minutes, seconds) => tr!(
            "{minutes} min {seconds} s",
            minutes = minutes,
            seconds = seconds
        ),
    }
}

/// The spans the totals add up: what each one covers and how a tooltip says so.
#[derive(Clone, Copy)]
enum Span {
    Minute,
    FiveMinutes,
    Hour,
}

impl Span {
    const fn ms(self) -> i64 {
        match self {
            Self::Minute => MINUTE_MS,
            Self::FiveMinutes => 5 * MINUTE_MS,
            Self::Hour => HOUR_MS,
        }
    }

    /// What a tooltip says of the span when none of it was observed.
    fn nothing_observed(self) -> String {
        match self {
            Self::Minute => {
                tr!("Nothing of the last minute was observed: the sockets were not connected.")
                    .to_owned()
            }
            Self::FiveMinutes => {
                tr!("Nothing of the last 5 minutes was observed: the sockets were not connected.")
                    .to_owned()
            }
            Self::Hour => {
                tr!("Nothing of the last hour was observed: the sockets were not connected.")
                    .to_owned()
            }
        }
    }

    /// What a tooltip says of the span when only `observed` (a duration) of it was.
    fn part_observed(self, observed: String) -> String {
        match self {
            Self::Minute => tr!(
                "Only {observed} of the last minute was observed: the sockets were not connected for the rest.",
                observed = observed
            ),
            Self::FiveMinutes => tr!(
                "Only {observed} of the last 5 minutes was observed: the sockets were not connected for the rest.",
                observed = observed
            ),
            Self::Hour => tr!(
                "Only {observed} of the last hour was observed: the sockets were not connected for the rest.",
                observed = observed
            ),
        }
    }
}

/// What a span's tooltip adds when only part of it was observed.
fn observed_note(observed: i64, span: Span) -> Option<String> {
    (!full(observed, span.ms())).then(|| {
        if observed <= 0 {
            span.nothing_observed()
        } else {
            span.part_observed(duration(observed))
        }
    })
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
                tr!("Net taker flow, all venues"),
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
        (
            tr!("Last minute"),
            all.minute,
            all.observed.minute,
            Span::Minute,
        ),
        (
            tr!("5 minutes"),
            all.five_minutes,
            all.observed.five_minutes,
            Span::FiveMinutes,
        ),
        (tr!("Hour"), all.hour, all.observed.hour, Span::Hour),
    ];
    let gap = 12.0;
    let column = (ui.available_width() - 2.0 * MARGIN - 2.0 * gap) / 3.0;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        ui.add_space(MARGIN);
        for (index, (name, volumes, observed, span)) in periods.into_iter().enumerate() {
            if index > 0 {
                ui.add_space(gap);
            }
            ui.allocate_ui_with_layout(vec2(column, 0.0), Layout::top_down(Align::Min), |ui| {
                ui.spacing_mut().item_spacing.y = 3.0;
                let note = observed_note(observed, span);
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
                split_bar(ui, volumes, column, note.as_deref());
                if note.is_some() {
                    ui.label(kit::text(
                        tr!("partial"),
                        size::CAPTION,
                        REGULAR,
                        theme::text_tertiary(),
                    ));
                }
            });
        }
    });
    ui.add_space(12.0);
}

/// Buying (green) and selling (red) as shares of what was traded.
fn split_bar(ui: &mut Ui, volumes: Volumes, width: f32, note: Option<&str>) {
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
    let mut text = tr!(
        "Takers bought {bought} and sold {sold}",
        bought = usd(volumes.buy),
        sold = usd(volumes.sell)
    );
    if let Some(note) = note {
        text = format!("{text}\n{note}");
    }
    response.on_hover_text(text);
}

/// A bar for each of the last half hour's minutes, the newest at the right: up when takers bought
/// more than they sold, down when they sold more.
fn minutes(ui: &mut Ui, snapshot: &Snapshot) {
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        ui.add_space(MARGIN);
        ui.label(kit::text(
            tr!("Net flow a minute, last 30 minutes"),
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
        let observed = snapshot.minutes_observed[index];
        let left = area.left() + slot * index as f32 + 1.0;
        let column = Rect::from_min_max(
            pos2(left, area.top()),
            pos2(left + slot - 2.0, area.bottom()),
        );
        if observed <= 0 {
            // Nothing was watched: not a quiet minute.
            painter.rect_filled(column, 1.0, theme::quaternary(0.3));
            continue;
        }
        let net = minute.net();
        if net == 0.0 {
            continue;
        }
        let length = ((net.abs() / largest) as f32 * reach).max(1.0);
        let bar = if net > 0.0 {
            Rect::from_min_max(pos2(left, middle - length), pos2(left + slot - 2.0, middle))
        } else {
            Rect::from_min_max(pos2(left, middle), pos2(left + slot - 2.0, middle + length))
        };
        let color = if full(observed, VENUES * MINUTE_MS) {
            net_color(net)
        } else {
            net_color(net).gamma_multiply(0.45)
        };
        painter.rect_filled(bar, 1.0, color);
    }
    if let Some(pointer) = response.hover_pos().filter(|at| area.contains(*at)) {
        let index = (((pointer.x - area.left()) / slot) as usize).min(MINUTES - 1);
        let minute = snapshot.minutes[index];
        let observed = snapshot.minutes_observed[index];
        let ago = MINUTES - 1 - index;
        let text = minute_tooltip(ago, minute, observed);
        response.on_hover_text(text);
    }
    ui.add_space(12.0);
}

/// What a bar of the half hour says: which minute it is (`ago` minutes back), the net, and how much
/// of it the sockets saw. The minute is in each message, not put in from another, so a language
/// may write the sentence its own way.
fn minute_tooltip(ago: usize, minute: Volumes, observed: i64) -> String {
    let next = ago + 1;
    if observed <= 0 {
        return if ago == 0 {
            tr!("Last minute: not observed, the sockets were not connected").to_owned()
        } else {
            tr!(
                "{ago} to {next} minutes ago: not observed, the sockets were not connected",
                ago = ago,
                next = next
            )
        };
    }
    let (net, bought, sold) = (signed_usd(minute.net()), usd(minute.buy), usd(minute.sell));
    let mut text = if ago == 0 {
        tr!(
            "Last minute: net {net} (bought {bought}, sold {sold})",
            net = net,
            bought = bought,
            sold = sold
        )
    } else {
        tr!(
            "{ago} to {next} minutes ago: net {net} (bought {bought}, sold {sold})",
            ago = ago,
            next = next,
            net = net,
            bought = bought,
            sold = sold
        )
    };
    if !full(observed, VENUES * MINUTE_MS) {
        text = format!(
            "{text}\n{}",
            tr!("Some venues were not connected for all of it.")
        );
    }
    text
}

/// What a venue's row says about each span it did not observe whole, the shortest first.
fn venue_notes(observed: Observed) -> [(i64, Span); 3] {
    [
        (observed.minute, Span::Minute),
        (observed.five_minutes, Span::FiveMinutes),
        (observed.hour, Span::Hour),
    ]
}

/// Each venue's net over the last minute, five minutes and hour.
fn venues(ui: &mut Ui, snapshot: &Snapshot) {
    ui.add_space(4.0);
    let (header, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
    let painter = ui.painter();
    let columns = [
        (tr_c!("net over the last minute, column head", "1m"), 2),
        (tr_c!("net over the last 5 minutes, column head", "5m"), 1),
        (tr_c!("net over the last hour, column head", "1h"), 0),
    ];
    painter.text(
        pos2(header.left() + MARGIN, header.center().y),
        egui::Align2::LEFT_CENTER,
        tr!("Venue"),
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
        let cells = [
            (spans.minute, spans.observed.minute_full(), 2),
            (spans.five_minutes, spans.observed.five_minutes_full(), 1),
            (spans.hour, spans.observed.hour_full(), 0),
        ];
        for (volumes, whole, from_right) in cells {
            let net = volumes.net();
            let text = if volumes.total() > 0.0 {
                signed_usd(net)
            } else {
                "\u{2014}".to_owned()
            };
            // A figure that misses part of its span is muted: it is not the market's number.
            let color = if whole {
                net_color(net)
            } else {
                theme::text_tertiary()
            };
            let galley = kit::galley(ui, &text, 12.0, REGULAR, color, NUMBER_WIDTH);
            let origin = pos2(
                rect.right() - MARGIN - NUMBER_WIDTH * from_right as f32 - galley.size().x,
                rect.center().y - galley.size().y / 2.0,
            );
            ui.painter().galley(origin, galley, Color32::PLACEHOLDER);
        }
        let mut text = tr!(
            "{venue}: in the last hour takers bought {bought} and sold {sold}",
            venue = venue.label(),
            bought = usd(spans.hour.buy),
            sold = usd(spans.hour.sell)
        );
        for (observed, span) in venue_notes(spans.observed) {
            if let Some(note) = observed_note(observed, span) {
                text = format!("{text}\n{note}");
            }
        }
        response.on_hover_text(text);
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

    #[test]
    fn a_span_is_noted_only_when_part_of_it_was_not_observed() {
        assert_eq!(observed_note(MINUTE_MS, Span::Minute), None);
        assert_eq!(
            observed_note(MINUTE_MS - 1_500, Span::Minute),
            None,
            "a moment short is whole"
        );
        let partial = observed_note(40_000, Span::FiveMinutes).unwrap();
        assert!(
            partial.contains("Only 40 s of the last 5 minutes"),
            "{partial}"
        );
        let none = observed_note(0, Span::Hour).unwrap();
        assert!(none.contains("Nothing of the last hour"), "{none}");
    }

    #[test]
    fn a_duration_reads_in_minutes_and_seconds() {
        assert_eq!(duration(0), "0 s");
        assert_eq!(duration(59_999), "59 s");
        assert_eq!(duration(60_000), "1 min");
        assert_eq!(duration(185_000), "3 min 5 s");
        assert_eq!(duration(-5), "0 s");
    }
}

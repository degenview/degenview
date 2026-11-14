//! Table view: the bars a chart draws as rows, newest first, with the values of the indicators
//! and scripts it shows.
//!
//! A docked panel under the charts: a toolbar (what is shown, the order, close), a sticky header
//! and the rows. The columns share the panel's width and scroll sideways together with the header
//! when there are more than fit. Only the rows on screen are laid out, so a long history costs no
//! more than a short one; the numbers are written by `model::chart::table`, the way the chart
//! writes them.

use crate::format::group_thousands;
use crate::model::PriceFormat;
use crate::model::chart::table::{Column as Field, Order, TableFeed, ValueKind};
use crate::ui::icons::{self, Icon};
use crate::ui::{kit, theme};
use eframe::egui::{self, Align, Align2, Color32, Layout, Rect, Sense, Ui, pos2, vec2};

const TOOLBAR_HEIGHT: f32 = 44.0;
const HEADER_HEIGHT: f32 = 32.0;
const ROW_HEIGHT: f32 = 30.0;
/// Space between a cell's edge and its text, and at the table's own edges.
const PAD: f32 = 16.0;
const TIME_WIDTH: f32 = 188.0;
const NUMBER_WIDTH: f32 = 112.0;
const EXTRA_WIDTH: f32 = 128.0;
const CELL_SIZE: f32 = 12.5;

/// What the panel keeps between frames.
#[derive(Default)]
pub struct TableViewState {
    pub order: Order,
}

/// What the panel asks for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TableAction {
    Close,
}

// MARK: Layout

/// The width of each column: its natural width, and when the columns do not fill `available`
/// the rest shared by every column but the time, so the numbers sit under their headers across
/// the whole panel instead of huddling on the left. When they do not fit, the table scrolls.
fn column_widths(
    feed: &TableFeed,
    columns: &[Field],
    available: f32,
    title_width: impl Fn(&str) -> f32,
) -> Vec<f32> {
    let mut widths: Vec<f32> = columns
        .iter()
        .map(|column| match column {
            Field::Time => TIME_WIDTH,
            Field::Extra(_) => {
                let title = title_width(&feed.title(*column)) + PAD * 2.0 + 14.0;
                title.max(EXTRA_WIDTH)
            }
            _ => NUMBER_WIDTH,
        })
        .collect();
    let total: f32 = widths.iter().sum();
    let growing = columns.len().saturating_sub(1);
    if total < available && growing > 0 {
        let share = (available - total) / growing as f32;
        for width in widths.iter_mut().skip(1) {
            *width += share;
        }
    }
    widths
}

/// A time as `(date, time of day)`, the way `format::timestamp` writes it.
fn split_time(text: &str) -> (&str, &str) {
    text.split_once(' ').unwrap_or((text, ""))
}

// MARK: Panel

pub fn show(
    ui: &mut Ui,
    state: &mut TableViewState,
    title: &str,
    feed: Option<&TableFeed>,
    price: PriceFormat,
) -> Option<TableAction> {
    let mut action = None;
    // The panel is as tall as the user made it; the content fills it rather than hugging its own
    // height, or a short table would collapse the panel.
    ui.set_min_height(ui.available_height());
    toolbar(ui, state, title, feed, &mut action);
    kit::divider(ui);
    let Some(feed) = feed.filter(|feed| !feed.is_empty()) else {
        empty(ui);
        return action;
    };
    let columns = feed.columns();
    let available = ui.available_width();
    let body_height = ui.available_height();
    let widths = column_widths(feed, &columns, available, |text| {
        kit::galley(
            ui,
            text,
            kit::size::SUBHEADLINE,
            kit::SEMIBOLD,
            Color32::WHITE,
            f32::INFINITY,
        )
        .size()
        .x
    });
    let total: f32 = widths.iter().sum::<f32>().max(available);
    egui::ScrollArea::horizontal()
        .id_salt("chart-table-x")
        .auto_shrink([false, false])
        .max_height(body_height)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            header(ui, state, feed, &columns, &widths, total);
            let rows_height = (body_height - HEADER_HEIGHT - 1.0).max(ROW_HEIGHT);
            ui.allocate_ui(vec2(total, rows_height), |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("chart-table-y")
                    .auto_shrink([false, false])
                    .max_height(rows_height)
                    .show_rows(ui, ROW_HEIGHT, feed.len(), |ui, range| {
                        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                        for row in range {
                            let Some(bar) = feed.bar_at(row, state.order) else {
                                continue;
                            };
                            paint_row(ui, feed, bar, &columns, &widths, total, price);
                        }
                    });
            });
        });
    action
}

fn toolbar(
    ui: &mut Ui,
    state: &mut TableViewState,
    title: &str,
    feed: Option<&TableFeed>,
    action: &mut Option<TableAction>,
) {
    let width = ui.available_width();
    ui.allocate_ui_with_layout(
        vec2(width, TOOLBAR_HEIGHT),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 0.0);
            ui.add_space(PAD);
            ui.label(kit::text(
                "Table view",
                kit::size::HEADLINE,
                kit::SEMIBOLD,
                theme::text_primary(),
            ));
            ui.add_space(4.0);
            chip(ui, title);
            if let Some(feed) = feed {
                chip(ui, &feed.bars_label);
                chip(
                    ui,
                    &format!("{} bars", group_thousands(&feed.len().to_string())),
                );
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(PAD - 4.0);
                let close = icons::button(
                    ui,
                    Icon::Xmark,
                    26.0,
                    theme::text_secondary(),
                    "Close table view",
                );
                if close.clicked() {
                    *action = Some(TableAction::Close);
                }
                ui.add_space(4.0);
                let (icon, label) = match state.order {
                    Order::NewestFirst => (Icon::ArrowDown, "Newest first"),
                    Order::OldestFirst => (Icon::ArrowUp, "Oldest first"),
                };
                if kit::push_button(ui, Some(icon), label, true)
                    .on_hover_text("Change the order of the rows")
                    .clicked()
                {
                    state.order = state.order.flipped();
                }
            });
        },
    );
}

/// A small read-only label: what the table is of.
fn chip(ui: &mut Ui, text: &str) {
    let galley = kit::galley(
        ui,
        text,
        kit::size::SUBHEADLINE,
        kit::REGULAR,
        theme::text_secondary(),
        f32::INFINITY,
    );
    let size = vec2(galley.size().x + 16.0, 22.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(rect, 6.0, theme::ink(14));
    ui.painter().galley(
        pos2(rect.left() + 8.0, rect.center().y - galley.size().y / 2.0),
        galley,
        theme::text_secondary(),
    );
}

fn empty(ui: &mut Ui) {
    let rect = ui.available_rect_before_wrap();
    let painter = ui.painter();
    painter.text(
        pos2(rect.center().x, rect.center().y - 8.0),
        Align2::CENTER_CENTER,
        "No bars to show yet",
        egui::FontId::proportional(kit::size::HEADLINE),
        theme::text_secondary(),
    );
    painter.text(
        pos2(rect.center().x, rect.center().y + 12.0),
        Align2::CENTER_CENTER,
        "Rows appear as soon as the chart has data.",
        egui::FontId::proportional(kit::size::SUBHEADLINE),
        theme::text_tertiary(),
    );
}

// MARK: Header and rows

fn header(
    ui: &mut Ui,
    state: &mut TableViewState,
    feed: &TableFeed,
    columns: &[Field],
    widths: &[f32],
    total: f32,
) {
    let (rect, _) = ui.allocate_exact_size(vec2(total, HEADER_HEIGHT), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, theme::ink(8));
    let mut left = rect.left();
    for (column, width) in columns.iter().zip(widths) {
        let cell = Rect::from_min_size(pos2(left, rect.top()), vec2(*width, HEADER_HEIGHT));
        left += width;
        let sortable = *column == Field::Time;
        let title = feed.title(*column);
        let response = ui.interact(
            cell,
            ui.id().with(("table-header", title.as_str())),
            if sortable {
                Sense::click()
            } else {
                Sense::hover()
            },
        );
        let hovered = sortable && response.hovered();
        if hovered {
            ui.painter().rect_filled(cell, 0.0, theme::ink(10));
        }
        let color = if hovered {
            theme::text_primary()
        } else {
            theme::text_secondary()
        };
        let galley = kit::galley(
            ui,
            &title,
            kit::size::SUBHEADLINE,
            kit::SEMIBOLD,
            color,
            (width - PAD * 2.0).max(10.0),
        );
        let text_y = cell.center().y - galley.size().y / 2.0;
        if *column == Field::Time {
            ui.painter()
                .galley(pos2(cell.left() + PAD, text_y), galley.clone(), color);
            if sortable {
                let arrow = match state.order {
                    Order::NewestFirst => Icon::ArrowDown,
                    Order::OldestFirst => Icon::ArrowUp,
                };
                icons::paint(
                    ui.painter(),
                    Rect::from_center_size(
                        pos2(cell.left() + PAD + galley.size().x + 12.0, cell.center().y),
                        vec2(10.0, 10.0),
                    ),
                    arrow,
                    color,
                );
            }
        } else {
            ui.painter().galley(
                pos2(cell.right() - PAD - galley.size().x, text_y),
                galley,
                color,
            );
        }
        if sortable {
            let tip = match state.order {
                Order::NewestFirst => "Newest first: click for oldest first",
                Order::OldestFirst => "Oldest first: click for newest first",
            };
            if crate::ui::accessibility::label(response.on_hover_text(tip), format!("Time, {tip}"))
                .clicked()
            {
                state.order = state.order.flipped();
            }
        }
    }
    let line = Rect::from_min_size(pos2(rect.left(), rect.bottom()), vec2(total, 1.0));
    ui.painter().rect_filled(line, 0.0, kit::hairline());
    ui.allocate_exact_size(vec2(0.0, 1.0), Sense::hover());
}

fn paint_row(
    ui: &mut Ui,
    feed: &TableFeed,
    bar: usize,
    columns: &[Field],
    widths: &[f32],
    total: f32,
    price: PriceFormat,
) {
    let (rect, response) = ui.allocate_exact_size(vec2(total, ROW_HEIGHT), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter();
    if response.hovered() {
        painter.rect_filled(rect, 0.0, theme::ink(10));
    }
    let line = Rect::from_min_size(pos2(rect.left(), rect.bottom() - 1.0), vec2(total, 1.0));
    painter.rect_filled(line, 0.0, theme::ink(7));

    let kline = &feed.bars[bar];
    let mut left = rect.left();
    let mut summary = Vec::new();
    for (column, width) in columns.iter().zip(widths) {
        let cell = Rect::from_min_size(pos2(left, rect.top()), vec2(*width, ROW_HEIGHT));
        left += width;
        let text = feed.cell(bar, *column, price);
        let room = (width - PAD * 2.0).max(10.0);
        match column {
            Field::Time => {
                let (date, time) = split_time(&text);
                let date_galley = kit::digits(
                    ui,
                    date,
                    CELL_SIZE,
                    kit::REGULAR,
                    theme::text_primary(),
                    room,
                );
                let y = cell.center().y - date_galley.size().y / 2.0;
                let x = cell.left() + PAD;
                let date_width = date_galley.size().x;
                ui.painter()
                    .galley(pos2(x, y), date_galley, theme::text_primary());
                if !time.is_empty() {
                    let time_galley = kit::digits(
                        ui,
                        time,
                        CELL_SIZE,
                        kit::REGULAR,
                        theme::text_tertiary(),
                        room,
                    );
                    ui.painter().galley(
                        pos2(
                            x + date_width + 8.0,
                            cell.center().y - time_galley.size().y / 2.0,
                        ),
                        time_galley,
                        theme::text_tertiary(),
                    );
                }
                summary.push(text);
            }
            _ => {
                let color = value_color(feed, kline.is_up(), *column, &text);
                let galley = kit::digits(ui, &text, CELL_SIZE, kit::REGULAR, color, room);
                ui.painter().galley(
                    pos2(
                        cell.right() - PAD - galley.size().x,
                        cell.center().y - galley.size().y / 2.0,
                    ),
                    galley,
                    color,
                );
                if matches!(column, Field::Close | Field::Volume) {
                    summary.push(format!("{} {text}", feed.title(*column)));
                }
            }
        }
    }
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Label, true, summary.join(", "))
    });
}

/// How a value is coloured: the close takes the bar's direction, a trend flip its own, a missing
/// value and the volume stay quiet, everything else is plain.
fn value_color(feed: &TableFeed, up: bool, column: Field, text: &str) -> Color32 {
    match column {
        Field::Close => {
            if up {
                theme::UP
            } else {
                theme::DOWN
            }
        }
        Field::Volume => theme::text_secondary(),
        Field::Extra(index) => {
            let flip = feed
                .extras
                .get(index)
                .is_some_and(|extra| extra.kind == ValueKind::Flip);
            match (text, flip) {
                ("—", _) => theme::text_tertiary(),
                ("Bullish", true) => theme::UP,
                ("Bearish", true) => theme::DOWN,
                _ => theme::text_primary(),
            }
        }
        _ => theme::text_primary(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Kline;
    use crate::model::chart::table::Extra;

    fn feed(extras: usize) -> TableFeed {
        TableFeed {
            bars_label: "Candles".to_owned(),
            ohlc: true,
            volume: true,
            bars: vec![Kline {
                open_time: 0,
                close_time: 1,
                open: 1.0,
                high: 2.0,
                low: 0.5,
                close: 1.5,
                volume: 10.0,
            }],
            extras: (0..extras)
                .map(|index| Extra {
                    title: format!("Plot {index}"),
                    kind: ValueKind::Price,
                    values: vec![Some(1.0)],
                })
                .collect(),
        }
    }

    fn width_of(text: &str) -> f32 {
        text.chars().count() as f32 * 7.0
    }

    #[test]
    fn columns_share_a_wide_panel_so_numbers_sit_under_their_headers() {
        let feed = feed(0);
        let columns = feed.columns();
        let widths = column_widths(&feed, &columns, 1_600.0, width_of);
        assert_eq!(widths.len(), columns.len());
        let total: f32 = widths.iter().sum();
        assert!((total - 1_600.0).abs() < 0.01, "{total}");
        // The time column keeps its width; the rest grow equally.
        assert_eq!(widths[0], TIME_WIDTH);
        assert!(widths[1..].iter().all(|w| (*w - widths[1]).abs() < 0.01));
        assert!(widths[1] > NUMBER_WIDTH);
    }

    #[test]
    fn a_narrow_panel_keeps_natural_widths_and_scrolls() {
        let feed = feed(4);
        let columns = feed.columns();
        let widths = column_widths(&feed, &columns, 400.0, width_of);
        let total: f32 = widths.iter().sum();
        assert!(
            total > 400.0,
            "the table is wider than the panel and scrolls"
        );
        assert_eq!(widths[0], TIME_WIDTH);
        assert_eq!(widths[1], NUMBER_WIDTH, "no slack to hand out");
    }

    #[test]
    fn a_long_script_title_widens_its_column() {
        let mut feed = feed(1);
        feed.extras[0].title = "A very long script name: its first plot".to_owned();
        let columns = feed.columns();
        let widths = column_widths(&feed, &columns, 100.0, width_of);
        let extra = *widths.last().unwrap();
        assert!(extra > EXTRA_WIDTH, "{extra}");
    }

    #[test]
    fn a_table_with_only_a_time_has_nothing_to_share_out() {
        let mut feed = feed(0);
        feed.ohlc = false;
        feed.volume = false;
        let widths = column_widths(&feed, &[Field::Time], 1_000.0, width_of);
        assert_eq!(widths, [TIME_WIDTH]);
        assert!(column_widths(&feed, &[], 1_000.0, width_of).is_empty());
    }

    #[test]
    fn a_time_splits_into_its_date_and_time_of_day() {
        assert_eq!(split_time("2026-10-19 00:00"), ("2026-10-19", "00:00"));
        assert_eq!(split_time("2026-10-19"), ("2026-10-19", ""));
    }

    #[test]
    fn colours_follow_the_bar_and_the_trend() {
        let mut feed = feed(1);
        assert_eq!(value_color(&feed, true, Field::Close, "1"), theme::UP);
        assert_eq!(value_color(&feed, false, Field::Close, "1"), theme::DOWN);
        assert_eq!(
            value_color(&feed, true, Field::Extra(0), "—"),
            theme::text_tertiary()
        );
        feed.extras[0].kind = ValueKind::Flip;
        assert_eq!(
            value_color(&feed, false, Field::Extra(0), "Bullish"),
            theme::UP
        );
        assert_eq!(
            value_color(&feed, true, Field::Extra(0), "Bearish"),
            theme::DOWN
        );
        assert_eq!(
            value_color(&feed, true, Field::Open, "1"),
            theme::text_primary()
        );
    }
}

//! The Journal's Daily page: one calendar day's trades and the notes written around them, the plan
//! before and the lessons after. A view; the tab owns the draft and decides when to save.

use eframe::egui::{self, Ui, vec2};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use super::header_controls;
use super::icons::Icon;
use super::kit::{self, MEDIUM, REGULAR, SEMIBOLD, size};
use super::paper_grid;
use super::theme;
use crate::model::journal::calendar::{Civil, Offset};
use crate::model::journal::daily::DailyEntry;
use crate::model::journal::format;
use crate::model::journal::model::Direction;
use crate::model::journal::reviews::parse_list;

pub struct DayTrade {
    pub key: String,
    pub symbol: String,
    pub direction: Direction,
    pub net: Decimal,
    pub close_ts_ms: i64,
}

pub struct Summary {
    pub trades: usize,
    pub wins: usize,
    pub losses: usize,
    pub net: Decimal,
    pub currency: String,
}

pub struct Draft {
    pub entry: DailyEntry,
    pub saved: DailyEntry,
    pub mood_text: String,
    pub edited_at: Option<f64>,
    pub saved_until: Option<f64>,
    pub error: Option<String>,
}

impl Draft {
    pub fn new(date: &str, existing: Option<&DailyEntry>) -> Self {
        let entry = existing.cloned().unwrap_or_else(|| DailyEntry::new(date));
        Self {
            mood_text: entry.mood.join(", "),
            saved: entry.clone(),
            entry,
            edited_at: None,
            saved_until: None,
            error: None,
        }
    }

    pub fn sync_fields(&mut self) {
        self.entry.mood = parse_list(&self.mood_text);
    }

    pub fn is_dirty(&self) -> bool {
        let mut now = self.entry.clone();
        now.tidy();
        let mut saved = self.saved.clone();
        saved.tidy();
        now.created_ms = saved.created_ms;
        now.updated_ms = saved.updated_ms;
        now != saved
    }
}

#[derive(Default)]
pub struct Output {
    /// Days to move: -1 or 1.
    pub shift: Option<i64>,
    pub today: bool,
    pub open_trade: Option<String>,
    pub changed: bool,
    pub blurred: bool,
}

fn tone(value: Decimal) -> egui::Color32 {
    match value.to_f64().unwrap_or(0.0) {
        v if v > 0.0 => theme::WIN,
        v if v < 0.0 => theme::LOSS,
        _ => theme::text_primary(),
    }
}

fn area(
    ui: &mut Ui,
    id: egui::Id,
    title: &str,
    text: &mut String,
    hint: &str,
    rows: usize,
) -> bool {
    ui.add_space(10.0);
    ui.label(kit::text(
        title,
        size::CAPTION + 1.0,
        MEDIUM,
        theme::text_secondary(),
    ));
    ui.add_space(3.0);
    let focused = ui.memory(|memory| memory.has_focus(id));
    kit::field_frame(focused)
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(text)
                    .id(id)
                    .desired_rows(rows)
                    .desired_width(f32::INFINITY)
                    .frame(egui::Frame::NONE)
                    .hint_text(egui::RichText::new(hint).color(theme::text_tertiary())),
            )
        })
        .inner
        .lost_focus()
}

pub fn show(
    ui: &mut Ui,
    date: Civil,
    summary: &Summary,
    trades: &[DayTrade],
    draft: &mut Draft,
    mood_suggestions: &[String],
    offset: Offset<'_>,
) -> Output {
    let mut output = Output::default();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.label(kit::text(
            format::date(date),
            18.0,
            SEMIBOLD,
            theme::text_primary(),
        ));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if header_controls::icon_button(ui, Icon::ChevronRight, "Next day", false).clicked() {
                output.shift = Some(1);
            }
            if header_controls::icon_button(ui, Icon::ChevronLeft, "Previous day", false).clicked()
            {
                output.shift = Some(-1);
            }
            if kit::sheet_button(ui, "Today", false, true).clicked() {
                output.today = true;
            }
        });
    });
    ui.add_space(12.0);

    kit::card_frame(false).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(kit::text(
                "The day's trades",
                size::BODY + 1.0,
                SEMIBOLD,
                theme::text_primary(),
            ));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let text = format!("{} {}", format::signed_money(summary.net), summary.currency)
                    .trim()
                    .to_owned();
                ui.label(kit::text(
                    if summary.trades == 0 {
                        String::new()
                    } else {
                        text
                    },
                    size::BODY,
                    MEDIUM,
                    tone(summary.net),
                ));
                ui.label(kit::text(
                    if summary.trades == 0 {
                        "No closed trades".to_owned()
                    } else {
                        format!(
                            "{} closed \u{00b7} {} W / {} L",
                            summary.trades, summary.wins, summary.losses
                        )
                    },
                    size::BODY,
                    REGULAR,
                    theme::text_secondary(),
                ));
            });
        });
        for trade in trades {
            ui.add_space(4.0);
            let (rect, response) =
                ui.allocate_exact_size(vec2(ui.available_width(), 30.0), egui::Sense::click());
            if response.hovered() {
                ui.painter().rect_filled(rect, 6.0, kit::primary(0.05));
            }
            let mut row = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(rect.shrink2(vec2(6.0, 0.0)))
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            row.spacing_mut().item_spacing.x = 8.0;
            paper_grid::figure(
                &mut row,
                &format::datetime(trade.close_ts_ms, offset),
                theme::text_tertiary(),
            );
            paper_grid::text_cell(&mut row, &trade.symbol, theme::text_primary());
            let (side, tint) = match trade.direction {
                Direction::Long => ("LONG", theme::WIN),
                Direction::Short => ("SHORT", theme::LOSS),
            };
            paper_grid::tag(&mut row, side, tint);
            row.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                paper_grid::figure_bold(ui, &format::signed_money(trade.net), tone(trade.net));
            });
            if response
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                output.open_trade = Some(trade.key.clone());
            }
        }
    });
    ui.add_space(12.0);

    let before = (draft.entry.clone(), draft.mood_text.clone());
    let date_key = draft.entry.date.clone();
    let mut blurred = false;
    kit::card_frame(false).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(kit::text(
            "Daily journal",
            size::BODY + 1.0,
            SEMIBOLD,
            theme::text_primary(),
        ));
        ui.label(kit::text(
            "Write as much or as little as you like. Nothing here is required.",
            size::CAPTION + 1.0,
            REGULAR,
            theme::text_tertiary(),
        ));
        let id = |salt: &str| egui::Id::new(("journal-day", &date_key, salt.to_owned()));
        blurred |= area(
            ui,
            id("outlook"),
            "Market outlook",
            &mut draft.entry.outlook,
            "What is the market doing?",
            3,
        );
        blurred |= area(
            ui,
            id("plan"),
            "Trading plan",
            &mut draft.entry.plan,
            "What will you trade, and what will you not?",
            3,
        );
        blurred |= area(
            ui,
            id("goals"),
            "Goals",
            &mut draft.entry.goals,
            "What would make today a good day?",
            2,
        );

        ui.add_space(10.0);
        ui.label(kit::text(
            "Emotional state",
            size::CAPTION + 1.0,
            MEDIUM,
            theme::text_secondary(),
        ));
        ui.add_space(3.0);
        let focused = ui.memory(|memory| memory.has_focus(id("mood")));
        let response = kit::field_frame(focused)
            .inner_margin(egui::Margin::symmetric(10, 6))
            .show(ui, |ui| {
                let width = ui.available_width();
                kit::plain_field(
                    ui,
                    id("mood"),
                    &mut draft.mood_text,
                    "Comma separated",
                    size::BODY,
                    REGULAR,
                    width,
                )
            })
            .inner;
        blurred |= response.lost_focus();
        let mut items = parse_list(&draft.mood_text);
        let mut changed = false;
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(5.0, 5.0);
            for suggestion in mood_suggestions {
                let on = items
                    .iter()
                    .any(|item| item.eq_ignore_ascii_case(suggestion));
                if kit::value_chip(ui, suggestion, on).clicked() {
                    if on {
                        items.retain(|item| !item.eq_ignore_ascii_case(suggestion));
                    } else {
                        items.push(suggestion.clone());
                    }
                    changed = true;
                }
            }
        });
        if changed {
            draft.mood_text = items.join(", ");
        }

        blurred |= area(
            ui,
            id("notes"),
            "Session notes",
            &mut draft.entry.notes,
            "What happened?",
            4,
        );
        blurred |= area(
            ui,
            id("mistakes"),
            "Mistakes",
            &mut draft.entry.mistakes,
            "What went against your plan?",
            3,
        );
        blurred |= area(
            ui,
            id("lessons"),
            "Lessons learned",
            &mut draft.entry.lessons,
            "What will you do differently?",
            3,
        );
        blurred |= area(
            ui,
            id("review"),
            "End-of-day review",
            &mut draft.entry.review,
            "How did the day go overall?",
            4,
        );

        ui.add_space(8.0);
        let now = ui.input(|input| input.time);
        if let Some(error) = &draft.error {
            ui.label(kit::text(
                error.as_str(),
                size::CAPTION + 1.0,
                REGULAR,
                theme::LOSS,
            ));
        } else if draft.edited_at.is_some() && draft.is_dirty() {
            ui.label(kit::text(
                "Saving\u{2026}",
                size::CAPTION + 1.0,
                REGULAR,
                theme::text_tertiary(),
            ));
        } else if draft.saved_until.is_some_and(|until| until > now) {
            ui.label(kit::text(
                "Saved",
                size::CAPTION + 1.0,
                REGULAR,
                theme::text_secondary(),
            ));
        }
    });
    draft.sync_fields();
    output.changed = before != (draft.entry.clone(), draft.mood_text.clone());
    output.blurred = blurred;
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::journal::calendar::utc;

    #[test]
    fn a_draft_is_clean_until_something_is_written() {
        let mut draft = Draft::new("2026-11-09", None);
        assert!(!draft.is_dirty());
        draft.entry.plan = "x".into();
        assert!(draft.is_dirty());
        draft.mood_text = "calm, Calm, tired".into();
        draft.sync_fields();
        assert_eq!(draft.entry.mood, ["calm", "tired"]);
        let mut existing = DailyEntry::new("2026-11-09");
        existing.lessons = "Wait.".into();
        let loaded = Draft::new("2026-11-09", Some(&existing));
        assert!(!loaded.is_dirty());
        assert_eq!(loaded.entry.lessons, "Wait.");
    }

    #[test]
    fn the_page_draws_for_a_day_with_and_without_trades() {
        let ctx = egui::Context::default();
        let date = Civil {
            year: 2026,
            month: 11,
            day: 9,
        };
        for count in [0usize, 3] {
            let trades: Vec<DayTrade> = (0..count)
                .map(|i| DayTrade {
                    key: format!("k{i}"),
                    symbol: "BTCUSDT".into(),
                    direction: Direction::Long,
                    net: Decimal::from(i as i64 - 1),
                    close_ts_ms: 1_000 * i as i64,
                })
                .collect();
            let summary = Summary {
                trades: count,
                wins: 1,
                losses: 1,
                net: Decimal::from(5),
                currency: "USD".into(),
            };
            let mut draft = Draft::new("2026-11-09", None);
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::pos2(0.0, 0.0),
                        vec2(1000.0, 1800.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        let result = show(
                            ui,
                            date,
                            &summary,
                            &trades,
                            &mut draft,
                            &["Calm".to_owned()],
                            &utc,
                        );
                        assert!(result.shift.is_none() && !result.changed);
                    });
                },
            );
            out.textures_delta.clear();
        }
    }
}

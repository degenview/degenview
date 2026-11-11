//! The Journal's Daily page: one calendar day's trades and the notes written around them, the plan
//! before and the lessons after. A view; the tab owns the draft and decides when to save.

use eframe::egui::{self, Ui, vec2};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use super::header_controls;
use super::icons::Icon;
use super::journal_kit::{self as jk, Card, Sign};
use super::kit::{self, MEDIUM, REGULAR, SEMIBOLD, size};
use super::paper_grid;
use super::theme;
use degenview_journal::calendar::{Civil, Offset};
use degenview_journal::daily::DailyEntry;
use degenview_journal::format;
use degenview_journal::model::Direction;
use degenview_journal::reviews::parse_list;

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
    Sign::of(value.to_f64().unwrap_or(0.0)).text()
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

    let tally = if summary.trades == 0 {
        "No closed trades".to_owned()
    } else {
        format!(
            "{} closed \u{00b7} {} W / {} L",
            summary.trades, summary.wins, summary.losses
        )
    };
    let net_text = if summary.trades == 0 {
        String::new()
    } else {
        format!("{} {}", format::signed_money(summary.net), summary.currency)
            .trim()
            .to_owned()
    };
    jk::card(
        ui,
        &Card::new("The day's trades"),
        |ui| {
            ui.label(kit::text(net_text, size::BODY, MEDIUM, tone(summary.net)));
            ui.label(kit::text(
                tally,
                size::BODY,
                REGULAR,
                theme::text_secondary(),
            ));
        },
        |ui| {
            for trade in trades {
                ui.add_space(2.0);
                let (rect, response) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 30.0), egui::Sense::click());
                jk::paint_row(ui, rect, false, response.hovered());
                let mut row = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(rect.shrink2(vec2(8.0, 0.0)))
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                );
                row.spacing_mut().item_spacing.x = 8.0;
                paper_grid::figure(
                    &mut row,
                    &format::datetime(trade.close_ts_ms, offset),
                    theme::text_tertiary(),
                );
                super::journal_charts::fixed_cell(&mut row, vec2(120.0, 30.0), |ui| {
                    paper_grid::text_cell(ui, &trade.symbol, theme::text_primary());
                });
                let (side, side_tone) = match trade.direction {
                    Direction::Long => ("LONG", kit::Tone::Good),
                    Direction::Short => ("SHORT", kit::Tone::Bad),
                };
                jk::pill(&mut row, side, side_tone);
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
        },
    );
    ui.add_space(jk::GAP);

    let before = (draft.entry.clone(), draft.mood_text.clone());
    let date_key = draft.entry.date.clone();
    let mut blurred = false;
    let journal = Card::new("Daily journal")
        .subtitle("Write as much or as little as you like. Nothing here is required.");
    jk::card(
        ui,
        &journal,
        |_| {},
        |ui| {
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
            jk::save_status(
                ui,
                draft.error.as_deref(),
                draft.edited_at.is_some() && draft.is_dirty(),
                draft.saved_until.is_some_and(|until| until > now),
            );
        },
    );
    draft.sync_fields();
    output.changed = before != (draft.entry.clone(), draft.mood_text.clone());
    output.blurred = blurred;
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use degenview_journal::calendar::utc;

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

//! The Journal's Playbooks page: every playbook compared on how its trades did and how closely
//! they followed it, and an editor for the selected one. A view; the tab owns the draft and decides
//! when to save.

use eframe::egui::{self, Ui, vec2};
use rust_decimal::prelude::ToPrimitive;

use super::icons::Icon;
use super::journal_charts as charts;
use super::journal_kit::{self as jk, Card, Sign};
use super::kit::{self, MEDIUM, REGULAR, SEMIBOLD, size};
use super::paper_grid;
use super::theme;
use degenview_journal::breakdown::{Sample, Slice};
use degenview_journal::cells::NONE;
use degenview_journal::format;
use degenview_journal::metric::Metric;
use degenview_journal::playbook::Playbook;
use degenview_journal::reviews::parse_list;

/// A playbook with how its trades did.
pub struct Row<'a> {
    pub playbook: &'a Playbook,
    pub slice: Option<&'a Slice>,
    /// Mean checklist share over its reviewed trades.
    pub adherence: Option<f64>,
}

pub struct Draft {
    pub playbook: Playbook,
    pub saved: Playbook,
    pub checklist_text: String,
    pub tags_text: String,
    pub edited_at: Option<f64>,
    pub saved_until: Option<f64>,
    pub error: Option<String>,
}

impl Draft {
    pub fn new(playbook: &Playbook) -> Self {
        Self {
            checklist_text: playbook.checklist.join("\n"),
            tags_text: playbook.tags.join(", "),
            saved: playbook.clone(),
            playbook: playbook.clone(),
            edited_at: None,
            saved_until: None,
            error: None,
        }
    }

    pub fn sync_fields(&mut self) {
        self.playbook.checklist = parse_list(&self.checklist_text);
        self.playbook.tags = parse_list(&self.tags_text);
    }

    pub fn is_dirty(&self) -> bool {
        let mut now = self.playbook.clone();
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
    pub select: Option<String>,
    pub create: bool,
    pub delete: Option<String>,
    pub changed: bool,
    pub blurred: bool,
}

fn field(ui: &mut Ui, id: egui::Id, text: &mut String, hint: &str, rows: usize) -> egui::Response {
    let focused = ui.memory(|memory| memory.has_focus(id));
    kit::field_frame(focused)
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            if rows <= 1 {
                let width = ui.available_width();
                kit::plain_field(ui, id, text, hint, size::BODY, REGULAR, width)
            } else {
                ui.add(
                    egui::TextEdit::multiline(text)
                        .id(id)
                        .desired_rows(rows)
                        .desired_width(f32::INFINITY)
                        .frame(egui::Frame::NONE)
                        .hint_text(egui::RichText::new(hint).color(theme::text_tertiary())),
                )
            }
        })
        .inner
}

fn label(ui: &mut Ui, text: &str) {
    ui.add_space(10.0);
    ui.label(kit::text(
        text,
        size::CAPTION + 1.0,
        MEDIUM,
        theme::text_secondary(),
    ));
    ui.add_space(3.0);
}

pub fn show(
    ui: &mut Ui,
    rows: &[Row<'_>],
    selected: Option<&str>,
    draft: Option<&mut Draft>,
    currency: &str,
) -> Output {
    let mut output = Output::default();
    let list = Card::new("Playbooks").subtitle(
        "The setups you repeat, with how their trades did and how closely you followed them.",
    );
    let mut create = false;
    jk::card(
        ui,
        &list,
        |ui| {
            if super::header_controls::prominent_button(ui, Icon::Plus, "New playbook").clicked() {
                create = true;
            }
        },
        |ui| {
            if rows.is_empty() {
                ui.label(kit::text(
                "No playbooks yet. Create one, then choose it in a trade's review to see how that setup performs.",
                size::BODY,
                REGULAR,
                theme::text_secondary(),
            ));
                return;
            }
            comparison(ui, rows, selected, currency, &mut output);
        },
    );
    output.create |= create;
    if let Some(draft) = draft {
        ui.add_space(jk::GAP);
        editor(ui, draft, &mut output);
    }
    output
}

fn comparison(
    ui: &mut Ui,
    rows: &[Row<'_>],
    selected: Option<&str>,
    currency: &str,
    output: &mut Output,
) {
    let widths = [1.8, 0.7, 0.9, 0.9, 1.0, 0.9, 1.2, 1.0, 0.7];
    let heads = [
        "Playbook",
        "Trades",
        "Win rate",
        "Profit factor",
        "Expectancy",
        "Avg R",
        "Net P&L",
        "Adherence",
        "",
    ];
    let total: f32 = widths.iter().sum();
    let available = ui.available_width();
    let col = |i: usize| available * widths[i] / total;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for (index, head) in heads.iter().enumerate() {
            charts::fixed_cell(ui, vec2(col(index), 20.0), |ui| {
                ui.label(kit::text(
                    head.to_uppercase(),
                    size::CAPTION,
                    SEMIBOLD,
                    theme::text_tertiary(),
                ));
            });
        }
    });
    for row in rows {
        let (rect, response) = ui.allocate_exact_size(vec2(available, 30.0), egui::Sense::click());
        jk::paint_row(
            ui,
            rect,
            selected == Some(row.playbook.id.as_str()),
            response.hovered(),
        );
        let mut line = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        line.spacing_mut().item_spacing.x = 0.0;
        let s = row.slice;
        let cells: [String; 8] = [
            row.playbook.name.clone(),
            s.map_or("0".to_owned(), |s| s.trades.to_string()),
            s.map_or(NONE.to_owned(), |s| match &s.win_rate {
                Metric::Value(v) => format::percent(*v),
                Metric::Unavailable(_) => NONE.to_owned(),
            }),
            s.map_or(NONE.to_owned(), |s| match &s.profit_factor {
                Metric::Value(v) => format::ratio(*v),
                Metric::Unavailable(_) => NONE.to_owned(),
            }),
            s.map_or(NONE.to_owned(), |s| match &s.expectancy {
                Metric::Value(v) => format::signed_money(*v),
                Metric::Unavailable(_) => NONE.to_owned(),
            }),
            s.map_or(NONE.to_owned(), |s| match &s.average_r {
                Metric::Value(v) => format::r_multiple(*v),
                Metric::Unavailable(_) => NONE.to_owned(),
            }),
            s.map_or(NONE.to_owned(), |s| {
                format!("{} {currency}", format::signed_money(s.net))
                    .trim()
                    .to_owned()
            }),
            row.adherence.map_or(NONE.to_owned(), format::percent),
        ];
        for (index, text) in cells.iter().enumerate() {
            charts::fixed_cell(&mut line, vec2(col(index), 30.0), |ui| {
                let color = match (index, s) {
                    (6, Some(s)) => match s.net.to_f64().unwrap_or(0.0) {
                        v if v > 0.0 => Sign::Gain.text(),
                        v if v < 0.0 => Sign::Loss.text(),
                        _ => theme::text_primary(),
                    },
                    _ => theme::text_primary(),
                };
                paper_grid::text_cell(ui, text, color);
            });
        }
        charts::fixed_cell(&mut line, vec2(col(8), 30.0), |ui| {
            match s.map(|s| s.sample) {
                Some(Sample::Thin) => {
                    jk::pill(ui, "THIN", kit::Tone::Warning);
                }
                Some(Sample::Low) => {
                    jk::pill(ui, "SMALL", kit::Tone::Neutral);
                }
                _ => {}
            }
        });
        if response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
        {
            output.select = Some(row.playbook.id.clone());
        }
    }
    ui.add_space(6.0);
    ui.label(kit::text(
        "Adherence is the mean share of a playbook's checklist you ticked. THIN: under 5 trades. SMALL: under 30. Neither is evidence of a real difference.",
        size::CAPTION,
        REGULAR,
        theme::text_tertiary(),
    ));
}

fn editor(ui: &mut Ui, draft: &mut Draft, output: &mut Output) {
    let before = (
        draft.playbook.clone(),
        draft.checklist_text.clone(),
        draft.tags_text.clone(),
    );
    let id = draft.playbook.id.clone();
    let mut blurred = false;
    let mut delete = false;
    jk::card(
        ui,
        &Card::new("Edit playbook"),
        |ui| {
            if kit::destructive_sheet_button(ui, Icon::Trash, "Delete").clicked() {
                delete = true;
            }
        },
        |ui| {
            let mut line = |ui: &mut Ui,
                            salt: &str,
                            title: &str,
                            text: &mut String,
                            hint: &str,
                            rows: usize| {
                label(ui, title);
                let response = field(
                    ui,
                    egui::Id::new(("journal-playbook", &id, salt)),
                    text,
                    hint,
                    rows,
                );
                blurred |= response.lost_focus();
            };
            line(
                ui,
                "name",
                "Name",
                &mut draft.playbook.name,
                "Breakout, mean reversion\u{2026}",
                1,
            );
            line(
                ui,
                "description",
                "Description",
                &mut draft.playbook.description,
                "What is this setup?",
                2,
            );
            line(
                ui,
                "setup",
                "Setup criteria",
                &mut draft.playbook.setup_criteria,
                "What has to be true before you look at it?",
                3,
            );
            line(
                ui,
                "entry",
                "Entry rules",
                &mut draft.playbook.entry_rules,
                "When and how you enter",
                3,
            );
            line(
                ui,
                "exit",
                "Exit rules",
                &mut draft.playbook.exit_rules,
                "Targets, stops, time exits",
                3,
            );
            line(
                ui,
                "risk",
                "Risk rules",
                &mut draft.playbook.risk_rules,
                "Size, maximum loss, number of attempts",
                3,
            );
            line(
                ui,
                "checklist",
                "Checklist (one rule per line)",
                &mut draft.checklist_text,
                "Volume spike\nAbove VWAP\nStop placed",
                5,
            );
            line(
                ui,
                "tags",
                "Tags",
                &mut draft.tags_text,
                "Comma separated",
                1,
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
    if delete {
        output.delete = Some(id.clone());
    }
    draft.sync_fields();
    let after = (
        draft.playbook.clone(),
        draft.checklist_text.clone(),
        draft.tags_text.clone(),
    );
    output.changed = before != after;
    output.blurred = blurred;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn playbook(id: &str, name: &str) -> Playbook {
        let mut p = Playbook::new(id.into(), name);
        p.checklist = vec!["a".into(), "b".into()];
        p
    }

    #[test]
    fn the_checklist_and_tags_round_trip_through_their_text() {
        let mut draft = Draft::new(&playbook("1", "Breakout"));
        assert_eq!(draft.checklist_text, "a\nb");
        assert!(!draft.is_dirty());
        draft.checklist_text = "a\nb\nc".into();
        draft.tags_text = "x, X, y".into();
        draft.sync_fields();
        assert_eq!(draft.playbook.checklist, ["a", "b", "c"]);
        assert_eq!(draft.playbook.tags, ["x", "y"]);
        assert!(draft.is_dirty());
    }

    #[test]
    fn the_page_draws_empty_and_with_a_selected_playbook() {
        let ctx = egui::Context::default();
        let one = playbook("1", "Breakout");
        let rows = [Row {
            playbook: &one,
            slice: None,
            adherence: Some(0.5),
        }];
        let mut draft = Draft::new(&one);
        for (rows, selected) in [(&[][..], None), (&rows[..], Some("1"))] {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::pos2(0.0, 0.0),
                        vec2(1200.0, 1600.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        let result = show(ui, rows, selected, selected.map(|_| &mut draft), "USD");
                        assert!(
                            result.select.is_none() && !result.create && result.delete.is_none()
                        );
                    });
                },
            );
            out.textures_delta.clear();
        }
    }
}

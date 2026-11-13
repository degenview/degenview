//! The sheets for a chart's Pine script alerts: create one (`show`), and choose which webhooks an
//! existing one posts to (`show_webhooks`). Pure views over plain values: the Pine alert store
//! and runtime host (which compile the script, hash its source and own the subscriptions) feed
//! the facts in and apply the outcome. A port of `PineAlertEditor` and `PineAlertWebhooksSheet`,
//! laid out like the price alert sheet beside it (`alert_editor`): a header, a card for what is
//! being armed, cards for the rows, the shared webhook picker, and a footer that says what will
//! happen.

use crate::model::alert;
use crate::model::alert::pine::{ArmingFacts, Blocker, CallRow, PineAlertSubscription};
use crate::model::webhook::{EndpointId, WebhookEndpoint};
use crate::pine::builtins::alert::call::AlertCallSite;
use crate::ui::icons::{self, Icon};
use crate::ui::kit::{self, REGULAR, SEMIBOLD, size};
use crate::ui::{kit::sheet, theme, webhook::picker};
use eframe::egui::{self, Id, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};

/// More call sites than this scroll instead of growing the sheet.
pub const INLINE_CALL_LIMIT: usize = 4;

pub const FOOTNOTE: &str = "Fires only while this chart shows this symbol and timeframe and its live feed is running. \
Editing the script pauses the alert until you re-arm it.";

const WIDTH: f32 = 480.0;
const INSET: f32 = 24.0;
const MAX_FORM_HEIGHT: f32 = 620.0;
const ROW_HEIGHT: f32 = 54.0;
const ROW_GAP: f32 = 6.0;
/// Space between the sheet's sections, as in the price alert sheet.
const GAP: f32 = 18.0;

#[derive(Clone, Default)]
pub struct PineAlertEditorState {
    pub note: String,
    pub sends_to_webhooks: bool,
    pub webhook_ids: Vec<EndpointId>,
}

pub struct PineAlertEditorInput<'a> {
    /// The chart's market, "BTC/USDT".
    pub chart_title: &'a str,
    /// "Binance".
    pub source_name: &'a str,
    pub timeframe: &'a str,
    /// The saved script's name, else the declaration's title; empty until known.
    pub script_name: &'a str,
    pub facts: ArmingFacts,
    pub call_sites: &'a [AlertCallSite],
    pub endpoints: &'a [WebhookEndpoint],
}

#[derive(Clone, Debug, PartialEq)]
pub enum PineAlertEditorOutcome {
    /// Arm it with this note and these endpoints (none unless webhooks are switched on).
    Create {
        note: String,
        webhook_endpoint_ids: Vec<EndpointId>,
    },
    Cancel,
    /// "Add in Settings" in the webhook section; the sheet stays open.
    OpenWebhookSettings,
}

impl PineAlertEditorState {
    #[cfg(test)]
    pub fn toggle_webhook(&mut self, endpoint: EndpointId, on: bool) {
        self.webhook_ids.retain(|id| *id != endpoint);
        if on {
            self.webhook_ids.push(endpoint);
        }
    }

    /// The webhooks that will be used: none unless the switch is on.
    pub fn chosen_webhooks(&self) -> Vec<EndpointId> {
        if self.sends_to_webhooks {
            self.webhook_ids.clone()
        } else {
            Vec::new()
        }
    }
}

// MARK: Pure text

pub fn can_create(facts: ArmingFacts) -> bool {
    facts.loaded && alert::pine::blocker(facts).is_none()
}

/// The script name shown in the card: its own, else a placeholder while loading or untitled.
pub fn displayed_script_name(script_name: &str, loaded: bool) -> String {
    if !script_name.is_empty() {
        script_name.to_owned()
    } else if loaded {
        "Untitled script".to_owned()
    } else {
        "Loading…".to_owned()
    }
}

/// "Fires on live bars of BTC/USDT · 1h", shown beside the Create button.
pub fn summary(facts: ArmingFacts, chart_title: &str, timeframe: &str) -> String {
    if can_create(facts) {
        format!("Fires on live bars of {chart_title} · {timeframe}")
    } else {
        "Nothing to arm yet.".to_owned()
    }
}

/// The readiness badge on the script card.
pub fn status_badge(facts: ArmingFacts) -> (&'static str, kit::Tone) {
    if !facts.loaded {
        ("Checking…", kit::Tone::Neutral)
    } else if alert::pine::blocker(facts).is_none() {
        ("Ready", kit::Tone::Good)
    } else {
        ("Can't arm", kit::Tone::Warning)
    }
}

/// "1 alert" / "3 alerts", above the list of calls.
pub fn call_count(count: usize) -> String {
    format!("{count} alert{}", if count == 1 { "" } else { "s" })
}

// MARK: Create sheet

pub fn show(
    ctx: &egui::Context,
    state: &mut PineAlertEditorState,
    input: &PineAlertEditorInput,
) -> Option<PineAlertEditorOutcome> {
    let mut outcome = None;
    sheet::show(ctx, Id::new("pine-alert-editor"), WIDTH, |ui| {
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        egui::ScrollArea::vertical()
            .id_salt("pine-alert-form")
            .max_height(MAX_FORM_HEIGHT.min(ctx.content_rect().height() - 140.0))
            .auto_shrink([false, true])
            .show(ui, |ui| {
                egui::Frame::new()
                    .inner_margin(egui::Margin::same(INSET as i8))
                    .show(ui, |ui| {
                        ui.set_width(WIDTH - INSET * 2.0);
                        if form(ui, state, input) {
                            outcome = Some(PineAlertEditorOutcome::OpenWebhookSettings);
                        }
                    });
            });
        let (line, _) = ui.allocate_exact_size(vec2(WIDTH, 1.0), Sense::hover());
        ui.painter().hline(
            line.x_range(),
            line.center().y,
            Stroke::new(1.0, theme::separator()),
        );
        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(INSET as i8, 16))
            .show(ui, |ui| {
                ui.set_width(WIDTH - INSET * 2.0);
                if let Some(footer) = footer(ui, state, input) {
                    outcome = Some(footer);
                }
            });
    });
    let (escape, enter) = ctx.input(|input| {
        (
            input.key_pressed(egui::Key::Escape),
            input.key_pressed(egui::Key::Enter) && !input.modifiers.shift,
        )
    });
    if escape {
        outcome = Some(PineAlertEditorOutcome::Cancel);
    } else if enter && outcome.is_none() && can_create(input.facts) {
        outcome = Some(create(state));
    }
    outcome
}

fn create(state: &PineAlertEditorState) -> PineAlertEditorOutcome {
    PineAlertEditorOutcome::Create {
        note: state.note.trim().to_owned(),
        webhook_endpoint_ids: state.chosen_webhooks(),
    }
}

fn section_title(ui: &mut Ui, title: &str) {
    ui.label(kit::text(
        title,
        size::SUBHEADLINE + 1.0,
        SEMIBOLD,
        theme::text_primary(),
    ));
}

/// Returns whether "Add in Settings" was clicked in the webhook section.
fn form(ui: &mut Ui, state: &mut PineAlertEditorState, input: &PineAlertEditorInput) -> bool {
    sheet::header(
        ui,
        Icon::AngleBrackets,
        "Create Script Alert",
        &format!("{} · {}", input.chart_title, input.source_name),
        theme::text_primary(),
    );
    ui.add_space(GAP);
    script_card(ui, input);
    if let Some(Blocker { title, detail }) = alert::pine::blocker(input.facts) {
        ui.add_space(12.0);
        kit::notice_card(
            ui,
            Icon::TriangleFill,
            kit::ORANGE,
            title,
            Some(detail),
            None,
        );
    }
    ui.add_space(GAP);

    // A script with no alert calls is explained by the notice above; the list would repeat it.
    if !input.facts.loaded || !input.call_sites.is_empty() {
        triggers(ui, input);
        ui.add_space(GAP);
    }

    note_field(ui, state);
    ui.add_space(GAP);

    let open_settings = picker::send_section(
        ui,
        Id::new("pine-alert-webhooks"),
        &mut state.sends_to_webhooks,
        &mut state.webhook_ids,
        input.endpoints,
        None,
    );
    ui.add_space(GAP);

    footnote(ui);
    open_settings
}

/// What is being armed: the script and where it runs, with whether it can be armed.
fn script_card(ui: &mut Ui, input: &PineAlertEditorInput) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 66.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 12.0, theme::quaternary(0.35));
    painter.rect_stroke(
        rect,
        12.0,
        Stroke::new(1.0, kit::hairline()),
        StrokeKind::Inside,
    );
    let (label, tone) = status_badge(input.facts);
    let badge_width = kit::status_badge_size(ui, label).x;
    let room = rect.width() - 28.0 - badge_width - 12.0;
    let caption = kit::galley(
        ui,
        &format!("Applied on {} · {}", input.chart_title, input.timeframe),
        size::CAPTION + 0.5,
        REGULAR,
        theme::text_secondary(),
        room,
    );
    let name = kit::galley(
        ui,
        &displayed_script_name(input.script_name, input.facts.loaded),
        size::TITLE2 + 1.0,
        SEMIBOLD,
        theme::text_primary(),
        room,
    );
    let top = rect.center().y - (caption.size().y + 2.0 + name.size().y) / 2.0;
    let caption_height = caption.size().y;
    ui.painter().galley(
        pos2(rect.left() + 14.0, top),
        caption,
        theme::text_secondary(),
    );
    ui.painter().galley(
        pos2(rect.left() + 14.0, top + caption_height + 2.0),
        name,
        theme::text_primary(),
    );
    kit::paint_status_badge(ui, rect.right() - 14.0, rect.center().y, label, tone);
}

/// The script's `alert()` and `alertcondition()` calls, one card each.
fn triggers(ui: &mut Ui, input: &PineAlertEditorInput) {
    ui.horizontal(|ui| {
        section_title(ui, "Alerts in this script");
        if !input.call_sites.is_empty() {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(kit::text(
                    call_count(input.call_sites.len()),
                    size::CAPTION + 0.5,
                    REGULAR,
                    theme::text_tertiary(),
                ));
            });
        }
    });
    ui.add_space(8.0);
    if input.call_sites.is_empty() {
        row_card(
            ui,
            Icon::Bell,
            "Reading the script…",
            Some("Looking for its alert calls."),
            None,
            None,
            "",
        );
        return;
    }
    let rows = |ui: &mut Ui| {
        for (index, site) in input.call_sites.iter().enumerate() {
            if index > 0 {
                ui.add_space(ROW_GAP);
            }
            call_row(ui, &alert::pine::call_row(site));
        }
    };
    if input.call_sites.len() > INLINE_CALL_LIMIT {
        let height =
            INLINE_CALL_LIMIT as f32 * ROW_HEIGHT + (INLINE_CALL_LIMIT - 1) as f32 * ROW_GAP;
        egui::ScrollArea::vertical()
            .id_salt("pine-alert-calls")
            .max_height(height)
            .auto_shrink([false, true])
            .show(ui, rows);
    } else {
        rows(ui);
    }
}

fn call_row(ui: &mut Ui, row: &CallRow) {
    // Under a title the message is the detail; with only a message the line says what kind of
    // call it is.
    let detail = row.detail.clone().unwrap_or_else(|| row.help.to_owned());
    row_card(
        ui,
        Icon::Bell,
        &row.headline,
        Some(&detail),
        Some(row.frequency),
        Some(format!("Line {}", row.line)),
        row.help,
    );
}

/// A 54-point card: a neutral badge, a headline and a detail line, and at the right an optional
/// capsule with a caption under it.
fn row_card(
    ui: &mut Ui,
    icon: Icon,
    headline: &str,
    detail: Option<&str>,
    capsule: Option<&str>,
    caption: Option<String>,
    help: &str,
) {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::hover());
    if !help.is_empty() {
        response.on_hover_text(help);
    }
    let painter = ui.painter();
    painter.rect_filled(rect, 10.0, kit::primary(0.025));
    painter.rect_stroke(
        rect,
        10.0,
        Stroke::new(1.0, kit::primary(0.12)),
        StrokeKind::Inside,
    );
    let badge = Rect::from_min_size(
        pos2(rect.left() + 12.0, rect.center().y - 15.0),
        Vec2::splat(30.0),
    );
    painter.rect_filled(badge, 8.0, theme::with_alpha(theme::text_secondary(), 0.14));
    icons::paint(
        painter,
        Rect::from_center_size(badge.center(), Vec2::splat(13.0)),
        icon,
        theme::text_secondary(),
    );

    // The right column first, so the text knows how much room it has.
    let mut right = rect.right() - 12.0;
    if capsule.is_some() || caption.is_some() {
        let capsule_galley = capsule.map(|label| {
            kit::galley(
                ui,
                label,
                size::CAPTION,
                SEMIBOLD,
                theme::text_secondary(),
                150.0,
            )
        });
        let caption_galley = caption.as_deref().map(|label| {
            kit::galley(
                ui,
                label,
                size::CAPTION,
                REGULAR,
                theme::text_tertiary(),
                80.0,
            )
        });
        let capsule_width = capsule_galley.as_ref().map_or(0.0, |g| g.size().x + 14.0);
        let caption_width = caption_galley.as_ref().map_or(0.0, |g| g.size().x);
        let column = capsule_width.max(caption_width);
        let mut y = rect.center().y
            - (capsule_galley.as_ref().map_or(0.0, |g| g.size().y + 4.0)
                + if capsule_galley.is_some() && caption_galley.is_some() {
                    3.0
                } else {
                    0.0
                }
                + caption_galley.as_ref().map_or(0.0, |g| g.size().y))
                / 2.0;
        if let Some(galley) = capsule_galley {
            let pill = Rect::from_min_size(
                pos2(right - capsule_width, y),
                vec2(capsule_width, galley.size().y + 4.0),
            );
            ui.painter()
                .rect_filled(pill, pill.height() / 2.0, kit::primary(0.08));
            ui.painter().galley(
                pill.center() - galley.size() / 2.0,
                galley,
                theme::text_secondary(),
            );
            y = pill.bottom() + 3.0;
        }
        if let Some(galley) = caption_galley {
            ui.painter().galley(
                pos2(right - galley.size().x, y),
                galley,
                theme::text_tertiary(),
            );
        }
        right -= column + 12.0;
    }

    let left = badge.right() + 12.0;
    let name = kit::galley(
        ui,
        headline,
        size::SUBHEADLINE + 1.0,
        SEMIBOLD,
        theme::text_primary(),
        right - left,
    );
    let detail = detail.map(|text| {
        kit::galley(
            ui,
            text,
            size::CAPTION + 0.5,
            REGULAR,
            theme::text_secondary(),
            right - left,
        )
    });
    let height = name.size().y + detail.as_ref().map_or(0.0, |g| 1.0 + g.size().y);
    let top = rect.center().y - height / 2.0;
    let name_height = name.size().y;
    ui.painter()
        .galley(pos2(left, top), name, theme::text_primary());
    if let Some(detail) = detail {
        ui.painter().galley(
            pos2(left, top + name_height + 1.0),
            detail,
            theme::text_secondary(),
        );
    }
}

fn note_field(ui: &mut Ui, state: &mut PineAlertEditorState) {
    let id = Id::new("pine-alert-note");
    let focused = ui.memory(|memory| memory.has_focus(id));
    kit::field_frame(focused).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            icons::inline(ui, Icon::List, 13.0, theme::text_secondary());
            let width = ui.available_width();
            kit::plain_field(
                ui,
                id,
                &mut state.note,
                "Add a note (optional)",
                size::BODY,
                REGULAR,
                width,
            );
        });
    });
}

fn footnote(ui: &mut Ui) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        icons::inline(ui, Icon::InfoCircle, 11.0, theme::text_tertiary());
        ui.add(
            egui::Label::new(kit::text(
                FOOTNOTE,
                size::CAPTION + 0.5,
                REGULAR,
                theme::text_tertiary(),
            ))
            .wrap(),
        );
    });
}

fn footer(
    ui: &mut Ui,
    state: &PineAlertEditorState,
    input: &PineAlertEditorInput,
) -> Option<PineAlertEditorOutcome> {
    let mut outcome = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        let width = ui.available_width() - 76.0 - 120.0 - 24.0;
        ui.allocate_ui_with_layout(
            vec2(width, 30.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.set_width(width);
                ui.add(
                    egui::Label::new(kit::text(
                        summary(input.facts, input.chart_title, input.timeframe),
                        size::CAPTION + 0.5,
                        REGULAR,
                        theme::text_secondary(),
                    ))
                    .wrap(),
                );
            },
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            if kit::sheet_button(ui, "Create Alert", true, can_create(input.facts)).clicked() {
                outcome = Some(create(state));
            }
            if kit::sheet_button(ui, "Cancel", false, true).clicked() {
                outcome = Some(PineAlertEditorOutcome::Cancel);
            }
        });
    });
    outcome
}

// MARK: Webhooks of an existing alert

pub struct PineAlertWebhooksState {
    pub selection: Vec<EndpointId>,
}

impl PineAlertWebhooksState {
    pub fn new(subscription: &PineAlertSubscription) -> Self {
        Self {
            selection: subscription.webhook_endpoint_ids.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum PineAlertWebhooksOutcome {
    Save(Vec<EndpointId>),
    Cancel,
    /// "Add in Settings"; the sheet stays open.
    OpenSettings,
}

/// Chooses which webhook endpoints an existing Pine alert posts to.
pub fn show_webhooks(
    ctx: &egui::Context,
    state: &mut PineAlertWebhooksState,
    subscription: &PineAlertSubscription,
    endpoints: &[WebhookEndpoint],
) -> Option<PineAlertWebhooksOutcome> {
    let mut outcome = None;
    sheet::show(ctx, Id::new("pine-alert-webhooks"), 440.0, |ui| {
        ui.add_space(24.0);
        sheet::inset(ui, |ui| {
            sheet::header(
                ui,
                Icon::ArrowUpRightSquare,
                "Alert Webhooks",
                &subscription.script_name,
                theme::text_primary(),
            );
            ui.add_space(GAP);
            if picker::endpoint_picker(
                ui,
                Id::new("pine-alert-webhook-list"),
                &mut state.selection,
                endpoints,
            ) {
                outcome = Some(PineAlertWebhooksOutcome::OpenSettings);
            }
            ui.add_space(12.0);
            ui.add(
                egui::Label::new(kit::text(
                    "The alert's message is posted as is. Frequency and re-arming work exactly as they do for notifications.",
                    size::CAPTION + 0.5,
                    REGULAR,
                    theme::text_tertiary(),
                ))
                .wrap(),
            );
            ui.add_space(GAP);
        });
        sheet::footer(ui, |ui| {
            if kit::sheet_button(ui, "Save", true, true).clicked() {
                outcome = Some(PineAlertWebhooksOutcome::Save(state.selection.clone()));
            }
            if kit::sheet_button(ui, "Cancel", false, true).clicked() {
                outcome = Some(PineAlertWebhooksOutcome::Cancel);
            }
        });
    });
    if outcome.is_none() && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        outcome = Some(PineAlertWebhooksOutcome::Cancel);
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::alert::pine::PineAlertState;
    use crate::pine::builtins::alert::frequency::AlertFrequency;
    use crate::pine::syntax::token::{SourcePosition, SourceRange};

    fn facts() -> ArmingFacts {
        ArmingFacts {
            loaded: true,
            instance_exists: true,
            source_loaded: true,
            call_sites: 1,
            duplicate_exists: false,
        }
    }

    fn facts_n(call_sites: usize) -> ArmingFacts {
        ArmingFacts {
            call_sites,
            ..facts()
        }
    }

    fn site() -> AlertCallSite {
        AlertCallSite {
            frequency: Some(AlertFrequency::OncePerBarClose),
            is_condition: false,
            range: SourceRange::new(SourcePosition::new(4, 1, 0), SourcePosition::new(4, 5, 4)),
            title: None,
            message: Some("Buy".into()),
        }
    }

    #[test]
    fn creating_needs_a_loaded_unblocked_script() {
        assert!(can_create(facts()));
        assert!(
            !can_create(ArmingFacts {
                loaded: false,
                ..facts()
            }),
            "unknown is not ready"
        );
        assert!(!can_create(ArmingFacts {
            call_sites: 0,
            ..facts()
        }));
        assert!(!can_create(ArmingFacts {
            duplicate_exists: true,
            ..facts()
        }));
    }

    #[test]
    fn the_card_and_footer_say_what_will_happen() {
        assert_eq!(displayed_script_name("RSI", true), "RSI");
        assert_eq!(displayed_script_name("", true), "Untitled script");
        assert_eq!(displayed_script_name("", false), "Loading…");
        assert_eq!(
            summary(facts(), "BTC/USDT", "1h"),
            "Fires on live bars of BTC/USDT · 1h"
        );
        assert_eq!(
            summary(
                ArmingFacts {
                    call_sites: 0,
                    ..facts()
                },
                "BTC/USDT",
                "1h"
            ),
            "Nothing to arm yet."
        );
        assert_eq!(
            status_badge(ArmingFacts {
                loaded: false,
                ..facts()
            })
            .0,
            "Checking…"
        );
        assert_eq!(status_badge(facts()).0, "Ready");
        assert_eq!(
            status_badge(ArmingFacts {
                call_sites: 0,
                ..facts()
            })
            .0,
            "Can't arm"
        );
    }

    #[test]
    fn the_list_counts_its_alerts() {
        assert_eq!(call_count(1), "1 alert");
        assert_eq!(call_count(3), "3 alerts");
    }

    #[test]
    fn webhooks_are_used_only_when_the_switch_is_on() {
        let mut state = PineAlertEditorState::default();
        state.toggle_webhook(4, true);
        state.toggle_webhook(5, true);
        state.toggle_webhook(4, true);
        assert_eq!(state.webhook_ids, [5, 4], "no duplicates");
        assert!(
            state.chosen_webhooks().is_empty(),
            "switched off: none, whatever was picked"
        );
        state.sends_to_webhooks = true;
        assert_eq!(state.chosen_webhooks(), [5, 4]);
        state.toggle_webhook(5, false);
        assert_eq!(state.chosen_webhooks(), [4]);
    }

    fn titled_site(title: &str, message: &str, line: usize, once: bool) -> AlertCallSite {
        AlertCallSite {
            frequency: Some(if once {
                AlertFrequency::OncePerBar
            } else {
                AlertFrequency::OncePerBarClose
            }),
            is_condition: title.is_empty(),
            range: SourceRange::new(
                SourcePosition::new(line, 1, 0),
                SourcePosition::new(line, 5, 4),
            ),
            title: (!title.is_empty()).then(|| title.to_owned()),
            message: Some(message.to_owned()),
        }
    }

    /// The sheets as pictures, in both themes. Run by hand with `DEGENVIEW_GALLERY_DIR` set.
    #[test]
    #[ignore = "writes images for a person to look at"]
    #[allow(clippy::type_complexity)]
    fn gallery_of_the_script_alert_sheets() {
        use crate::ui::render_tests::{Screen, gallery_dir};
        use crate::ui::theme::AppTheme;
        if gallery_dir().is_none() {
            return;
        }
        let sites = vec![
            titled_site(
                "RSI crossed 70",
                "RSI is overbought on {{ticker}}",
                14,
                false,
            ),
            titled_site("", "Golden cross: fast MA above slow MA", 22, true),
            titled_site("Volume spike", "Volume is 3x its 20 bar average", 31, false),
        ];
        let many: Vec<_> = (0..7)
            .map(|index| {
                titled_site(
                    &format!("Level {}", index + 1),
                    "Price touched the level",
                    10 + index * 3,
                    false,
                )
            })
            .collect();
        let endpoints = vec![
            WebhookEndpoint::new("Trading bot", "https://bot.example.com/hooks/{{secret}}", 0),
            {
                let mut paused =
                    WebhookEndpoint::new("Discord", "https://discord.com/api/webhooks/1/abc", 0);
                paused.is_enabled = false;
                paused
            },
            WebhookEndpoint::new("Home server", "http://192.168.1.20:8080/alert", 0),
        ];
        let cases: Vec<(
            &str,
            PineAlertEditorState,
            ArmingFacts,
            &[AlertCallSite],
            &[WebhookEndpoint],
            &str,
        )> = vec![
            (
                "ready",
                PineAlertEditorState::default(),
                facts_n(3),
                &sites,
                &endpoints,
                "Relative Strength Index",
            ),
            (
                "webhooks",
                PineAlertEditorState {
                    note: "Check the 4h trend first".into(),
                    sends_to_webhooks: true,
                    webhook_ids: vec![endpoints[0].id],
                },
                facts_n(7),
                &many,
                &endpoints,
                "A rather long script name that will have to be cut with an ellipsis somewhere",
            ),
            (
                "no-webhooks",
                PineAlertEditorState {
                    sends_to_webhooks: true,
                    ..Default::default()
                },
                facts_n(3),
                &sites,
                &[],
                "RSI",
            ),
            (
                "blocked",
                PineAlertEditorState::default(),
                ArmingFacts {
                    call_sites: 0,
                    ..facts()
                },
                &[],
                &endpoints,
                "Plain indicator",
            ),
            (
                "armed",
                PineAlertEditorState::default(),
                ArmingFacts {
                    duplicate_exists: true,
                    ..facts_n(3)
                },
                &sites,
                &endpoints,
                "RSI",
            ),
            (
                "loading",
                PineAlertEditorState::default(),
                ArmingFacts {
                    loaded: false,
                    call_sites: 0,
                    ..facts()
                },
                &[],
                &[],
                "",
            ),
        ];
        for (theme_name, app_theme) in [("dark", AppTheme::Dark), ("light", AppTheme::Light)] {
            for (name, state, case_facts, calls, case_endpoints, script) in cases.iter().cloned() {
                let mut state = state;
                let mut screen = Screen::new(egui::vec2(640.0, 900.0));
                crate::ui::theme::apply(&screen.ctx, app_theme);
                let mut last = None;
                for _ in 0..4 {
                    crate::ui::theme::sync(&screen.ctx);
                    last = Some(screen.run(Vec::new(), |ui| {
                        show(
                            ui.ctx(),
                            &mut state,
                            &PineAlertEditorInput {
                                chart_title: "BTC/USDT",
                                source_name: "Binance",
                                timeframe: "1h",
                                script_name: script,
                                facts: case_facts,
                                call_sites: calls,
                                endpoints: case_endpoints,
                            },
                        );
                    }));
                }
                screen.save(
                    &format!("script-alert-{name}-{theme_name}"),
                    last.expect("a frame"),
                );
            }
            let subscription = PineAlertSubscription {
                id: 1,
                chart_id: 1,
                instance_id: None,
                script_id: None,
                script_name: "Relative Strength Index".into(),
                source: String::new(),
                inputs: Default::default(),
                symbol_key: "BINANCE:BTCUSDT".into(),
                timeframe: "1h".into(),
                source_hash: "h".into(),
                note: String::new(),
                state: crate::model::alert::pine::PineAlertState::Active,
                created_at_ms: 0,
                webhook_endpoint_ids: vec![endpoints[0].id],
            };
            for (name, list) in [("list", &endpoints[..]), ("none", &[][..])] {
                let mut screen = Screen::new(egui::vec2(640.0, 700.0));
                crate::ui::theme::apply(&screen.ctx, app_theme);
                let mut state = PineAlertWebhooksState::new(&subscription);
                let mut last = None;
                for _ in 0..4 {
                    crate::ui::theme::sync(&screen.ctx);
                    last = Some(screen.run(Vec::new(), |ui| {
                        show_webhooks(ui.ctx(), &mut state, &subscription, list);
                    }));
                }
                screen.save(
                    &format!("script-alert-webhooks-sheet-{name}-{theme_name}"),
                    last.expect("a frame"),
                );
            }
        }
    }

    fn run_frame(ctx: &egui::Context, paint: impl FnOnce(&egui::Context)) {
        let mut paint = Some(paint);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            if let Some(paint) = paint.take() {
                paint(ui.ctx());
            }
        });
        output.textures_delta.clear();
    }

    #[test]
    fn both_sheets_draw_headlessly() {
        let ctx = egui::Context::default();
        let sites = [site(), site(), site(), site(), site()];
        for (loaded, calls) in [
            (false, &sites[..0]),
            (true, &sites[..1]),
            (true, &sites[..]),
        ] {
            let mut state = PineAlertEditorState {
                sends_to_webhooks: true,
                ..PineAlertEditorState::default()
            };
            let mut outcome = None;
            run_frame(&ctx, |ctx| {
                outcome = show(
                    ctx,
                    &mut state,
                    &PineAlertEditorInput {
                        chart_title: "BTC/USDT",
                        source_name: "Binance",
                        timeframe: "1h",
                        script_name: "RSI",
                        facts: ArmingFacts {
                            loaded,
                            call_sites: calls.len(),
                            ..facts()
                        },
                        call_sites: calls,
                        endpoints: &[],
                    },
                );
            });
            assert_eq!(outcome, None);
        }
        let subscription = PineAlertSubscription {
            id: 1,
            chart_id: 1,
            instance_id: None,
            script_id: None,
            script_name: "RSI".into(),
            source: String::new(),
            inputs: Default::default(),
            symbol_key: "BINANCE:BTCUSDT".into(),
            timeframe: "1h".into(),
            source_hash: "h".into(),
            note: String::new(),
            state: PineAlertState::Active,
            created_at_ms: 0,
            webhook_endpoint_ids: vec![2, 3],
        };
        let mut state = PineAlertWebhooksState::new(&subscription);
        assert_eq!(state.selection, [2, 3]);
        let mut outcome = None;
        run_frame(&ctx, |ctx| {
            outcome = show_webhooks(ctx, &mut state, &subscription, &[]);
        });
        assert_eq!(outcome, None);
    }
}

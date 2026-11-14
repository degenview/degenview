//! Create or edit one price alert: what to watch for, at what level, how often, a note
//! and the webhooks it posts to. A pure view over a `PriceAlertEditorState`; all the
//! parsing, hints and summary live on its `model::alert::draft::PriceAlertDraft` so they are
//! testable and the host (which owns the quote layer and the alert store) only feeds prices in
//! and takes the alert out.

use crate::model::alert::draft::{Direction, HintTone, PriceAlertDraft};
use crate::model::alert::{AlertAsset, AlertCurrency, AlertFrequency, PriceAlert};
use crate::model::webhook::WebhookEndpoint;
use crate::service::watchlist_quotes::WatchlistQuoteBook;
use crate::ui::icons::{self, Icon};
use crate::ui::kit::{self, SEMIBOLD, size};
use crate::ui::{theme, webhook::picker};
use eframe::egui::{self, Color32, Id, Pos2, Rect, Sense, Stroke, Vec2};
use std::collections::HashMap;

pub const PERCENT_OPTIONS: [u32; 5] = [1, 2, 5, 10, 20];

/// The price-alert sheet: the draft it edits, and what only the sheet needs.
#[derive(Clone, Debug)]
pub struct PriceAlertEditorState {
    pub draft: PriceAlertDraft,
    /// Set by the host when an identical alert already exists.
    pub identical: bool,
    focus_value: bool,
}

impl PriceAlertEditorState {
    /// `default_currency` is the asset's own quote currency when it is one an alert can use.
    pub fn new(
        asset: AlertAsset,
        existing: Option<PriceAlert>,
        default_currency: Option<AlertCurrency>,
    ) -> Self {
        Self {
            draft: PriceAlertDraft::new(asset, existing, default_currency),
            identical: false,
            focus_value: true,
        }
    }

    /// Reads the market from the shared quote book.
    pub fn feed_from_book(
        &mut self,
        book: &WatchlistQuoteBook,
        rates: &HashMap<AlertCurrency, f64>,
    ) {
        let quote = book.get_by_key(&self.draft.asset().key());
        self.draft.feed_quote(quote.as_ref(), rates);
    }
}

// MARK: View

/// Quote-demand owner for a sheet open in tab `tab`, clear of the tab ids the watchlist
/// sidebars use and of the alert runtime's own owner.
pub fn quote_owner(tab: u64) -> u64 {
    (1 << 62) + tab
}

/// Quote-demand owner for the sheet the Alerts window hosts (Edit).
pub const ALERTS_WINDOW_QUOTE_OWNER: u64 = u64::MAX - 2;

#[derive(Clone, Debug, PartialEq)]
pub enum PriceAlertEditorOutcome {
    Save(Box<PriceAlert>),
    Cancel,
    /// "Add in Settings" from the empty webhook list; the sheet stays open.
    OpenWebhookSettings,
}

const WIDTH: f32 = 480.0;
const INSET: f32 = 24.0;
const MAX_FORM_HEIGHT: f32 = 620.0;

fn direction_icon(direction: Direction) -> Icon {
    match direction {
        Direction::Above => Icon::ArrowUpRight,
        Direction::Below => Icon::ArrowDownRight,
        Direction::Rises => Icon::ArrowUp,
        Direction::Falls => Icon::ArrowDown,
    }
}

pub fn show(
    ctx: &egui::Context,
    state: &mut PriceAlertEditorState,
    endpoints: &[WebhookEndpoint],
    now_ms: i64,
) -> Option<PriceAlertEditorOutcome> {
    let mut outcome = None;
    let shown = egui::Modal::new(Id::new("price-alert-editor"))
        .backdrop_color(Color32::from_black_alpha(110))
        .frame(
            egui::Frame::new()
                .fill(theme::chrome())
                .corner_radius(12.0)
                .stroke(Stroke::new(1.0, theme::ink(22)))
                .shadow(egui::Shadow {
                    offset: [0, 12],
                    blur: 40,
                    spread: 0,
                    color: Color32::from_black_alpha(130),
                }),
        )
        .show(ctx, |ui| {
            ui.set_width(WIDTH);
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            egui::ScrollArea::vertical()
                .id_salt("price-alert-form")
                .max_height(MAX_FORM_HEIGHT.min(ctx.content_rect().height() - 140.0))
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    egui::Frame::new()
                        .inner_margin(egui::Margin::same(INSET as i8))
                        .show(ui, |ui| {
                            ui.set_width(WIDTH - INSET * 2.0);
                            if form(ui, state, endpoints, now_ms) {
                                outcome = Some(PriceAlertEditorOutcome::OpenWebhookSettings);
                            }
                        });
                });
            let (line, _) = ui.allocate_exact_size(Vec2::new(WIDTH, 1.0), Sense::hover());
            ui.painter().hline(
                line.x_range(),
                line.center().y,
                Stroke::new(1.0, theme::separator()),
            );
            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(INSET as i8, 16))
                .show(ui, |ui| {
                    ui.set_width(WIDTH - INSET * 2.0);
                    if let Some(footer) = footer(ui, state, now_ms) {
                        outcome = Some(footer);
                    }
                });
        });
    // A click on the dimmed area around the sheet cancels it, like Escape. A click that only
    // closed a pop-up (a menu, the date picker) does not.
    if shown.backdrop_response.clicked() && !shown.any_popup_open {
        outcome = Some(PriceAlertEditorOutcome::Cancel);
    }
    let (escape, enter) = ctx.input(|input| {
        (
            input.key_pressed(egui::Key::Escape),
            input.key_pressed(egui::Key::Enter) && !input.modifiers.shift,
        )
    });
    let typing_message = ctx.memory(|memory| {
        memory.has_focus(Id::new("price-alert-webhooks").with("message").with("text"))
    });
    if escape {
        outcome = Some(PriceAlertEditorOutcome::Cancel);
    } else if enter && !typing_message && outcome.is_none() && state.draft.can_save() {
        outcome = state
            .draft
            .result(now_ms)
            .map(|alert| PriceAlertEditorOutcome::Save(Box::new(alert)));
    }
    outcome
}

fn section_title(ui: &mut egui::Ui, title: &str) {
    ui.label(kit::text(
        title,
        size::SUBHEADLINE + 1.0,
        SEMIBOLD,
        theme::text_primary(),
    ));
}

fn hint(ui: &mut egui::Ui, text: &str, color: Color32) {
    ui.add(egui::Label::new(kit::text(text, size::CAPTION + 0.5, kit::REGULAR, color)).wrap());
}

/// Returns whether "Add in Settings" was clicked in the webhook section.
fn form(
    ui: &mut egui::Ui,
    state: &mut PriceAlertEditorState,
    endpoints: &[WebhookEndpoint],
    now_ms: i64,
) -> bool {
    header(ui, state);
    ui.add_space(18.0);
    price_card(ui, state);
    ui.add_space(18.0);

    section_title(ui, "Alert me when the price");
    ui.add_space(8.0);
    let items: Vec<kit::TabItem<Direction>> = Direction::ALL
        .into_iter()
        .map(|direction| kit::TabItem {
            value: direction,
            title: direction.title(),
            icon: direction_icon(direction),
            count: 0,
        })
        .collect();
    let before = state.draft.direction;
    kit::tab_bar(
        ui,
        Id::new("price-alert-direction"),
        &items,
        &mut state.draft.direction,
    );
    if state.draft.direction != before {
        state.focus_value = true;
    }
    ui.add_space(8.0);
    hint(ui, state.draft.direction.blurb(), theme::text_secondary());
    ui.add_space(18.0);

    value_section(ui, state);
    ui.add_space(18.0);

    // How often: a slim row, since most alerts keep the default.
    ui.horizontal(|ui| {
        ui.label(kit::text(
            "How often",
            size::SUBHEADLINE + 1.0,
            kit::REGULAR,
            theme::text_secondary(),
        ));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            kit::segmented(
                ui,
                Id::new("price-alert-frequency"),
                &[
                    (AlertFrequency::Once, "Once"),
                    (AlertFrequency::EveryTime, "Every time"),
                ],
                &mut state.draft.frequency,
            );
        });
    })
    .response
    .on_hover_text(if state.draft.frequency == AlertFrequency::Once {
        "Fires once, then waits until you turn it back on."
    } else {
        "Fires on each crossing, re-arming in between."
    });
    ui.add_space(18.0);

    let note_id = Id::new("price-alert-note");
    let focused = ui.memory(|memory| memory.has_focus(note_id));
    kit::field_frame(focused).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            icons::inline(ui, Icon::List, 13.0, theme::text_secondary());
            let width = ui.available_width();
            kit::plain_field(
                ui,
                note_id,
                &mut state.draft.note,
                "Add a note (optional)",
                size::BODY,
                kit::REGULAR,
                width,
            );
        });
    });
    ui.add_space(18.0);

    let context = state.draft.example_context(now_ms);
    let mut message = std::mem::take(&mut state.draft.webhook_message);
    let open_settings = picker::send_section(
        ui,
        Id::new("price-alert-webhooks"),
        &mut state.draft.sends_to_webhooks,
        &mut state.draft.webhook_ids,
        endpoints,
        Some((&mut message, &context)),
    );
    state.draft.webhook_message = message;

    if state.identical {
        ui.add_space(18.0);
        kit::notice_card(
            ui,
            Icon::TriangleFill,
            kit::ORANGE,
            "An identical alert already exists",
            Some("Saving adds a second one that fires at the same time."),
            None,
        );
    }
    open_settings
}

fn header(ui: &mut egui::Ui, state: &PriceAlertEditorState) {
    let (row, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 46.0), Sense::hover());
    let icon = Rect::from_min_size(row.min + Vec2::new(0.0, 1.0), Vec2::splat(40.0));
    kit::ticker_icon(
        ui,
        icon,
        &state.draft.asset().symbol,
        state.draft.asset().source(),
    );
    kit::source_badge(ui, icon, state.draft.asset().source());
    let left = icon.right() + 3.0 + 14.0;
    let width = row.right() - left;
    let title = kit::galley(
        ui,
        if state.draft.is_new() {
            "Create Price Alert"
        } else {
            "Edit Price Alert"
        },
        size::TITLE2 + 1.0,
        SEMIBOLD,
        theme::text_primary(),
        width,
    );
    let subtitle = kit::galley(
        ui,
        &format!(
            "{} · {}",
            state.draft.asset().name,
            state.draft.asset().source().display_name()
        ),
        size::SUBHEADLINE + 1.0,
        kit::REGULAR,
        theme::text_secondary(),
        width,
    );
    let top = row.center().y - (title.size().y + 2.0 + subtitle.size().y) / 2.0;
    let title_height = title.size().y;
    ui.painter()
        .galley(Pos2::new(left, top), title, theme::text_primary());
    ui.painter().galley(
        Pos2::new(left, top + title_height + 2.0),
        subtitle,
        theme::text_secondary(),
    );
}

fn price_card(ui: &mut egui::Ui, state: &PriceAlertEditorState) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 66.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 12.0, theme::quaternary(0.35));
    painter.rect_stroke(
        rect,
        12.0,
        Stroke::new(1.0, kit::hairline()),
        egui::StrokeKind::Inside,
    );
    let caption = kit::galley(
        ui,
        "Current price",
        size::CAPTION + 0.5,
        kit::REGULAR,
        theme::text_secondary(),
        200.0,
    );
    let value = kit::digits(
        ui,
        &state.draft.current_price().map_or_else(
            || "—".to_owned(),
            |price| state.draft.currency().format_price(price),
        ),
        size::TITLE2 + 1.0,
        SEMIBOLD,
        theme::text_primary(),
        rect.width() - 160.0,
    );
    let top = rect.center().y - (caption.size().y + 2.0 + value.size().y) / 2.0;
    let caption_height = caption.size().y;
    ui.painter().galley(
        Pos2::new(rect.left() + 14.0, top),
        caption,
        theme::text_secondary(),
    );
    ui.painter().galley(
        Pos2::new(rect.left() + 14.0, top + caption_height + 2.0),
        value,
        theme::text_primary(),
    );
    let (label, tone) = if state.draft.current_price().is_some() {
        ("Live", kit::Tone::Good)
    } else {
        ("No price yet", kit::Tone::Warning)
    };
    kit::paint_status_badge(ui, rect.right() - 14.0, rect.center().y, label, tone);
}

/// The value input with its label in front of it, like a row in a settings form.
fn value_field(ui: &mut egui::Ui, id: Id, label: &str, content: impl FnOnce(&mut egui::Ui, bool)) {
    let focused = ui.memory(|memory| memory.has_focus(id));
    // Clicking anywhere on the row, label included, focuses the field. The hit area is last
    // frame's row, registered before the contents so the currency menu still gets its clicks.
    let row_key = id.with("row");
    if let Some(rect) = ui.data(|data| data.get_temp::<Rect>(row_key))
        && ui
            .interact(rect, row_key, Sense::click())
            .on_hover_cursor(egui::CursorIcon::Text)
            .clicked()
    {
        ui.memory_mut(|memory| memory.request_focus(id));
    }
    let response = kit::field_frame(focused)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.label(kit::text(
                    label,
                    size::SUBHEADLINE + 1.0,
                    kit::MEDIUM,
                    theme::text_secondary(),
                ));
                let (divider, _) = ui.allocate_exact_size(Vec2::new(1.0, 18.0), Sense::hover());
                ui.painter().vline(
                    divider.center().x,
                    divider.y_range(),
                    Stroke::new(1.0, theme::separator()),
                );
                content(ui, focused);
            });
        })
        .response;
    ui.data_mut(|data| data.insert_temp(row_key, response.rect));
}

fn value_section(ui: &mut egui::Ui, state: &mut PriceAlertEditorState) {
    let focus = std::mem::take(&mut state.focus_value);
    if state.draft.direction.is_percent() {
        let id = Id::new("price-alert-percent");
        value_field(ui, id, "By", |ui, _| {
            let width = ui.available_width() - 24.0;
            let response =
                kit::plain_field(ui, id, &mut state.draft.percent, "0", 15.0, SEMIBOLD, width);
            if focus {
                response.request_focus();
            }
            ui.label(kit::text("%", 15.0, kit::REGULAR, theme::text_secondary()));
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for option in PERCENT_OPTIONS {
                let text = option.to_string();
                if kit::value_chip(ui, &format!("{option}%"), state.draft.percent == text).clicked()
                {
                    state.draft.percent = text;
                }
            }
        });
        if state.draft.reference().is_none() {
            ui.add_space(8.0);
            hint(
                ui,
                "A fresh current price is required for percentage alerts.",
                kit::ORANGE,
            );
        } else if let Some(level) = state
            .draft
            .condition()
            .and_then(|condition| condition.target())
        {
            ui.add_space(8.0);
            hint(
                ui,
                &format!("Fires at {}", state.draft.currency().format_price(level)),
                theme::text_secondary(),
            );
        }
        return;
    }

    let id = Id::new("price-alert-target");
    let mut currency = state.draft.currency();
    value_field(ui, id, "Target price", |ui, _| {
        ui.label(kit::text(
            currency.glyph(),
            15.0,
            kit::REGULAR,
            theme::text_secondary(),
        ));
        let width = ui.available_width() - 52.0;
        let response = kit::plain_field(
            ui,
            id,
            &mut state.draft.target,
            "0.00",
            15.0,
            SEMIBOLD,
            width,
        );
        if focus {
            response.request_focus();
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            currency_menu(ui, &mut currency);
        });
    });
    state.draft.set_currency(currency);
    let offsets = state.draft.offset_options();
    if !offsets.is_empty() {
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for offset in offsets {
                if kit::value_chip(
                    ui,
                    &offset.label(),
                    state.draft.target == offset.target_text,
                )
                .clicked()
                {
                    state.draft.target = offset.target_text;
                }
            }
        });
    }
    if let Some((text, tone)) = state.draft.target_hint() {
        ui.add_space(8.0);
        hint(
            ui,
            &text,
            match tone {
                HintTone::Warning => kit::ORANGE,
                HintTone::Neutral => theme::text_secondary(),
            },
        );
    }
}

/// The currency the alert is checked in, as a borderless menu at the end of the price field.
fn currency_menu(ui: &mut egui::Ui, currency: &mut AlertCurrency) {
    let label = kit::galley(
        ui,
        currency.code(),
        size::BODY - 0.5,
        SEMIBOLD,
        theme::text_primary(),
        60.0,
    );
    let (rect, response) =
        ui.allocate_exact_size(label.size() + Vec2::new(22.0, 6.0), Sense::click());
    let response = crate::ui::accessibility::label(response, "Currency")
        .on_hover_text("Currency the alert is checked in")
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.hovered() {
        ui.painter().rect_filled(rect, 5.0, kit::primary(0.08));
    }
    ui.painter().galley(
        Pos2::new(rect.left() + 4.0, rect.center().y - label.size().y / 2.0),
        label,
        theme::text_primary(),
    );
    icons::paint(
        ui.painter(),
        Rect::from_center_size(
            Pos2::new(rect.right() - 7.0, rect.center().y),
            Vec2::splat(8.0),
        ),
        Icon::ChevronUpDown,
        theme::text_secondary(),
    );
    let labels: Vec<String> = AlertCurrency::ALL
        .into_iter()
        .map(|option| format!("{} \u{2014} {}", option.code(), option.display_name()))
        .collect();
    kit::fitted_menu(ui, &response, egui::RectAlign::BOTTOM_END, &labels, |ui| {
        for (option, label) in AlertCurrency::ALL.into_iter().zip(&labels) {
            let icon = if option == *currency {
                Icon::Check
            } else {
                Icon::Circle
            };
            // Only the chosen one carries a mark.
            if kit::menu_item_marked(ui, icon, label, option == *currency) {
                *currency = option;
                ui.close();
            }
        }
    });
}

fn footer(
    ui: &mut egui::Ui,
    state: &PriceAlertEditorState,
    now_ms: i64,
) -> Option<PriceAlertEditorOutcome> {
    let mut outcome = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        let summary = state
            .draft
            .summary()
            .unwrap_or_else(|| "Set a level to create the alert.".to_owned());
        let width = ui.available_width() - 76.0 - 120.0 - 24.0;
        ui.allocate_ui_with_layout(
            Vec2::new(width, 30.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.set_width(width);
                ui.add(
                    egui::Label::new(kit::text(
                        summary,
                        size::CAPTION + 0.5,
                        kit::REGULAR,
                        theme::text_secondary(),
                    ))
                    .wrap(),
                );
            },
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            let label = if state.draft.is_new() {
                "Create Alert"
            } else {
                "Save Changes"
            };
            if kit::sheet_button(ui, label, true, state.draft.can_save()).clicked() {
                outcome = state
                    .draft
                    .result(now_ms)
                    .map(|alert| PriceAlertEditorOutcome::Save(Box::new(alert)));
            }
            if kit::sheet_button(ui, "Cancel", false, true).clicked() {
                outcome = Some(PriceAlertEditorOutcome::Cancel);
            }
        });
    });
    outcome
}

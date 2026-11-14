//! Create or edit one price alert: what to watch for, at what level, how often, a note
//! and the webhooks it posts to. A pure view over a `PriceAlertEditorState`; all the
//! parsing, hints and summary live on the state so they are testable and the host (which
//! owns the quote layer and the alert store) only feeds prices in and takes the alert out.

use crate::model::alert::message::AlertMessageContext;
use crate::model::alert::{
    AlertAsset, AlertCondition, AlertCurrency, AlertFrequency, AlertState, PriceAlert,
};
use crate::model::watchlist::quote::{Freshness, WatchlistQuote};
use crate::model::webhook::{EndpointId, WebhookEndpoint};
use crate::service::watchlist_quotes::WatchlistQuoteBook;
use crate::ui::icons::{self, Icon};
use crate::ui::kit::{self, SEMIBOLD, size};
use crate::ui::{theme, webhook::picker};
use eframe::egui::{self, Color32, Id, Pos2, Rect, Sense, Stroke, Vec2};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Direction {
    Above,
    Below,
    Rises,
    Falls,
}

impl Direction {
    pub const ALL: [Self; 4] = [Self::Above, Self::Below, Self::Rises, Self::Falls];

    pub fn title(self) -> &'static str {
        match self {
            Self::Above => "Above",
            Self::Below => "Below",
            Self::Rises => "Rises",
            Self::Falls => "Falls",
        }
    }

    pub fn is_percent(self) -> bool {
        matches!(self, Self::Rises | Self::Falls)
    }

    pub fn blurb(self) -> &'static str {
        match self {
            Self::Above => "Notify me when the price crosses above a level.",
            Self::Below => "Notify me when the price crosses below a level.",
            Self::Rises => "Notify me when the price climbs by a percentage from now.",
            Self::Falls => "Notify me when the price drops by a percentage from now.",
        }
    }
}

/// A quick target a few percent from the current price.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TargetOffset {
    pub percent: i32,
    pub target_text: String,
}

impl TargetOffset {
    /// "+5%" / "−5%".
    pub fn label(&self) -> String {
        format!(
            "{}{}%",
            if self.percent > 0 { "+" } else { "−" },
            self.percent.abs()
        )
    }
}

pub const PERCENT_OPTIONS: [u32; 5] = [1, 2, 5, 10, 20];

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HintTone {
    Warning,
    Neutral,
}

#[derive(Clone, Debug)]
pub struct PriceAlertEditorState {
    asset: AlertAsset,
    existing: Option<PriceAlert>,
    pub direction: Direction,
    pub target: String,
    pub percent: String,
    currency: AlertCurrency,
    pub frequency: AlertFrequency,
    pub note: String,
    pub sends_to_webhooks: bool,
    pub webhook_ids: Vec<EndpointId>,
    pub webhook_message: String,
    /// The price a percentage alert counts from; fixed when the sheet opens (or kept from
    /// the saved alert).
    reference: Option<f64>,
    /// The market now, in the chosen currency; the host refreshes it while the sheet is open.
    current_price: Option<f64>,
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
        let mut state = Self {
            asset,
            direction: Direction::Above,
            target: String::new(),
            percent: String::new(),
            currency: default_currency.unwrap_or(AlertCurrency::Usd),
            frequency: AlertFrequency::Once,
            note: String::new(),
            sends_to_webhooks: false,
            webhook_ids: Vec::new(),
            webhook_message: String::new(),
            reference: None,
            current_price: None,
            identical: false,
            focus_value: true,
            existing: None,
        };
        if let Some(alert) = existing {
            state.currency = alert.currency;
            state.frequency = alert.frequency;
            state.note = alert.note.clone();
            state.sends_to_webhooks = !alert.webhook_endpoint_ids.is_empty();
            state.webhook_ids = alert.webhook_endpoint_ids.clone();
            state.webhook_message = alert.webhook_message.clone().unwrap_or_default();
            match &alert.condition {
                AlertCondition::CrossesAbove { target } => state.target = plain(*target),
                AlertCondition::CrossesBelow { target } => {
                    state.direction = Direction::Below;
                    state.target = plain(*target);
                }
                AlertCondition::RisesBy {
                    percent, reference, ..
                } => {
                    state.direction = Direction::Rises;
                    state.percent = plain(*percent);
                    state.reference = Some(*reference);
                }
                AlertCondition::FallsBy {
                    percent, reference, ..
                } => {
                    state.direction = Direction::Falls;
                    state.percent = plain(*percent);
                    state.reference = Some(*reference);
                }
                AlertCondition::Unsupported { .. } => {}
            }
            state.existing = Some(alert);
        }
        state
    }

    pub fn is_new(&self) -> bool {
        self.existing.is_none()
    }

    pub fn asset(&self) -> &AlertAsset {
        &self.asset
    }

    #[cfg(test)]
    pub fn currency(&self) -> AlertCurrency {
        self.currency
    }

    pub fn current_price(&self) -> Option<f64> {
        self.current_price
    }

    #[cfg(test)]
    pub fn reference(&self) -> Option<f64> {
        self.reference
    }

    /// Picks the currency. The price and the percentage reference were in the old one, so both
    /// are dropped until the host feeds a price in the new one.
    pub fn set_currency(&mut self, currency: AlertCurrency) {
        if currency != self.currency {
            self.currency = currency;
            self.current_price = None;
            self.reference = None;
        }
    }

    /// Feeds the market price (the host calls this every frame while the sheet is open). A
    /// percentage alert counts from the first price it sees, and again after a currency change.
    pub fn set_current_price(&mut self, price: Option<f64>) {
        if let Some(price) = price {
            self.current_price = Some(price);
            if self.reference.is_none() {
                self.reference = Some(price);
            }
        }
    }

    /// Reads the market from the shared quote book. A price that is not live, or a currency with
    /// no FX rate yet, leaves the sheet without one rather than showing a wrong number.
    pub fn feed_from_book(
        &mut self,
        book: &WatchlistQuoteBook,
        rates: &HashMap<AlertCurrency, f64>,
    ) {
        let quote = book.get_by_key(&self.asset.key());
        self.set_current_price(converted_price(quote.as_ref(), self.currency, rates));
    }

    /// The chosen currency is not the quotes' own, so the host needs FX rates.
    pub fn needs_fx_rates(&self) -> bool {
        self.currency != AlertCurrency::Usd
    }

    // MARK: Parsing and hints

    fn parse_positive(text: &str) -> Option<f64> {
        text.trim()
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite() && *value > 0.0)
    }

    pub fn parsed_target(&self) -> Option<f64> {
        Self::parse_positive(&self.target)
    }

    pub fn parsed_percent(&self) -> Option<f64> {
        Self::parse_positive(&self.percent)
    }

    /// Targets a few percent away from the current price, on the side the alert watches.
    pub fn offset_options(&self) -> Vec<TargetOffset> {
        let Some(price) = self.current_price else {
            return Vec::new();
        };
        let sign = if self.direction == Direction::Below {
            -1
        } else {
            1
        };
        [1, 2, 5, 10]
            .into_iter()
            .map(|step| {
                let percent = step * sign;
                let level = price * (1.0 + f64::from(percent) / 100.0);
                TargetOffset {
                    percent,
                    target_text: self.currency.plain_price_text(level),
                }
            })
            .collect()
    }

    /// How the typed level sits against the market, and a warning when the alert could not
    /// fire soon.
    pub fn target_hint(&self) -> Option<(String, HintTone)> {
        let level = self.parsed_target()?;
        let price = self.current_price.filter(|price| *price > 0.0)?;
        let change = (level - price) / price * 100.0;
        let signed = format!(
            "{}{:.1}%",
            if change >= 0.0 { "+" } else { "−" },
            change.abs()
        );
        if self.direction == Direction::Above && level <= price {
            return Some((
                format!(
                    "{signed} from now. The price is already above this level; it fires after dropping below and crossing back up."
                ),
                HintTone::Warning,
            ));
        }
        if self.direction == Direction::Below && level >= price {
            return Some((
                format!(
                    "{signed} from now. The price is already below this level; it fires after rising above and crossing back down."
                ),
                HintTone::Warning,
            ));
        }
        Some((
            format!("{signed} from the current price"),
            HintTone::Neutral,
        ))
    }

    /// The rule being built, or `None` while the sheet is not complete.
    pub fn condition(&self) -> Option<AlertCondition> {
        match self.direction {
            Direction::Above => self
                .parsed_target()
                .map(|target| AlertCondition::CrossesAbove { target }),
            Direction::Below => self
                .parsed_target()
                .map(|target| AlertCondition::CrossesBelow { target }),
            Direction::Rises => {
                let (percent, reference) = (self.parsed_percent()?, self.reference?);
                Some(AlertCondition::rises_by(percent, reference))
            }
            Direction::Falls => {
                let (percent, reference) = (self.parsed_percent()?, self.reference?);
                Some(AlertCondition::falls_by(percent, reference))
            }
        }
    }

    pub fn can_save(&self) -> bool {
        self.condition().is_some()
    }

    /// "BTC crosses above $70,000.00 · once", shown beside the Save button.
    pub fn summary(&self) -> Option<String> {
        let condition = self.condition()?;
        let ticker = &self.asset.symbol;
        let price = |value: f64| self.currency.format_price(value);
        let what = match &condition {
            AlertCondition::CrossesAbove { target } => {
                format!("{ticker} crosses above {}", price(*target))
            }
            AlertCondition::CrossesBelow { target } => {
                format!("{ticker} crosses below {}", price(*target))
            }
            AlertCondition::RisesBy {
                percent, target, ..
            } => {
                format!("{ticker} rises {}% to {}", plain(*percent), price(*target))
            }
            AlertCondition::FallsBy {
                percent, target, ..
            } => {
                format!("{ticker} falls {}% to {}", plain(*percent), price(*target))
            }
            AlertCondition::Unsupported { .. } => return None,
        };
        Some(format!(
            "{what} · {}",
            if self.frequency == AlertFrequency::Once {
                "once"
            } else {
                "every time"
            }
        ))
    }

    /// The saved form: the existing alert updated in place (id, creation time kept), or a
    /// new one; always active. Webhooks off means none, whatever was picked before.
    pub fn result(&self, now_ms: i64) -> Option<PriceAlert> {
        let condition = self.condition()?;
        let mut alert = self
            .existing
            .clone()
            .unwrap_or_else(|| PriceAlert::new(self.asset.clone(), condition.clone(), now_ms));
        alert.condition = condition;
        alert.currency = self.currency;
        alert.frequency = self.frequency;
        alert.note = self.note.clone();
        alert.webhook_endpoint_ids = if self.sends_to_webhooks {
            self.webhook_ids.clone()
        } else {
            Vec::new()
        };
        let message = self.webhook_message.trim();
        alert.webhook_message = (!message.is_empty()).then(|| self.webhook_message.clone());
        alert.state = AlertState::Active;
        Some(alert)
    }

    /// Example values for the webhook message preview: what this alert would send for its
    /// own market.
    pub fn example_context(&self, now_ms: i64) -> AlertMessageContext {
        AlertMessageContext {
            ticker: Some(self.asset.instrument.api_symbol()),
            exchange: Some(self.asset.source().display_name().to_owned()),
            interval: Some("1".to_owned()),
            // A converted quote carries float noise (`83356.00999999998`); show what a person
            // would read, as the alert states prices.
            close: self
                .current_price
                .and_then(|price| self.currency.plain_price_text(price).parse().ok()),
            time_ms: Some(now_ms),
            now_ms,
            ..AlertMessageContext::default()
        }
    }
}

/// A plain, locale-independent number: the fields are parsed with `str::parse`.
fn plain(value: f64) -> String {
    format!("{value}")
}

// MARK: View

/// Quote-demand owner for a sheet open in tab `tab`, clear of the tab ids the watchlist
/// sidebars use and of the alert runtime's own owner.
pub fn quote_owner(tab: u64) -> u64 {
    (1 << 62) + tab
}

/// Quote-demand owner for the sheet the Alerts window hosts (Edit).
pub const ALERTS_WINDOW_QUOTE_OWNER: u64 = u64::MAX - 2;

/// The live USD quote in `currency` (`rates` are USD → currency), or `None` while it is stale,
/// missing, or the rate is unknown.
pub fn converted_price(
    quote: Option<&WatchlistQuote>,
    currency: AlertCurrency,
    rates: &HashMap<AlertCurrency, f64>,
) -> Option<f64> {
    let quote = quote.filter(|quote| quote.freshness == Freshness::Live)?;
    let usd = quote
        .last
        .filter(|price| price.is_finite() && *price > 0.0)?;
    if currency == AlertCurrency::Usd {
        return Some(usd);
    }
    rates
        .get(&currency)
        .copied()
        .filter(|rate| rate.is_finite() && *rate > 0.0)
        .map(|rate| usd * rate)
}

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

impl Direction {
    fn icon(self) -> Icon {
        match self {
            Self::Above => Icon::ArrowUpRight,
            Self::Below => Icon::ArrowDownRight,
            Self::Rises => Icon::ArrowUp,
            Self::Falls => Icon::ArrowDown,
        }
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
    } else if enter && !typing_message && outcome.is_none() && state.can_save() {
        outcome = state
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
            icon: direction.icon(),
            count: 0,
        })
        .collect();
    let before = state.direction;
    kit::tab_bar(
        ui,
        Id::new("price-alert-direction"),
        &items,
        &mut state.direction,
    );
    if state.direction != before {
        state.focus_value = true;
    }
    ui.add_space(8.0);
    hint(ui, state.direction.blurb(), theme::text_secondary());
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
                &mut state.frequency,
            );
        });
    })
    .response
    .on_hover_text(if state.frequency == AlertFrequency::Once {
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
                &mut state.note,
                "Add a note (optional)",
                size::BODY,
                kit::REGULAR,
                width,
            );
        });
    });
    ui.add_space(18.0);

    let context = state.example_context(now_ms);
    let mut message = std::mem::take(&mut state.webhook_message);
    let open_settings = picker::send_section(
        ui,
        Id::new("price-alert-webhooks"),
        &mut state.sends_to_webhooks,
        &mut state.webhook_ids,
        endpoints,
        Some((&mut message, &context)),
    );
    state.webhook_message = message;

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
    kit::ticker_icon(ui, icon, &state.asset.symbol, state.asset.source());
    kit::source_badge(ui, icon, state.asset.source());
    let left = icon.right() + 3.0 + 14.0;
    let width = row.right() - left;
    let title = kit::galley(
        ui,
        if state.is_new() {
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
            state.asset.name,
            state.asset.source().display_name()
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
        &state.current_price.map_or_else(
            || "—".to_owned(),
            |price| state.currency.format_price(price),
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
    let (label, tone) = if state.current_price.is_some() {
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
    if state.direction.is_percent() {
        let id = Id::new("price-alert-percent");
        value_field(ui, id, "By", |ui, _| {
            let width = ui.available_width() - 24.0;
            let response = kit::plain_field(ui, id, &mut state.percent, "0", 15.0, SEMIBOLD, width);
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
                if kit::value_chip(ui, &format!("{option}%"), state.percent == text).clicked() {
                    state.percent = text;
                }
            }
        });
        if state.reference.is_none() {
            ui.add_space(8.0);
            hint(
                ui,
                "A fresh current price is required for percentage alerts.",
                kit::ORANGE,
            );
        } else if let Some(level) = state.condition().and_then(|condition| condition.target()) {
            ui.add_space(8.0);
            hint(
                ui,
                &format!("Fires at {}", state.currency.format_price(level)),
                theme::text_secondary(),
            );
        }
        return;
    }

    let id = Id::new("price-alert-target");
    let mut currency = state.currency;
    value_field(ui, id, "Target price", |ui, _| {
        ui.label(kit::text(
            currency.glyph(),
            15.0,
            kit::REGULAR,
            theme::text_secondary(),
        ));
        let width = ui.available_width() - 52.0;
        let response = kit::plain_field(ui, id, &mut state.target, "0.00", 15.0, SEMIBOLD, width);
        if focus {
            response.request_focus();
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            currency_menu(ui, &mut currency);
        });
    });
    state.set_currency(currency);
    let offsets = state.offset_options();
    if !offsets.is_empty() {
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for offset in offsets {
                if kit::value_chip(ui, &offset.label(), state.target == offset.target_text)
                    .clicked()
                {
                    state.target = offset.target_text;
                }
            }
        });
    }
    if let Some((text, tone)) = state.target_hint() {
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
            let label = if state.is_new() {
                "Create Alert"
            } else {
                "Save Changes"
            };
            if kit::sheet_button(ui, label, true, state.can_save()).clicked() {
                outcome = state
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::alert::message::{DEFAULT_PRICE_ALERT_TEMPLATE, render};
    use crate::model::{DataSource, InstrumentId};

    fn asset() -> AlertAsset {
        AlertAsset::new(
            InstrumentId::new(DataSource::Binance, "BTCUSDT"),
            "BTC",
            "Bitcoin",
        )
    }

    fn state() -> PriceAlertEditorState {
        PriceAlertEditorState::new(asset(), None, None)
    }

    #[test]
    fn an_empty_sheet_cannot_be_saved() {
        let state = state();
        assert!(state.is_new() && !state.can_save());
        assert_eq!(state.summary(), None);
        assert_eq!(state.current_price(), None);
    }

    #[test]
    fn targets_parse_as_positive_finite_numbers() {
        let mut state = state();
        for (text, expected) in [
            ("70000", Some(70_000.0)),
            ("  0.5 ", Some(0.5)),
            ("1e5", Some(100_000.0)),
            ("0", None),
            ("-3", None),
            ("abc", None),
            ("", None),
            ("NaN", None),
            ("inf", None),
        ] {
            state.target = text.into();
            assert_eq!(state.parsed_target(), expected, "{text:?}");
        }
    }

    #[test]
    fn a_price_level_builds_a_crossing_condition_and_a_summary() {
        let mut state = state();
        state.target = "70000".into();
        assert_eq!(
            state.condition(),
            Some(AlertCondition::CrossesAbove { target: 70_000.0 })
        );
        assert_eq!(
            state.summary().as_deref(),
            Some("BTC crosses above $70,000.00 · once")
        );
        state.direction = Direction::Below;
        state.frequency = AlertFrequency::EveryTime;
        assert_eq!(
            state.summary().as_deref(),
            Some("BTC crosses below $70,000.00 · every time")
        );
    }

    #[test]
    fn percentage_alerts_need_a_reference_price() {
        let mut state = state();
        state.direction = Direction::Rises;
        state.percent = "10".into();
        assert!(!state.can_save(), "no price yet, nothing to count from");
        state.set_current_price(Some(100.0));
        assert_eq!(state.reference(), Some(100.0));
        let condition = state.condition().unwrap();
        assert!((condition.target().unwrap() - 110.0).abs() < 1e-9);
        assert_eq!(
            state.summary().as_deref(),
            Some("BTC rises 10% to $110.00 · once")
        );
        // The reference is fixed when first seen; later prices do not move it.
        state.set_current_price(Some(200.0));
        assert_eq!(state.reference(), Some(100.0));
        assert_eq!(state.current_price(), Some(200.0));
        state.direction = Direction::Falls;
        state.percent = "25".into();
        assert!((state.condition().unwrap().target().unwrap() - 75.0).abs() < 1e-9);
        assert_eq!(
            state.summary().as_deref(),
            Some("BTC falls 25% to $75.00 · once")
        );
    }

    #[test]
    fn a_saved_alert_reopens_exactly_as_it_was_made() {
        let mut saved = PriceAlert::new(asset(), AlertCondition::rises_by(5.0, 80.0), 7);
        saved.currency = AlertCurrency::Eur;
        saved.frequency = AlertFrequency::EveryTime;
        saved.note = "swing".into();
        saved.webhook_endpoint_ids = vec![3, 4];
        saved.webhook_message = Some("{{ticker}} up".into());
        let state = PriceAlertEditorState::new(asset(), Some(saved.clone()), None);
        assert!(!state.is_new());
        assert_eq!(state.direction, Direction::Rises);
        assert_eq!(state.percent, "5");
        assert_eq!(
            state.reference(),
            Some(80.0),
            "a saved percentage alert keeps its price"
        );
        assert_eq!(state.currency(), AlertCurrency::Eur);
        assert!(state.sends_to_webhooks);
        assert_eq!(state.webhook_ids, vec![3, 4]);
        let result = state.result(99).unwrap();
        assert_eq!(result.id, saved.id);
        assert_eq!(result.created_at_ms, 7, "editing keeps the creation time");
        assert_eq!(result.condition, saved.condition);
        assert_eq!(result.note, "swing");
        assert_eq!(
            result.state,
            AlertState::Active,
            "saving re-enables a triggered alert"
        );
    }

    #[test]
    fn webhooks_off_means_none_even_if_some_were_picked() {
        let mut state = state();
        state.target = "100".into();
        state.sends_to_webhooks = true;
        state.webhook_ids = vec![6];
        state.webhook_message = "  hello  ".into();
        let on = state.result(1).unwrap();
        assert_eq!(on.webhook_endpoint_ids, vec![6]);
        assert_eq!(
            on.webhook_message.as_deref(),
            Some("  hello  "),
            "kept as typed"
        );
        state.sends_to_webhooks = false;
        assert!(state.result(1).unwrap().webhook_endpoint_ids.is_empty());
        state.sends_to_webhooks = true;
        state.webhook_message = "   ".into();
        assert_eq!(state.result(1).unwrap().webhook_message, None);
    }

    #[test]
    fn quick_targets_sit_on_the_side_the_alert_watches() {
        let mut state = state();
        assert!(
            state.offset_options().is_empty(),
            "no price, no suggestions"
        );
        state.set_current_price(Some(1_000.0));
        let up = state.offset_options();
        assert_eq!(
            up.iter().map(TargetOffset::label).collect::<Vec<_>>(),
            ["+1%", "+2%", "+5%", "+10%"]
        );
        assert_eq!(up[0].target_text, "1010.00");
        assert_eq!(up[3].target_text, "1100.00");
        state.direction = Direction::Below;
        let down = state.offset_options();
        assert_eq!(down[0].label(), "−1%");
        assert_eq!(down[0].target_text, "990.00");
        // Every suggestion parses back as a usable target.
        state.target = down[2].target_text.clone();
        assert!((state.parsed_target().unwrap() - 950.0).abs() < 1e-9);
    }

    #[test]
    fn the_hint_warns_when_the_price_is_already_past_the_level() {
        let mut state = state();
        state.set_current_price(Some(100.0));
        state.target = "120".into();
        let (text, tone) = state.target_hint().unwrap();
        assert_eq!(tone, HintTone::Neutral);
        assert_eq!(text, "+20.0% from the current price");

        state.target = "90".into();
        let (text, tone) = state.target_hint().unwrap();
        assert_eq!(tone, HintTone::Warning);
        assert!(text.starts_with("−10.0% from now. The price is already above this level"));

        state.direction = Direction::Below;
        state.target = "110".into();
        let (text, tone) = state.target_hint().unwrap();
        assert_eq!(tone, HintTone::Warning);
        assert!(text.contains("already below this level"));

        state.target = "".into();
        assert_eq!(state.target_hint(), None);
        state.target = "90".into();
        state.set_current_price(None);
        state.current_price = None;
        assert_eq!(state.target_hint(), None, "no price, no hint");
    }

    #[test]
    fn changing_currency_drops_the_old_price_and_reference() {
        let mut state = state();
        state.set_current_price(Some(100.0));
        state.set_currency(AlertCurrency::Usd);
        assert_eq!(state.current_price(), Some(100.0), "same currency, kept");
        state.set_currency(AlertCurrency::Eur);
        assert_eq!(state.current_price(), None);
        assert_eq!(state.reference(), None);
        state.set_current_price(Some(92.0));
        assert_eq!(state.reference(), Some(92.0));
    }

    fn live(last: f64) -> WatchlistQuote {
        WatchlistQuote {
            last: Some(last),
            freshness: Freshness::Live,
            ..WatchlistQuote::default()
        }
    }

    #[test]
    fn the_price_is_the_live_quote_in_the_chosen_currency() {
        let rates = HashMap::from([(AlertCurrency::Eur, 0.9)]);
        let quote = live(100.0);
        assert_eq!(
            converted_price(Some(&quote), AlertCurrency::Usd, &rates),
            Some(100.0)
        );
        assert_eq!(
            converted_price(Some(&quote), AlertCurrency::Eur, &rates),
            Some(90.0)
        );
        assert_eq!(
            converted_price(Some(&quote), AlertCurrency::Gbp, &rates),
            None,
            "no rate yet, no price"
        );
        let stale = WatchlistQuote {
            freshness: Freshness::Stale,
            ..live(100.0)
        };
        assert_eq!(
            converted_price(Some(&stale), AlertCurrency::Usd, &rates),
            None
        );
        assert_eq!(converted_price(None, AlertCurrency::Usd, &rates), None);
    }

    #[test]
    fn the_default_currency_is_the_assets_own_when_alertable() {
        let eur = PriceAlertEditorState::new(asset(), None, Some(AlertCurrency::Eur));
        assert_eq!(eur.currency(), AlertCurrency::Eur);
        assert_eq!(state().currency(), AlertCurrency::Usd);
    }

    #[test]
    fn the_webhook_preview_renders_with_this_alerts_own_market() {
        let mut state = state();
        state.set_current_price(Some(67_432.5));
        let context = state.example_context(1_714_571_130_000);
        assert_eq!(
            render(DEFAULT_PRICE_ALERT_TEMPLATE, &context),
            "BTCUSDT alert: 67432.5"
        );
        assert_eq!(
            render(
                "{{exchange}} {{ticker}} {{interval}} {{time}} {{open}}",
                &context
            ),
            "Binance BTCUSDT 1 2024-05-01T13:45:30Z {{open}}",
            "a value the alert does not have stays a placeholder"
        );
    }

    #[test]
    fn directions_describe_themselves() {
        assert!(Direction::Rises.is_percent() && !Direction::Above.is_percent());
        assert_eq!(Direction::Below.title(), "Below");
        assert!(Direction::Falls.blurb().contains("drops by a percentage"));
    }
}

//! "Send to webhooks" in an alert editor (Swift `WebhookSendSection`, `WebhookEndpointPicker`,
//! `WebhookMessageEditor`): a toggle card that reveals selectable endpoint cards and, for price
//! alerts, the message with one-tap placeholders and an example of what is sent.

use crate::model::alert_message::{
    AlertMessageContext, DEFAULT_PRICE_ALERT_TEMPLATE, SUPPORTED_PLACEHOLDERS, render,
};
use crate::model::webhook::{EndpointId, WebhookEndpoint};
use crate::model::webhook_policy;
use crate::ui::icons::{self, Icon};
use crate::ui::kit::{self, MEDIUM, REGULAR, SEMIBOLD, size};
use crate::ui::theme;
use eframe::egui::{self, Color32, Id, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};

const ROW_HEIGHT: f32 = 48.0;
/// Endpoints shown before the list scrolls.
const VISIBLE_ROWS: usize = 4;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Summary {
    /// Selected endpoints that still exist.
    pub selected: usize,
    /// Selected ids whose endpoint was deleted; delivery skips them.
    pub dangling: usize,
    /// At least one endpoint is selected and every one is switched off in Settings.
    pub all_paused: bool,
}

pub fn summary(selection: &[EndpointId], endpoints: &[WebhookEndpoint]) -> Summary {
    let known: Vec<&WebhookEndpoint> = selection
        .iter()
        .filter_map(|id| endpoints.iter().find(|endpoint| endpoint.id == *id))
        .collect();
    Summary {
        selected: known.len(),
        dangling: selection.len() - known.len(),
        all_paused: !known.is_empty() && known.iter().all(|endpoint| !endpoint.is_enabled),
    }
}

/// The host (and port) of a webhook URL and nothing else, so never anything that could hold a
/// token. Works on templates that contain `{{secret}}`.
pub fn host(url: &str) -> Option<String> {
    let neutral = webhook_policy::replace_secret(url.trim(), "x");
    let parsed = webhook_policy::parse_url(&neutral)?;
    if parsed.host.is_empty() {
        return None;
    }
    Some(match parsed.port {
        Some(port) => format!("{}:{port}", parsed.host),
        None => parsed.host,
    })
}

/// `POST · example.com`, or what is wrong with the endpoint.
pub fn subtitle(endpoint: &WebhookEndpoint) -> String {
    if !endpoint.is_enabled {
        return "Paused in Settings".to_owned();
    }
    match host(&endpoint.url) {
        Some(host) => format!("{} · {host}", endpoint.method.as_str()),
        None => endpoint.method.as_str().to_owned(),
    }
}

/// Adds `token` after what is typed, with a single space between when one is needed.
pub fn appending(token: &str, text: &str) -> String {
    match text.chars().last() {
        None => token.to_owned(),
        Some(last) if last.is_whitespace() => format!("{text}{token}"),
        Some(_) => format!("{text} {token}"),
    }
}

/// The message as the alert would send it, from example values, and whether it goes as JSON.
pub fn example_preview(template: &str, context: &AlertMessageContext) -> (String, bool) {
    let source = if template.trim().is_empty() {
        DEFAULT_PRICE_ALERT_TEMPLATE
    } else {
        template
    };
    let text = render(source, context);
    let json = webhook_policy::is_json(&text);
    (text, json)
}

/// The toggle card, then (when on) the endpoint cards and the message editor. `message` is
/// `None` for script alerts, whose message comes from the script. Returns whether "Add in
/// Settings" was clicked.
pub fn send_section(
    ui: &mut Ui,
    id: Id,
    is_on: &mut bool,
    selection: &mut Vec<EndpointId>,
    endpoints: &[WebhookEndpoint],
    message: Option<(&mut String, &AlertMessageContext)>,
) -> bool {
    let picked = summary(selection, endpoints);
    let subtitle = if !*is_on {
        "Also post this alert to your own server or bot.".to_owned()
    } else if picked.selected > 0 {
        format!("{} selected", picked.selected)
    } else if endpoints.is_empty() {
        "Add a webhook to choose where to post.".to_owned()
    } else {
        "Choose where to post.".to_owned()
    };
    toggle_card(ui, id, is_on, &subtitle, *is_on && picked.selected > 0);
    if !*is_on {
        return false;
    }
    let mut open_settings = false;
    ui.add_space(12.0);
    if endpoints.is_empty() {
        open_settings = empty_state(ui);
    } else {
        endpoint_list(ui, id, selection, endpoints);
    }
    if picked.dangling > 0 {
        ui.add_space(10.0);
        warning(
            ui,
            &format!(
                "{} selected webhook{} deleted and will be skipped.",
                picked.dangling,
                if picked.dangling == 1 {
                    " was"
                } else {
                    "s were"
                }
            ),
        );
    }
    if picked.all_paused {
        ui.add_space(10.0);
        warning(
            ui,
            "Every selected webhook is turned off in Settings, so nothing will be sent.",
        );
    }
    if let Some((message, context)) = message
        && picked.selected > 0
    {
        ui.add_space(14.0);
        message_editor(ui, id.with("message"), message, context);
    }
    open_settings
}

fn badge(ui: &Ui, rect: Rect, on: bool) {
    let tint = if on {
        theme::text_primary()
    } else {
        theme::text_secondary()
    };
    ui.painter()
        .rect_filled(rect, 8.0, theme::with_alpha(tint, 0.14));
    icons::paint(
        ui.painter(),
        Rect::from_center_size(rect.center(), Vec2::splat(13.0)),
        Icon::ArrowUpRightSquare,
        tint,
    );
}

fn toggle_card(ui: &mut Ui, id: Id, is_on: &mut bool, subtitle: &str, highlight: bool) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 50.0), Sense::hover());
    let response = ui
        .interact(rect, id.with("card"), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        12.0,
        theme::quaternary(if response.hovered() { 0.45 } else { 0.3 }),
    );
    painter.rect_stroke(
        rect,
        12.0,
        Stroke::new(1.0, kit::hairline()),
        StrokeKind::Inside,
    );
    let icon = Rect::from_min_size(
        pos2(rect.left() + 12.0, rect.center().y - 15.0),
        Vec2::splat(30.0),
    );
    badge(ui, icon, *is_on);
    let left = icon.right() + 12.0;
    let title = kit::galley(
        ui,
        "Send to webhooks",
        size::SUBHEADLINE + 1.0,
        SEMIBOLD,
        theme::text_primary(),
        rect.width(),
    );
    let detail = kit::galley(
        ui,
        subtitle,
        size::CAPTION + 0.5,
        REGULAR,
        if highlight {
            kit::ACCENT
        } else {
            theme::text_secondary()
        },
        rect.right() - 60.0 - left,
    );
    let top = rect.center().y - (title.size().y + 2.0 + detail.size().y) / 2.0;
    let title_height = title.size().y;
    ui.painter()
        .galley(pos2(left, top), title, theme::text_primary());
    ui.painter().galley(
        pos2(left, top + title_height + 2.0),
        detail,
        theme::text_secondary(),
    );
    let switch = Rect::from_min_size(
        pos2(rect.right() - 12.0 - 32.0, rect.center().y - 9.0),
        vec2(32.0, 18.0),
    );
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(switch));
    let flipped = kit::switch(&mut child, is_on, "Send to webhooks").clicked();
    if response.clicked() && !flipped {
        *is_on = !*is_on;
    }
}

fn empty_state(ui: &mut Ui) -> bool {
    let (rect, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT + 8.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 12.0, theme::quaternary(0.35));
    painter.rect_stroke(
        rect,
        12.0,
        Stroke::new(1.0, kit::hairline()),
        StrokeKind::Inside,
    );
    let icon = Rect::from_min_size(
        pos2(rect.left() + 12.0, rect.center().y - 15.0),
        Vec2::splat(30.0),
    );
    badge(ui, icon, false);
    let left = icon.right() + 12.0;
    let title = kit::galley(
        ui,
        "No webhooks yet",
        size::SUBHEADLINE + 1.0,
        SEMIBOLD,
        theme::text_primary(),
        260.0,
    );
    let detail = kit::galley(
        ui,
        "Send alerts to your own server or bot.",
        size::CAPTION + 0.5,
        REGULAR,
        theme::text_secondary(),
        260.0,
    );
    let top = rect.center().y - (title.size().y + 1.0 + detail.size().y) / 2.0;
    let title_height = title.size().y;
    ui.painter()
        .galley(pos2(left, top), title, theme::text_primary());
    ui.painter().galley(
        pos2(left, top + title_height + 1.0),
        detail,
        theme::text_secondary(),
    );
    let mut row = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(vec2(12.0, 0.0)))
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    kit::push_button(&mut row, None, "Add in Settings", true).clicked()
}

fn endpoint_list(
    ui: &mut Ui,
    id: Id,
    selection: &mut Vec<EndpointId>,
    endpoints: &[WebhookEndpoint],
) {
    let rows = |ui: &mut Ui, selection: &mut Vec<EndpointId>| {
        for (index, endpoint) in endpoints.iter().enumerate() {
            if index > 0 {
                ui.add_space(6.0);
            }
            let selected = selection.contains(&endpoint.id);
            if endpoint_row(ui, id.with(endpoint.id), endpoint, selected).clicked() {
                if selected {
                    selection.retain(|picked| *picked != endpoint.id);
                } else {
                    selection.push(endpoint.id);
                }
            }
        }
    };
    if endpoints.len() > VISIBLE_ROWS {
        let height = VISIBLE_ROWS as f32 * ROW_HEIGHT + (VISIBLE_ROWS - 1) as f32 * 6.0;
        egui::ScrollArea::vertical()
            .id_salt(id.with("endpoints"))
            .max_height(height)
            .auto_shrink([false, true])
            .show(ui, |ui| rows(ui, selection));
    } else {
        rows(ui, selection);
    }
}

fn endpoint_row(ui: &mut Ui, id: Id, endpoint: &WebhookEndpoint, selected: bool) -> egui::Response {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::hover());
    let response = ui
        .interact(rect, id, Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let response = crate::ui::accessibility::label(
        response,
        format!("{}, {}", endpoint.name, subtitle(endpoint)),
    );
    let response = if endpoint.is_enabled {
        response
    } else {
        response.on_hover_text("Turned off in Settings: skipped until you turn it back on")
    };
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
    if !endpoint.is_enabled {
        child.multiply_opacity(0.65);
    }
    let ui = &mut child;
    let fill = if selected {
        theme::selected_tint(false)
    } else {
        kit::primary(if response.hovered() { 0.06 } else { 0.025 })
    };
    let painter = ui.painter();
    painter.rect_filled(rect, 10.0, fill);
    painter.rect_stroke(
        rect,
        10.0,
        if selected {
            theme::selected_stroke()
        } else {
            Stroke::new(1.0, kit::primary(0.12))
        },
        StrokeKind::Inside,
    );
    let icon = Rect::from_min_size(
        pos2(rect.left() + 12.0, rect.center().y - 15.0),
        Vec2::splat(30.0),
    );
    badge(ui, icon, selected);

    let mut right = rect.right() - 12.0;
    let radio = Rect::from_min_size(pos2(right - 16.0, rect.center().y - 8.0), Vec2::splat(16.0));
    icons::paint(
        ui.painter(),
        radio,
        if selected {
            Icon::CheckCircleFill
        } else {
            Icon::Circle
        },
        if selected {
            theme::text_primary()
        } else {
            theme::with_alpha(theme::text_secondary(), 0.6)
        },
    );
    right = radio.left() - 8.0;
    if !endpoint.is_enabled {
        let off = kit::galley(
            ui,
            "Off",
            size::CAPTION,
            SEMIBOLD,
            theme::text_secondary(),
            40.0,
        );
        let pill = Rect::from_min_size(
            pos2(
                right - off.size().x - 14.0,
                rect.center().y - (off.size().y + 4.0) / 2.0,
            ),
            vec2(off.size().x + 14.0, off.size().y + 4.0),
        );
        ui.painter()
            .rect_filled(pill, pill.height() / 2.0, kit::primary(0.08));
        ui.painter().galley(
            pill.center() - off.size() / 2.0,
            off,
            theme::text_secondary(),
        );
        right = pill.left() - 8.0;
    }
    let left = icon.right() + 12.0;
    let name = kit::galley(
        ui,
        &endpoint.name,
        size::SUBHEADLINE + 1.0,
        SEMIBOLD,
        theme::text_primary(),
        right - left,
    );
    let detail = kit::galley(
        ui,
        &subtitle(endpoint),
        size::CAPTION + 0.5,
        REGULAR,
        theme::text_secondary(),
        right - left,
    );
    let top = rect.center().y - (name.size().y + 1.0 + detail.size().y) / 2.0;
    let name_height = name.size().y;
    ui.painter()
        .galley(pos2(left, top), name, theme::text_primary());
    ui.painter().galley(
        pos2(left, top + name_height + 1.0),
        detail,
        theme::text_secondary(),
    );
    response
}

fn warning(ui: &mut Ui, text: &str) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        icons::inline(ui, Icon::TriangleFill, 11.0, kit::ORANGE);
        ui.label(kit::text(text, size::CAPTION + 0.5, REGULAR, kit::ORANGE));
    });
}

fn message_editor(ui: &mut Ui, id: Id, text: &mut String, context: &AlertMessageContext) {
    ui.horizontal(|ui| {
        ui.label(kit::text(
            "Message",
            size::SUBHEADLINE + 1.0,
            SEMIBOLD,
            theme::text_primary(),
        ));
        if !text.is_empty() {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let reset = ui
                    .add(
                        egui::Label::new(kit::text(
                            "Use default",
                            size::CAPTION + 0.5,
                            REGULAR,
                            kit::ACCENT,
                        ))
                        .sense(Sense::click()),
                    )
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                if reset.clicked() {
                    text.clear();
                }
            });
        }
    });
    ui.add_space(8.0);
    let editor = id.with("text");
    let focused = ui.memory(|memory| memory.has_focus(editor));
    kit::field_frame(focused).show(ui, |ui| {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.add_space(0.0);
            icons::inline(ui, Icon::TextFormat, 13.0, theme::text_secondary());
            ui.add(
                egui::TextEdit::multiline(text)
                    .id(editor)
                    .frame(egui::Frame::NONE)
                    .font(egui::FontId::monospace(size::BODY - 1.0))
                    .text_color(theme::text_primary())
                    .hint_text(kit::text(
                        DEFAULT_PRICE_ALERT_TEMPLATE,
                        size::BODY - 1.0,
                        REGULAR,
                        theme::text_tertiary(),
                    ))
                    .desired_rows(1)
                    .desired_width(ui.available_width()),
            );
        });
    });
    ui.add_space(8.0);
    // Tap a placeholder to add it to the message.
    egui::ScrollArea::horizontal()
        .id_salt(id.with("placeholders"))
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                for name in SUPPORTED_PLACEHOLDERS {
                    let token = format!("{{{{{name}}}}}");
                    if placeholder_chip(ui, &token).clicked() {
                        *text = appending(&token, text);
                    }
                }
            });
        });
    ui.add_space(8.0);
    let (preview, json) = example_preview(text, context);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.label(kit::text(
            "Example",
            size::CAPTION + 0.5,
            MEDIUM,
            theme::text_tertiary(),
        ));
        let width = ui.available_width() - 44.0;
        ui.add_sized(
            [width, 14.0],
            egui::Label::new(
                egui::RichText::new(&preview)
                    .font(egui::FontId::monospace(size::CAPTION))
                    .color(theme::text_secondary()),
            )
            .truncate(),
        );
        let (tint, label, tip) = if json {
            (kit::GREEN, "JSON", "Sent as application/json")
        } else {
            (theme::text_secondary(), "Text", "Sent as text/plain")
        };
        let galley = kit::galley(ui, label, size::CAPTION, kit::BOLD, tint, 60.0);
        let (pill, response) =
            ui.allocate_exact_size(galley.size() + vec2(12.0, 4.0), Sense::hover());
        ui.painter()
            .rect_filled(pill, pill.height() / 2.0, theme::with_alpha(tint, 0.14));
        ui.painter()
            .galley(pill.center() - galley.size() / 2.0, galley, tint);
        response.on_hover_text(tip);
    });
    ui.add_space(6.0);
    ui.label(kit::text(
        "Sent as the request body. Leave empty for the default.",
        size::CAPTION + 0.5,
        REGULAR,
        theme::text_secondary(),
    ));
}

fn placeholder_chip(ui: &mut Ui, token: &str) -> egui::Response {
    let galley_for = |ui: &Ui, color: Color32| {
        ui.painter().layout_no_wrap(
            token.to_owned(),
            egui::FontId::monospace(size::CAPTION),
            color,
        )
    };
    let size = galley_for(ui, Color32::WHITE).size() + vec2(18.0, 8.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let response = response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(format!("Add {token} to the message"));
    let hovered = response.hovered();
    let color = if hovered {
        theme::text_primary()
    } else {
        theme::text_secondary()
    };
    ui.painter().rect_filled(
        rect,
        rect.height() / 2.0,
        if hovered {
            theme::selected_tint(true)
        } else {
            kit::primary(0.06)
        },
    );
    let galley = galley_for(ui, color);
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, color);
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::webhook::WebhookHttpMethod;

    fn endpoint(id: EndpointId, url: &str, enabled: bool) -> WebhookEndpoint {
        WebhookEndpoint {
            id,
            name: format!("Hook {id}"),
            url: url.into(),
            description: String::new(),
            method: WebhookHttpMethod::Post,
            headers: Vec::new(),
            is_enabled: enabled,
            created_at_ms: 0,
            updated_at_ms: 0,
        }
    }

    #[test]
    fn the_summary_counts_live_dangling_and_paused_selections() {
        let endpoints = [
            endpoint(1, "https://a.example", true),
            endpoint(2, "https://b.example", false),
        ];
        assert_eq!(
            summary(&[1, 9], &endpoints),
            Summary {
                selected: 1,
                dangling: 1,
                all_paused: false
            }
        );
        assert!(summary(&[2], &endpoints).all_paused);
        assert!(
            !summary(&[], &endpoints).all_paused,
            "nothing picked is not 'all paused'"
        );
    }

    #[test]
    fn subtitles_name_only_the_host_never_the_path_or_secret() {
        let hook = endpoint(1, "https://hooks.example.com:8443/t/{{secret}}?k=v", true);
        assert_eq!(subtitle(&hook), "POST · hooks.example.com:8443");
        assert_eq!(subtitle(&endpoint(2, "not a url", true)), "POST");
        assert_eq!(
            subtitle(&endpoint(3, "https://x.example", false)),
            "Paused in Settings"
        );
    }

    #[test]
    fn placeholders_are_appended_with_one_space() {
        assert_eq!(appending("{{close}}", ""), "{{close}}");
        assert_eq!(appending("{{close}}", "Price"), "Price {{close}}");
        assert_eq!(appending("{{close}}", "Price "), "Price {{close}}");
    }

    #[test]
    fn the_preview_falls_back_to_the_default_and_spots_json() {
        let context = AlertMessageContext {
            ticker: Some("BTCUSDT".into()),
            close: Some(1.5),
            ..AlertMessageContext::default()
        };
        assert_eq!(example_preview("  ", &context).0, "BTCUSDT alert: 1.5");
        let (text, json) = example_preview(r#"{"t":"{{ticker}}"}"#, &context);
        assert_eq!(text, r#"{"t":"BTCUSDT"}"#);
        assert!(json);
    }
}

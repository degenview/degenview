//! Sheet pieces shared by the symbol picker and the portfolio pickers: header, search box, cards,
//! buttons and the empty-result notice.

use super::art;
use super::artwork::{prediction_icon, result_icon};
use super::recents::RecentMarket;
use super::results::{result_row, same_market};
use super::text::{BOLD, MEDIUM, SEMIBOLD, galley, size, text};
use crate::model::DataSource;
use crate::service::SearchResult;
use crate::ui::icons::{self, Icon};
use crate::ui::kit::loading::LoadingIndicator;
use crate::ui::{kit::source_logo, theme};
use eframe::egui::{
    self, Color32, FontId, Id, Rect, RichText, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2,
};

const SEARCH_FIELD_HEIGHT: f32 = 36.0;

pub(crate) const PREDICTION_PROVIDERS: [DataSource; 2] =
    [DataSource::Polymarket, DataSource::Kalshi];

/// Size of a prediction market's artwork beside its title or choice.
/// The recent markets sit in this many columns, this far apart.
const RECENT_COLUMNS: f32 = 4.0;

const RECENT_GAP: f32 = 8.0;

/// What the user did in the Recent card this frame.
#[derive(Default)]
pub(crate) struct RecentsOutcome {
    /// A market was clicked: select it.
    pub pick: Option<SearchResult>,
    /// A market was double-clicked: take it.
    pub commit: Option<SearchResult>,
    /// A market was removed or the list cleared: save it.
    pub changed: bool,
}

/// The "Recent" card: the markets picked before, in rows with a Clear link and a context menu to
/// remove one. Shared by the portfolio's asset picker.
pub(crate) fn recents_card(
    ui: &mut Ui,
    recents: &mut Vec<RecentMarket>,
    selected: Option<&SearchResult>,
) -> RecentsOutcome {
    let mut outcome = RecentsOutcome::default();
    let mut remove = None;
    let mut clear = false;
    card(ui, |ui| {
        ui.horizontal(|ui| {
            caption_label(ui, Icon::ClockArrow, "Recent");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let clear_button = ui.add(
                    egui::Label::new(text(
                        "Clear",
                        size::CAPTION + 0.5,
                        400.0,
                        theme::text_secondary(),
                    ))
                    .sense(Sense::click()),
                );
                if clear_button
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    clear = true;
                }
            });
        });
        ui.add_space(8.0);
        let column = (ui.available_width() - RECENT_GAP * (RECENT_COLUMNS - 1.0)) / RECENT_COLUMNS;
        for (row, group) in recents.chunks(RECENT_COLUMNS as usize).enumerate() {
            if row > 0 {
                ui.add_space(2.0);
            }
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = RECENT_GAP;
                for (offset, market) in group.iter().enumerate() {
                    let result = market.result();
                    let is_selected = selected.is_some_and(|picked| same_market(picked, &result));
                    let response = result_row(ui, &result, is_selected, true, column);
                    response.context_menu(|ui| {
                        if ui.button("Remove from Recent").clicked() {
                            remove = Some(row * RECENT_COLUMNS as usize + offset);
                            ui.close();
                        }
                    });
                    if response.double_clicked() {
                        outcome.commit = Some(result);
                    } else if response.clicked() {
                        outcome.pick = Some(result);
                    }
                }
            });
        }
    });
    if let Some(index) = remove {
        recents.remove(index);
        outcome.changed = true;
    }
    if clear {
        recents.clear();
        outcome.changed = true;
    }
    outcome
}

/// A sheet's header: `scene`'s picture, then the title and a one-line description. The portfolio
/// sheets open with it.
pub(crate) fn header(ui: &mut Ui, scene: art::Scene, title: &str, subtitle: &str) {
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), art::SIZE.y), Sense::hover());
    let art = Rect::from_min_size(row.min, art::SIZE);
    art::paint(ui, art, scene);
    paint_header_text(ui, row, art.right() + 16.0, title, subtitle);
}

/// A header's title and description, vertically centred in `row` from `left`.
fn paint_header_text(ui: &Ui, row: Rect, left: f32, title: &str, subtitle: &str) {
    let painter = ui.painter();
    let width = row.right() - left;
    let title = galley(
        ui,
        title,
        size::TITLE2 + 1.0,
        SEMIBOLD,
        theme::text_primary(),
        width,
    );
    let subtitle = galley(
        ui,
        subtitle,
        size::SUBHEADLINE + 1.0,
        400.0,
        theme::text_secondary(),
        width,
    );
    let total = title.size().y + 2.0 + subtitle.size().y;
    let top = row.center().y - total / 2.0;
    painter.galley(pos2(left, top), title.clone(), theme::text_primary());
    painter.galley(
        pos2(left, top + title.size().y + 2.0),
        subtitle,
        theme::text_secondary(),
    );
}

/// What happened in a [`search_box`] this frame.
#[derive(Default)]
pub(crate) struct SearchBoxEvent {
    pub changed: bool,
    pub submitted: bool,
    pub cleared: bool,
}

pub(crate) fn search_box(
    ui: &mut Ui,
    query: &mut String,
    searching: bool,
    placeholder: &str,
    id: Id,
    focus: bool,
) -> SearchBoxEvent {
    let focused = ui.memory(|memory| memory.has_focus(id));
    let mut event = SearchBoxEvent::default();
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, SEARCH_FIELD_HEIGHT), Sense::hover());
    ui.painter().rect_filled(rect, 9.0, theme::quaternary(0.5));
    ui.painter().rect_stroke(
        rect,
        9.0,
        Stroke::new(
            1.0,
            if focused {
                theme::with_alpha(theme::text_secondary(), 0.7)
            } else {
                theme::ink(31)
            },
        ),
        StrokeKind::Inside,
    );
    let mut inner = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(vec2(12.0, 0.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    inner.spacing_mut().item_spacing.x = 8.0;
    icons::inline(
        &mut inner,
        Icon::Magnifier,
        14.0,
        if focused {
            theme::text_primary()
        } else {
            theme::text_secondary()
        },
    );
    let trailing = if query.is_empty() { 0.0 } else { 24.0 } + if searching { 24.0 } else { 0.0 };
    let edit = egui::TextEdit::singleline(query)
        .id(id)
        .frame(egui::Frame::NONE)
        .font(FontId::proportional(size::BODY))
        .text_color(theme::text_primary())
        .hint_text(text(placeholder, size::BODY, 400.0, theme::text_tertiary()))
        .desired_width((inner.available_width() - trailing).max(60.0));
    let response = inner.add(edit);
    if focus {
        response.request_focus();
    }
    event.changed = response.changed();
    if response.lost_focus() && inner.input(|input| input.key_pressed(egui::Key::Enter)) {
        event.submitted = true;
        response.request_focus();
    }
    if searching {
        inner.add(LoadingIndicator::new().size(13.0));
    }
    if !query.is_empty()
        && icons::button(
            &mut inner,
            Icon::XmarkCircleFill,
            22.0,
            theme::text_tertiary(),
            "Clear search",
        )
        .clicked()
    {
        event.cleared = true;
        response.request_focus();
    }
    event
}

/// The Polymarket / Kalshi switch, styled like the search box beside it.
pub(crate) fn provider_menu(ui: &mut Ui, provider: DataSource) -> Option<DataSource> {
    let name = galley(
        ui,
        provider.display_name(),
        size::BODY,
        MEDIUM,
        theme::text_primary(),
        f32::INFINITY,
    );
    let width = 12.0 + 18.0 + 8.0 + name.size().x + 8.0 + 9.0 + 12.0;
    let (rect, response) = ui.allocate_exact_size(vec2(width, SEARCH_FIELD_HEIGHT), Sense::click());
    let response = crate::ui::accessibility::label(response, "Prediction market provider")
        .on_hover_text("Prediction market provider")
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        9.0,
        if response.hovered() {
            theme::quaternary(0.7)
        } else {
            theme::quaternary(0.5)
        },
    );
    painter.rect_stroke(
        rect,
        9.0,
        Stroke::new(1.0, theme::ink(31)),
        StrokeKind::Inside,
    );
    let logo = Rect::from_min_size(
        pos2(rect.left() + 12.0, rect.center().y - 9.0),
        Vec2::splat(18.0),
    );
    source_logo::paint(ui, provider, logo);
    painter.galley(
        pos2(logo.right() + 8.0, rect.center().y - name.size().y / 2.0),
        name,
        theme::text_primary(),
    );
    icons::paint(
        painter,
        Rect::from_center_size(pos2(rect.right() - 16.5, rect.center().y), Vec2::splat(9.0)),
        Icon::ChevronDown,
        theme::text_secondary(),
    );
    let mut picked = None;
    egui::Popup::menu(&response).gap(4.0).show(|ui| {
        ui.set_min_width(150.0);
        for source in PREDICTION_PROVIDERS {
            let row = ui
                .horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let (mark, _) = ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
                    if source == provider {
                        icons::paint(ui.painter(), mark, Icon::Check, theme::text_primary());
                    }
                    source_logo::show(ui, source, 16.0);
                    ui.add(
                        egui::Button::new(text(
                            source.display_name(),
                            size::BODY,
                            400.0,
                            theme::text_primary(),
                        ))
                        .frame(false),
                    )
                })
                .inner;
            if row.clicked() {
                picked = Some(source);
            }
        }
    });
    picked
}

/// The quiet rounded panel suggestions and recents sit in.
pub(crate) fn card(ui: &mut Ui, content: impl FnOnce(&mut Ui)) {
    egui::Frame::new()
        .fill(theme::quaternary(0.3))
        .corner_radius(10.0)
        .stroke(Stroke::new(1.0, theme::ink(14)))
        .inner_margin(egui::Margin::same(14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            content(ui);
        });
}

/// A card's heading: a small icon and a medium-weight secondary caption.
pub(crate) fn caption_label(ui: &mut Ui, icon: Icon, caption: &str) {
    ui.spacing_mut().item_spacing.x = 5.0;
    icons::inline(ui, icon, 12.0, theme::text_secondary());
    ui.label(text(
        caption,
        size::SUBHEADLINE + 1.0,
        MEDIUM,
        theme::text_secondary(),
    ));
}

/// `ContentUnavailableView.search`: a large magnifier, a title and a hint, centred.
pub(crate) fn no_results(ui: &mut Ui, title: &str, detail: &str) {
    let rect = ui.available_rect_before_wrap();
    let painter = ui.painter();
    let title = galley(
        ui,
        title,
        15.0,
        BOLD,
        theme::text_primary(),
        rect.width() - 40.0,
    );
    let detail = galley(
        ui,
        detail,
        size::BODY,
        400.0,
        theme::text_secondary(),
        rect.width() - 40.0,
    );
    let total = 40.0 + 14.0 + title.size().y + 6.0 + detail.size().y;
    let mut y = rect.center().y - total / 2.0 - 10.0;
    icons::paint(
        painter,
        Rect::from_center_size(pos2(rect.center().x, y + 20.0), Vec2::splat(40.0)),
        Icon::Magnifier,
        theme::text_secondary(),
    );
    y += 54.0;
    painter.galley(
        pos2(rect.center().x - title.size().x / 2.0, y),
        title.clone(),
        theme::text_primary(),
    );
    y += title.size().y + 6.0;
    painter.galley(
        pos2(rect.center().x - detail.size().x / 2.0, y),
        detail,
        theme::text_secondary(),
    );
}

/// What the footer calls a pick: the event for several ticked choices, else the market.
fn banner_label(result: &SearchResult) -> &str {
    match &result.event {
        Some(event) if event.series.len() > 1 && !event.title.is_empty() => &event.title,
        _ => &result.symbol,
    }
}

/// The confirmation strip in the footer, showing what Add will commit.
pub(crate) fn selected_banner(ui: &mut Ui, result: &SearchResult, max_width: f32) {
    egui::Frame::new()
        .fill(theme::selected_tint(false))
        .corner_radius(10.0)
        .stroke(theme::selected_stroke())
        .inner_margin(egui::Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.set_max_width(max_width - 20.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let (icon, _) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::hover());
                let artwork = result
                    .event
                    .as_ref()
                    .and_then(|event| event.image_url.as_deref());
                if let Some(url) = artwork {
                    prediction_icon(ui, icon, &result.symbol, Some(url));
                } else if result.source.is_prediction_market() {
                    source_logo::paint(ui, result.source, icon);
                } else {
                    result_icon(ui, icon, result);
                }
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.label(text(
                        "Selected",
                        size::CAPTION,
                        400.0,
                        theme::text_secondary(),
                    ));
                    ui.add(
                        egui::Label::new(text(
                            banner_label(result),
                            size::CALLOUT + 0.5,
                            SEMIBOLD,
                            theme::text_primary(),
                        ))
                        .truncate(),
                    );
                });
                if let Some(price) = result.price {
                    let price = if result.source.is_prediction_market() {
                        crate::format::headline(price, true)
                    } else {
                        format!("${}", crate::format::price(price))
                    };
                    ui.label(
                        RichText::new(price)
                            .font(FontId::monospace(size::CAPTION + 0.5))
                            .color(theme::text_secondary()),
                    );
                }
            });
        });
}

/// A large push button: the prominent one is accent-filled, the other a quiet fill.
pub(crate) fn sheet_button(
    ui: &mut Ui,
    label: &str,
    prominent: bool,
    enabled: bool,
) -> egui::Response {
    let galley = galley(
        ui,
        label,
        size::BODY,
        if prominent { MEDIUM } else { 400.0 },
        Color32::PLACEHOLDER,
        f32::INFINITY,
    );
    let width = (galley.size().x + 32.0).max(76.0);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(vec2(width, 28.0), sense);
    let response = crate::ui::accessibility::label(response, label);
    let pressed = response.is_pointer_button_down_on();
    let hovered = response.hovered() && enabled;
    let (fill, color) = match (prominent, enabled) {
        (true, true) => (
            theme::ink(if pressed {
                56
            } else if hovered {
                44
            } else {
                36
            }),
            theme::text_primary(),
        ),
        (true, false) => (theme::ink(14), theme::text_tertiary()),
        (false, _) => (
            theme::ink(if pressed {
                46
            } else if hovered {
                36
            } else {
                26
            }),
            theme::text_primary(),
        ),
    };
    let painter = ui.painter();
    painter.rect_filled(rect, 7.0, fill);
    if !prominent || enabled {
        painter.rect_stroke(
            rect,
            7.0,
            if prominent {
                theme::selected_stroke()
            } else {
                Stroke::new(1.0, theme::ink(14))
            },
            StrokeKind::Inside,
        );
    }
    painter.galley(rect.center() - galley.size() / 2.0, galley, color);
    response
}

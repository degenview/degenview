//! The market picker under the toolbar's market button: crypto and stock search, recent
//! markets, and the rows they are listed in.

use super::{PreviewMarket, PreviewState};
use crate::model::DataSource;
use crate::service::keychain::api_keys::alpaca_configured;
use crate::service::work_queue::{self, PendingAll};
use crate::service::{self, SearchResult};
use crate::ui::icons::{self, Icon};
use crate::ui::kit::loading::LoadingIndicator;
use crate::ui::{
    chart::picker_kit::results,
    kit::{self, source_logo},
    theme,
};
use eframe::egui::{
    self, Align, Color32, CursorIcon, Id, Layout, Margin, Rect, Response, RichText, Sense, Stroke,
    Ui, Vec2, pos2, vec2,
};
use std::time::{Duration, Instant};

const SEARCH_DELAY: Duration = Duration::from_millis(300);

/// The crypto sources the picker searches, in order.
const CRYPTO_SOURCES: [DataSource; 4] = [
    DataSource::Binance,
    DataSource::Coinbase,
    DataSource::CoinGecko,
    DataSource::DexScreener,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AssetType {
    Crypto,
    Stock,
}

pub(super) struct Picker {
    asset: AssetType,
    crypto_text: String,
    stock_text: String,
    last_edit: Option<Instant>,
    results: Vec<SearchResult>,
    /// The search under way, one job per provider; a newer query replaces it.
    search: Option<PendingAll<Vec<SearchResult>>>,
    needs_focus: bool,
    /// The keyboard-highlighted row of the results (or of the recents).
    cursor: usize,
}

impl Picker {
    fn new(current: &PreviewMarket) -> Self {
        Self {
            asset: if current.source == DataSource::Alpaca {
                AssetType::Stock
            } else {
                AssetType::Crypto
            },
            crypto_text: String::new(),
            stock_text: String::new(),
            last_edit: None,
            results: Vec::new(),
            search: None,
            needs_focus: true,
            cursor: 0,
        }
    }

    fn text(&mut self) -> &mut String {
        match self.asset {
            AssetType::Crypto => &mut self.crypto_text,
            AssetType::Stock => &mut self.stock_text,
        }
    }
}

/// The market picker under the toolbar's market button (`response`): the button toggles it, a press
/// anywhere else outside the popup dismisses it, and a market picked there is tried at once.
pub(in crate::pine_studio) fn market_picker(
    ui: &mut Ui,
    preview: &mut PreviewState,
    response: &Response,
) {
    // The button toggles; a press anywhere else outside the popup dismisses it (below).
    let toggled_closed = response.clicked() && preview.picker.is_some();
    if toggled_closed {
        preview.picker = None;
        egui::Popup::close_id(ui.ctx(), Id::new("script-preview-picker"));
    } else if response.clicked() {
        preview.picker = Some(Picker::new(&preview.market));
    }
    if preview.picker.is_some() {
        let popup = egui::Popup::from_response(response)
            .id(Id::new("script-preview-picker"))
            .open_memory(Some(egui::SetOpenCommand::Bool(true)))
            .anchor(egui::PopupAnchor::ParentRect(response.rect))
            .align(egui::RectAlign::BOTTOM_START)
            .gap(6.0)
            .frame(kit::menu_frame().inner_margin(Margin::same(10)))
            .close_behavior(egui::PopupCloseBehavior::IgnoreClicks);
        let mut selected = None;
        let mut close = false;
        let opened = popup.show(|ui| {
            ui.set_width(380.0);
            selected = picker_ui(ui, preview);
        });
        match &opened {
            None => close = true,
            Some(inner) => {
                // A press outside the popup (and not on its button, which toggles) closes it.
                let outside = ui.input(|i| {
                    i.pointer.any_pressed()
                        && i.pointer.interact_pos().is_some_and(|pos| {
                            !inner.response.rect.contains(pos) && !response.rect.contains(pos)
                        })
                });
                if outside {
                    close = true;
                }
            }
        }
        if let Some(market) = selected {
            preview.select_market(market);
            close = true;
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            close = true;
        }
        if close {
            preview.picker = None;
            egui::Popup::close_id(ui.ctx(), Id::new("script-preview-picker"));
        }
    }
}

fn picker_ui(ui: &mut Ui, preview: &mut PreviewState) -> Option<PreviewMarket> {
    let mut selected = None;
    let mut picker = preview.picker.take()?;
    let ctx = ui.ctx().clone();
    let width = ui.available_width();
    ui.spacing_mut().item_spacing.y = 10.0;

    // The same icon tab bar the alert centre and chart settings use.
    let tabs = [
        kit::TabItem {
            value: AssetType::Crypto,
            title: "Crypto",
            icon: Icon::BitcoinCircle,
            count: 0,
        },
        kit::TabItem {
            value: AssetType::Stock,
            title: "Stocks",
            icon: Icon::BuildingColumns,
            count: 0,
        },
    ];
    let before = picker.asset;
    kit::tab_bar(
        ui,
        Id::new("script-preview-asset"),
        &tabs,
        &mut picker.asset,
    );
    if picker.asset != before {
        picker.results.clear();
        picker.cursor = 0;
        picker.needs_focus = true;
        picker.last_edit = Some(Instant::now());
    }

    let recents: Vec<PreviewMarket> = preview
        .recents
        .iter()
        .filter(|m| (m.source == DataSource::Alpaca) == (picker.asset == AssetType::Stock))
        .cloned()
        .collect();
    let query_empty = picker.text().trim().is_empty();
    let rows = if query_empty {
        recents.len()
    } else {
        picker.results.len()
    };
    // Arrow keys move the highlight; Return picks it. Consumed before the field sees them.
    let mut moved = false;
    if rows > 0 {
        let (down, up) = ui.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
            )
        });
        if down {
            picker.cursor = (picker.cursor + 1).min(rows - 1);
            moved = true;
        }
        if up {
            picker.cursor = picker.cursor.saturating_sub(1);
            moved = true;
        }
    }
    picker.cursor = picker.cursor.min(rows.saturating_sub(1));

    let placeholder = match picker.asset {
        AssetType::Crypto => "Search coins and tokens, e.g. BTC, ETH, PEPE",
        AssetType::Stock => "Search US stocks and ETFs, e.g. AAPL, Tesla",
    };
    let field_id = Id::new("script-preview-search");
    let searching = picker.search.is_some();
    let mut submitted = false;
    let mut text_changed = false;
    let focus = std::mem::take(&mut picker.needs_focus);
    let focused = ui.memory(|m| m.has_focus(field_id));
    kit::field_frame(focused)
        .inner_margin(Margin::symmetric(10, 0))
        .show(ui, |ui| {
            // A fixed-height row: `horizontal_centered` would claim the popup's whole height.
            ui.set_width(ui.available_width());
            let row = vec2(ui.available_width(), 34.0);
            ui.allocate_ui_with_layout(row, Layout::left_to_right(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                icons::inline(ui, Icon::Magnifier, 13.0, theme::text_secondary());
                let field_width = (ui.available_width() - 26.0).max(40.0);
                let field = kit::plain_field(
                    ui,
                    field_id,
                    picker.text(),
                    placeholder,
                    13.0,
                    kit::REGULAR,
                    field_width,
                );
                if focus {
                    field.request_focus();
                }
                text_changed = field.changed();
                submitted = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if searching {
                    ui.add(LoadingIndicator::new().size(13.0));
                } else if !picker.text().is_empty()
                    && icons::button(
                        ui,
                        Icon::XmarkCircleFill,
                        18.0,
                        theme::text_secondary(),
                        "Clear search",
                    )
                    .clicked()
                {
                    picker.text().clear();
                    picker.results.clear();
                    text_changed = true;
                    field.request_focus();
                }
            });
        });
    if text_changed {
        picker.last_edit = Some(Instant::now());
        picker.cursor = 0;
    }
    drive_search(&mut picker, &ctx);

    // The body keeps one height, so the popup does not jump between empty, loading and results.
    ui.allocate_ui_with_layout(
        vec2(width, BODY_HEIGHT),
        Layout::top_down(Align::Min),
        |ui| {
            ui.set_min_height(BODY_HEIGHT);
            ui.spacing_mut().item_spacing.y = 0.0;
            if picker.asset == AssetType::Stock && !alpaca_configured() {
                if kit::notice_card(
                    ui,
                    Icon::TriangleFill,
                    theme::YELLOW,
                    "Stocks need Alpaca market data",
                    Some("Add your Alpaca API keys in Settings."),
                    Some("Open Settings"),
                ) {
                    preview.pending_settings = true;
                }
                return;
            }
            let rows_width = ui.available_width();
            if query_empty && recents.is_empty() {
                body_message(
                    ui,
                    Icon::Magnifier,
                    "Search markets",
                    "Find a symbol to run this script on.",
                );
            } else if query_empty {
                section_header(ui, "Recent", None);
                for (index, market) in recents.iter().enumerate() {
                    let response = recent_row(ui, market, &preview.market, index == picker.cursor);
                    if index == picker.cursor && moved {
                        response.scroll_to_me(None);
                    }
                    if response.clicked() {
                        selected = Some(market.clone());
                    }
                }
                if submitted {
                    selected = recents.get(picker.cursor).cloned();
                }
            } else if !picker.results.is_empty() {
                if submitted {
                    selected = picker.results.get(picker.cursor).map(market_of);
                }
                egui::ScrollArea::vertical()
                    .max_height(BODY_HEIGHT)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.set_width(rows_width);
                        let mut last_source = None;
                        for (index, result) in picker.results.iter().enumerate() {
                            if last_source != Some(result.source) {
                                last_source = Some(result.source);
                                section_header(
                                    ui,
                                    result.source.display_name(),
                                    Some(result.source),
                                );
                            }
                            let current = market_of(result) == preview.market;
                            // The keyboard highlight sits under the row's own paint.
                            let slot = ui.painter().add(egui::Shape::Noop);
                            let response =
                                results::result_row(ui, result, current, false, rows_width - 8.0);
                            if index == picker.cursor && !current {
                                ui.painter().set(
                                    slot,
                                    egui::Shape::rect_filled(response.rect, 8.0, theme::ink(15)),
                                );
                                if moved {
                                    response.scroll_to_me(None);
                                }
                            }
                            if response.clicked() {
                                selected = Some(market_of(result));
                            }
                        }
                    });
            } else if picker.search.is_some() || picker.last_edit.is_some() {
                let (rect, _) =
                    ui.allocate_exact_size(vec2(rows_width, BODY_HEIGHT), Sense::hover());
                ui.put(
                    Rect::from_center_size(rect.center(), Vec2::splat(40.0)),
                    LoadingIndicator::new().size(40.0),
                );
            } else {
                body_message(
                    ui,
                    Icon::Magnifier,
                    "No markets found",
                    "Try another symbol or name.",
                );
            }
        },
    );

    if rows > 0 {
        ui.label(
            RichText::new("↑↓ Navigate    ↩ Select    esc Close")
                .size(10.0)
                .color(theme::text_tertiary()),
        );
    }
    preview.picker = Some(picker);
    selected
}

/// Height of the picker's results area.
const BODY_HEIGHT: f32 = 300.0;

/// A quiet message that fills the results area.
fn body_message(ui: &mut Ui, icon: Icon, title: &str, detail: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), BODY_HEIGHT), Sense::hover());
    kit::empty_state(ui, rect, icon, title, detail);
}

/// A group caption: the source's logo and name, or just a title.
fn section_header(ui: &mut Ui, title: &str, source: Option<DataSource>) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::hover());
    let mut x = rect.left() + 10.0;
    if let Some(source) = source {
        let logo = Rect::from_center_size(pos2(x + 6.0, rect.center().y + 2.0), Vec2::splat(12.0));
        source_logo::paint(ui, source, logo);
        x = logo.right() + 6.0;
    }
    let label = kit::galley(
        ui,
        &title.to_uppercase(),
        10.5,
        kit::SEMIBOLD,
        theme::text_secondary(),
        f32::INFINITY,
    );
    ui.painter().galley(
        pos2(x, rect.center().y + 2.0 - label.size().y / 2.0),
        label,
        theme::text_secondary(),
    );
}

fn market_of(result: &SearchResult) -> PreviewMarket {
    PreviewMarket {
        ticker: result.full_symbol.clone(),
        source: result.source,
    }
}

/// A recently used market, drawn like a search hit.
fn recent_row(
    ui: &mut Ui,
    market: &PreviewMarket,
    current: &PreviewMarket,
    highlighted: bool,
) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width() - 8.0, 38.0), Sense::click());
    let response = response.on_hover_cursor(CursorIcon::PointingHand);
    let is_current = market == current;
    let fill = if is_current {
        theme::with_alpha(theme::ACCENT, 0.12)
    } else if response.hovered() || highlighted {
        theme::ink(15)
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, 8.0, fill);
    if is_current {
        ui.painter().rect_stroke(
            rect,
            8.0,
            Stroke::new(1.0, theme::with_alpha(theme::ACCENT, 0.5)),
            egui::StrokeKind::Inside,
        );
    }
    let content = rect.shrink2(vec2(10.0, 5.0));
    let icon = Rect::from_min_size(
        pos2(content.left(), rect.center().y - 14.0),
        Vec2::splat(28.0),
    );
    kit::chart_icon(ui, icon, &market.ticker, market.source, true);
    let mut right = content.right();
    if is_current {
        let check = Rect::from_center_size(pos2(right - 7.0, rect.center().y), Vec2::splat(14.0));
        icons::paint(ui.painter(), check, Icon::CheckCircleFill, theme::ACCENT);
        right = check.left() - 10.0;
    }
    let source = kit::galley(
        ui,
        market.source.display_name(),
        11.0,
        kit::REGULAR,
        theme::text_secondary(),
        f32::INFINITY,
    );
    right -= source.size().x;
    ui.painter().galley(
        pos2(right, rect.center().y - source.size().y / 2.0),
        source,
        theme::text_secondary(),
    );
    let left = icon.right() + 10.0;
    let title = kit::galley(
        ui,
        &market.title(),
        13.0,
        kit::SEMIBOLD,
        theme::text_primary(),
        (right - left - 10.0).max(20.0),
    );
    ui.painter().galley(
        pos2(left, rect.center().y - title.size().y / 2.0),
        title,
        theme::text_primary(),
    );
    response
}

/// Debounces the query by 300 ms, then searches the right providers, each on its own workers.
fn drive_search(picker: &mut Picker, ctx: &egui::Context) {
    if let Some(answers) = picker.search.as_mut().and_then(PendingAll::poll) {
        picker.search = None;
        // In provider order; a provider that failed adds nothing.
        picker.results = answers.into_iter().flatten().flatten().collect();
        picker.cursor = 0;
    }
    let Some(at) = picker.last_edit else {
        return;
    };
    if at.elapsed() < SEARCH_DELAY {
        ctx.request_repaint_after(SEARCH_DELAY);
        return;
    }
    picker.last_edit = None;
    let query = picker.text().trim().to_owned();
    // A newer query (or none) replaces the search under way; its answers are dropped.
    picker.search = None;
    if query.is_empty() {
        picker.results.clear();
        return;
    }
    let sources: Vec<DataSource> = match picker.asset {
        AssetType::Crypto => CRYPTO_SOURCES.to_vec(),
        AssetType::Stock => vec![DataSource::Alpaca],
    };
    let searches = sources.into_iter().map(|source| {
        let query = query.clone();
        work_queue::start_search(source, Some(ctx), move || {
            service::for_source(source)
                .search_tickers(&query)
                .unwrap_or_default()
        })
    });
    picker.search = Some(PendingAll::new(searches));
}

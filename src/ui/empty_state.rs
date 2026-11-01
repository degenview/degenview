//! What a chart tab shows before it has a chart (Swift `EmptyStateView`): a search launcher,
//! then one-click ways back into the user's own markets — favorites, recently added markets and
//! saved layouts — or, for someone with none yet, the same starting points as Add Chart.

use crate::model::DataSource;
use crate::model::watchlist::WatchlistInstrument;
use crate::ui::add_chart_sheet::RecentMarket;
use crate::ui::icons::{self, Icon};
use crate::ui::kit::{self, MEDIUM, REGULAR, SEMIBOLD, size};
use crate::ui::{empty_art, theme};
use eframe::egui::{self, Color32, Id, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};

const MAX_WIDTH: f32 = 720.0;
/// macOS `.callout`.
const CALLOUT: f32 = 12.0;
const LAUNCHER_HEIGHT: f32 = 50.0;
const CHIP_MIN_WIDTH: f32 = 160.0;
const STARTER_CHIP_MIN_WIDTH: f32 = 128.0;
const CARD_MIN_WIDTH: f32 = 200.0;
/// Chips shown per section; the rest stays reachable from the sidebar or Add Chart.
pub const ITEM_LIMIT: usize = 8;
/// Ticker icons drawn on a saved-layout card.
const PREVIEW_LIMIT: usize = 3;

/// Starter crypto markets: ticker and name.
const CRYPTO: [(&str, &str); 10] = [
    ("BTC", "Bitcoin"),
    ("ETH", "Ethereum"),
    ("SOL", "Solana"),
    ("BNB", "BNB"),
    ("XRP", "XRP"),
    ("ADA", "Cardano"),
    ("DOGE", "Dogecoin"),
    ("AVAX", "Avalanche"),
    ("DOT", "Polkadot"),
    ("LINK", "Chainlink"),
];
/// Starter stocks and ETFs: ticker and name.
const STOCKS: [(&str, &str); 10] = [
    ("AAPL", "Apple"),
    ("MSFT", "Microsoft"),
    ("NVDA", "NVIDIA"),
    ("AMZN", "Amazon"),
    ("GOOGL", "Alphabet"),
    ("META", "Meta"),
    ("TSLA", "Tesla"),
    ("SPY", "S&P 500"),
    ("QQQ", "Nasdaq 100"),
    ("AMD", "AMD"),
];

/// One saved layout as its card shows it.
pub struct LayoutSummary {
    pub id: u64,
    pub name: String,
    pub chart_count: usize,
    pub timeframe: &'static str,
    /// The first few markets, as `(source, symbol)`.
    pub previews: Vec<(DataSource, String)>,
}

pub struct EmptyStateInput<'a> {
    /// Sidebar order, already capped at `ITEM_LIMIT`.
    pub favorites: &'a [WatchlistInstrument],
    /// Favorites that did not fit.
    pub hidden_favorites: usize,
    /// Newest first, capped, without markets that are already favorites.
    pub recents: &'a [RecentMarket],
    pub layouts: &'a [LayoutSummary],
    /// Stocks need an Alpaca feed, so their chips would go nowhere without one.
    pub offers_stocks: bool,
}

impl EmptyStateInput<'_> {
    fn has_any(&self) -> bool {
        !self.favorites.is_empty() || !self.recents.is_empty() || !self.layouts.is_empty()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EmptyStateAction {
    AddChart,
    OpenFavorite(usize),
    OpenRecent(usize),
    OpenLayout(u64),
    /// A starter market: `(source, provider symbol, display symbol)`.
    OpenSuggestion(DataSource, String, String),
    RemoveRecent(usize),
    ClearRecents,
    ShowFavorites,
}

/// Recents newest first, capped, leaving out markets the favorites already offer.
pub fn trim_recents(recents: &[RecentMarket], favorites: &[WatchlistInstrument]) -> Vec<usize> {
    recents
        .iter()
        .enumerate()
        .filter(|(_, market)| {
            !favorites.iter().any(|item| {
                item.instrument.source == market.source
                    && item
                        .instrument
                        .symbol
                        .eq_ignore_ascii_case(&market.full_symbol)
            })
        })
        .map(|(index, _)| index)
        .take(ITEM_LIMIT)
        .collect()
}

pub fn show(ui: &mut Ui, input: &EmptyStateInput) -> Vec<EmptyStateAction> {
    let mut actions = Vec::new();
    let area = ui.max_rect();
    let content_id = Id::new("empty-state-height");
    egui::ScrollArea::vertical()
        .id_salt("empty-state")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let width = (area.width() - 48.0).min(MAX_WIDTH);
            // Centred in the window while it fits, from the top once it scrolls.
            let last_height: f32 = ui.data(|data| data.get_temp(content_id)).unwrap_or(0.0);
            let top = ((area.height() - last_height) / 2.0).max(36.0);
            ui.add_space(top);
            let column = ui
                .vertical_centered(|ui| {
                    ui.set_max_width(width);
                    ui.allocate_ui_with_layout(
                        vec2(width, 0.0),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_width(width);
                            ui.spacing_mut().item_spacing = Vec2::ZERO;
                            hero(ui, input.has_any());
                            ui.add_space(28.0);
                            if launcher(ui).clicked() {
                                actions.push(EmptyStateAction::AddChart);
                            }
                            ui.add_space(28.0);
                            // The starting points stay above the user's own markets.
                            starters(ui, input.offers_stocks, &mut actions);
                            if input.has_any() {
                                ui.add_space(16.0);
                                sections(ui, input, &mut actions);
                            } else {
                                ui.add_space(28.0);
                                tip(ui);
                            }
                        },
                    )
                    .response
                    .rect
                })
                .inner;
            ui.data_mut(|data| data.insert_temp(content_id, column.height()));
            ui.add_space(36.0);
        });
    actions
}

fn hero(ui: &mut Ui, has_any: bool) {
    let (rect, _) = ui.allocate_exact_size(
        vec2(ui.available_width(), empty_art::SIZE.y),
        Sense::hover(),
    );
    empty_art::paint(ui, Rect::from_center_size(rect.center(), empty_art::SIZE));
    ui.add_space(14.0);
    centred_line(
        ui,
        "Start a new chart",
        size::TITLE2 + 4.0,
        SEMIBOLD,
        theme::text_primary(),
    );
    ui.add_space(5.0);
    centred_line(
        ui,
        if has_any {
            "Search a market, or pick up where you left off."
        } else {
            "Search crypto, stocks and prediction markets."
        },
        size::BODY + 1.0,
        REGULAR,
        theme::text_secondary(),
    );
}

fn centred_line(ui: &mut Ui, text: &str, font: f32, weight: f32, color: Color32) {
    let galley = kit::galley(ui, text, font, weight, color, ui.available_width());
    let (rect, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), galley.size().y), Sense::hover());
    ui.painter().galley(
        pos2(rect.center().x - galley.size().x / 2.0, rect.top()),
        galley,
        color,
    );
}

/// A search box that is really a button: it opens Add Chart, where the typing happens.
fn launcher(ui: &mut Ui) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), LAUNCHER_HEIGHT), Sense::click());
    let response = crate::ui::accessibility::label(response, "Add Chart")
        .on_hover_text("Add Chart")
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let hovered = response.hovered();
    let radius = 14.0;
    let painter = ui.painter();
    painter.add(
        egui::Shadow {
            offset: [0, 3],
            blur: if hovered { 16 } else { 10 },
            spread: 0,
            color: Color32::from_black_alpha(if hovered { 46 } else { 28 }),
        }
        .as_shape(rect, radius),
    );
    painter.rect_filled(rect, radius, theme::editor_background());
    painter.rect_filled(
        rect,
        radius,
        theme::quaternary(if hovered { 0.7 } else { 0.4 }),
    );
    painter.rect_stroke(
        rect,
        radius,
        Stroke::new(
            1.0,
            if hovered {
                theme::with_alpha(theme::text_secondary(), 0.6)
            } else {
                kit::primary(0.14)
            },
        ),
        StrokeKind::Inside,
    );
    icons::paint(
        painter,
        Rect::from_center_size(pos2(rect.left() + 24.0, rect.center().y), Vec2::splat(16.0)),
        Icon::Magnifier,
        if hovered {
            theme::text_primary()
        } else {
            theme::text_secondary()
        },
    );
    // What it finds, as quiet marks at the right.
    let mut x = rect.right() - 20.0;
    for icon in [Icon::Percent, Icon::BuildingColumns, Icon::BitcoinCircle] {
        icons::paint(
            painter,
            Rect::from_center_size(pos2(x, rect.center().y), Vec2::splat(15.0)),
            icon,
            theme::text_tertiary(),
        );
        x -= 25.0;
    }
    painter.vline(
        x + 6.0,
        rect.center().y - 9.0..=rect.center().y + 9.0,
        Stroke::new(1.0, theme::ink(16)),
    );
    let prompt = kit::galley(
        ui,
        "Search crypto, stocks, prediction markets…",
        size::BODY + 1.0,
        REGULAR,
        theme::text_secondary(),
        (x - rect.left() - 56.0).max(40.0),
    );
    ui.painter().galley(
        pos2(rect.left() + 46.0, rect.center().y - prompt.size().y / 2.0),
        prompt,
        theme::text_secondary(),
    );
    response
}

fn tip(ui: &mut Ui) {
    let text = "Save a layout from the layout menu and it will show up here.";
    let galley = kit::galley(
        ui,
        text,
        size::CAPTION + 0.5,
        REGULAR,
        theme::text_secondary(),
        ui.available_width() - 60.0,
    );
    let height = galley.size().y + 16.0;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let total = 14.0 + 8.0 + galley.size().x;
    let pill = Rect::from_center_size(rect.center(), vec2(total + 28.0, height));
    ui.painter().rect_filled(pill, height / 2.0, theme::ink(6));
    ui.painter().rect_stroke(
        pill,
        height / 2.0,
        Stroke::new(1.0, kit::hairline()),
        StrokeKind::Inside,
    );
    let left = pill.left() + 14.0;
    icons::paint(
        ui.painter(),
        Rect::from_center_size(pos2(left + 7.0, rect.center().y), Vec2::splat(14.0)),
        Icon::Lightbulb,
        theme::YELLOW,
    );
    ui.painter().galley(
        pos2(left + 22.0, rect.center().y - galley.size().y / 2.0),
        galley,
        theme::text_secondary(),
    );
}

/// A titled card, like the suggestion and recents cards in Add Chart. `accessory` draws at the
/// heading's right and says whether it was clicked.
fn section(
    ui: &mut Ui,
    icon: Icon,
    title: &str,
    accessory: Option<&str>,
    content: impl FnOnce(&mut Ui),
) -> bool {
    let mut clicked = false;
    egui::Frame::new()
        .fill(theme::quaternary(0.3))
        .corner_radius(10.0)
        .stroke(Stroke::new(1.0, kit::hairline()))
        .inner_margin(egui::Margin::same(14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 5.0;
                icons::inline(ui, icon, 12.0, theme::text_secondary());
                ui.label(kit::text(
                    title,
                    size::SUBHEADLINE + 1.0,
                    MEDIUM,
                    theme::text_secondary(),
                ));
                if let Some(accessory) = accessory {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        clicked = ui
                            .add(
                                egui::Label::new(kit::text(
                                    accessory,
                                    size::CAPTION + 0.5,
                                    REGULAR,
                                    theme::text_secondary(),
                                ))
                                .sense(Sense::click()),
                            )
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .clicked();
                    });
                }
            });
            ui.add_space(10.0);
            content(ui);
        });
    clicked
}

/// Lays `count` tiles in as many columns of at least `min_width` as fit; `tile` draws one into
/// its rect and returns its response.
fn grid(
    ui: &mut Ui,
    count: usize,
    min_width: f32,
    height: f32,
    mut tile: impl FnMut(&mut Ui, usize, Rect),
) {
    let spacing = 8.0;
    let width = ui.available_width();
    let columns = (((width + spacing) / (min_width + spacing)).floor() as usize).max(1);
    let tile_width = (width - spacing * (columns - 1) as f32) / columns as f32;
    let rows = count.div_ceil(columns);
    let (area, _) = ui.allocate_exact_size(
        vec2(
            width,
            rows as f32 * height + rows.saturating_sub(1) as f32 * spacing,
        ),
        Sense::hover(),
    );
    for index in 0..count {
        let (row, column) = (index / columns, index % columns);
        let rect = Rect::from_min_size(
            pos2(
                area.left() + column as f32 * (tile_width + spacing),
                area.top() + row as f32 * (height + spacing),
            ),
            vec2(tile_width, height),
        );
        tile(ui, index, rect);
    }
}

/// The button every chip and card is: a quiet tile that lifts and firms its outline on hover.
fn tile(ui: &mut Ui, rect: Rect, id: Id, tooltip: &str) -> egui::Response {
    let response = ui
        .interact(rect, id, Sense::click())
        .on_hover_text(tooltip)
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let hovered = response.hovered();
    ui.painter()
        .rect_filled(rect, 9.0, kit::primary(if hovered { 0.1 } else { 0.05 }));
    ui.painter().rect_stroke(
        rect,
        9.0,
        Stroke::new(
            1.0,
            if hovered {
                theme::with_alpha(theme::text_secondary(), 0.5)
            } else {
                kit::primary(0.1)
            },
        ),
        StrokeKind::Inside,
    );
    response
}

/// What a market shortcut shows.
struct Market<'a> {
    icon_symbol: &'a str,
    source: DataSource,
    title: &'a str,
    subtitle: Option<&'a str>,
    badged: bool,
}

/// One market as a shortcut: artwork (with its data source on the corner when `badged`), ticker
/// and an optional second line.
fn market_chip(ui: &mut Ui, rect: Rect, id: Id, market: Market) -> egui::Response {
    let Market {
        icon_symbol,
        source,
        title,
        subtitle,
        badged,
    } = market;
    let response = tile(ui, rect, id, &format!("Add {title} to this tab"));
    let response = crate::ui::accessibility::label(
        response,
        subtitle.map_or_else(
            || title.to_owned(),
            |subtitle| format!("{title}, {subtitle}"),
        ),
    );
    let icon = Rect::from_min_size(
        pos2(rect.left() + 10.0, rect.center().y - 12.0),
        Vec2::splat(24.0),
    );
    kit::ticker_icon(ui, icon, icon_symbol, source);
    if badged {
        kit::source_badge(ui, icon, source);
    }
    let left = icon.right() + 10.0;
    let width = rect.right() - 10.0 - left;
    let title = kit::galley(ui, title, CALLOUT, MEDIUM, theme::text_primary(), width);
    let subtitle = subtitle.map(|text| {
        kit::galley(
            ui,
            text,
            size::CAPTION,
            REGULAR,
            theme::text_secondary(),
            width,
        )
    });
    let total = title.size().y + subtitle.as_ref().map_or(0.0, |s| 1.0 + s.size().y);
    let top = rect.center().y - total / 2.0;
    let title_height = title.size().y;
    ui.painter()
        .galley(pos2(left, top), title, theme::text_primary());
    if let Some(subtitle) = subtitle {
        ui.painter().galley(
            pos2(left, top + title_height + 1.0),
            subtitle,
            theme::text_secondary(),
        );
    }
    response
}

/// Common starting points, the same as Add Chart offers; always shown above the user's own
/// favorites, recents and layouts.
fn starters(ui: &mut Ui, offers_stocks: bool, actions: &mut Vec<EmptyStateAction>) {
    section(ui, Icon::Sparkles, "Suggestions", None, |ui| {
        grid(
            ui,
            CRYPTO.len(),
            STARTER_CHIP_MIN_WIDTH,
            44.0,
            |ui, index, rect| {
                let (base, name) = CRYPTO[index];
                if market_chip(
                    ui,
                    rect,
                    Id::new(("empty-crypto", base)),
                    Market {
                        icon_symbol: base,
                        source: DataSource::Binance,
                        title: base,
                        subtitle: Some(name),
                        badged: false,
                    },
                )
                .clicked()
                {
                    actions.push(EmptyStateAction::OpenSuggestion(
                        DataSource::Binance,
                        format!("{base}USDT"),
                        format!("{base}/USDT"),
                    ));
                }
            },
        );
    });
    if offers_stocks {
        ui.add_space(16.0);
        section(
            ui,
            Icon::BuildingColumns,
            "Popular stocks and ETFs",
            None,
            |ui| {
                grid(
                    ui,
                    STOCKS.len(),
                    STARTER_CHIP_MIN_WIDTH,
                    44.0,
                    |ui, index, rect| {
                        let (symbol, name) = STOCKS[index];
                        if market_chip(
                            ui,
                            rect,
                            Id::new(("empty-stock", symbol)),
                            Market {
                                icon_symbol: symbol,
                                source: DataSource::Alpaca,
                                title: symbol,
                                subtitle: Some(name),
                                badged: false,
                            },
                        )
                        .clicked()
                        {
                            actions.push(EmptyStateAction::OpenSuggestion(
                                DataSource::Alpaca,
                                symbol.to_owned(),
                                symbol.to_owned(),
                            ));
                        }
                    },
                );
            },
        );
    }
}

fn sections(ui: &mut Ui, input: &EmptyStateInput, actions: &mut Vec<EmptyStateAction>) {
    let mut first = true;
    let mut gap = |ui: &mut Ui| {
        if !std::mem::take(&mut first) {
            ui.add_space(16.0);
        }
    };
    if !input.favorites.is_empty() {
        gap(ui);
        let more = (input.hidden_favorites > 0).then(|| format!("{} more", input.hidden_favorites));
        if section(ui, Icon::StarFill, "Favorites", more.as_deref(), |ui| {
            grid(
                ui,
                input.favorites.len(),
                CHIP_MIN_WIDTH,
                46.0,
                |ui, index, rect| {
                    let item = &input.favorites[index];
                    if market_chip(
                        ui,
                        rect,
                        Id::new(("empty-favorite", item.id)),
                        Market {
                            icon_symbol: &item.label,
                            source: item.instrument.source,
                            title: &item.label,
                            subtitle: Some(&item.name),
                            badged: true,
                        },
                    )
                    .clicked()
                    {
                        actions.push(EmptyStateAction::OpenFavorite(index));
                    }
                },
            );
        }) {
            actions.push(EmptyStateAction::ShowFavorites);
        }
    }
    if !input.recents.is_empty() {
        gap(ui);
        if section(ui, Icon::ClockArrow, "Recent", Some("Clear"), |ui| {
            grid(
                ui,
                input.recents.len(),
                CHIP_MIN_WIDTH,
                46.0,
                |ui, index, rect| {
                    let market = &input.recents[index];
                    let response = market_chip(
                        ui,
                        rect,
                        Id::new(("empty-recent", index)),
                        Market {
                            icon_symbol: &market.symbol,
                            source: market.source,
                            title: &market.symbol,
                            subtitle: Some(market.source.display_name()),
                            badged: true,
                        },
                    );
                    egui::Popup::context_menu(&response)
                        .frame(kit::menu_frame())
                        .show(|ui| {
                            ui.set_width(180.0);
                            if kit::menu_item(ui, Icon::Trash, "Remove from Recent", true, false) {
                                actions.push(EmptyStateAction::RemoveRecent(index));
                                ui.close();
                            }
                        });
                    if response.clicked() {
                        actions.push(EmptyStateAction::OpenRecent(index));
                    }
                },
            );
        }) {
            actions.push(EmptyStateAction::ClearRecents);
        }
    }
    if !input.layouts.is_empty() {
        gap(ui);
        section(ui, Icon::Books, "Saved Layouts", None, |ui| {
            grid(
                ui,
                input.layouts.len(),
                CARD_MIN_WIDTH,
                52.0,
                |ui, index, rect| {
                    if layout_card(ui, rect, &input.layouts[index]).clicked() {
                        actions.push(EmptyStateAction::OpenLayout(input.layouts[index].id));
                    }
                },
            );
        });
    }
}

/// One saved layout: its name, what is in it, and its first few markets as artwork.
fn layout_card(ui: &mut Ui, rect: Rect, layout: &LayoutSummary) -> egui::Response {
    let response = tile(
        ui,
        rect,
        Id::new(("empty-layout", layout.id)),
        &format!("Open {}", layout.name),
    );
    let subtitle = format!(
        "{} · {}",
        if layout.chart_count == 1 {
            "1 chart".to_owned()
        } else {
            format!("{} charts", layout.chart_count)
        },
        layout.timeframe
    );
    let response =
        crate::ui::accessibility::label(response, format!("{}, {subtitle}", layout.name));
    let inner = rect.shrink2(vec2(10.0, 8.0));
    let name = kit::galley(
        ui,
        &layout.name,
        CALLOUT,
        SEMIBOLD,
        theme::text_primary(),
        inner.width() - 16.0,
    );
    let name_height = name.size().y;
    ui.painter().galley(inner.min, name, theme::text_primary());
    icons::paint(
        ui.painter(),
        Rect::from_min_size(
            pos2(inner.right() - 9.0, inner.top() + name_height / 2.0 - 4.5),
            Vec2::splat(9.0),
        ),
        Icon::ChevronRight,
        theme::text_tertiary(),
    );
    let detail = kit::galley(
        ui,
        &subtitle,
        size::CAPTION,
        REGULAR,
        theme::text_secondary(),
        inner.width() - 60.0,
    );
    ui.painter().galley(
        pos2(inner.left(), inner.bottom() - detail.size().y),
        detail,
        theme::text_secondary(),
    );
    // Overlapping artwork, each ringed in the window colour.
    let mut x = inner.right() - 18.0;
    for (source, symbol) in layout.previews.iter().take(PREVIEW_LIMIT).rev() {
        let icon = Rect::from_min_size(pos2(x, inner.bottom() - 18.0), Vec2::splat(18.0));
        ui.painter()
            .circle_filled(icon.center(), 10.5, theme::chrome());
        kit::ticker_icon(ui, icon, &source.display_symbol(symbol), *source);
        x -= 13.0;
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::InstrumentId;

    fn favorite(symbol: &str) -> WatchlistInstrument {
        WatchlistInstrument {
            id: 1,
            instrument: InstrumentId::new(DataSource::Binance, symbol),
            name: "Bitcoin".into(),
            label: "BTC".into(),
            display_name: None,
            image_url: None,
            card: None,
        }
    }

    fn recent(symbol: &str) -> RecentMarket {
        RecentMarket {
            symbol: symbol.into(),
            full_symbol: symbol.into(),
            source: DataSource::Binance,
        }
    }

    #[test]
    fn recents_skip_favorites_and_stop_at_the_limit() {
        let recents: Vec<RecentMarket> = (0..12)
            .map(|n| recent(&format!("C{n}USDT")))
            .chain([recent("BTCUSDT")])
            .collect();
        let kept = trim_recents(&recents, &[favorite("BTCUSDT")]);
        assert_eq!(kept.len(), ITEM_LIMIT);
        assert!(!kept.contains(&12), "a favorite is not offered twice");
        assert_eq!(
            trim_recents(&[recent("btcusdt")], &[favorite("BTCUSDT")]),
            Vec::<usize>::new()
        );
    }

    #[test]
    fn the_page_draws_headlessly_for_new_and_returning_users() {
        let favorites = [favorite("BTCUSDT")];
        let recents = [recent("ETHUSDT")];
        let layouts = [LayoutSummary {
            id: 3,
            name: "Majors".into(),
            chart_count: 2,
            timeframe: "1D",
            previews: vec![(DataSource::Binance, "BTCUSDT".into())],
        }];
        let ctx = egui::Context::default();
        for returning in [false, true] {
            let input = EmptyStateInput {
                favorites: if returning { &favorites } else { &[] },
                hidden_favorites: 0,
                recents: if returning { &recents } else { &[] },
                layouts: if returning { &layouts } else { &[] },
                offers_stocks: true,
            };
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                assert!(show(ui, &input).is_empty());
            });
            output.textures_delta.clear();
        }
    }

    /// Pictures of the page for a person to look at. Run by hand:
    /// `DEGENVIEW_GALLERY_DIR=… cargo test --lib empty_state -- --ignored`.
    #[test]
    #[ignore = "writes images for a person to look at"]
    fn gallery_of_the_page() {
        use crate::ui::render_tests::{Screen, gallery_dir};
        use crate::ui::theme::AppTheme;
        if gallery_dir().is_none() {
            return;
        }
        let _theme = crate::ui::theme::RestoreDefaultTheme;
        let favorites: Vec<WatchlistInstrument> = ["BTCUSDT", "ETHUSDT", "SOLUSDT", "BNBUSDT"]
            .into_iter()
            .enumerate()
            .map(|(index, symbol)| WatchlistInstrument {
                id: index as _,
                ..favorite(symbol)
            })
            .collect();
        let recents: Vec<RecentMarket> = ["DOGEUSDT", "AVAXUSDT", "LINKUSDT"]
            .into_iter()
            .map(recent)
            .collect();
        let layouts = [
            LayoutSummary {
                id: 1,
                name: "Majors".into(),
                chart_count: 4,
                timeframe: "1D",
                previews: vec![
                    (DataSource::Binance, "BTCUSDT".into()),
                    (DataSource::Binance, "ETHUSDT".into()),
                    (DataSource::Binance, "SOLUSDT".into()),
                ],
            },
            LayoutSummary {
                id: 2,
                name: "Tech stocks".into(),
                chart_count: 1,
                timeframe: "4H",
                previews: vec![(DataSource::Alpaca, "NVDA".into())],
            },
        ];
        for (theme_name, app_theme) in [("dark", AppTheme::Dark), ("light", AppTheme::Light)] {
            for returning in [false, true] {
                let mut screen =
                    Screen::new(egui::vec2(1000.0, if returning { 980.0 } else { 760.0 }));
                crate::ui::theme::apply(&screen.ctx, app_theme);
                let input = EmptyStateInput {
                    favorites: if returning { &favorites } else { &[] },
                    hidden_favorites: 0,
                    recents: if returning { &recents } else { &[] },
                    layouts: if returning { &layouts } else { &[] },
                    offers_stocks: true,
                };
                let mut last = None;
                for _ in 0..4 {
                    last = Some(screen.run(Vec::new(), |ui| {
                        show(ui, &input);
                    }));
                }
                screen.save(
                    &format!(
                        "empty-page-{}-{theme_name}",
                        if returning { "returning" } else { "new" }
                    ),
                    last.expect("a frame"),
                );
            }
        }
    }
}

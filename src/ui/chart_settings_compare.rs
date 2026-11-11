//! The Compare page of the chart settings: every market drawn over the chart, with its look, its
//! scale and its place in the drawing order.
//!
//! One card per market. The header always shows what matters (colour, line style, name, exchange,
//! how its data is doing, last price) with the eye, a menu and a chevron; the settings sit under the
//! chevron, open by default only while there are a few markets, so a chart comparing forty is still
//! a list you can scan. Past a few markets a search field appears. Everything reads the card and
//! answers with a [`ChartSettingsAction`]; nothing here edits a chart.

use super::chart_card::ChartCard;
use super::chart_settings::ChartSettingsAction;
use super::icons::{self, Icon};
use super::overlay_settings::{self, rgb};
use super::{kit, settings_card, theme};
use crate::model::chart_type::LineDash;
use crate::model::overlay::{ChartOverlay, Move, OverlayCommand, OverlayId, OverlayScale};
use crate::model::overlay_feed::OverlayStatus;
use crate::model::price_scale::PriceScaleMode;
use eframe::egui::{self, Align, Color32, Id, Layout, Rect, Sense, Stroke, Ui, Vec2, pos2, vec2};
use std::collections::HashMap;

/// Up to this many markets, every card starts open.
const OPEN_UP_TO: usize = 3;
/// A search field appears past this many.
const SEARCH_FROM: usize = 6;

/// What the page remembers while the sheet is open.
#[derive(Default)]
pub struct CompareState {
    /// Cards the user opened or closed; the rest follow [`OPEN_UP_TO`].
    open: HashMap<OverlayId, bool>,
    query: String,
    /// "Remove all" was asked for and waits for its answer.
    confirm_remove_all: bool,
}

impl CompareState {
    fn is_open(&self, id: OverlayId, count: usize) -> bool {
        self.open.get(&id).copied().unwrap_or(count <= OPEN_UP_TO)
    }
}

/// What the page reads of the chart.
pub struct CompareInput<'a> {
    pub card: &'a ChartCard,
    /// The chart's own market, as the sheet's title says it.
    pub chart_name: &'a str,
    pub scale_mode: PriceScaleMode,
    /// The chart draws a price-based type, which has no shared time axis to compare on.
    pub price_based: bool,
}

pub fn page(
    ui: &mut Ui,
    state: &mut CompareState,
    input: &CompareInput,
) -> Option<ChartSettingsAction> {
    let mut action = None;
    settings_card::section_title(
        ui,
        "Compare",
        Some("Draw other markets on this chart, on the same time axis."),
    );
    let overlays: Vec<ChartOverlay> = input.card.overlay_list().iter().cloned().collect();
    if overlays.is_empty() {
        state.confirm_remove_all = false;
        return empty_state(ui, input);
    }
    ui.spacing_mut().item_spacing.y = 8.0;
    if let Some(next) = toolbar(ui, state, &overlays) {
        action = Some(next);
    }
    if overlays.len() > SEARCH_FROM {
        kit::search_field(
            ui,
            Id::new("compare-search"),
            &mut state.query,
            "Search markets",
            ui.available_width(),
        );
    } else {
        // A filter nobody can see must not hide cards.
        state.query.clear();
    }
    if state.confirm_remove_all
        && let Some(next) = confirm_remove_all(ui, state, overlays.len())
    {
        action = Some(next);
    }
    if let Some(next) = axis_notice(ui, input, &overlays) {
        action = Some(next);
    }

    let query = state.query.trim().to_lowercase();
    let filtering = !query.is_empty();
    let own_scales: Vec<(OverlayId, String)> = overlays
        .iter()
        .filter(|overlay| overlay.scale == OverlayScale::Own(overlay.id))
        .map(|overlay| (overlay.id, overlay.name()))
        .collect();
    let mut shown = 0;
    for (index, overlay) in overlays.iter().enumerate() {
        if !matches_query(overlay, &query) {
            continue;
        }
        shown += 1;
        let place = Place {
            index,
            count: overlays.len(),
            // Moving a card inside a filtered list would not look like anything happened.
            can_move: !filtering,
        };
        if let Some(next) = overlay_card(ui, state, input, overlay, place, &own_scales) {
            action = Some(next);
        }
    }
    if shown == 0 {
        ui.add_space(8.0);
        ui.vertical_centered(|ui| {
            ui.label(kit::text(
                format!("No market matches “{}”", state.query.trim()),
                12.0,
                kit::REGULAR,
                theme::text_secondary(),
            ));
        });
    }
    action
}

// MARK: Empty state

fn empty_state(ui: &mut Ui, input: &CompareInput) -> Option<ChartSettingsAction> {
    let mut action = None;
    ui.add_space(8.0);
    kit::card_frame(false).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.vertical_centered(|ui| {
            ui.add_space(14.0);
            let (badge, _) = ui.allocate_exact_size(Vec2::splat(44.0), Sense::hover());
            kit::paint_card_badge(ui, badge, Icon::Compare);
            ui.add_space(10.0);
            ui.label(kit::text(
                "Compare markets",
                14.0,
                kit::SEMIBOLD,
                theme::text_primary(),
            ));
            ui.add_space(2.0);
            ui.add(
                egui::Label::new(kit::text(
                    format!(
                        "See how other markets move against {} on the same chart and time axis. \
                         Any number can be added, each with its own colour and scale.",
                        input.chart_name
                    ),
                    11.5,
                    kit::REGULAR,
                    theme::text_secondary(),
                ))
                .wrap(),
            );
            ui.add_space(14.0);
            if kit::sheet_button(ui, "Add symbol…", true, true).clicked() {
                action = Some(ChartSettingsAction::AddOverlay);
            }
            ui.add_space(12.0);
            ui.label(kit::text(
                "In the symbol search (⌘K), ⌥↵ adds the highlighted market to this chart.",
                10.5,
                kit::REGULAR,
                theme::text_tertiary(),
            ));
            ui.add_space(10.0);
        });
    });
    action
}

// MARK: Toolbar, notices

/// "3 markets", and how many of them are hidden.
pub fn summary(count: usize, hidden: usize) -> String {
    let noun = if count == 1 { "market" } else { "markets" };
    if hidden == 0 {
        format!("{count} {noun}")
    } else {
        format!("{count} {noun} · {hidden} hidden")
    }
}

fn toolbar(
    ui: &mut Ui,
    state: &mut CompareState,
    overlays: &[ChartOverlay],
) -> Option<ChartSettingsAction> {
    let mut action = None;
    let hidden = overlays.iter().filter(|overlay| !overlay.visible).count();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        if kit::sheet_button(ui, "Add symbol…", true, true).clicked() {
            action = Some(ChartSettingsAction::AddOverlay);
        }
        ui.label(kit::text(
            summary(overlays.len(), hidden),
            11.5,
            kit::REGULAR,
            theme::text_secondary(),
        ));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let more = icons::button(ui, Icon::Ellipsis, 26.0, theme::text_secondary(), "More");
            let labels = ["Show all", "Hide all", "Remove all…"];
            kit::fitted_menu(ui, &more, egui::RectAlign::BOTTOM_END, &labels, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                if kit::menu_item(ui, Icon::Eye, labels[0], hidden > 0, false) {
                    action = Some(ChartSettingsAction::Overlay(OverlayCommand::SetAllVisible(
                        true,
                    )));
                    ui.close();
                }
                if kit::menu_item(
                    ui,
                    Icon::EyeSlash,
                    labels[1],
                    hidden < overlays.len(),
                    false,
                ) {
                    action = Some(ChartSettingsAction::Overlay(OverlayCommand::SetAllVisible(
                        false,
                    )));
                    ui.close();
                }
                kit::menu_separator(ui);
                if kit::menu_item(ui, Icon::Trash, labels[2], true, true) {
                    state.confirm_remove_all = true;
                    ui.close();
                }
            });
        });
    });
    action
}

fn confirm_remove_all(
    ui: &mut Ui,
    state: &mut CompareState,
    count: usize,
) -> Option<ChartSettingsAction> {
    let mut action = None;
    kit::card_frame(false).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            icons::inline(ui, Icon::TriangleFill, 14.0, kit::ORANGE);
            ui.label(kit::text(
                format!(
                    "Stop comparing {}?",
                    if count == 1 {
                        "this market".to_owned()
                    } else {
                        format!("all {count} markets")
                    }
                ),
                12.0,
                kit::MEDIUM,
                theme::text_primary(),
            ));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if kit::sheet_button(ui, "Remove all", false, true).clicked() {
                    action = Some(ChartSettingsAction::Overlay(OverlayCommand::RemoveAll));
                    state.confirm_remove_all = false;
                }
                if kit::sheet_button(ui, "Cancel", false, true).clicked() {
                    state.confirm_remove_all = false;
                }
            });
        });
    });
    action
}

/// Whether a market drawn on the chart's own scale is being read on an axis that cannot compare it:
/// the line is rebased to start where the chart does, which only means something as a change.
pub fn needs_relative_axis(overlays: &[ChartOverlay], mode: PriceScaleMode) -> bool {
    matches!(mode, PriceScaleMode::Regular | PriceScaleMode::Logarithmic)
        && overlays
            .iter()
            .any(|overlay| overlay.visible && overlay.scale == OverlayScale::Main)
}

fn axis_notice(
    ui: &mut Ui,
    input: &CompareInput,
    overlays: &[ChartOverlay],
) -> Option<ChartSettingsAction> {
    if !needs_relative_axis(overlays, input.scale_mode) {
        return None;
    }
    let mut action = None;
    kit::card_frame(false).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 4.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            icons::inline(ui, Icon::InfoCircle, 14.0, theme::text_secondary());
            ui.label(kit::text(
                "The price axis can't compare these markets",
                12.0,
                kit::SEMIBOLD,
                theme::text_primary(),
            ));
        });
        ui.add(
            egui::Label::new(kit::text(
                "Markets on the chart's scale are drawn from where the chart starts, so their \
                 prices only line up as a change. Read them on a relative axis:",
                11.0,
                kit::REGULAR,
                theme::text_secondary(),
            ))
            .wrap(),
        );
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if kit::sheet_button(ui, "Percent", true, true).clicked() {
                action = Some(ChartSettingsAction::SetScaleMode(PriceScaleMode::Percent));
            }
            if kit::sheet_button(ui, "Indexed to 100", false, true).clicked() {
                action = Some(ChartSettingsAction::SetScaleMode(
                    PriceScaleMode::IndexedTo100,
                ));
            }
        });
    });
    action
}

// MARK: Cards

/// Where a card sits in the list.
#[derive(Clone, Copy)]
struct Place {
    index: usize,
    count: usize,
    can_move: bool,
}

/// Whether `overlay` matches what the search field holds (already trimmed and lower-cased).
pub fn matches_query(overlay: &ChartOverlay, query: &str) -> bool {
    query.is_empty()
        || overlay.name().to_lowercase().contains(query)
        || overlay.instrument.symbol.to_lowercase().contains(query)
        || overlay
            .instrument
            .source
            .display_name()
            .to_lowercase()
            .contains(query)
}

/// How a market's data is doing, in words.
#[derive(Debug, Eq, PartialEq)]
pub enum Health {
    Fine,
    /// Worth saying, not a problem (loading, waiting).
    Note(String),
    /// Something the user may have to act on.
    Problem(String),
}

pub fn health(status: Option<&OverlayStatus>, price_based: bool) -> Health {
    if price_based {
        return Health::Problem("Not shown on price-based chart types".to_owned());
    }
    match status {
        Some(OverlayStatus::Error(message)) => Health::Problem(format!("Couldn't load: {message}")),
        Some(OverlayStatus::Blocked(reason)) => Health::Problem((*reason).to_owned()),
        Some(OverlayStatus::Loading) => Health::Note("Loading…".to_owned()),
        Some(OverlayStatus::Queued) => Health::Note("Waiting to load".to_owned()),
        Some(OverlayStatus::Ready) | None => Health::Fine,
    }
}

fn overlay_card(
    ui: &mut Ui,
    state: &mut CompareState,
    input: &CompareInput,
    overlay: &ChartOverlay,
    place: Place,
    own_scales: &[(OverlayId, String)],
) -> Option<ChartSettingsAction> {
    let mut action = None;
    let id = ui.id().with(("compare-card", overlay.id));
    let open = state.is_open(overlay.id, place.count);
    let dim = if overlay.visible { 1.0 } else { 0.5 };
    let width = ui.available_width();

    let mut prepared = kit::card_frame(false).begin(ui);
    let inner = &mut prepared.content_ui;
    let inner_width = width - 28.0 - 2.0;
    inner.set_width(inner_width);

    // The whole header toggles the card; made first so the buttons over it win their clicks.
    let header = Rect::from_min_size(inner.cursor().min, vec2(inner_width, 34.0));
    let toggle = inner.interact(header, id.with("toggle"), Sense::click());
    inner.allocate_ui_with_layout(header.size(), Layout::left_to_right(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        let (tile, _) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::hover());
        paint_line_tile(ui, tile, overlay, dim);
        let controls = 3.0 * 22.0 + 2.0 * 4.0 + 8.0;
        ui.vertical(|ui| {
            ui.set_max_width((inner_width - 30.0 - 12.0 - controls).max(60.0));
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(kit::text(
                overlay.name(),
                12.0,
                kit::SEMIBOLD,
                theme::with_alpha(theme::text_primary(), dim),
            ));
            subtitle(ui, input, overlay, dim);
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let chevron = icons::button(
                ui,
                if open {
                    Icon::ChevronUp
                } else {
                    Icon::ChevronDown
                },
                22.0,
                theme::text_secondary(),
                if open {
                    "Hide settings"
                } else {
                    "Show settings"
                },
            );
            if chevron.clicked() {
                state.open.insert(overlay.id, !open);
            }
            let more = icons::button(ui, Icon::Ellipsis, 22.0, theme::text_secondary(), "More");
            if let Some(next) = card_menu(ui, &more, overlay, place) {
                action = Some(next);
            }
            let (eye, tip) = if overlay.visible {
                (Icon::Eye, "Hide on the chart")
            } else {
                (Icon::EyeSlash, "Show on the chart")
            };
            let eye = icons::button(ui, eye, 22.0, theme::text_secondary(), tip);
            if eye.clicked() {
                action = Some(ChartSettingsAction::Overlay(OverlayCommand::SetVisible(
                    overlay.id,
                    !overlay.visible,
                )));
            }
        });
    });
    if toggle.clicked() {
        state.open.insert(overlay.id, !open);
    }
    if toggle.hovered() {
        toggle.on_hover_cursor(egui::CursorIcon::PointingHand);
    }

    if open {
        inner.add_space(10.0);
        let rule = inner.allocate_space(vec2(inner_width, 1.0)).1;
        inner.painter().rect_filled(rule, 0.0, theme::separator());
        inner.add_space(12.0);
        if let Some(next) = details(inner, input, overlay, own_scales) {
            action = Some(next);
        }
        inner.add_space(2.0);
    }
    let hovered = ui.rect_contains_pointer(prepared.content_ui.min_rect().expand(12.0));
    prepared.frame = kit::card_frame(hovered);
    prepared.end(ui);
    action
}

/// The tile at a card's left: the line as it is drawn, in its colour, width and style.
fn paint_line_tile(ui: &Ui, tile: Rect, overlay: &ChartOverlay, dim: f32) {
    ui.painter().rect_filled(tile, 8.0, theme::ink(10));
    ui.painter().rect_stroke(
        tile,
        8.0,
        Stroke::new(1.0, theme::ink(14)),
        egui::StrokeKind::Inside,
    );
    let color = theme::with_alpha(rgb(overlay.color), dim);
    let stroke = Stroke::new(overlay.width.clamp(1.0, 4.0), color);
    let (from, to) = (
        pos2(tile.left() + 7.0, tile.center().y),
        pos2(tile.right() - 7.0, tile.center().y),
    );
    match overlay.dash {
        LineDash::Solid => {
            ui.painter().line_segment([from, to], stroke);
        }
        LineDash::Dashed => {
            ui.painter()
                .extend(egui::Shape::dashed_line(&[from, to], stroke, 4.0, 3.0));
        }
        LineDash::Dotted => {
            ui.painter().extend(egui::Shape::dashed_line(
                &[from, to],
                stroke,
                stroke.width,
                3.0,
            ));
        }
    }
}

/// The exchange, then how the data is doing or, when it is fine, the last price.
fn subtitle(ui: &mut Ui, input: &CompareInput, overlay: &ChartOverlay, dim: f32) {
    let feed = input.card.overlay_feed(overlay.id);
    let secondary = theme::with_alpha(theme::text_secondary(), dim);
    let exchange = overlay.instrument.source.display_name();
    let health = health(feed.map(|feed| &feed.status), input.price_based);
    // A row of small text: not the height of a button. Set before the row, which reads it when it
    // starts.
    ui.scope(|ui| {
        ui.spacing_mut().interact_size.y = 14.0;
        subtitle_row(ui, input, overlay, secondary, exchange, &health);
    });
}

fn subtitle_row(
    ui: &mut Ui,
    input: &CompareInput,
    overlay: &ChartOverlay,
    secondary: Color32,
    exchange: &str,
    health: &Health,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        if let Health::Problem(message) = health {
            ui.label(kit::text(exchange, 10.5, kit::REGULAR, secondary));
            icons::inline(ui, Icon::TriangleFill, 9.0, kit::ORANGE);
            ui.add(
                egui::Label::new(kit::text(message, 10.5, kit::REGULAR, kit::ORANGE)).truncate(),
            );
            return;
        }
        let mut line = exchange.to_owned();
        match health {
            Health::Note(message) => line = format!("{line} · {message}"),
            _ => match input.card.overlay_last_price(overlay.id) {
                Some(price) => {
                    let format = input.card.appearance().price_format();
                    line = format!("{line} · {}", format.format(price));
                }
                None => line.push_str(" · No data"),
            },
        }
        if !overlay.visible {
            line.push_str(" · Hidden");
        }
        ui.label(kit::text(&line, 10.5, kit::REGULAR, secondary));
    });
}

/// The card's menu: its place in the drawing order, and removing it.
fn card_menu(
    ui: &mut Ui,
    anchor: &egui::Response,
    overlay: &ChartOverlay,
    place: Place,
) -> Option<ChartSettingsAction> {
    let mut action = None;
    let labels = [
        "Bring to front",
        "Bring forward",
        "Send backward",
        "Send to back",
        "Remove",
    ];
    let can_forward = place.can_move && place.index + 1 < place.count;
    let can_back = place.can_move && place.index > 0;
    kit::fitted_menu(ui, anchor, egui::RectAlign::BOTTOM_END, &labels, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let mut send = |ui: &mut Ui, command: OverlayCommand| {
            action = Some(ChartSettingsAction::Overlay(command));
            ui.close();
        };
        for (label, to, enabled) in [
            (labels[0], Move::ToFront, can_forward),
            (labels[1], Move::Forward, can_forward),
            (labels[2], Move::Backward, can_back),
            (labels[3], Move::ToBack, can_back),
        ] {
            if kit::menu_item(ui, Icon::ArrowUp, label, enabled, false) {
                send(ui, OverlayCommand::Move(overlay.id, to));
            }
        }
        kit::menu_separator(ui);
        if kit::menu_item(ui, Icon::Trash, labels[4], true, true) {
            send(ui, OverlayCommand::Remove(overlay.id));
        }
    });
    action
}

/// The settings under a card's chevron.
fn details(
    ui: &mut Ui,
    input: &CompareInput,
    overlay: &ChartOverlay,
    own_scales: &[(OverlayId, String)],
) -> Option<ChartSettingsAction> {
    overlay_settings::body(
        ui,
        overlay,
        own_scales,
        input.scale_mode,
        overlay_settings::Density::Roomy,
    )
    .map(ChartSettingsAction::Overlay)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DataSource;
    use crate::model::InstrumentId;
    use crate::model::overlay::OverlayList;
    use crate::model::overlay::ScaleChoice;
    use crate::model::overlay_feed::OverlayFeed;
    use crate::ui::overlay_settings::scale_hint;

    fn list_of(symbols: &[&str]) -> OverlayList {
        let main = InstrumentId::new(DataSource::Binance, "BTCUSDT");
        let mut list = OverlayList::new();
        for symbol in symbols {
            list.add(&main, InstrumentId::new(DataSource::Binance, *symbol), None);
        }
        list
    }

    fn card_with(list: OverlayList, statuses: Vec<OverlayStatus>) -> ChartCard {
        let feeds = statuses.into_iter().map(OverlayFeed::empty).collect();
        let mut card = ChartCard::default();
        card.set_overlays(list, feeds, 1, DataSource::Binance, "BTCUSDT");
        card
    }

    /// The text the page paints, after a few frames so layout settles.
    fn texts(card: &ChartCard, mode: PriceScaleMode, state: &mut CompareState) -> Vec<String> {
        let ctx = egui::Context::default();
        crate::ui::theme::install_offscreen(&ctx);
        let mut found = Vec::new();
        for _ in 0..3 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                let input = CompareInput {
                    card,
                    chart_name: "BTC/USDT",
                    scale_mode: mode,
                    price_based: false,
                };
                let _ = page(ui, state, &input);
            });
            found.clear();
            fn walk(shape: &egui::Shape, found: &mut Vec<String>) {
                match shape {
                    egui::Shape::Text(text) => found.push(text.galley.text().to_owned()),
                    egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, found)),
                    _ => {}
                }
            }
            for clipped in &output.shapes {
                walk(&clipped.shape, &mut found);
            }
            output.textures_delta.clear();
        }
        found
    }

    #[test]
    fn the_summary_counts_markets_and_the_hidden_ones() {
        assert_eq!(summary(1, 0), "1 market");
        assert_eq!(summary(3, 0), "3 markets");
        assert_eq!(summary(12, 2), "12 markets · 2 hidden");
    }

    #[test]
    fn a_card_starts_open_only_while_there_are_a_few_and_remembers_the_user() {
        let mut state = CompareState::default();
        let id = OverlayId(1);
        assert!(state.is_open(id, 1));
        assert!(state.is_open(id, OPEN_UP_TO));
        assert!(!state.is_open(id, OPEN_UP_TO + 1));
        state.open.insert(id, true);
        assert!(
            state.is_open(id, 40),
            "opened by hand stays open among many"
        );
        state.open.insert(id, false);
        assert!(
            !state.is_open(id, 1),
            "closed by hand stays closed among few"
        );
    }

    #[test]
    fn the_search_matches_the_name_the_symbol_and_the_exchange() {
        let list = list_of(&["ETHUSDT", "SOLUSDT"]);
        let eth = &list.as_slice()[0];
        for query in ["", "eth", "usdt", "binance", "ethusdt"] {
            assert!(matches_query(eth, query), "{query}");
        }
        assert!(!matches_query(eth, "sol"));
        let titled = {
            let main = InstrumentId::new(DataSource::Binance, "BTCUSDT");
            let mut list = OverlayList::new();
            list.add(
                &main,
                InstrumentId::new(DataSource::DexScreener, "0xpair"),
                Some("PEPE/WETH".to_owned()),
            );
            list.as_slice()[0].clone()
        };
        assert!(
            matches_query(&titled, "pepe"),
            "a DEX pair is found by its title"
        );
    }

    #[test]
    fn what_is_wrong_with_a_market_is_said_in_words() {
        assert_eq!(health(Some(&OverlayStatus::Ready), false), Health::Fine);
        assert_eq!(health(None, false), Health::Fine);
        assert_eq!(
            health(Some(&OverlayStatus::Loading), false),
            Health::Note("Loading…".into())
        );
        assert_eq!(
            health(Some(&OverlayStatus::Error("HTTP 429".into())), false),
            Health::Problem("Couldn't load: HTTP 429".into())
        );
        assert_eq!(
            health(
                Some(&OverlayStatus::Blocked("Same market as the chart")),
                false
            ),
            Health::Problem("Same market as the chart".into())
        );
        // A price-based chart type outranks whatever the data is doing.
        assert!(matches!(
            health(Some(&OverlayStatus::Ready), true),
            Health::Problem(_)
        ));
    }

    #[test]
    fn only_markets_on_the_charts_scale_need_a_relative_axis() {
        let mut list = list_of(&["ETHUSDT", "SOLUSDT"]);
        let overlays = |list: &OverlayList| list.iter().cloned().collect::<Vec<_>>();
        assert!(needs_relative_axis(
            &overlays(&list),
            PriceScaleMode::Regular
        ));
        assert!(needs_relative_axis(
            &overlays(&list),
            PriceScaleMode::Logarithmic
        ));
        assert!(!needs_relative_axis(
            &overlays(&list),
            PriceScaleMode::Percent
        ));
        assert!(!needs_relative_axis(
            &overlays(&list),
            PriceScaleMode::IndexedTo100
        ));
        list.apply(OverlayCommand::SetScale(OverlayId(1), ScaleChoice::Own));
        list.apply(OverlayCommand::SetVisible(OverlayId(2), false));
        assert!(!needs_relative_axis(
            &overlays(&list),
            PriceScaleMode::Regular
        ));
    }

    #[test]
    fn each_scale_says_what_it_does() {
        let mut list = list_of(&["ETHUSDT", "SOLUSDT", "BNBUSDT"]);
        list.apply(OverlayCommand::SetScale(OverlayId(2), ScaleChoice::Own));
        list.apply(OverlayCommand::SetScale(
            OverlayId(3),
            ScaleChoice::SharedWith(OverlayId(2)),
        ));
        let own: Vec<_> = vec![(OverlayId(2), "SOLUSDT".to_owned())];
        let hint = |n: u64, mode| scale_hint(list.get(OverlayId(n)).unwrap(), &own, mode);
        assert!(hint(1, PriceScaleMode::Percent).contains("own change"));
        assert!(hint(1, PriceScaleMode::Regular).contains("Percent or Indexed"));
        assert!(hint(2, PriceScaleMode::Percent).contains("tag and the legend"));
        assert!(hint(3, PriceScaleMode::Percent).contains("SOLUSDT"));
    }

    #[test]
    fn an_empty_chart_gets_an_invitation_not_an_empty_list() {
        let card = ChartCard::default();
        let found = texts(&card, PriceScaleMode::Percent, &mut CompareState::default());
        assert!(found.iter().any(|t| t == "Compare markets"), "{found:?}");
        assert!(found.iter().any(|t| t == "Add symbol…"));
        assert!(
            found
                .iter()
                .any(|t| t.contains("against BTC/USDT") && !t.contains('\\')),
            "no stray characters: {found:?}"
        );
        assert!(found.iter().all(|t| !t.contains("markets ·")));
    }

    #[test]
    fn a_few_markets_show_their_settings_and_many_fold_them() {
        let few = card_with(
            list_of(&["ETHUSDT", "SOLUSDT"]),
            vec![OverlayStatus::Ready; 2],
        );
        let found = texts(&few, PriceScaleMode::Percent, &mut CompareState::default());
        for wanted in [
            "2 markets",
            "ETHUSDT",
            "Colour",
            "Line",
            "Price",
            "Scale",
            "Last value tag",
        ] {
            assert!(found.iter().any(|t| t == wanted), "{wanted}: {found:?}");
        }
        assert!(
            found.iter().all(|t| t != "Search markets"),
            "no search for two"
        );

        let symbols: Vec<String> = (0..9).map(|n| format!("S{n}USDT")).collect();
        let refs: Vec<&str> = symbols.iter().map(String::as_str).collect();
        let many = card_with(list_of(&refs), vec![OverlayStatus::Ready; 9]);
        let found = texts(&many, PriceScaleMode::Percent, &mut CompareState::default());
        assert!(found.iter().any(|t| t == "9 markets"));
        assert!(
            found.iter().all(|t| t != "Last value tag"),
            "folded: {found:?}"
        );
        assert!(found.iter().any(|t| t == "S0USDT"));
    }

    #[test]
    fn a_failing_market_is_flagged_in_its_header_without_opening_it() {
        let symbols: Vec<String> = (0..5).map(|n| format!("S{n}USDT")).collect();
        let refs: Vec<&str> = symbols.iter().map(String::as_str).collect();
        let mut statuses = vec![OverlayStatus::Ready; 5];
        statuses[2] = OverlayStatus::Error("HTTP 429".into());
        let card = card_with(list_of(&refs), statuses);
        let found = texts(&card, PriceScaleMode::Percent, &mut CompareState::default());
        assert!(
            found.iter().any(|t| t == "Couldn't load: HTTP 429"),
            "{found:?}"
        );
    }

    #[test]
    fn the_axis_notice_shows_only_while_the_axis_cannot_compare() {
        let card = card_with(list_of(&["ETHUSDT"]), vec![OverlayStatus::Ready]);
        let regular = texts(&card, PriceScaleMode::Regular, &mut CompareState::default());
        assert!(regular.iter().any(|t| t == "Percent"), "{regular:?}");
        assert!(regular.iter().any(|t| t == "Indexed to 100"));
        let percent = texts(&card, PriceScaleMode::Percent, &mut CompareState::default());
        assert!(percent.iter().all(|t| t != "Indexed to 100"), "{percent:?}");
    }

    #[test]
    fn remove_all_waits_for_an_answer() {
        let card = card_with(
            list_of(&["ETHUSDT", "SOLUSDT"]),
            vec![OverlayStatus::Ready; 2],
        );
        let mut state = CompareState {
            confirm_remove_all: true,
            ..CompareState::default()
        };
        let found = texts(&card, PriceScaleMode::Percent, &mut state);
        assert!(
            found.iter().any(|t| t == "Stop comparing all 2 markets?"),
            "{found:?}"
        );
        assert!(found.iter().any(|t| t == "Cancel"));
        // With nothing left to remove the question goes away on its own.
        let empty = ChartCard::default();
        texts(&empty, PriceScaleMode::Percent, &mut state);
        assert!(!state.confirm_remove_all);
    }
}

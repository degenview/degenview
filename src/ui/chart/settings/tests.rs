use super::appearance::{PRICE_SAMPLES, decimals_hint, price_axis, scale_hint, small_prices_hint};
use super::scripts::{RowEvent, apply_events};
use super::*;
use crate::model::chart::price_format::MAX_DECIMALS;
use crate::model::chart::price_scale::PriceScaleMode;
use crate::model::pine::script::{LocalScript, ScriptType};
use crate::model::{DataSource, IndicatorConfig, PriceFormat, SmallPrices};
use crate::store::SharedStore;
use crate::store::Store;
use crate::ui::chart::card::ChartCard;
use crate::ui::pine::script_catalog;
use eframe::egui;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

fn test_store() -> SharedStore {
    Arc::new(Mutex::new(Store::open_in_memory().expect("open")))
}

fn draw(state: &mut ChartSettingsState, card: &mut ChartCard, indicators: &mut IndicatorConfig) {
    draw_in(&egui::Context::default(), state, card, indicators);
}

fn draw_in(
    ctx: &egui::Context,
    state: &mut ChartSettingsState,
    card: &mut ChartCard,
    indicators: &mut IndicatorConfig,
) {
    let store = test_store();
    for _ in 0..2 {
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let _ = show(
                ui.ctx(),
                state,
                ChartSettingsContext {
                    title: "BTC/USDT",
                    symbol: "BTCUSDT",
                    source: DataSource::Binance,
                    loaded_candles: 0,
                    indicators,
                    card,
                    scale_mode: PriceScaleMode::Regular,
                    store: &store,
                },
            );
        });
        output.textures_delta.clear();
    }
}

#[test]
fn coinmarketcap_charts_offer_only_the_pages_that_apply_to_them() {
    assert_eq!(
        Tab::for_source(DataSource::CoinMarketCap),
        &[Tab::Appearance, Tab::Canvas]
    );
    assert_eq!(Tab::for_source(DataSource::Binance), &Tab::ALL);
}

#[test]
fn the_sheet_draws_each_page_empty_and_with_scripts() {
    let mut state = ChartSettingsState::default();
    let mut card = ChartCard::default();
    let mut indicators = IndicatorConfig::default();
    for tab in Tab::ALL {
        state.tab = tab;
        draw(&mut state, &mut card, &mut indicators);
    }
    card.add_pine_instance(
        None,
        "RSI",
        "//@version=6\nindicator(\"x\")\nlen = input.int(14)\nplot(close)\n",
    );
    card.add_pine_instance(
        None,
        "Strat",
        "//@version=6\nstrategy(\"s\")\nplot(close)\n",
    );
    card.set_pine_visible(2, false);
    state.tab = Tab::Scripts;
    draw(&mut state, &mut card, &mut indicators);
}

#[test]
fn the_compare_page_lists_every_market_however_many_and_says_what_is_wrong_with_one() {
    use crate::model::overlay::OverlayList;
    use crate::model::overlay::feed::{OverlayFeed, OverlayStatus};
    let main = crate::model::InstrumentId::new(DataSource::Binance, "BTCUSDT");
    let mut list = OverlayList::new();
    let mut feeds = Vec::new();
    for n in 0..40 {
        list.add(
            &main,
            crate::model::InstrumentId::new(DataSource::Binance, format!("S{n}USDT")),
            None,
        );
        feeds.push(OverlayFeed::empty(match n {
            3 => OverlayStatus::Error("no such market".into()),
            4 => OverlayStatus::Blocked("Same market as the chart"),
            _ => OverlayStatus::Ready,
        }));
    }
    let mut card = ChartCard::default();
    card.set_overlays(list, feeds, 1, DataSource::Binance, "BTCUSDT");
    let mut state = ChartSettingsState {
        tab: Tab::Overlays,
        ..ChartSettingsState::default()
    };
    let mut indicators = IndicatorConfig::default();
    let ctx = egui::Context::default();
    let store = test_store();
    let mut texts = Vec::new();
    for _ in 0..2 {
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let _ = show(
                ui.ctx(),
                &mut state,
                ChartSettingsContext {
                    title: "BTC/USDT",
                    symbol: "BTCUSDT",
                    source: DataSource::Binance,
                    loaded_candles: 0,
                    indicators: &mut indicators,
                    card: &mut card,
                    scale_mode: PriceScaleMode::Percent,
                    store: &store,
                },
            );
        });
        texts.clear();
        fn walk(shape: &egui::Shape, texts: &mut Vec<String>) {
            match shape {
                egui::Shape::Text(text) => texts.push(text.galley.text().to_owned()),
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, texts)),
                _ => {}
            }
        }
        for clipped in &output.shapes {
            walk(&clipped.shape, &mut texts);
        }
        output.textures_delta.clear();
    }
    let has = |needle: &str| texts.iter().any(|text| text.contains(needle));
    assert!(has("40 markets"), "{texts:?}");
    assert!(has("Add symbol"));
    assert!(has("S0USDT"));
    assert!(has("Couldn't load: no such market"));
    assert!(has("Same market as the chart"));
}

fn library() -> Vec<LocalScript> {
    use crate::ui::pine::script_catalog::sample;
    (0..9)
        .map(|i| {
            sample(
                &i.to_string(),
                &format!("Script {i}"),
                if i % 3 == 0 {
                    ScriptType::Strategy
                } else {
                    ScriptType::Indicator
                },
            )
        })
        .collect()
}

#[test]
fn the_scripts_page_lists_saved_scripts_with_and_without_a_search_field() {
    let ctx = egui::Context::default();
    let mut state = ChartSettingsState {
        tab: Tab::Scripts,
        ..ChartSettingsState::default()
    };
    let mut card = ChartCard::default();
    let mut indicators = IndicatorConfig::default();
    script_catalog::publish(&ctx, library());
    draw_in(&ctx, &mut state, &mut card, &mut indicators);
    // One script on the chart moves out of "Available"; a search narrows both lists.
    let saved = script_catalog::scripts(&ctx);
    card.add_pine_instance(Some(saved[0].id.clone()), &saved[0].name, &saved[0].source);
    state.script_query = "script 4".to_owned();
    draw_in(&ctx, &mut state, &mut card, &mut indicators);
    state.script_query = "nothing like it".to_owned();
    draw_in(&ctx, &mut state, &mut card, &mut indicators);
    // A short library has no search field, and a stale query must not hide its rows.
    script_catalog::publish(&ctx, library().into_iter().take(2).collect());
    draw_in(&ctx, &mut state, &mut card, &mut indicators);
    assert!(state.script_query.is_empty());
}

#[test]
fn switching_a_script_on_applies_it_and_off_keeps_its_settings() {
    use crate::pine::compiler::program::InputValue;
    let script = Arc::new(crate::ui::pine::script_catalog::sample(
        "abc",
        "Squeeze",
        ScriptType::Indicator,
    ));
    let mut card = ChartCard::default();
    apply_events(&mut card, &[], vec![RowEvent::Enable(script.clone())]);
    let instances = card.pine_instances().to_vec();
    assert_eq!(instances.len(), 1);
    assert_eq!(instances[0].script_id.as_deref(), Some("abc"));
    assert!(instances[0].visible);

    card.set_pine_instance_inputs(
        instances[0].id,
        &HashMap::from([("len".to_owned(), InputValue::Int(21))]),
    );
    card.set_pine_instance_style(
        instances[0].id,
        HashMap::from([("plot.0.color".to_owned(), "#ff0000".to_owned())]),
    );
    let id = instances[0].id;
    apply_events(&mut card, &instances, vec![RowEvent::ToggleVisible(id)]);
    let off = card.pine_instances()[0].clone();
    assert!(!off.visible);
    assert!(!off.inputs.is_empty() && !off.style_overrides.is_empty());

    let instances = card.pine_instances().to_vec();
    apply_events(&mut card, &instances, vec![RowEvent::ToggleVisible(id)]);
    let on = card.pine_instances()[0].clone();
    assert!(on.visible);
    assert_eq!(on.inputs, off.inputs);
    assert_eq!(card.pine_instances().len(), 1, "never a second instance");

    let instances = card.pine_instances().to_vec();
    apply_events(&mut card, &instances, vec![RowEvent::Duplicate(id)]);
    assert_eq!(card.pine_instances().len(), 2);
    assert!(card.pine_instances()[1].inputs.is_empty());
    let instances = card.pine_instances().to_vec();
    apply_events(&mut card, &instances, vec![RowEvent::Remove(id)]);
    assert_eq!(card.pine_instances().len(), 1);
}

#[test]
fn the_sheet_draws_each_line_source() {
    let mut state = ChartSettingsState::default();
    let mut card = ChartCard::default();
    let mut indicators = IndicatorConfig::default();
    draw(&mut state, &mut card, &mut indicators);
    state.tab = Tab::Appearance;
    draw(&mut state, &mut card, &mut indicators);
}

#[test]
fn the_price_axis_hints_say_what_each_choice_does() {
    assert!(decimals_hint(None).contains("Picks the precision"));
    assert_eq!(
        decimals_hint(Some(1)),
        "Shows 1 decimal place on the price axis."
    );
    assert_eq!(
        decimals_hint(Some(0)),
        "Shows 0 decimal places on the price axis."
    );
    assert_eq!(
        decimals_hint(Some(MAX_DECIMALS)),
        "Shows 12 decimal places on the price axis."
    );
    for style in SmallPrices::ALL {
        let hint = small_prices_hint(style);
        assert!(hint.contains("0.001") && !hint.contains("  "), "{hint}");
    }
    assert!(small_prices_hint(SmallPrices::ZeroCount).contains(SmallPrices::ZeroCount.example()));
}

#[test]
fn the_example_prices_follow_the_chosen_format() {
    let write = |format: PriceFormat| PRICE_SAMPLES.map(|(_, price)| format.format(price));
    assert_eq!(write(PriceFormat::AUTO), ["67,432", "0.12", "0.0₅278"]);
    let pinned = PriceFormat {
        decimals: Some(2),
        small: SmallPrices::Decimals,
    };
    assert_eq!(write(pinned), ["67,432.19", "0.12", "0.00"]);
}

#[test]
fn tabs_have_titles() {
    assert_eq!(
        Tab::ALL.map(Tab::title),
        ["Indicators", "Scripts", "Compare", "Appearance", "Canvas"]
    );
}

#[test]
fn the_sheet_uses_more_of_a_tall_parent_without_crowding_a_small_one() {
    assert_eq!(page_height(480.0), MIN_PAGE_HEIGHT);
    assert_eq!(page_height(800.0), 550.0);
    assert_eq!(page_height(900.0), MAX_PAGE_HEIGHT);
    assert_eq!(page_height(1_400.0), MAX_PAGE_HEIGHT);
}

fn shape_text(shape: &egui::Shape, out: &mut String) {
    match shape {
        egui::Shape::Text(text) => {
            out.push_str(text.galley.text());
            out.push('\n');
        }
        egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| shape_text(shape, out)),
        _ => {}
    }
}

/// Every piece of text the Appearance page draws for a chart from `source`.
fn appearance_text(source: DataSource) -> String {
    let ctx = egui::Context::default();
    let store = test_store();
    let mut state = ChartSettingsState {
        tab: Tab::Appearance,
        ..ChartSettingsState::default()
    };
    let mut card = ChartCard::default();
    let mut indicators = IndicatorConfig::default();
    let mut text = String::new();
    for _ in 0..2 {
        text.clear();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let _ = show(
                ui.ctx(),
                &mut state,
                ChartSettingsContext {
                    title: "Chart",
                    symbol: "SYM",
                    source,
                    loaded_candles: 0,
                    indicators: &mut indicators,
                    card: &mut card,
                    scale_mode: PriceScaleMode::Regular,
                    store: &store,
                },
            );
        });
        for clipped in &output.shapes {
            shape_text(&clipped.shape, &mut text);
        }
        output.textures_delta.clear();
    }
    text
}

#[test]
fn the_price_axis_section_names_the_scale_and_says_what_each_mode_reads() {
    let hints: Vec<&str> = PriceScaleMode::ALL.into_iter().map(scale_hint).collect();
    for (index, hint) in hints.iter().enumerate() {
        assert!(!hint.is_empty());
        assert!(hints.iter().skip(index + 1).all(|other| other != hint));
    }
    for mode in PriceScaleMode::ALL {
        let ctx = egui::Context::default();
        let mut card = ChartCard::default();
        let mut text = String::new();
        let mut picked = Some(PriceScaleMode::Regular);
        for _ in 0..2 {
            text.clear();
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                picked = price_axis(ui, &mut card, mode);
            });
            for clipped in &output.shapes {
                shape_text(&clipped.shape, &mut text);
            }
            output.textures_delta.clear();
        }
        assert!(text.contains("Scale"), "{mode:?}: {text}");
        assert!(text.contains(scale_hint(mode)), "{mode:?}: {text}");
        assert_eq!(picked, None, "drawing the section picks nothing");
    }
}

#[test]
fn a_candle_chart_has_one_set_of_colours_and_a_line_chart_keeps_its_own() {
    let candles = appearance_text(DataSource::Binance);
    assert!(candles.contains("Chart type") && candles.contains("Preview"));
    // Rows below the fold are culled in a small test window, so look at the first one.
    assert!(
        candles.contains("Show candle bodies."),
        "the type's own settings: {candles}"
    );
    assert!(!candles.contains("Bullish candle") && !candles.contains("Bearish candle"));
    assert!(!candles.contains("Reset Colors"));
    let line = appearance_text(DataSource::CoinMarketCap);
    assert!(line.contains("Line up") && line.contains("Reset Colors"));
    assert!(!line.contains("Chart type"));
}

#[test]
fn the_preview_sits_between_the_style_picker_and_the_settings() {
    let text = appearance_text(DataSource::Binance);
    let (style, preview, body) = (
        text.find("Style").expect("picker"),
        text.find("Preview").expect("preview"),
        text.find("Show candle bodies.").expect("settings"),
    );
    assert!(style < preview && preview < body, "{text}");
}

/// The Scripts page as a picture. Run by hand with `DEGENVIEW_GALLERY_DIR` set.
#[test]
#[ignore = "writes images for a person to look at"]
fn gallery_of_the_scripts_page() {
    use crate::ui::render_tests::{Screen, gallery_dir};
    if gallery_dir().is_none() {
        return;
    }
    for (name, typed, applied) in [
        ("scripts-page", "", 2),
        ("scripts-page-search", "sq", 2),
        ("scripts-page-none", "", 0),
    ] {
        let mut screen = Screen::new(egui::vec2(820.0, 780.0));
        script_catalog::publish(&screen.ctx, library());
        let mut state = ChartSettingsState {
            tab: Tab::Scripts,
            script_query: typed.to_owned(),
            ..ChartSettingsState::default()
        };
        let mut card = ChartCard::default();
        let saved = script_catalog::scripts(&screen.ctx);
        for script in saved.iter().take(applied) {
            card.add_pine_instance(Some(script.id.clone()), &script.name, &script.source);
        }
        if applied > 1 {
            card.set_pine_visible(2, false);
        }
        let mut indicators = IndicatorConfig::default();
        let store = test_store();
        let mut last = None;
        for _ in 0..4 {
            last = Some(screen.run(Vec::new(), |ui| {
                let _ = show(
                    ui.ctx(),
                    &mut state,
                    ChartSettingsContext {
                        title: "BTC/USDT",
                        symbol: "BTCUSDT",
                        source: DataSource::Binance,
                        loaded_candles: 0,
                        indicators: &mut indicators,
                        card: &mut card,
                        scale_mode: PriceScaleMode::Regular,
                        store: &store,
                    },
                );
            }));
        }
        screen.save(name, last.expect("a frame"));
    }
}

/// A card comparing `count` markets, a few of them in trouble, for the Compare page's pictures.
fn card_with_overlays(count: usize) -> ChartCard {
    use crate::model::overlay::feed::{OverlayFeed, OverlayStatus};
    use crate::model::overlay::{OverlayCommand, OverlayList, ScaleChoice};
    let main = crate::model::InstrumentId::new(DataSource::Binance, "BTCUSDT");
    let names = [
        "ETHUSDT",
        "SOLUSDT",
        "BNBUSDT",
        "DOGEUSDT",
        "XRPUSDT",
        "ADAUSDT",
        "AVAXUSDT",
        "LINKUSDT",
        "DOTUSDT",
        "MATICUSDT",
        "LTCUSDT",
        "ATOMUSDT",
    ];
    let mut list = OverlayList::new();
    for n in 0..count {
        let name = names
            .get(n)
            .map_or_else(|| format!("COIN{n}USDT"), |name| (*name).to_owned());
        list.add(
            &main,
            crate::model::InstrumentId::new(DataSource::Binance, name),
            None,
        );
    }
    let ids: Vec<_> = list.iter().map(|overlay| overlay.id).collect();
    if let Some(id) = ids.get(3) {
        list.apply(OverlayCommand::SetScale(*id, ScaleChoice::Own));
    }
    if let Some(id) = ids.get(4) {
        list.apply(OverlayCommand::SetScale(
            *id,
            ScaleChoice::SharedWith(ids[3]),
        ));
    }
    if let Some(id) = ids.get(1) {
        list.apply(OverlayCommand::SetVisible(*id, false));
    }
    let feeds = list
        .iter()
        .enumerate()
        .map(|(n, _)| {
            let mut feed = OverlayFeed::empty(match n {
                2 => OverlayStatus::Error("HTTP 429: rate limited".into()),
                5 => OverlayStatus::Loading,
                6 => OverlayStatus::Queued,
                7 => OverlayStatus::Blocked("Same market as the chart"),
                _ => OverlayStatus::Ready,
            });
            feed.candles = std::sync::Arc::new(vec![crate::model::Kline {
                open_time: 0,
                close_time: 1,
                open: 1.0,
                high: 1.0,
                low: 1.0,
                close: 100.0 / (n as f64 + 1.0),
                volume: 1.0,
            }]);
            feed
        })
        .collect();
    let mut card = ChartCard::default();
    card.set_overlays(list, feeds, 1, DataSource::Binance, "BTCUSDT");
    card
}

/// The Compare page as a picture. Run by hand with `DEGENVIEW_GALLERY_DIR` set.
#[test]
#[ignore = "writes images for a person to look at"]
fn gallery_of_the_compare_page() {
    use crate::ui::render_tests::{Screen, gallery_dir};
    if gallery_dir().is_none() {
        return;
    }
    for (name, count, mode) in [
        ("compare-empty", 0, PriceScaleMode::Percent),
        ("compare-few", 3, PriceScaleMode::Percent),
        ("compare-regular", 2, PriceScaleMode::Regular),
        ("compare-many", 12, PriceScaleMode::Percent),
    ] {
        let mut screen = Screen::new(egui::vec2(820.0, 860.0));
        let mut state = ChartSettingsState {
            tab: Tab::Overlays,
            ..ChartSettingsState::default()
        };
        let mut card = card_with_overlays(count);
        let mut indicators = IndicatorConfig::default();
        let store = test_store();
        let mut last = None;
        for _ in 0..4 {
            last = Some(screen.run(Vec::new(), |ui| {
                let _ = show(
                    ui.ctx(),
                    &mut state,
                    ChartSettingsContext {
                        title: "BTC/USDT",
                        symbol: "BTCUSDT",
                        source: DataSource::Binance,
                        loaded_candles: 0,
                        indicators: &mut indicators,
                        card: &mut card,
                        scale_mode: mode,
                        store: &store,
                    },
                );
            }));
        }
        screen.save(name, last.expect("a frame"));
    }
}

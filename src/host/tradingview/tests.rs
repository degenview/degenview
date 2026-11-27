//! The whole import: a folder read, symbols matched, previewed and written into real stores.

use super::*;
use crate::model::Timeframe;
use crate::model::tradingview::fixtures;
use crate::model::tradingview::parse::{self, Document};
use crate::model::tradingview::plan::LeftOut;
use crate::service::SearchResult;
use crate::service::tradingview::cache::testing::{self, Fake, Scratch};
use crate::store::scripts::ScriptStore;
use crate::store::watchlist::WatchlistStore;
use crate::store::{SharedStore, Store};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A data folder holding the fixtures' documents.
fn data_folder(scratch: &Scratch) -> PathBuf {
    folder_of(scratch, fixtures::documents())
}

fn folder_of(scratch: &Scratch, documents: Vec<parse::Fetched>) -> PathBuf {
    let folder = scratch.path().join("TradingView");
    let entries: Vec<(String, String)> = documents
        .into_iter()
        .enumerate()
        .map(|(index, fetched)| match fetched.document {
            Document::LayoutPage { html } => {
                ("https://www.tradingview.com/chart/".to_owned(), html)
            }
            Document::Sources { layout, json } => (
                format!(
                    "https://charts-storage.tradingview.com/charts-storage/get/layout/{layout}\
                     /sources?n={index}"
                ),
                json,
            ),
            Document::Watchlists { json, .. } => (
                "https://www.tradingview.com/api/v1/symbols_list/all/".to_owned(),
                json,
            ),
            Document::Script { json, .. } => (
                format!("https://pine-facade.tradingview.com/pine-facade/get/USER%3Babc/{index}"),
                json,
            ),
            Document::SavedScripts { json } => (
                "https://pine-facade.tradingview.com/pine-facade/list/?filter=saved".to_owned(),
                json,
            ),
        })
        .collect();
    let fakes: Vec<Fake> = entries
        .iter()
        .map(|(url, body)| Fake::gzipped(url, body))
        .collect();
    testing::simple(&folder.join("Cache/Cache_Data"), &fakes);
    folder
}

fn search() -> Arc<Search> {
    Arc::new(|source, query| {
        use crate::model::DataSource;
        let listed: &[&str] = match source {
            DataSource::Binance => &["BTCUSDT"],
            DataSource::Okx => &["ETH-USDT"],
            DataSource::Coinbase => &["ETH-USD"],
            _ => &[],
        };
        Ok(listed
            .iter()
            .filter(|name| **name == query)
            .map(|name| SearchResult {
                symbol: (*name).to_owned(),
                full_symbol: (*name).to_owned(),
                source,
                price: None,
                event: None,
                base: None,
                quote: None,
            })
            .collect())
    })
}

/// A day of candles whose bar at the post's time runs from 90 to 110, in a range of 80 to 180.
fn candles() -> Arc<Candles> {
    Arc::new(|spot| {
        let bar = |offset: i64, low: f64, high: f64| crate::model::Kline {
            open_time: spot.time_ms + offset * spot.timeframe.nominal_ms(),
            close_time: spot.time_ms + (offset + 1) * spot.timeframe.nominal_ms() - 1,
            open: low,
            high,
            low,
            close: high,
            volume: 1.0,
        };
        Some(vec![
            bar(-1, 80.0, 100.0),
            bar(0, 90.0, 110.0),
            bar(1, 120.0, 180.0),
        ])
    })
}

fn targets(scratch: &Scratch) -> Targets {
    let store: SharedStore = Arc::new(Mutex::new(Store::open_in_memory().expect("open")));
    let watchlist = Arc::new(Mutex::new(WatchlistStore::new(store.clone())));
    let scripts = scratch.path().join("Scripts");
    let meta = scratch.path().join("ScriptMetadata");
    Targets {
        store,
        watchlist,
        scripts: Arc::new(move || Some(ScriptStore::new(scripts.clone(), meta.clone()))),
    }
}

fn open(targets: &Targets, folder: PathBuf, ctx: &egui::Context) -> TradingViewImport {
    let providers = Providers {
        search: search(),
        candles: candles(),
    };
    TradingViewImport::with_providers(targets.clone(), providers, Some(folder), Start::Choose, ctx)
}

/// Polls until `done`, as frames would.
fn settle(import: &mut TradingViewImport, ctx: &egui::Context, done: impl Fn(&Stage) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done(&import.stage) {
        assert!(Instant::now() < deadline, "the import never got there");
        std::thread::sleep(Duration::from_millis(5));
        import.poll(ctx);
    }
}

fn matched(stage: &Stage) -> bool {
    let Stage::Review(review) = stage else {
        return false;
    };
    review
        .selection
        .symbols(&review.library)
        .iter()
        .all(|(symbol, _)| review.mappings.has_answer(symbol))
}

fn review(import: &TradingViewImport) -> &Review {
    match &import.stage {
        Stage::Review(review) => review,
        _ => panic!("not reviewing"),
    }
}

#[test]
fn a_folder_is_read_matched_previewed_and_imported() {
    let scratch = Scratch::new();
    let targets = targets(&scratch);
    let ctx = egui::Context::default();
    let mut import = open(&targets, data_folder(&scratch), &ctx);
    settle(&mut import, &ctx, matched);

    import.act(Action::Preview, &ctx);
    let plan = review(&import).plan.as_ref().expect("a plan");
    assert_eq!(plan.layouts[0].charts.len(), 2);
    assert_eq!(plan.drawing_count(), 2, "the later delta deleted the third");

    import.act(Action::Import, &ctx);
    let Stage::Done(summary) = &import.stage else {
        panic!("done");
    };
    assert!(summary.failures.is_empty(), "{:?}", summary.failures);
    assert_eq!(summary.layouts.len(), 1);
    assert_eq!((summary.watchlists.len(), summary.scripts.len()), (1, 1));
    assert!(import.take_added_scripts());
    assert!(!import.take_added_scripts(), "told once");

    let views = targets.store.lock().unwrap().saved_views();
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].name, "Majors");
    let lists = targets.watchlist.lock().unwrap().lists().to_vec();
    assert!(lists.iter().any(|list| list.name == "Coins"));
    let mut library = (targets.scripts)().unwrap();
    assert_eq!(library.all_scripts().unwrap()[0].name, "My bands");
}

#[test]
fn importing_again_adds_beside_what_is_there() {
    let scratch = Scratch::new();
    let targets = targets(&scratch);
    let ctx = egui::Context::default();
    let folder = data_folder(&scratch);
    for _ in 0..2 {
        let mut import = open(&targets, folder.clone(), &ctx);
        settle(&mut import, &ctx, matched);
        import.act(Action::Preview, &ctx);
        import.act(Action::Import, &ctx);
    }
    let names: Vec<String> = targets
        .store
        .lock()
        .unwrap()
        .saved_views()
        .into_iter()
        .map(|view| view.name)
        .collect();
    assert_eq!(names, ["Majors", "Majors 2"]);
}

#[test]
fn an_import_opened_by_hand_starts_on_the_welcome() {
    let scratch = Scratch::new();
    let targets = targets(&scratch);
    let ctx = egui::Context::default();
    let providers = Providers {
        search: search(),
        candles: candles(),
    };
    let mut import = TradingViewImport::with_providers(
        targets,
        providers,
        Some(data_folder(&scratch)),
        Start::Welcome,
        &ctx,
    );
    assert!(matches!(import.view(), View::Welcome(None)));
    settle(&mut import, &ctx, matched);
    let View::Welcome(Some(found)) = import.view() else {
        panic!("not on the welcome");
    };
    assert!(!found.layouts.is_empty() && !found.scripts.is_empty());

    import.act(Action::Begin, &ctx);
    assert!(matches!(import.view(), View::Content(content) if content.step == Step::Choose));
    // Reading again is not opening again.
    import.act(Action::Reload, &ctx);
    assert!(matches!(import.view(), View::Reading));
    settle(&mut import, &ctx, matched);
    assert!(matches!(import.view(), View::Content(_)));
}

#[test]
fn a_welcome_with_nothing_to_bring_says_why() {
    let scratch = Scratch::new();
    let ctx = egui::Context::default();
    let providers = Providers {
        search: search(),
        candles: candles(),
    };
    let mut import = TradingViewImport::with_providers(
        targets(&scratch),
        providers,
        Some(scratch.path().to_owned()),
        Start::Welcome,
        &ctx,
    );
    settle(&mut import, &ctx, |stage| {
        matches!(stage, Stage::Failed { .. })
    });
    assert!(matches!(import.view(), View::Failed(..)));
}

#[test]
fn a_kind_is_taken_or_left_as_a_whole() {
    let scratch = Scratch::new();
    let targets = targets(&scratch);
    let ctx = egui::Context::default();
    let mut import = open(&targets, data_folder(&scratch), &ctx);
    settle(&mut import, &ctx, matched);
    let all = review(&import).selection.clone();
    assert!(!all.layouts.is_empty() && !all.scripts.is_empty());

    import.act(Action::SelectAll(Group::Layouts, false), &ctx);
    assert!(review(&import).selection.layouts.is_empty());
    assert_eq!(
        review(&import).selection.scripts,
        all.scripts,
        "one kind only"
    );
    import.act(Action::SelectAll(Group::Layouts, true), &ctx);
    assert_eq!(review(&import).selection, all);
}

#[test]
fn choices_stand_until_reset_and_closing_writes_nothing() {
    let scratch = Scratch::new();
    let targets = targets(&scratch);
    let ctx = egui::Context::default();
    let mut import = open(&targets, data_folder(&scratch), &ctx);
    settle(&mut import, &ctx, matched);
    let btc = TvSymbol::read("BINANCE:BTCUSDT").unwrap();
    import.act(Action::Skip(btc.clone()), &ctx);
    import.act(Action::ToggleScript("USER;abc".into()), &ctx);
    assert_eq!(*review(&import).mappings.status(&btc), Match::Skipped);
    assert!(review(&import).selection.scripts.is_empty());
    import.act(Action::Reset(btc.clone()), &ctx);
    assert!(review(&import).mappings.market(&btc).is_some());
    assert_eq!(import.act(Action::Close, &ctx), Outcome::Closed);
    assert!(targets.store.lock().unwrap().saved_views().is_empty());
}

#[test]
fn a_folder_with_nothing_in_it_says_so() {
    let scratch = Scratch::new();
    let ctx = egui::Context::default();
    let mut import = open(&targets(&scratch), scratch.path().to_owned(), &ctx);
    settle(&mut import, &ctx, |stage| {
        matches!(stage, Stage::Failed { .. })
    });
}

#[test]
fn reading_again_keeps_the_choices_made() {
    let scratch = Scratch::new();
    let ctx = egui::Context::default();
    let mut import = open(&targets(&scratch), data_folder(&scratch), &ctx);
    settle(&mut import, &ctx, matched);
    let btc = TvSymbol::read("BINANCE:BTCUSDT").unwrap();
    import.act(Action::Skip(btc.clone()), &ctx);
    import.act(Action::ToggleScript("USER;abc".into()), &ctx);

    import.act(Action::Reload, &ctx);
    assert!(matches!(import.stage, Stage::Reading(_)));
    settle(&mut import, &ctx, matched);
    assert_eq!(*review(&import).mappings.status(&btc), Match::Skipped);
    assert!(review(&import).selection.scripts.is_empty());
    // The same folder again, not the app's default.
    assert!(review(&import).folder.ends_with("TradingView"));
}

#[test]
fn a_name_taken_after_the_preview_is_still_not_taken_over() {
    let scratch = Scratch::new();
    let targets = targets(&scratch);
    let ctx = egui::Context::default();
    let folder = data_folder(&scratch);
    let mut first = open(&targets, folder.clone(), &ctx);
    let mut second = open(&targets, folder, &ctx);
    for import in [&mut first, &mut second] {
        settle(import, &ctx, matched);
        import.act(Action::Preview, &ctx);
    }
    first.act(Action::Import, &ctx);
    second.act(Action::Import, &ctx);
    let names: Vec<String> = targets
        .store
        .lock()
        .unwrap()
        .saved_views()
        .into_iter()
        .map(|view| view.name)
        .collect();
    assert_eq!(names, ["Majors", "Majors 2"]);
    assert!(first.take_wrote() && !first.take_wrote());
}

/// An import is only as good as what another computer accepts from iCloud, which refuses names,
/// texts and collections past its limits.
#[test]
fn everything_imported_is_accepted_by_sync() {
    use crate::model::tradingview::drawing::RawDrawing;
    use crate::model::tradingview::library::WatchRow;
    use crate::model::tradingview::plan::{self, Mappings, Market, Selection, Taken};
    use crate::sync::record::validate::{Verdict, validate};
    use crate::sync::record::{RecordName, SCHEMA_VERSION, SyncRecord, project};

    let long = "é".repeat(2_000);
    let huge = "x".repeat(30_000);
    let mut library = parse::library(fixtures::documents());
    let layout = &mut library.layouts[0];
    layout.name = Some(long.clone());
    let many: Vec<(i64, f64)> = (0..5_000).map(|i| (i, 100.0)).collect();
    let cells = vec![vec![huge.clone(); 40]; 80];
    for (id, tool, props, points) in [
        (
            "text",
            "LineToolText",
            serde_json::json!({"text": huge}),
            &many[..1],
        ),
        (
            "note",
            "LineToolTextNote",
            serde_json::json!({"text": huge}),
            &many[..2],
        ),
        (
            "table",
            "LineToolTable",
            serde_json::json!({"cells": cells}),
            &many[..1],
        ),
        ("brush", "LineToolBrush", serde_json::json!({}), &many[..]),
    ] {
        let source = fixtures::drawing(id, tool, "BINANCE:BTCUSDT", props, points);
        layout
            .drawings
            .push(RawDrawing::read(id, &source).expect(id));
    }
    library.watchlists[0].name = long.clone();
    library.watchlists[0]
        .rows
        .insert(0, WatchRow::Section(long.clone()));
    library.scripts[0].name = format!("a/b:c\\{long}");

    let selection = Selection::all(&library);
    let mut mappings = Mappings::default();
    for (symbol, _) in selection.symbols(&library) {
        let market = Market {
            source: crate::model::DataSource::Binance,
            symbol: symbol.to_string(),
            name: long.clone(),
            image_url: None,
        };
        mappings.found(symbol.clone(), Match::Exact(market));
    }
    let plan = plan::plan(
        &library,
        &selection,
        &mappings,
        &Default::default(),
        &Taken::default(),
        &Default::default(),
    );
    assert_eq!(plan.drawing_count(), 6, "{:?}", plan.layouts[0].left_out);

    let scratch = Scratch::new();
    let targets = targets(&scratch);
    let summary = apply::apply(&plan, &targets);
    assert!(summary.failures.is_empty(), "{:?}", summary.failures);

    let mut projected = Vec::new();
    for view in targets.store.lock().unwrap().saved_views() {
        let payload = serde_json::to_string(&view).unwrap();
        projected.extend(project::project_row(
            "saved_view",
            &view.id.to_string(),
            &payload,
        ));
    }
    for list in targets.watchlist.lock().unwrap().lists() {
        let payload = serde_json::to_string(list).unwrap();
        projected.extend(project::project_row(
            "watchlist",
            &list.id.to_string(),
            &payload,
        ));
    }
    assert!(projected.len() > 6);
    for entity in projected {
        let verdict = validate(&SyncRecord {
            name: RecordName::new(entity.kind, entity.sync_id.clone()).expect(&entity.sync_id),
            schema_version: SCHEMA_VERSION,
            revision: 1,
            origin_device: "d".into(),
            modified_ms: 0,
            payload: entity.payload,
        });
        assert!(
            matches!(verdict, Verdict::Accept(_)),
            "{:?} {verdict:?}",
            entity.kind
        );
    }
    let mut scripts = (targets.scripts)().unwrap();
    let script = scripts.all_scripts().unwrap().remove(0);
    assert!(script.name.chars().count() <= 200 && !script.name.contains(['/', ':', '\\']));
}

/// The fixtures with a post beside the 90–110 bar.
fn with_a_post() -> Vec<parse::Fetched> {
    let mut post = fixtures::drawing(
        "post",
        "LineToolTweet",
        "BINANCE:BTCUSDT",
        serde_json::json!({"position": 50.0}),
        &[(0, 7.0)],
    );
    post["state"]["tweetData"] =
        serde_json::json!({"tweetUrl": "https://x.com/someone/status/1234567890123456789"});
    let mut documents = fixtures::documents();
    documents.push(fixtures::sources("Ab", 70, &[post]));
    documents
}

fn placed(stage: &Stage) -> bool {
    matched(stage)
        && matches!(stage, Stage::Review(review)
            if review.selection.spots(&review.library, &review.mappings)
                .iter()
                .all(|spot| review.placements.contains_key(spot)))
}

fn post_price(import: &TradingViewImport) -> Option<f64> {
    use crate::model::drawings::Geometry;
    let plan = review(import).plan.as_ref()?;
    plan.layouts[0].charts.iter().find_map(|chart| {
        let key = crate::model::InstrumentId::new(chart.source, &chart.symbol).key();
        chart
            .drawings
            .get(&key)
            .iter()
            .find_map(|drawing| match drawing {
                crate::model::Drawing::Shape {
                    geometry: Geometry::Card { anchor, .. },
                    ..
                } => Some(anchor.price),
                _ => None,
            })
    })
}

#[test]
fn a_post_is_put_beside_its_bar_from_the_markets_candles() {
    let scratch = Scratch::new();
    let targets = targets(&scratch);
    let ctx = egui::Context::default();
    let mut import = open(&targets, folder_of(&scratch, with_a_post()), &ctx);
    settle(&mut import, &ctx, placed);
    import.act(Action::Preview, &ctx);
    // Half way from the high, 110, to the top of the room, 190.
    assert_eq!(post_price(&import), Some(150.0));
    let left_out = &review(&import).plan.as_ref().unwrap().layouts[0].left_out;
    assert!(
        left_out.contains(&LeftOut::PostsEstimated(1)),
        "{left_out:?}"
    );
}

#[test]
fn a_post_without_candles_is_left_out_and_can_be_tried_again() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let scratch = Scratch::new();
    let targets = targets(&scratch);
    let ctx = egui::Context::default();
    let online = Arc::new(AtomicBool::new(false));
    let (flag, working) = (Arc::clone(&online), candles());
    let providers = Providers {
        search: search(),
        candles: Arc::new(move |spot| flag.load(Ordering::SeqCst).then(|| working(spot)).flatten()),
    };
    let folder = folder_of(&scratch, with_a_post());
    let mut import =
        TradingViewImport::with_providers(targets, providers, Some(folder), Start::Choose, &ctx);
    settle(&mut import, &ctx, placed);
    import.act(Action::Preview, &ctx);
    assert_eq!(post_price(&import), None);
    let left_out = &review(&import).plan.as_ref().unwrap().layouts[0].left_out;
    assert!(
        left_out.contains(&LeftOut::PostsUnplaced(1)),
        "{left_out:?}"
    );

    online.store(true, Ordering::SeqCst);
    import.act(Action::Back, &ctx);
    import.act(Action::Retry, &ctx);
    settle(&mut import, &ctx, placed);
    import.act(Action::Preview, &ctx);
    assert_eq!(post_price(&import), Some(150.0));
}

/// The preview is rebuilt as each price arrives; the names in use are read once for it.
#[test]
fn rebuilding_the_preview_does_not_read_the_script_library_again() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let scratch = Scratch::new();
    let mut targets = targets(&scratch);
    let opened = Arc::new(AtomicUsize::new(0));
    let (counter, library) = (Arc::clone(&opened), Arc::clone(&targets.scripts));
    targets.scripts = Arc::new(move || {
        counter.fetch_add(1, Ordering::SeqCst);
        library()
    });
    let ctx = egui::Context::default();
    let mut import = open(&targets, data_folder(&scratch), &ctx);
    settle(&mut import, &ctx, matched);
    import.act(Action::Preview, &ctx);
    let once = opened.load(Ordering::SeqCst);
    assert_eq!(once, 1);
    for timeframe in [Timeframe::HOUR_1, Timeframe::DAY_1, Timeframe::MINUTE_5] {
        import.act(Action::SetTimeframe("Ab".into(), timeframe), &ctx);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            import.show(ui.ctx());
        });
        output.textures_delta.clear();
        assert!(review(&import).plan.is_some(), "rebuilt");
    }
    assert_eq!(opened.load(Ordering::SeqCst), once);
}

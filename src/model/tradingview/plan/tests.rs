use super::*;
use crate::model::tradingview::{fixtures, parse};
use crate::model::{BuiltIn, DataSource, InstrumentId};

fn library() -> Library {
    parse::library(fixtures::documents())
}

fn market(source: DataSource, symbol: &str) -> Market {
    Market {
        source,
        symbol: symbol.into(),
        name: symbol.into(),
        image_url: None,
    }
}

fn symbol(text: &str) -> TvSymbol {
    TvSymbol::read(text).unwrap()
}

fn mapped() -> Mappings {
    let mut mappings = Mappings::default();
    mappings.found(
        symbol("BINANCE:BTCUSDT"),
        Match::Exact(market(DataSource::Binance, "BTCUSDT")),
    );
    mappings.found(
        symbol("OKX:ETHUSDT"),
        Match::Exact(market(DataSource::Okx, "ETH-USDT")),
    );
    mappings.found(
        symbol("BITSTAMP:ETHUSD"),
        Match::Substitute(market(DataSource::Coinbase, "ETH-USD")),
    );
    mappings.found(symbol("NOPE:XYZ"), Match::Unmatched(Unmatched::Venue));
    mappings
}

fn built(mappings: &Mappings) -> Plan {
    let library = library();
    plan(
        &library,
        &Selection::all(&library),
        mappings,
        &Default::default(),
        &Taken::default(),
        &ChartAppearance::default(),
    )
}

#[test]
fn a_layout_becomes_charts_in_its_grid_with_their_drawings() {
    let plan = built(&mapped());
    let layout = &plan.layouts[0];
    assert_eq!(layout.name, "Majors");
    assert_eq!(layout.charts.len(), 2);
    assert_eq!(layout.columns.len(), 2);
    assert_eq!(layout.timeframe, Timeframe::MINUTE_5);
    assert_eq!(layout.timeframes, [Timeframe::MINUTE_5, Timeframe::HOUR_1]);
    assert!(layout.left_out.contains(&LeftOut::HiddenCharts(1)));
    let btc = &layout.charts[0];
    assert_eq!(btc.source, DataSource::Binance);
    assert_eq!(btc.overlays.len(), 1);
    assert_eq!(btc.pine_instances.len(), 1, "RSI is the bundled script");
    let key = InstrumentId::new(DataSource::Binance, "BTCUSDT").key();
    assert_eq!(btc.drawings.get(&key).len(), 2);
    assert_eq!(layout.drawings, 2);
    assert!(!BuiltIn::Volume.applied(&btc.indicators));
}

#[test]
fn a_chart_with_no_market_is_left_out_and_so_are_its_drawings() {
    let mut mappings = mapped();
    mappings.skip(&symbol("BINANCE:BTCUSDT"));
    let layout = built(&mappings).layouts.remove(0);
    assert_eq!(layout.charts.len(), 1);
    assert!(
        layout
            .left_out
            .contains(&LeftOut::Chart(symbol("BINANCE:BTCUSDT")))
    );
    assert!(layout.left_out.contains(&LeftOut::DrawingsOfMarket {
        symbol: symbol("BINANCE:BTCUSDT"),
        count: 2,
    }));
    assert_eq!(layout.drawings, 0);
}

#[test]
fn watchlists_keep_sections_and_count_what_has_no_market() {
    let plan = built(&mapped());
    let list = &plan.watchlists[0];
    assert_eq!(list.list.name, "Coins");
    assert_eq!(list.markets, 1);
    assert_eq!(list.list.sections().count(), 1);
    assert_eq!(list.left_out, [LeftOut::Markets(1)]);
}

#[test]
fn names_in_use_are_never_taken_over() {
    let library = library();
    let taken = Taken {
        layouts: vec!["majors".into()],
        watchlists: vec!["Coins".into(), "Coins 2".into()],
        scripts: vec!["My bands".into()],
    };
    let plan = plan(
        &library,
        &Selection::all(&library),
        &mapped(),
        &Default::default(),
        &taken,
        &ChartAppearance::default(),
    );
    assert_eq!(plan.layouts[0].name, "Majors 2");
    assert_eq!(plan.watchlists[0].list.name, "Coins 3");
    assert_eq!(plan.scripts[0].name, "My bands 2");
}

/// APFS takes a precomposed and a decomposed accent for one name: the preview must say the name
/// the library will give.
#[test]
fn a_script_is_numbered_past_a_name_spelt_another_way() {
    let mut library = library();
    library.scripts[0].name = "Caf\u{e9}".into();
    let mut second = library.scripts[0].clone();
    second.id = "again".into();
    second.name = "Cafe\u{301}".into();
    library.scripts.push(second);
    let taken = Taken {
        scripts: vec!["cafe\u{301}".into()],
        ..Taken::default()
    };
    let plan = plan(
        &library,
        &Selection::all(&library),
        &mapped(),
        &Default::default(),
        &taken,
        &ChartAppearance::default(),
    );
    assert_eq!(plan.scripts[0].name, "Caf\u{e9} 2");
    assert_eq!(plan.scripts[1].name, "Cafe\u{301} 3");
}

#[test]
fn the_symbols_asked_about_are_those_of_what_is_chosen() {
    let library = library();
    let mut selection = Selection::all(&library);
    selection.layouts.clear();
    let only_list: Vec<String> = selection
        .symbols(&library)
        .iter()
        .map(|(symbol, _)| symbol.to_string())
        .collect();
    assert_eq!(only_list, ["BINANCE:BTCUSDT", "NOPE:XYZ"]);
    let all = Selection::all(&library).symbols(&library);
    let btc = &all
        .iter()
        .find(|(s, _)| s.as_str() == "BINANCE:BTCUSDT")
        .unwrap()
        .1;
    assert_eq!((btc.charts, btc.drawings, btc.watchlists), (1, 2, 1));
}

#[test]
fn a_choice_stands_over_the_search_until_reset() {
    let mut mappings = Mappings::default();
    let btc = symbol("BINANCE:BTCUSDT");
    mappings.skip(&btc);
    mappings.found(
        btc.clone(),
        Match::Exact(market(DataSource::Binance, "BTCUSDT")),
    );
    assert_eq!(*mappings.status(&btc), Match::Skipped);
    mappings.reset(&btc);
    assert!(mappings.market(&btc).is_some());
}

#[test]
fn a_chart_left_out_leaves_no_hole_in_the_grid() {
    let mut mappings = mapped();
    mappings.skip(&symbol("BINANCE:BTCUSDT"));
    let layout = built(&mappings).layouts.remove(0);
    assert_eq!(layout.columns.len(), 1);
    assert_eq!(layout.columns[0].chart_ids, [layout.charts[0].id]);
}

#[test]
fn reading_again_keeps_what_was_left_out_and_decided() {
    let before = library();
    let mut selection = Selection::all(&before);
    selection.scripts.clear();
    let mut after = before.clone();
    let mut new_list = after.watchlists[0].clone();
    new_list.id = "99".into();
    after.watchlists.push(new_list);
    let carried = selection.carried_to(&before, &after);
    assert!(carried.scripts.is_empty());
    assert!(carried.watchlists.contains("99"));

    let mut mappings = mapped();
    mappings.skip(&symbol("BINANCE:BTCUSDT"));
    let kept = mappings.decisions();
    assert_eq!(*kept.status(&symbol("BINANCE:BTCUSDT")), Match::Skipped);
    assert_eq!(*kept.status(&symbol("OKX:ETHUSDT")), Match::Searching);
}

#[test]
fn drawings_on_a_market_no_chart_opens_on_are_said_to_be_there() {
    let mut library = library();
    let line = fixtures::drawing(
        "elsewhere",
        "LineToolTrendLine",
        "COINBASE:BTCUSD",
        serde_json::json!({}),
        &[(0, 1.0), (1, 2.0)],
    );
    library.layouts[0]
        .drawings
        .push(crate::model::tradingview::drawing::RawDrawing::read("elsewhere", &line).unwrap());
    let mut mappings = mapped();
    mappings.found(
        symbol("COINBASE:BTCUSD"),
        Match::Exact(market(DataSource::Coinbase, "BTC-USD")),
    );
    let layout = plan(
        &library,
        &Selection::all(&library),
        &mappings,
        &Default::default(),
        &Taken::default(),
        &ChartAppearance::default(),
    )
    .layouts
    .remove(0);
    assert!(
        layout.left_out.contains(&LeftOut::OtherMarket {
            source: DataSource::Coinbase,
            symbol: "BTC-USD".into(),
            count: 1,
        }),
        "{:?}",
        layout.left_out
    );
}

#[test]
fn a_layout_whose_charts_have_no_market_keeps_the_drawings_that_do() {
    let mut library = parse::library(fixtures::documents());
    for chart in &mut library.layouts[0].charts {
        chart.symbol = symbol("COINBASE:BTCUSD27X2026");
    }
    let selection = Selection::all(&library);
    let plan_with = |mappings: &Mappings| {
        plan(
            &library,
            &selection,
            mappings,
            &Default::default(),
            &Taken::default(),
            &Default::default(),
        )
        .layouts
        .remove(0)
    };

    let layout = plan_with(&mapped());
    assert_eq!(layout.charts.len(), 1, "one chart stands in, not each");
    assert_eq!(layout.charts[0].symbol, "BTCUSDT");
    assert!(layout.drawings > 0);
    assert!(layout.left_out.contains(&LeftOut::StandIn {
        symbol: symbol("COINBASE:BTCUSD27X2026"),
        source: DataSource::Binance,
        market: "BTCUSDT".into(),
    }));

    // With no market for any drawing either, there is nothing to open.
    let layout = plan_with(&Mappings::default());
    assert!(layout.charts.is_empty());
    assert!(layout.left_out.contains(&LeftOut::NoChart));
}

#[test]
fn a_watchlist_with_no_market_is_not_imported() {
    let plan = built(&Mappings::default());
    let list = &plan.watchlists[0];
    assert_eq!(list.markets, 0);
    assert!(
        list.left_out.contains(&LeftOut::NoMarket),
        "{:?}",
        list.left_out
    );
}

fn with_drawing(id: &str, tool: &str, on: &str, props: serde_json::Value) -> Library {
    let mut library = library();
    let source = fixtures::drawing(id, tool, on, props, &[(0, 1.0), (1, 2.0)]);
    library.layouts[0]
        .drawings
        .push(crate::model::tradingview::drawing::RawDrawing::read(id, &source).unwrap());
    library
}

fn first_layout(library: &Library, mappings: &Mappings) -> PlannedLayout {
    plan(
        library,
        &Selection::all(library),
        mappings,
        &Default::default(),
        &Taken::default(),
        &ChartAppearance::default(),
    )
    .layouts
    .remove(0)
}

#[test]
fn a_post_still_being_looked_up_is_not_called_unplaceable() {
    let library = with_drawing(
        "post",
        "LineToolTweet",
        "BINANCE:BTCUSDT",
        serde_json::json!({"position": 10.0}),
    );
    let left_out = first_layout(&library, &mapped()).left_out;
    assert!(left_out.contains(&LeftOut::PostsPlacing(1)), "{left_out:?}");
    assert!(
        !left_out.contains(&LeftOut::PostsUnplaced(1)),
        "{left_out:?}"
    );
}

#[test]
fn a_drawing_goes_to_the_chart_that_shows_the_market_it_became() {
    let library = with_drawing(
        "eth",
        "LineToolTrendLine",
        "KRAKEN:ETHUSD",
        serde_json::json!({}),
    );
    let mut mappings = mapped();
    // The second chart, BITSTAMP:ETHUSD, opens on Coinbase ETH-USD; so does this symbol.
    mappings.found(
        symbol("KRAKEN:ETHUSD"),
        Match::Substitute(market(DataSource::Coinbase, "ETH-USD")),
    );
    let layout = first_layout(&library, &mappings);
    let key = InstrumentId::new(DataSource::Coinbase, "ETH-USD").key();
    assert_eq!(layout.charts[1].drawings.get(&key).len(), 1);
    assert!(
        !layout
            .left_out
            .iter()
            .any(|entry| matches!(entry, LeftOut::OtherMarket { .. })),
        "{:?}",
        layout.left_out
    );
}

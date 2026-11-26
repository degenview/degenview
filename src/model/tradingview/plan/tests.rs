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
    TvSymbol::parse(text).unwrap()
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
        scripts: vec!["My zigzag".into()],
    };
    let plan = plan(
        &library,
        &Selection::all(&library),
        &mapped(),
        &taken,
        &ChartAppearance::default(),
    );
    assert_eq!(plan.layouts[0].name, "Majors 2");
    assert_eq!(plan.watchlists[0].list.name, "Coins 3");
    assert_eq!(plan.scripts[0].name, "My zigzag 2");
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

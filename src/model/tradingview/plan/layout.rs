//! One TradingView layout as a DegenView layout: its charts in their grid, each with its market,
//! look, comparisons, indicators and drawings. TradingView files drawings by layout and market,
//! DegenView by chart and market: a market's drawings go to the first chart that shows it, and
//! those of a market no chart shows to the first chart, where switching to that market finds them.

use super::{LeftOut, Mappings, Market, Placements, PlannedLayout};
use crate::model::chart::kind::ChartType;
use crate::model::chart::price_scale::PriceScaleMode;
use crate::model::drawings::market::MarketDrawings;
use crate::model::indicators::EMA_PERIODS;
use crate::model::pine::bundled;
use crate::model::pine::instance::PineInstance;
use crate::model::tradingview::convert::{self, Unsupported};
use crate::model::tradingview::drawing::RawDrawing;
use crate::model::tradingview::library::{Layout, LayoutChart, Library, Study};
use crate::model::tradingview::placement::Spot;
use crate::model::tradingview::{TvSymbol, interval};
use crate::model::{
    BuiltIn, ChartAppearance, ChartColumn, DataSource, Drawing, IndicatorConfig, InstrumentId,
    PersistedChart, Timeframe,
};
use std::collections::BTreeMap;

/// TradingView's built-in indicators DegenView draws natively.
const NATIVE: [(&str, BuiltIn); 2] = [("Volume", BuiltIn::Volume), ("BB", BuiltIn::Bollinger)];

/// TradingView's built-in indicators DegenView ships as Pine.
const BUNDLED: [(&str, &str); 19] = [
    ("RSI", "rsi"),
    ("MACD", "macd"),
    ("ATR", "atr"),
    ("Stochastic", "stochastic"),
    ("StochasticRSI", "stochastic-rsi"),
    ("CCI", "cci"),
    ("VWAP", "vwap"),
    ("OBV", "obv"),
    ("AROON", "aroon"),
    ("MF", "money-flow-index"),
    ("WilliamR", "williams-r"),
    ("IchimokuCloud", "ichimoku-cloud"),
    ("PSAR", "parabolic-sar"),
    ("DM", "adx-dmi"),
    ("AwesomeOscillator", "awesome-oscillator"),
    ("CMF", "chaikin-money-flow"),
    ("DONCH", "donchian-channels"),
    ("KLTNR", "keltner-channels"),
    ("WilliamsFractal", "williams-fractals"),
];

/// TradingView's series style numbers.
fn chart_type(style: i64) -> Option<ChartType> {
    Some(match style {
        0 => ChartType::Bars,
        1 => ChartType::Candles,
        2 => ChartType::Line,
        3 => ChartType::Area,
        4 => ChartType::Renko,
        5 => ChartType::Kagi,
        6 => ChartType::PointAndFigure,
        7 => ChartType::LineBreak,
        8 => ChartType::HeikinAshi,
        9 => ChartType::HollowCandles,
        10 => ChartType::Baseline,
        11 => ChartType::Range,
        12 => ChartType::HighLowBars,
        13 => ChartType::Columns,
        14 => ChartType::LineWithMarkers,
        15 => ChartType::StepLine,
        16 => ChartType::HlcArea,
        19 => ChartType::VolumeCandles,
        _ => return None,
    })
}

struct Building {
    chart: PersistedChart,
    symbol: TvSymbol,
    drawings: BTreeMap<String, Vec<Drawing>>,
    next_drawing: u64,
}

pub(super) fn plan(
    found: &Layout,
    chosen: Option<Timeframe>,
    mappings: &Mappings,
    placements: &Placements,
    ticks: &BTreeMap<&TvSymbol, f64>,
    library: &Library,
    base: &ChartAppearance,
) -> PlannedLayout {
    let mut left_out = Vec::new();
    let mut own_scripts = Vec::new();
    let mut building: Vec<Building> = Vec::new();
    // The chart id given to the chart at each place of TradingView's list.
    let mut places: BTreeMap<usize, u64> = BTreeMap::new();
    let mut timeframes: Vec<Timeframe> = Vec::new();

    let hidden = found.charts.iter().filter(|chart| !chart.shown).count();
    if hidden > 0 {
        left_out.push(LeftOut::HiddenCharts(hidden));
    }
    let mut stand_in = stand_in(found, mappings);
    for (place, source) in found.charts.iter().enumerate().filter(|(_, c)| c.shown) {
        // The symbol the chart is built on: its own, or the one standing in for it.
        let (symbol, market) = match mappings.market(&source.symbol) {
            Some(market) => (&source.symbol, market),
            None => match stand_in.take() {
                Some((symbol, market)) => {
                    left_out.push(LeftOut::StandIn {
                        symbol: source.symbol.clone(),
                        source: market.source,
                        market: market.symbol.clone(),
                    });
                    (symbol, market)
                }
                None => {
                    left_out.push(LeftOut::Chart(source.symbol.clone()));
                    continue;
                }
            },
        };
        match interval::timeframe(&source.interval) {
            Some(timeframe) if !timeframes.contains(&timeframe) => timeframes.push(timeframe),
            Some(_) => {}
            None => left_out.push(LeftOut::Interval(source.interval.clone())),
        }
        let id = building.len() as u64 + 1;
        let main = InstrumentId::new(market.source, &market.symbol);
        let mut appearance = *base;
        if let Some(style) = source.style {
            match chart_type(style) {
                Some(kind) => appearance.style.chart_type = kind,
                None => left_out.push(LeftOut::ChartStyle(style)),
            }
        }
        let mut chart = PersistedChart {
            id,
            symbol: market.symbol.clone(),
            source: market.source,
            log_scale: false,
            scale_mode: Some(PriceScaleMode::Regular),
            y_zoom: 1.0,
            indicators: IndicatorConfig::default(),
            appearance,
            drawings: MarketDrawings::default(),
            pine_instances: Vec::new(),
            candle_count: 0,
            viewport: Default::default(),
            // A DEX pair is named by its pair, not the address it is fetched by.
            title: (market.source == DataSource::DexScreener).then(|| market.name.clone()),
            pm_series: Vec::new(),
            image_url: market.image_url.clone(),
            card: None,
            overlays: Default::default(),
        };
        for overlay in &source.overlays {
            match mappings.market(overlay) {
                Some(other) => {
                    let instrument = InstrumentId::new(other.source, &other.symbol);
                    chart.overlays.add(&main, instrument, None);
                }
                None => left_out.push(LeftOut::Comparison(overlay.clone())),
            }
        }
        indicators(source, &mut chart, library, &mut own_scripts, &mut left_out);
        places.insert(place, id);
        building.push(Building {
            chart,
            symbol: symbol.clone(),
            drawings: BTreeMap::new(),
            next_drawing: 1,
        });
    }

    let drawings = if building.is_empty() {
        left_out.push(LeftOut::NoChart);
        0
    } else {
        file_drawings(
            found,
            mappings,
            placements,
            ticks,
            &mut building,
            &mut left_out,
        )
    };

    let count = building.len();
    let charts: Vec<PersistedChart> = building
        .into_iter()
        .map(|mut built| {
            for (market, drawings) in built.drawings {
                built.chart.drawings.put(&market, drawings);
            }
            built.chart
        })
        .collect();
    let timeframe = chosen
        .or_else(|| timeframes.first().copied())
        .unwrap_or(Timeframe::DAY_1);
    if timeframes.len() < 2 {
        timeframes.clear();
    }

    // TradingView's columns, without the places of charts that were left out.
    let columns = found
        .grid
        .columns
        .iter()
        .map(|column| {
            column
                .iter()
                .filter_map(|place| places.get(place).copied())
                .collect::<Vec<_>>()
        })
        .filter(|ids| !ids.is_empty())
        .enumerate()
        .map(|(index, ids)| ChartColumn::new(count as u64 + index as u64 + 1, ids))
        .collect();

    PlannedLayout {
        source_id: found.id.clone(),
        name: found.display_name(),
        timeframe,
        timeframes,
        charts,
        columns,
        own_scripts,
        drawings,
        left_out,
    }
}

/// For a layout none of whose charts has a market: the symbol most of its drawings are on, among
/// those that have one, so that the first chart can open there instead of the drawings being lost
/// with the layout. `None` while any shown chart has a market.
fn stand_in<'a>(found: &'a Layout, mappings: &'a Mappings) -> Option<(&'a TvSymbol, &'a Market)> {
    if found
        .shown_charts()
        .any(|chart| mappings.market(&chart.symbol).is_some())
    {
        return None;
    }
    let mut drawn: BTreeMap<&TvSymbol, usize> = BTreeMap::new();
    for drawing in &found.drawings {
        *drawn.entry(&drawing.symbol).or_default() += 1;
    }
    drawn
        .into_iter()
        .filter_map(|(symbol, count)| Some((count, symbol, mappings.market(symbol)?)))
        .max_by_key(|(count, ..)| *count)
        .map(|(_, symbol, market)| (symbol, market))
}

fn indicators(
    source: &LayoutChart,
    chart: &mut PersistedChart,
    library: &Library,
    own_scripts: &mut Vec<(u64, u64, String)>,
    left_out: &mut Vec<LeftOut>,
) {
    if !chart.source.takes_indicators() {
        left_out.extend(
            source
                .studies
                .iter()
                .map(|study| LeftOut::Indicator(study.name.clone())),
        );
        return;
    }
    for study in &source.studies {
        let (kind, package) = study.id.split_once('@').unwrap_or((&study.id, ""));
        let built_in = package == "tv-basicstudies";
        let instance_id = chart.pine_instances.len() as u64 + 1;
        // The native EMA is one line, at one of a few lengths.
        if built_in && kind == "MAExp" {
            // TradingView's default length, when the study does not say.
            let length = study.length.map_or(9, |length| length as usize);
            if chart.indicators.show_ema || !EMA_PERIODS.contains(&length) {
                left_out.push(LeftOut::Indicator(format!("EMA {length}")));
            } else {
                BuiltIn::Ema.set_applied(&mut chart.indicators, true);
                chart.indicators.ema_period = length;
            }
        } else if built_in && let Some((_, native)) = NATIVE.iter().find(|(id, _)| *id == kind) {
            native.set_applied(&mut chart.indicators, true);
            if *native == BuiltIn::Bollinger && study.length.is_some_and(|l| l != 20) {
                left_out.push(LeftOut::IndicatorSettings(study.name.clone()));
            }
        } else if built_in
            && let Some(indicator) = BUNDLED
                .iter()
                .find(|(id, _)| *id == kind)
                .and_then(|(_, id)| bundled::find(id))
        {
            chart.pine_instances.push(PineInstance::new(
                instance_id,
                Some(bundled::script_id(indicator)),
                indicator.name.text(),
                indicator.source,
            ));
            left_out.push(LeftOut::IndicatorSettings(study.name.clone()));
        } else if let Some(script) = own_script(study, library) {
            chart.pine_instances.push(PineInstance::new(
                instance_id,
                None,
                &script.name,
                &script.source,
            ));
            own_scripts.push((chart.id, instance_id, script.id.clone()));
            left_out.push(LeftOut::IndicatorSettings(script.name.clone()));
        } else {
            left_out.push(LeftOut::Indicator(study.name.clone()));
        }
    }
}

/// The account's own script a study runs, when its source was found.
fn own_script<'a>(
    study: &Study,
    library: &'a Library,
) -> Option<&'a crate::model::tradingview::Script> {
    library
        .scripts
        .iter()
        .find(|script| study.id.contains(script.id.as_str()))
}

/// Files the layout's drawings under its charts, bottom first; returns how many.
fn file_drawings(
    found: &Layout,
    mappings: &Mappings,
    placements: &Placements,
    ticks: &BTreeMap<&TvSymbol, f64>,
    building: &mut [Building],
    left_out: &mut Vec<LeftOut>,
) -> usize {
    let mut ordered: Vec<&RawDrawing> = found.drawings.iter().collect();
    ordered.sort_by(|a, b| a.z.total_cmp(&b.z).then_with(|| a.id.cmp(&b.id)));

    let mut filed = 0;
    let mut unmapped: BTreeMap<&TvSymbol, usize> = BTreeMap::new();
    let mut tools: BTreeMap<&str, usize> = BTreeMap::new();
    let (mut pictures, mut positions, mut cut, mut default_levels) = (0, 0, 0, 0);
    let mut broken = found.unreadable;
    let (mut pinned, mut estimated, mut unplaced, mut percent_bands) = (0, 0, 0, 0);
    // Whose price is still being looked up.
    let mut placing = 0;
    // Filed on a chart that opens on another market, by market.
    let mut elsewhere: BTreeMap<String, (DataSource, String, usize)> = BTreeMap::new();
    for raw in ordered {
        let Some(market) = mappings.market(&raw.symbol) else {
            *unmapped.entry(&raw.symbol).or_default() += 1;
            continue;
        };
        // The chart with TradingView's symbol, else one that opens on the market it became.
        let wanted = InstrumentId::new(market.source, &market.symbol).key();
        let index = building
            .iter()
            .position(|chart| chart.symbol == raw.symbol)
            .or_else(|| {
                building.iter().position(|built| {
                    InstrumentId::new(built.chart.source, &built.chart.symbol).key() == wanted
                })
            })
            .unwrap_or(0);
        let target = &mut building[index];
        // A drawing beside a bar is given the price worked out for it.
        let spot = Spot::of(raw, market);
        if spot
            .as_ref()
            .is_some_and(|spot| !placements.contains_key(spot))
        {
            placing += 1;
            continue;
        }
        let placed = spot
            .and_then(|spot| placements.get(&spot).copied().flatten())
            .map(|price| raw.at_price(price));
        let guessed = placed.is_some();
        let source = placed.as_ref().unwrap_or(raw);
        match convert::convert(source, target.next_drawing, ticks.get(&raw.symbol).copied()) {
            Ok(converted) => {
                let key = InstrumentId::new(market.source, &market.symbol).key();
                let shown = InstrumentId::new(target.chart.source, &target.chart.symbol).key();
                if key != shown {
                    elsewhere
                        .entry(key.clone())
                        .or_insert((market.source, market.symbol.clone(), 0))
                        .2 += 1;
                }
                target
                    .drawings
                    .entry(key)
                    .or_default()
                    .push(converted.drawing);
                target.next_drawing += 1;
                filed += 1;
                cut += usize::from(converted.cut_text);
                default_levels += usize::from(converted.default_levels);
                percent_bands += usize::from(converted.percent_bands);
                estimated += usize::from(guessed);
            }
            Err(Unsupported::Tool) => {
                *tools.entry(convert::class_label(&raw.tool)).or_default() += 1;
            }
            Err(Unsupported::Picture) => pictures += 1,
            Err(Unsupported::PriceStep) => positions += 1,
            Err(Unsupported::Points) => broken += 1,
            Err(Unsupported::Pinned) => pinned += 1,
            Err(Unsupported::Unplaced) => unplaced += 1,
        }
    }
    left_out.extend(
        unmapped
            .into_iter()
            .map(|(symbol, count)| LeftOut::DrawingsOfMarket {
                symbol: symbol.clone(),
                count,
            }),
    );
    left_out.extend(
        tools
            .into_iter()
            .map(|(class, count)| LeftOut::DrawingsOfTool {
                class: class.to_owned(),
                count,
            }),
    );
    left_out.extend(
        elsewhere
            .into_values()
            .map(|(source, symbol, count)| LeftOut::OtherMarket {
                source,
                symbol,
                count,
            }),
    );
    for (count, entry) in [
        (pictures, LeftOut::Pictures(pictures)),
        (positions, LeftOut::PositionsWithoutPriceStep(positions)),
        (broken, LeftOut::BrokenDrawings(broken)),
        (cut, LeftOut::CutText(cut)),
        (default_levels, LeftOut::DefaultLevels(default_levels)),
        (pinned, LeftOut::Pinned(pinned)),
        (estimated, LeftOut::PostsEstimated(estimated)),
        (unplaced, LeftOut::PostsUnplaced(unplaced)),
        (placing, LeftOut::PostsPlacing(placing)),
        (percent_bands, LeftOut::PercentBands(percent_bands)),
    ] {
        if count > 0 {
            left_out.push(entry);
        }
    }
    filed
}

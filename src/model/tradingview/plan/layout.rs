//! One TradingView layout as a DegenView layout: its charts in their grid, each with its market,
//! look, comparisons, indicators and drawings. TradingView files drawings by layout and market,
//! DegenView by chart and market: a market's drawings go to the first chart that shows it, and
//! those of a market no chart shows to the first chart, where switching to that market finds them.

use super::{LeftOut, Mappings, PlannedLayout};
use crate::model::chart::kind::ChartType;
use crate::model::chart::price_scale::PriceScaleMode;
use crate::model::drawings::market::MarketDrawings;
use crate::model::pine::bundled;
use crate::model::pine::instance::PineInstance;
use crate::model::tradingview::convert::{self, Unsupported};
use crate::model::tradingview::drawing::RawDrawing;
use crate::model::tradingview::library::{Layout, LayoutChart, Library, Study};
use crate::model::tradingview::{TvSymbol, interval};
use crate::model::{
    BuiltIn, ChartAppearance, ChartColumn, Drawing, IndicatorConfig, InstrumentId, PersistedChart,
    Timeframe,
};
use std::collections::BTreeMap;

/// TradingView's built-in indicators DegenView draws natively.
const NATIVE: [(&str, BuiltIn); 3] = [
    ("Volume", BuiltIn::Volume),
    ("MAExp", BuiltIn::Ema),
    ("BB", BuiltIn::Bollinger),
];

/// TradingView's built-in indicators DegenView ships as Pine.
const BUNDLED: [(&str, &str); 18] = [
    ("RSI", "rsi"),
    ("MACD", "macd"),
    ("ATR", "atr"),
    ("Stochastic", "stochastic"),
    ("StochasticRSI", "stochastic-rsi"),
    ("CCI", "cci"),
    ("VWAP", "vwap"),
    ("OnBalanceVolume", "obv"),
    ("MF", "money-flow-index"),
    ("WilliamR", "williams-r"),
    ("IchimokuCloud", "ichimoku-cloud"),
    ("PSAR", "parabolic-sar"),
    ("DM", "adx-dmi"),
    ("AwesomeOscillator", "awesome-oscillator"),
    ("CMF", "chaikin-money-flow"),
    ("DonchianChannels", "donchian-channels"),
    ("KeltnerChannels", "keltner-channels"),
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
    ticks: &BTreeMap<&TvSymbol, f64>,
    library: &Library,
    base: &ChartAppearance,
) -> PlannedLayout {
    let mut left_out = Vec::new();
    let mut own_scripts = Vec::new();
    let mut building: Vec<Building> = Vec::new();
    let mut timeframes: Vec<Timeframe> = Vec::new();

    let hidden = found.charts.iter().filter(|chart| !chart.shown).count();
    if hidden > 0 {
        left_out.push(LeftOut::HiddenCharts(hidden));
    }
    for source in found.shown_charts() {
        let Some(market) = mappings.market(&source.symbol) else {
            left_out.push(LeftOut::Chart(source.symbol.clone()));
            continue;
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
            title: (market.source == crate::model::DataSource::DexScreener)
                .then(|| market.name.clone()),
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
        building.push(Building {
            chart,
            symbol: source.symbol.clone(),
            drawings: BTreeMap::new(),
            next_drawing: 1,
        });
    }

    let drawings = if building.is_empty() {
        left_out.push(LeftOut::NoChart);
        0
    } else {
        file_drawings(found, mappings, ticks, &mut building, &mut left_out)
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

    // The grid's columns, filled row by row as TradingView numbers its charts.
    let columns = found.grid.columns.clamp(1, count.max(1));
    let mut grid: Vec<Vec<u64>> = vec![Vec::new(); columns];
    for (index, chart) in charts.iter().enumerate() {
        grid[index % columns].push(chart.id);
    }
    let columns = grid
        .into_iter()
        .enumerate()
        .filter(|(_, ids)| !ids.is_empty())
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
        if built_in && let Some((_, native)) = NATIVE.iter().find(|(id, _)| *id == kind) {
            native.set_applied(&mut chart.indicators, true);
            if *native == BuiltIn::Ema {
                if let Some(length) = study.length.filter(|length| *length > 0) {
                    chart.indicators.ema_period = length as usize;
                }
            } else if *native == BuiltIn::Bollinger && study.length.is_some_and(|l| l != 20) {
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
    ticks: &BTreeMap<&TvSymbol, f64>,
    building: &mut [Building],
    left_out: &mut Vec<LeftOut>,
) -> usize {
    let mut ordered: Vec<&RawDrawing> = found.drawings.iter().collect();
    ordered.sort_by(|a, b| a.z.total_cmp(&b.z).then_with(|| a.id.cmp(&b.id)));

    let mut filed = 0;
    let mut unmapped: BTreeMap<&TvSymbol, usize> = BTreeMap::new();
    let mut tools: BTreeMap<&str, usize> = BTreeMap::new();
    let (mut pictures, mut positions, mut broken, mut cut) = (0, 0, 0, 0);
    for raw in ordered {
        let Some(market) = mappings.market(&raw.symbol) else {
            *unmapped.entry(&raw.symbol).or_default() += 1;
            continue;
        };
        let index = building
            .iter()
            .position(|chart| chart.symbol == raw.symbol)
            .unwrap_or(0);
        let target = &mut building[index];
        match convert::convert(raw, target.next_drawing, ticks.get(&raw.symbol).copied()) {
            Ok(converted) => {
                let key = InstrumentId::new(market.source, &market.symbol).key();
                target
                    .drawings
                    .entry(key)
                    .or_default()
                    .push(converted.drawing);
                target.next_drawing += 1;
                filed += 1;
                cut += usize::from(converted.cut_text);
            }
            Err(Unsupported::Tool) => {
                *tools.entry(convert::class_label(&raw.tool)).or_default() += 1;
            }
            Err(Unsupported::Picture) => pictures += 1,
            Err(Unsupported::PriceStep) => positions += 1,
            Err(Unsupported::Points) => broken += 1,
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
    for (count, entry) in [
        (pictures, LeftOut::Pictures(pictures)),
        (positions, LeftOut::PositionsWithoutPriceStep(positions)),
        (broken, LeftOut::BrokenDrawings(broken)),
        (cut, LeftOut::CutText(cut)),
    ] {
        if count > 0 {
            left_out.push(entry);
        }
    }
    filed
}

//! The positions, the position forecast, the bars pattern, the ghost feed, the sector and the
//! measurers projected onto the plot. Each function turns one geometry and the projection into a
//! [`Screen`]; the arithmetic is in `model::{position, measure, ghost, bars_pattern}`.
//!
//! Everything a number says is worked from the stored prices and the bars the chart draws (never a
//! pixel), and nothing is read from a bar the chart does not have, so during a replay a position
//! or a forecast knows only what the replay has shown. A position is a **drawing**: nothing here
//! can place, change or cancel an order.

use super::Projection;
use super::shape::{
    Candle, CandleLook, ColorBox, LabelLook, LabelPlace, PivotLabel, Screen, Tag, vertex_handles,
    with_heads,
};
use crate::format::group_thousands;
use crate::model::bars_pattern::{BarsMode, BarsPattern};
use crate::model::chart_type::Rgba;
use crate::model::drawing_style::{DrawingStyle, LineEnd};
use crate::model::forecast_style::{GhostOptions, PositionOptions};
use crate::model::measure::{self, MeasureKind};
use crate::model::position::{self, ForecastResult, Outcome, Side, Sizing};
use crate::model::watchlist_quote::compact_number;
use crate::model::{Anchor, Geometry, PriceFormat, Tool, ghost};
use crate::ui::drawing_geometry as geo;
use crate::ui::drawing_render::RulerReadout;
use eframe::egui::{Pos2, Rect, pos2, vec2};

/// How far (px) beyond a shape its labels may reach, for its bounds: a position's stats are wide.
const POSITION_REACH: f32 = 260.0;
const LABEL_REACH: f32 = 60.0;
/// How tall, as a fraction of the plot, a new position's target and stop are, and how wide it is.
const TARGET_HEIGHT: f32 = 0.10;
const STOP_HEIGHT: f32 = 0.05;
const WIDTH_FRACTION: f32 = 0.20;
const MIN_WIDTH: f32 = 60.0;
/// The ink of a label on a plate of its own colour.
const WHITE: Rgba = [255, 255, 255, 255];

/// Builds the screen shape of a position, a forecast, a bars pattern, a ghost feed, a sector or a
/// measurer. `anchors` are the geometry's anchors on screen, already checked to be finite.
pub(super) fn project(
    geometry: &Geometry,
    style: &DrawingStyle,
    proj: &Projection,
    anchors: Vec<Pos2>,
) -> Option<Screen> {
    let (screen, reach) = match geometry {
        Geometry::Position {
            side,
            entry,
            target,
            stop,
        } => (
            position(*side, [*entry, *target, *stop], style, proj, &anchors),
            POSITION_REACH,
        ),
        Geometry::Forecast { source, target } => (
            forecast(*source, *target, style, proj, &anchors),
            POSITION_REACH,
        ),
        Geometry::Bars(pattern) => (bars_pattern(pattern, style, proj, &anchors), LABEL_REACH),
        Geometry::Ghost { start, end, seed } => (
            ghost_feed(*start, *end, *seed, &style.ghost, proj, &anchors),
            LABEL_REACH,
        ),
        Geometry::Sector { .. } => (sector(&anchors), LABEL_REACH),
        Geometry::Measure { kind, a, b } => (
            measurement(*kind, *a, *b, style, proj, &anchors),
            POSITION_REACH,
        ),
        _ => return None,
    };
    screen?.finish(style.stroke.width / 2.0 + reach)
}

fn sizing(options: &PositionOptions) -> Sizing {
    Sizing {
        account: options.account,
        lot: options.lot,
        risk: options.risk,
        risk_is_percent: options.risk_percent,
        leverage: options.leverage,
        // No contract multiplier is known for any market here.
        point_value: 1.0,
        qty_precision: options.qty_precision,
    }
}

/// The bar a time falls on, as an index into the bars the chart draws (never before the first).
fn bar_at(proj: &Projection, time_ms: i64) -> usize {
    proj.times.index_of(time_ms).round().max(0.0) as usize
}

fn solid(color: Rgba) -> Rgba {
    [color[0], color[1], color[2], 255]
}

/// A label on a plate of its own colour.
fn plate(
    at: Pos2,
    text: String,
    fill: Rgba,
    ink: Rgba,
    place: LabelPlace,
    push: f32,
) -> PivotLabel {
    PivotLabel {
        at,
        text,
        look: LabelLook::Plate { fill, ink },
        place,
        push: vec2(0.0, push),
        edge: vec2(0.0, 0.0),
        on_select: false,
    }
}

/// The bars pattern a person has chosen with three clicks: the bars the chart draws from the bar of
/// the first to the bar of the second, set down at the third. `None` for fewer than three anchors
/// or nothing to copy.
pub(super) fn captured_pattern(anchors: &[Anchor], proj: &Projection) -> Option<Geometry> {
    let [from, to, at, ..] = anchors else {
        return None;
    };
    BarsPattern::capture(
        proj.times.bars(),
        bar_at(proj, from.time_ms),
        bar_at(proj, to.time_ms),
        *at,
    )
    .map(Geometry::Bars)
}

/// What a bars pattern shows while it is placed: the range being chosen (a date range) after the
/// first click, and the bars themselves following the pointer after the second.
pub(super) fn bars_preview(anchors: &[Anchor], proj: &Projection) -> Option<Geometry> {
    match anchors {
        [from, to] => Some(Geometry::Measure {
            kind: MeasureKind::Date,
            a: *from,
            b: *to,
        }),
        [_, _, _, ..] => captured_pattern(anchors, proj),
        _ => None,
    }
}

// MARK: Text

/// `value` with its sign and the thousands grouped: `-1,250.00`, `3.5`. Money is shown to the cent
/// below a thousand and to the unit above, as an amount rather than a price.
fn money(value: f64) -> String {
    if !value.is_finite() {
        return "—".to_owned();
    }
    let text = if value.abs() >= 1_000.0 {
        format!("{value:.0}")
    } else {
        format!("{value:.2}")
    };
    group_thousands(&text)
}

/// A quantity to `places` decimals, trailing zeros dropped.
fn quantity_text(value: f64, places: u8) -> String {
    let text = format!("{value:.*}", usize::from(places.min(8)));
    let trimmed = if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        text
    };
    group_thousands(&trimmed)
}

/// A price distance as the chart writes it, with a minus when it is negative.
fn distance_text(format: PriceFormat, distance: f64) -> String {
    if distance < 0.0 {
        format!("-{}", format.format(distance.abs()))
    } else {
        format.format(distance)
    }
}

/// `{price} ({percent}) {ticks}`, each part only when it can be said.
fn offset_text(
    format: PriceFormat,
    distance: f64,
    percent: Option<f64>,
    ticks: Option<f64>,
) -> String {
    let mut parts = vec![distance_text(format, distance)];
    if let Some(percent) = percent {
        parts.push(format!("({percent:.2}%)"));
    }
    if let Some(ticks) = ticks {
        parts.push(group_thousands(&format!("{:.0}", ticks.round())));
    }
    parts.join(" ")
}

// MARK: Positions

/// The entry, target and stop of a new position of `tool` at `entry`, sized from the view: the
/// target a tenth of the plot's height away, the stop half that, the right edge a fifth of its
/// width on (**inferred**: TradingView does not say how it sizes one). `None` for a tool that is
/// not a position or a point that is not on the screen.
pub(super) fn sized_position(tool: Tool, entry: Anchor, proj: &Projection) -> Option<[Anchor; 3]> {
    let side = match tool {
        Tool::LongPosition => Side::Long,
        Tool::ShortPosition => Side::Short,
        _ => return None,
    };
    let at = proj.point(entry);
    let width = (proj.plot.width() * WIDTH_FRACTION).max(MIN_WIDTH);
    let target = pos2(
        at.x + width,
        at.y - side.sign() as f32 * proj.plot.height() * TARGET_HEIGHT,
    );
    let stop = pos2(
        at.x + width,
        at.y + side.sign() as f32 * proj.plot.height() * STOP_HEIGHT,
    );
    let time_ms = proj.time(at.x + width);
    let sized = [
        entry,
        Anchor {
            time_ms,
            price: proj.scale.price(target.y),
        },
        Anchor {
            time_ms,
            price: proj.scale.price(stop.y),
        },
    ];
    sized.iter().all(Anchor::is_finite).then_some(sized)
}

fn position(
    side: Side,
    stored: [Anchor; 3],
    style: &DrawingStyle,
    proj: &Projection,
    points: &[Pos2],
) -> Option<Screen> {
    let [entry, target, stop] = stored;
    let (at_entry, at_target, at_stop) = (points[0], points[1], points[2]);
    let options = &style.position;
    let (left, right) = (at_entry.x.min(at_target.x), at_entry.x.max(at_target.x));
    let middle = (left + right) / 2.0;
    let mut screen = Screen::empty();
    screen.handles = vertex_handles(points);

    // The zones, and the three levels across them.
    if right - left >= 0.5 {
        for (end, color) in [(at_target.y, options.profit), (at_stop.y, options.stop)] {
            screen.boxes.push(ColorBox {
                rect: Rect::from_min_max(
                    pos2(left, at_entry.y.min(end)),
                    pos2(right, at_entry.y.max(end)),
                ),
                color,
            });
        }
    }
    for y in [at_entry.y, at_target.y, at_stop.y] {
        screen.lines.push(vec![pos2(left, y), pos2(right, y)]);
    }

    // What the trade works out to.
    let stats = position::stats(
        side,
        entry.price,
        stop.price,
        target.price,
        &sizing(options),
    );
    let tick = proj.format.tick_size(entry.price);
    let ticks = |distance: f64| measure::ticks_between(0.0, distance, tick);
    let bars = proj.times.bars();
    let outcome = position::outcome(
        side,
        stop.price,
        target.price,
        bars,
        bar_at(proj, entry.time_ms),
        bar_at(proj, target.time_ms),
    );
    let when_selected = !options.always_stats;
    let away = |y: f32| if y < at_entry.y { -1.0 } else { 1.0 };
    let mut tag =
        |y: f32, name: &str, distance: f64, percent: Option<f64>, balance: Option<f64>| {
            let mut text = if options.compact {
                offset_text(proj.format, distance, percent, None)
            } else {
                format!(
                    "{name}: {}",
                    offset_text(proj.format, distance, percent, ticks(distance))
                )
            };
            if let Some(balance) = balance.filter(|_| !options.compact) {
                text.push_str(&format!(", Amount: {}", money(balance)));
            }
            let mut label = plate(
                pos2(middle, y),
                text,
                options.label,
                style.text.color,
                LabelPlace::Beside,
                away(y),
            );
            label.on_select = when_selected;
            screen.labels.push(label);
        };
    tag(
        at_target.y,
        "Target",
        stats.target_distance,
        stats.target_percent,
        stats.balance_at_target,
    );
    tag(
        at_stop.y,
        "Stop",
        stats.stop_distance,
        stats.stop_percent,
        stats.balance_at_stop,
    );

    // The middle: what it is, how it stands, its size and its reward over its risk. A position is
    // a drawing, not an order, and says so.
    let name = match side {
        Side::Long => "Long",
        Side::Short => "Short",
    };
    let (standing, fill) = match outcome {
        Some(Outcome::Target { .. }) => (
            stats
                .profit
                .map(|profit| format!("Closed P&L: {}", signed_money(profit))),
            options.success,
        ),
        Some(Outcome::Stop { .. }) => (
            stats
                .loss
                .map(|loss| format!("Closed P&L: {}", signed_money(loss))),
            options.failure,
        ),
        Some(Outcome::Open { last }) => (
            stats
                .quantity
                .and_then(|quantity| {
                    position::profit_at(side, entry.price, last, quantity, &sizing(options))
                })
                .map(|open| format!("Open P&L: {}", signed_money(open))),
            options.label,
        ),
        None => (None, options.label),
    };
    let standing = standing.unwrap_or_else(|| "P&L: —".to_owned());
    let quantity = stats.quantity.map_or_else(
        || "—".to_owned(),
        |quantity| quantity_text(quantity, options.qty_precision),
    );
    let mut lines = vec![format!("{name} (hypothetical)")];
    if options.compact {
        lines.push(standing);
    } else {
        lines.push(format!("{standing}, Qty: {quantity}"));
        lines.push(format!(
            "Risk/Reward Ratio: {}",
            stats
                .risk_reward
                .map_or_else(|| "—".to_owned(), |ratio| format!("{ratio:.2}"))
        ));
    }
    let mut label = plate(
        pos2(middle, at_entry.y),
        lines.join("\n"),
        fill,
        style.text.color,
        LabelPlace::On,
        -1.0,
    );
    label.on_select = when_selected;
    screen.labels.push(label);

    // The prices, on the axis.
    if style.price_labels {
        for (y, price, color) in [
            (at_entry.y, entry.price, style.stroke.color),
            (at_target.y, target.price, solid(options.profit)),
            (at_stop.y, stop.price, solid(options.stop)),
        ] {
            if y >= proj.plot.top() && y <= proj.plot.bottom() {
                screen.tags.push(Tag::Price { y, price, color });
            }
        }
    }
    Some(screen)
}

/// A profit or a loss with its sign: `+12.30`, `-4`.
fn signed_money(value: f64) -> String {
    if value > 0.0 {
        format!("+{}", money(value))
    } else {
        money(value)
    }
}

// MARK: Position forecast

fn forecast(
    source: Anchor,
    target: Anchor,
    style: &DrawingStyle,
    proj: &Projection,
    points: &[Pos2],
) -> Option<Screen> {
    let (from, to) = (points[0], points[1]);
    let options = &style.position;
    let mut screen = Screen::empty();
    let (line, heads) = with_heads(
        vec![from, to],
        LineEnd::Plain,
        LineEnd::Arrow,
        style.stroke.width,
    );
    screen.lines.push(line);
    screen.solids = heads;
    screen.handles = vertex_handles(points);

    let upward = to.y <= from.y;
    let plate_fill = style.stroke.color;
    // The source says where it starts; the target how far it goes and how long it takes.
    screen.labels.push(plate(
        from,
        proj.format.format(source.price),
        plate_fill,
        style.text.color,
        LabelPlace::Beside,
        if upward { 1.0 } else { -1.0 },
    ));
    let bars = proj.times.bars_between(source.time_ms, target.time_ms);
    let readout = RulerReadout::new(
        source.price,
        target.price,
        bars,
        (target.time_ms - source.time_ms) / 1000,
    );
    screen.labels.push(plate(
        to,
        format!(
            "{} ({})\n{}",
            readout.price_text(proj.format),
            readout.percent_text(),
            readout.coverage_text()
        ),
        plate_fill,
        style.text.color,
        LabelPlace::Beside,
        if upward { -1.0 } else { 1.0 },
    ));
    // Whether it came true, once the bars say.
    let result = position::forecast_result(
        source.price,
        target.price,
        proj.times.bars(),
        bar_at(proj, source.time_ms),
        bar_at(proj, target.time_ms),
    );
    if let Some(result) = result {
        let (text, fill) = match result {
            ForecastResult::Success => ("Success", options.success),
            ForecastResult::Failure => ("Failure", options.failure),
        };
        screen.labels.push(plate(
            to,
            text.to_owned(),
            fill,
            WHITE,
            LabelPlace::Beside,
            if upward { 1.0 } else { -1.0 },
        ));
    }
    Some(screen)
}

// MARK: Bars pattern

/// Half the width of a candle or a bar's ticks in a chart whose slots are `slot` px wide.
fn half_width(slot: f32) -> f32 {
    (slot * 0.32).clamp(0.75, 14.0)
}

fn bars_pattern(
    pattern: &BarsPattern,
    style: &DrawingStyle,
    proj: &Projection,
    points: &[Pos2],
) -> Option<Screen> {
    let mut screen = Screen::empty();
    screen.handles = vertex_handles(points);
    let first = proj.times.index_of(pattern.at.time_ms);
    let half = half_width(proj.slot_width);
    let color = style.stroke.color;
    let plot = proj.plot.expand2(vec2(half + 2.0, 0.0));
    let mut path: Vec<Pos2> = Vec::new();
    for (index, bar) in pattern.shown().iter().enumerate() {
        let x = proj.x_at_index(first + index as f64);
        if x < plot.left() || x > plot.right() {
            continue;
        }
        let y = |offset: f64| proj.scale.y(pattern.at.price + offset);
        let [open, high, low, close] = [y(bar[0]), y(bar[1]), y(bar[2]), y(bar[3])];
        if let Some(price) = pattern.mode.line_price(bar) {
            path.push(pos2(x, y(price)));
            continue;
        }
        let look = match pattern.mode {
            BarsMode::Candles => CandleLook::Body {
                // A rising candle is hollow-ish, a falling one solid, in the one colour.
                fill: if close <= open {
                    [color[0], color[1], color[2], color[3] / 3]
                } else {
                    color
                },
                border: Some(color),
                wick: Some(color),
            },
            _ => CandleLook::Stick { color },
        };
        screen.candles.push(Candle {
            x,
            open,
            high,
            low,
            close,
            half,
            look,
        });
        screen.hit.push(Rect::from_min_max(
            pos2(x - half, high.min(low)),
            pos2(x + half, high.max(low)),
        ));
    }
    if path.len() >= 2 {
        screen.lines.push(path);
    }
    Some(screen)
}

// MARK: Ghost feed

fn ghost_feed(
    start: Anchor,
    end: Anchor,
    seed: u64,
    options: &GhostOptions,
    proj: &Projection,
    points: &[Pos2],
) -> Option<Screen> {
    let mut screen = Screen::empty();
    screen.handles = vertex_handles(points);
    // Chronologically: the run goes from the earlier anchor's price to the later one's, whichever
    // was placed first (a run "backward" is simply placed from the right).
    let (earlier, later) = if start.time_ms <= end.time_ms {
        (start, end)
    } else {
        (end, start)
    };
    let (from, to) = (
        proj.times.index_of(earlier.time_ms).round(),
        proj.times.index_of(later.time_ms).round(),
    );
    let count = ((to - from).max(1.0) as usize).min(ghost::MAX_CANDLES);
    let base = if earlier.price != 0.0 {
        earlier.price.abs()
    } else {
        later.price.abs()
    };
    // The size of a candle: what was asked for, or what the bars before it were.
    let average = if options.average > 0.0 {
        f64::from(options.average) / 100.0 * base
    } else {
        ghost::recent_range(
            proj.times.bars(),
            from.max(0.0) as usize,
            ghost::RANGE_WINDOW,
        )
        .unwrap_or(0.005 * base)
    };
    let candles = ghost::candles(
        earlier.price,
        later.price,
        count,
        average,
        f64::from(options.variance) / 100.0,
        seed,
    );
    let half = half_width(proj.slot_width);
    let plot = proj.plot.expand2(vec2(half + 2.0, 0.0));
    for (index, candle) in candles.iter().enumerate() {
        let x = proj.x_at_index(from + (index + 1) as f64);
        if x < plot.left() || x > plot.right() {
            continue;
        }
        let [open, high, low, close] = [
            proj.scale.y(candle.open),
            proj.scale.y(candle.high),
            proj.scale.y(candle.low),
            proj.scale.y(candle.close),
        ];
        let up = candle.close >= candle.open;
        screen.candles.push(Candle {
            x,
            open,
            high,
            low,
            close,
            half,
            look: CandleLook::Body {
                fill: if up { options.up } else { options.down },
                border: options.draw_border.then_some(if up {
                    options.up_border
                } else {
                    options.down_border
                }),
                wick: options.draw_wick.then_some(options.wick),
            },
        });
        screen.hit.push(Rect::from_min_max(
            pos2(x - half, high.min(low)),
            pos2(x + half, high.max(low)),
        ));
    }
    Some(screen)
}

// MARK: Sector

fn sector(points: &[Pos2]) -> Option<Screen> {
    let (center, edge, tip) = (points[0], points[1], points[2]);
    let mut screen = Screen::empty();
    screen.handles = vertex_handles(points);
    screen.guides.push([center, tip]);
    let radius = center.distance(edge);
    if radius < 1.0 {
        return Some(screen);
    }
    // From the ray to the edge to the ray to the tip, the short way round.
    let start = (edge.y - center.y).atan2(edge.x - center.x);
    let end = (tip.y - center.y).atan2(tip.x - center.x);
    let mut sweep = end - start;
    while sweep > std::f32::consts::PI {
        sweep -= std::f32::consts::TAU;
    }
    while sweep < -std::f32::consts::PI {
        sweep += std::f32::consts::TAU;
    }
    let arc = geo::Arc {
        center,
        radius,
        start_angle: start,
        sweep,
    }
    .flatten();
    let mut polygon = vec![center];
    polygon.extend(arc);
    screen.outlines.push(polygon.clone());
    screen.fill = polygon;
    Some(screen)
}

// MARK: Measurers

fn measurement(
    kind: MeasureKind,
    a: Anchor,
    b: Anchor,
    style: &DrawingStyle,
    proj: &Projection,
    points: &[Pos2],
) -> Option<Screen> {
    let (pa, pb) = (points[0], points[1]);
    let options = &style.measure;
    let plot = proj.plot;
    // A price range is a box over the width of the drag, a date range one over the plot's height.
    let (top, bottom) = if kind.has_price() {
        (pa.y.min(pb.y), pa.y.max(pb.y))
    } else {
        (plot.top(), plot.bottom())
    };
    let (left, right) = if kind.has_time() {
        (pa.x.min(pb.x), pa.x.max(pb.x))
    } else {
        (pa.x.min(pb.x), pa.x.max(pb.x).max(pa.x.min(pb.x) + 1.0))
    };
    let mut screen = Screen::empty();
    screen.handles = vertex_handles(points);
    let rect = Rect::from_min_max(pos2(left, top), pos2(right, bottom));
    let polygon = vec![
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
    ];
    screen.outlines.push(polygon.clone());
    screen.fill = polygon;
    screen.hit.push(rect);

    // The arrows: one across the price move, one along the time.
    let middle = rect.center();
    let mut arrow = |from: Pos2, to: Pos2| {
        let (line, heads) = with_heads(
            vec![from, to],
            LineEnd::Plain,
            LineEnd::Arrow,
            style.stroke.width,
        );
        screen.lines.push(line);
        screen.solids.extend(heads);
    };
    if kind.has_price() && (pb.y - pa.y).abs() > 1.0 {
        arrow(pos2(middle.x, pa.y), pos2(middle.x, pb.y));
    }
    if kind.has_time() && (pb.x - pa.x).abs() > 1.0 {
        let y = if kind.has_price() {
            middle.y
        } else {
            ((pa.y + pb.y) / 2.0).clamp(plot.top(), plot.bottom())
        };
        arrow(pos2(pa.x, y), pos2(pb.x, y));
    }

    // What it says.
    let tick = proj.format.tick_size(a.price);
    let secs = (b.time_ms - a.time_ms) / 1000;
    let readout = RulerReadout::new(
        a.price,
        b.price,
        proj.times.bars_between(a.time_ms, b.time_ms),
        secs,
    );
    let mut lines: Vec<String> = Vec::new();
    if kind.has_price() {
        let mut parts: Vec<String> = Vec::new();
        if options.price {
            parts.push(readout.price_text(proj.format));
        }
        if options.percent && measure::percent_change(a.price, b.price).is_some() {
            parts.push(format!("({})", readout.percent_text()));
        }
        if options.ticks
            && let Some(ticks) = measure::ticks_between(a.price, b.price, tick)
        {
            let ticks = ticks.round();
            parts.push(format!(
                "{}{}",
                if ticks > 0.0 { "+" } else { "" },
                group_thousands(&format!("{ticks:.0}"))
            ));
        }
        if !parts.is_empty() {
            lines.push(parts.join(" "));
        }
    }
    if kind.has_time() {
        let mut parts: Vec<String> = Vec::new();
        if options.bars {
            parts.push(readout.bars_text());
        }
        let duration = readout.duration_text();
        if options.time && !duration.is_empty() {
            parts.push(duration);
        }
        if !parts.is_empty() {
            lines.push(parts.join(", "));
        }
        if options.volume
            && let Some(volume) = measure::volume_over(
                proj.times.bars(),
                proj.times.index_of(a.time_ms),
                proj.times.index_of(b.time_ms),
            )
        {
            lines.push(format!("Volume: {}", compact_number(volume)));
        }
    }
    if !lines.is_empty() {
        let (at, place, push) = match kind {
            MeasureKind::Date => (middle, LabelPlace::On, -1.0),
            _ => {
                let up = pb.y <= pa.y;
                (
                    pos2(middle.x, if up { top } else { bottom }),
                    LabelPlace::Beside,
                    if up { -1.0 } else { 1.0 },
                )
            }
        };
        screen.labels.push(plate(
            at,
            lines.join("\n"),
            options.label,
            WHITE,
            place,
            push,
        ));
    }
    // A note of the user's sits above the box.
    screen.text = Some(super::shape::TextPlace::At {
        point: pos2(middle.x, top),
        above: true,
    });
    Some(screen)
}

//! The little chart on a card: a fixed synthetic candle series with the indicator drawn over or
//! under it. Native indicators are computed on the spot; Pine ones run once on a worker thread
//! (the first time the picker opens) and are cached, so a frame never waits for a script.

use crate::model::indicator_catalog::{CatalogEntry, EntryKey};
use crate::model::{BuiltIn, Kline, bollinger, ema, rsi, supertrend_flips};
use crate::pine::outputs::{MarkerLocation, PlotStyle};
use crate::pine::program::SymbolInfo;
use crate::pine_studio::indicators;
use crate::ui::pine_geometry::rgba;
use crate::ui::theme;
use eframe::egui::{self, Color32, Painter, Pos2, Rect, Shape, Stroke, pos2};
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, OnceLock};

const BARS: usize = 120;

/// One drawn series, aligned to the preview's bars.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub values: Vec<Option<f64>>,
    pub color: Color32,
    /// Drawn as columns up from zero (a histogram) instead of a line.
    pub columns: bool,
}

/// A mark above or below a bar.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mark {
    pub bar: usize,
    pub above: bool,
    pub color: Color32,
}

/// What an indicator draws over the synthetic bars.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Preview {
    /// On the price pane; otherwise in a pane under the candles.
    pub overlay: bool,
    pub lines: Vec<Line>,
    pub levels: Vec<f64>,
    pub marks: Vec<Mark>,
}

/// The series every preview is drawn over: a rise, a pull-back and a second leg with wicks and
/// uneven volume, so trend, momentum and volume indicators all have something to show.
pub fn bars() -> &'static Arc<Vec<Kline>> {
    static BARS_CELL: OnceLock<Arc<Vec<Kline>>> = OnceLock::new();
    BARS_CELL.get_or_init(|| {
        let mut previous = 100.0_f64;
        let series = (0..BARS)
            .map(|i| {
                let t = i as f64;
                let trend = (t * 0.11).sin() * 15.0 + (t * 0.031).cos() * 7.0 + t * 0.08;
                let wobble = (t * 0.9).sin() * 0.9 + (t * 0.37).cos() * 0.6;
                let close = 100.0 + trend + wobble;
                let open = previous;
                previous = close;
                let wick = 0.8 + (t * 1.3).sin().abs() * 1.6;
                Kline {
                    open_time: 1_700_000_000_000 + i as i64 * 3_600_000,
                    close_time: 1_700_000_000_000 + (i as i64 + 1) * 3_600_000 - 1,
                    open,
                    high: open.max(close) + wick,
                    low: open.min(close) - wick,
                    close,
                    volume: 800.0 + (t * 0.5).sin().abs() * 900.0 + (t * 0.13).cos().abs() * 400.0,
                }
            })
            .collect();
        Arc::new(series)
    })
}

fn symbol() -> SymbolInfo {
    SymbolInfo {
        ticker: "PREVIEW".into(),
        ticker_id: "DEGENVIEW:PREVIEW".into(),
        currency: "USD".into(),
        kind: "crypto".into(),
        base_currency: None,
        ..SymbolInfo::default()
    }
}

fn line(values: Vec<Option<f64>>, color: Color32) -> Line {
    Line {
        values,
        color,
        columns: false,
    }
}

/// The preview of a native indicator, computed from the shared bars.
pub fn native(kind: BuiltIn) -> Preview {
    let bars = bars();
    match kind {
        BuiltIn::Rsi => Preview {
            overlay: false,
            lines: vec![line(rsi(bars, 14), theme::RSI)],
            levels: vec![70.0, 30.0],
            ..Preview::default()
        },
        BuiltIn::Ema => Preview {
            overlay: true,
            lines: vec![line(ema(bars, 20), theme::EMA)],
            ..Preview::default()
        },
        BuiltIn::Bollinger => {
            let (upper, middle, lower) = bollinger(bars, 20, 2.0);
            Preview {
                overlay: true,
                lines: vec![
                    line(upper, theme::BOLLINGER),
                    line(middle, theme::BOLLINGER.gamma_multiply(0.6)),
                    line(lower, theme::BOLLINGER),
                ],
                ..Preview::default()
            }
        }
        BuiltIn::TrendFlips => Preview {
            overlay: true,
            marks: supertrend_flips(bars, 10, 3.0)
                .into_iter()
                .enumerate()
                .filter_map(|(bar, flip)| {
                    flip.map(|up| Mark {
                        bar,
                        above: !up,
                        color: if up { theme::UP } else { theme::DOWN },
                    })
                })
                .collect(),
            ..Preview::default()
        },
        BuiltIn::Volume => Preview {
            overlay: false,
            lines: vec![Line {
                values: bars.iter().map(|bar| Some(bar.volume)).collect(),
                color: theme::UP.gamma_multiply(0.6),
                columns: true,
            }],
            ..Preview::default()
        },
    }
}

/// The preview of a Pine script: its plots, horizontal levels and markers over the shared bars.
/// `None` when it fails to run.
pub fn pine(source: &str) -> Option<Preview> {
    let (output, failure) =
        crate::host::pine_host::run_script_checked(source, bars(), &symbol(), &HashMap::new());
    if failure.is_some() {
        return None;
    }
    let mut preview = Preview {
        overlay: output.overlay,
        levels: output.hlines.iter().map(|level| level.value).collect(),
        ..Preview::default()
    };
    for plot in &output.plots {
        if plot.force_overlay || !crate::ui::pine_geometry::is_visible(plot.color) {
            continue;
        }
        let mut values = plot.values.clone();
        values.resize(BARS, None);
        preview.lines.push(Line {
            values,
            color: rgba(plot.color),
            columns: matches!(plot.style, PlotStyle::Columns | PlotStyle::Histogram),
        });
    }
    for marker in &output.markers {
        for (bar, shown) in marker.values.iter().enumerate() {
            if *shown && bar < BARS {
                preview.marks.push(Mark {
                    bar,
                    above: matches!(marker.location, MarkerLocation::AboveBar),
                    color: rgba(
                        marker
                            .colors
                            .get(bar)
                            .copied()
                            .flatten()
                            .unwrap_or(marker.color),
                    ),
                });
            }
        }
    }
    (!preview.lines.is_empty() || !preview.marks.is_empty()).then_some(preview)
}

type Finished = (EntryKey, Option<Preview>);

/// The cache the picker draws from.
#[derive(Default)]
pub struct Previews {
    ready: HashMap<EntryKey, Option<Arc<Preview>>>,
    receiver: Option<Receiver<Finished>>,
}

impl Previews {
    /// Starts the worker that runs every bundled script, once.
    fn start(&mut self, ctx: &egui::Context) {
        if self.receiver.is_some() {
            return;
        }
        let (sender, receiver) = channel();
        self.receiver = Some(receiver);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            for indicator in &indicators::ALL {
                let preview = pine(indicator.source);
                if sender
                    .send((EntryKey::Bundled(indicator.id), preview))
                    .is_err()
                {
                    return;
                }
                ctx.request_repaint();
            }
        });
    }

    /// The preview of `entry` if it is ready; starts the work the first time it is asked for.
    pub fn get(&mut self, ctx: &egui::Context, entry: &CatalogEntry) -> Option<Arc<Preview>> {
        self.start(ctx);
        if let Some(receiver) = &self.receiver {
            while let Ok((key, preview)) = receiver.try_recv() {
                self.ready.insert(key, preview.map(Arc::new));
            }
        }
        match &entry.key {
            EntryKey::Native(kind) => self
                .ready
                .entry(entry.key.clone())
                .or_insert_with(|| Some(Arc::new(native(*kind))))
                .clone(),
            EntryKey::Bundled(_) => self.ready.get(&entry.key).cloned().flatten(),
            EntryKey::Script(_) => None,
        }
    }

    /// Whether `entry` has no preview and will not get one (a script, or one that failed).
    pub fn is_missing(&self, entry: &CatalogEntry) -> bool {
        match &entry.key {
            EntryKey::Script(_) => true,
            EntryKey::Native(_) => false,
            EntryKey::Bundled(_) => matches!(self.ready.get(&entry.key), Some(None)),
        }
    }
}

/// Paints the candles and `preview` into `rect`. With no preview only the candles are drawn.
pub fn paint(painter: &Painter, rect: Rect, preview: Option<&Preview>) {
    let painter = painter.with_clip_rect(rect);
    let bars = bars();
    let overlay = preview.is_none_or(|preview| preview.overlay);
    let price = if overlay {
        rect
    } else {
        Rect::from_min_max(
            rect.min,
            pos2(rect.right(), rect.top() + rect.height() * 0.58),
        )
    };
    let (mut low, mut high) = bars.iter().fold((f64::MAX, f64::MIN), |(low, high), bar| {
        (low.min(bar.low), high.max(bar.high))
    });
    if overlay && let Some(preview) = preview {
        // Bands and clouds that leave the candles' range still fit.
        for value in preview
            .lines
            .iter()
            .filter(|line| !line.columns)
            .flat_map(|line| line.values.iter().flatten())
            .filter(|value| value.is_finite())
        {
            low = low.min(*value);
            high = high.max(*value);
        }
    }
    let inset = price.shrink2(egui::vec2(4.0, 6.0));
    let x_of = |i: usize| inset.left() + inset.width() * (i as f32 + 0.5) / bars.len() as f32;
    let y_of = |value: f64| {
        inset.bottom() - inset.height() * ((value - low) / (high - low).max(1e-9)) as f32
    };
    let step = inset.width() / bars.len() as f32;
    for (i, bar) in bars.iter().enumerate() {
        let color = if bar.close >= bar.open {
            theme::UP
        } else {
            theme::DOWN
        }
        .gamma_multiply(if preview.is_some() { 0.45 } else { 0.8 });
        let x = x_of(i);
        painter.line_segment(
            [pos2(x, y_of(bar.high)), pos2(x, y_of(bar.low))],
            Stroke::new(0.8, color),
        );
        let (top, bottom) = (y_of(bar.open.max(bar.close)), y_of(bar.open.min(bar.close)));
        painter.rect_filled(
            Rect::from_min_max(
                pos2(x - step * 0.32, top),
                pos2(x + step * 0.32, bottom.max(top + 0.8)),
            ),
            0.0,
            color,
        );
    }
    let Some(preview) = preview else { return };

    if overlay {
        draw_lines(&painter, preview, inset, &x_of, &|value| y_of(value), step);
        for mark in &preview.marks {
            let Some(bar) = bars.get(mark.bar) else {
                continue;
            };
            let y = if mark.above {
                y_of(bar.high) - 4.0
            } else {
                y_of(bar.low) + 4.0
            };
            painter.circle_filled(pos2(x_of(mark.bar), y), 2.2, mark.color);
        }
        return;
    }

    // A pane under the candles with its own scale.
    let pane = Rect::from_min_max(pos2(rect.left(), price.bottom()), rect.max);
    painter.hline(pane.x_range(), pane.top(), Stroke::new(1.0, theme::ink(18)));
    let inner = pane.shrink2(egui::vec2(4.0, 5.0));
    let mut values: Vec<f64> = preview.levels.clone();
    for line in &preview.lines {
        values.extend(line.values.iter().flatten());
        if line.columns {
            values.push(0.0);
        }
    }
    let (low, high) = values.iter().fold((f64::MAX, f64::MIN), |(low, high), v| {
        (low.min(*v), high.max(*v))
    });
    if low > high {
        return;
    }
    let span = (high - low).max(1e-9);
    let y_pane = |value: f64| inner.bottom() - inner.height() * ((value - low) / span) as f32;
    for level in &preview.levels {
        painter.hline(
            inner.x_range(),
            y_pane(*level),
            Stroke::new(0.8, theme::ink(30)),
        );
    }
    draw_lines(&painter, preview, inner, &x_of, &y_pane, step);
}

fn draw_lines(
    painter: &Painter,
    preview: &Preview,
    area: Rect,
    x_of: &dyn Fn(usize) -> f32,
    y_of: &dyn Fn(f64) -> f32,
    step: f32,
) {
    for line in &preview.lines {
        if line.columns {
            let base = y_of(0.0f64.max(0.0)).clamp(area.top(), area.bottom());
            for (i, value) in line.values.iter().enumerate() {
                let Some(value) = value else { continue };
                let x = x_of(i);
                painter.rect_filled(
                    Rect::from_x_y_ranges(
                        (x - step * 0.34)..=(x + step * 0.34),
                        y_of(*value).min(base)..=y_of(*value).max(base).max(y_of(*value) + 0.8),
                    ),
                    0.0,
                    line.color,
                );
            }
            continue;
        }
        let mut run: Vec<Pos2> = Vec::new();
        let flush = |run: &mut Vec<Pos2>| {
            if run.len() > 1 {
                painter.add(Shape::line(
                    std::mem::take(run),
                    Stroke::new(1.4, line.color),
                ));
            }
            run.clear();
        };
        for (i, value) in line.values.iter().enumerate() {
            match value {
                Some(value) if value.is_finite() => run.push(pos2(x_of(i), y_of(*value))),
                _ => flush(&mut run),
            }
        }
        flush(&mut run);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shared_bars_are_valid_candles() {
        let bars = bars();
        assert_eq!(bars.len(), BARS);
        for bar in bars.iter() {
            assert!(bar.high >= bar.open.max(bar.close) && bar.low <= bar.open.min(bar.close));
        }
    }

    #[test]
    fn every_native_has_something_to_draw() {
        for kind in BuiltIn::ALL {
            let preview = native(kind);
            assert!(
                !preview.lines.is_empty() || !preview.marks.is_empty(),
                "{}",
                kind.menu_title()
            );
        }
        assert!(!native(BuiltIn::Rsi).overlay && native(BuiltIn::Ema).overlay);
    }

    #[test]
    fn every_bundled_script_previews_and_agrees_on_its_pane() {
        for indicator in &indicators::ALL {
            let preview = pine(indicator.source)
                .unwrap_or_else(|| panic!("{} has no preview", indicator.name));
            assert_eq!(preview.overlay, indicator.overlay, "{}", indicator.name);
            if indicator.id == "williams-fractals" {
                assert!(!preview.marks.is_empty(), "fractals are marks");
            } else {
                assert!(
                    preview
                        .lines
                        .iter()
                        .any(|line| line.values.iter().flatten().count() > 10),
                    "{} drew a line with almost no values",
                    indicator.name
                );
            }
        }
    }

    #[test]
    fn previews_arrive_from_the_worker_and_natives_are_immediate() {
        let ctx = egui::Context::default();
        let mut previews = Previews::default();
        let catalog = crate::model::indicator_catalog::all(&[]);
        let rsi = catalog
            .iter()
            .find(|entry| entry.key == EntryKey::Native(BuiltIn::Rsi))
            .unwrap();
        assert!(previews.get(&ctx, rsi).is_some());
        let macd = catalog
            .iter()
            .find(|entry| entry.key == EntryKey::Bundled("macd"))
            .unwrap();
        let mut waited = 0;
        while previews.get(&ctx, macd).is_none() && waited < 500 {
            std::thread::sleep(std::time::Duration::from_millis(20));
            waited += 1;
        }
        assert!(previews.get(&ctx, macd).is_some(), "the worker delivered");
        assert!(!previews.is_missing(macd));
    }
}

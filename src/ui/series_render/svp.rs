//! Paints the Session volume profile chart type: for each session a horizontal histogram of the volume
//! traded at each price, up and down volume side by side, with the point of control, value area and
//! (when asked) how both developed through the session.
//!
//! Built elsewhere (`model::market_profile::svp`) from lower-timeframe candles and handed in as a
//! finished [`SvpSet`]; this turns it into shapes. Like the TradingView chart type it replaces the
//! candles of the sessions it covers; a bar no profile covers (history older than the candles reach)
//! stays a plain candle, and the status line says what the data is. With rows thinner than two
//! points the rows are merged into pixel-high buckets of the **average** volume per row, so the shape
//! of the distribution is the same at any zoom; only the detail goes.

use super::tpo::Geo;
use super::{SeriesFrame, color};
use crate::model::chart_type::{SvpLine, SvpSettings, SvpVolume, VolumePlacement};
use crate::model::market_profile::footprint::SetStatus;
use crate::model::market_profile::svp::{SvpProfile, SvpSet};
use crate::ui::theme;
use eframe::egui::{Align2, Color32, FontId, Pos2, Rect, Shape, Stroke};

const UP: [u8; 4] = [38, 166, 154, 255];
const DOWN: [u8; 4] = [239, 83, 80, 255];
const POC: [u8; 4] = [240, 185, 11, 255];
const EDGE: [u8; 4] = [160, 160, 160, 255];

pub fn default_colors() -> [[u8; 4]; 2] {
    [UP, DOWN]
}

fn tint(base: [u8; 4], alpha: f32) -> Color32 {
    let [r, g, b, a] = base;
    Color32::from_rgba_unmultiplied(
        r,
        g,
        b,
        (f32::from(a) * alpha).round().clamp(0.0, 255.0) as u8,
    )
}

/// Where a profile's histogram sits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub x0: f32,
    pub x1: f32,
    /// The edge the bars grow from, the longest a bar can be, and which way they grow.
    pub base: f32,
    pub length: f32,
    pub leftwards: bool,
}

pub fn layout(geo: &Geo, profile: &SvpProfile, settings: &SvpSettings) -> Layout {
    let x0 = geo.x_at(profile.start_ms);
    let x1 = geo.x_at(profile.end_ms);
    let length = ((x1 - x0).max(0.0) * settings.width_percent as f32 / 100.0).max(0.0);
    match settings.placement {
        VolumePlacement::Left => Layout {
            x0,
            x1,
            base: x0,
            length,
            leftwards: false,
        },
        VolumePlacement::Right => Layout {
            x0,
            x1,
            base: x1,
            length,
            leftwards: true,
        },
    }
}

impl Layout {
    /// The x range `length` points from the base cover.
    fn span(&self, length: f32) -> (f32, f32) {
        if self.leftwards {
            (self.base - length, self.base)
        } else {
            (self.base, self.base + length)
        }
    }
}

/// Rows merged into buckets of at least `min_height` points: `(first row index, last, average up,
/// average down)`. Rows already that tall come through one by one.
pub fn buckets(
    profile: &SvpProfile,
    row_height: f32,
    min_height: f32,
) -> Vec<(usize, usize, f64, f64)> {
    let per = if row_height >= min_height || row_height <= 0.0 {
        1
    } else {
        (min_height / row_height).ceil() as usize
    };
    profile
        .rows
        .chunks(per)
        .enumerate()
        .map(|(n, chunk)| {
            let count = chunk.len() as f64;
            let up: f64 = chunk.iter().map(|r| r.up).sum::<f64>() / count;
            let down: f64 = chunk.iter().map(|r| r.down).sum::<f64>() / count;
            (n * per, n * per + chunk.len() - 1, up, down)
        })
        .collect()
}

pub fn paint(frame: &SeriesFrame) {
    let settings = frame.style.svp;
    let set = frame.profiles.and_then(|feed| feed.svp.as_deref());
    let geo = Geo::of(frame);
    let mut covered = vec![false; frame.bars.len()];
    if let Some(set) = set {
        for profile in &set.profiles {
            for (index, bar) in frame.bars.iter().enumerate() {
                if profile.start_ms <= bar.open_time && bar.open_time < profile.end_ms {
                    covered[index] = true;
                }
            }
        }
    }
    for (index, is_covered) in covered.iter().enumerate() {
        if !is_covered {
            super::footprint::plain_candle(frame, index);
        }
    }
    if let Some(set) = set {
        for profile in &set.profiles {
            let lay = layout(&geo, profile, &settings);
            if lay.x1 < frame.plot.left() - 1.0 || lay.x0 > frame.plot.right() + 1.0 {
                continue;
            }
            paint_profile(frame, &geo, &settings, profile, &lay);
        }
    }
    paint_status(frame, set);
}

fn paint_profile(
    frame: &SeriesFrame,
    geo: &Geo,
    settings: &SvpSettings,
    profile: &SvpProfile,
    lay: &Layout,
) {
    let painter = frame.painter;
    let plot = frame.plot;
    let grid = profile.grid;
    let first = &profile.rows[0];
    let row_height = (geo.y(grid.bottom(first.row)) - geo.y(grid.top(first.row))).abs();
    let up_color = settings.up_color.unwrap_or(UP);
    let down_color = settings.down_color.unwrap_or(DOWN);
    let va_up = settings.value_area_up_color.unwrap_or(up_color);
    let va_down = settings.value_area_down_color.unwrap_or(down_color);
    // The box behind the histogram.
    if let Some(box_color) = settings.box_color {
        let (from, to) = lay.span(lay.length);
        let rect = Rect::from_min_max(
            Pos2::new(from, geo.y(profile.high)),
            Pos2::new(to, geo.y(profile.low)),
        );
        painter.rect_filled(
            rect,
            0.0,
            tint(box_color, settings.box_opacity as f32 / 100.0),
        );
    }
    let peak = match settings.volume {
        SvpVolume::Delta => profile.peak_delta(),
        _ => profile.peak(),
    };
    if peak > 0.0 && lay.length >= 1.0 {
        let (va_low, va_high) = profile.value_area;
        let font = FontId::monospace((row_height * 0.8).clamp(7.0, 10.0));
        for (first_index, last_index, up, down) in buckets(profile, row_height, 2.0) {
            let top_row = profile.rows[last_index].row;
            let bottom_row = profile.rows[first_index].row;
            let (top, bottom) = (geo.y(grid.top(top_row)), geo.y(grid.bottom(bottom_row)));
            if bottom < plot.top() || top > plot.bottom() {
                continue;
            }
            let inside = first_index >= va_low && last_index <= va_high;
            let pad = if row_height >= 4.0 { 0.5 } else { 0.0 };
            let rect = |from: f32, to: f32| {
                Rect::from_min_max(Pos2::new(from, top + pad), Pos2::new(to, bottom - pad))
            };
            let length = |volume: f64| lay.length * (volume / peak) as f32;
            let alpha = if inside { 0.9 } else { 0.5 };
            let end = match settings.volume {
                SvpVolume::UpDown => {
                    let (up_len, down_len) = (length(up), length(down));
                    let (a, b) = lay.span(up_len);
                    painter.rect_filled(
                        rect(a, b),
                        0.0,
                        tint(if inside { va_up } else { up_color }, alpha),
                    );
                    let (c, d) = if lay.leftwards {
                        (lay.base - up_len - down_len, lay.base - up_len)
                    } else {
                        (lay.base + up_len, lay.base + up_len + down_len)
                    };
                    painter.rect_filled(
                        rect(c, d),
                        0.0,
                        tint(if inside { va_down } else { down_color }, alpha),
                    );
                    up_len + down_len
                }
                SvpVolume::Total => {
                    let total = length(up + down);
                    let (a, b) = lay.span(total);
                    painter.rect_filled(
                        rect(a, b),
                        0.0,
                        tint(if inside { va_up } else { up_color }, alpha),
                    );
                    total
                }
                SvpVolume::Delta => {
                    let delta = up - down;
                    let len = length(delta.abs());
                    let base = if delta >= 0.0 {
                        (inside, up_color, va_up)
                    } else {
                        (inside, down_color, va_down)
                    };
                    let (a, b) = lay.span(len);
                    painter.rect_filled(
                        rect(a, b),
                        0.0,
                        tint(if base.0 { base.2 } else { base.1 }, alpha),
                    );
                    len
                }
            };
            if settings.values && row_height >= 9.0 && first_index == last_index {
                let total = profile.rows[first_index].total();
                let (anchor, align) = if lay.leftwards {
                    (lay.base - end - 2.0, Align2::RIGHT_CENTER)
                } else {
                    (lay.base + end + 2.0, Align2::LEFT_CENTER)
                };
                painter.text(
                    Pos2::new(anchor, (top + bottom) / 2.0),
                    align,
                    super::footprint::format_volume(total),
                    font.clone(),
                    theme::text_secondary(),
                );
            }
        }
    }
    paint_lines(frame, geo, settings, profile, lay);
}

fn paint_lines(
    frame: &SeriesFrame,
    geo: &Geo,
    settings: &SvpSettings,
    profile: &SvpProfile,
    lay: &Layout,
) {
    let painter = frame.painter;
    let plot = frame.plot;
    let font = FontId::proportional(9.5);
    let line = |spec: &SvpLine, price: f64, fallback: [u8; 4], label: &str, dashed: bool| {
        if !spec.show {
            return;
        }
        let y = geo.y(price);
        if y < plot.top() || y > plot.bottom() {
            return;
        }
        let end = if spec.extend && !profile.forming {
            super::tpo::extended_to_bar(geo, profile.end_ms, price).max(lay.x1)
        } else {
            lay.x1
        };
        let stroke = Stroke::new(
            if dashed { 1.0 } else { 1.5 },
            color(spec.color.unwrap_or(fallback)),
        );
        if dashed {
            painter.extend(Shape::dashed_line(
                &[Pos2::new(lay.x0, y), Pos2::new(end, y)],
                stroke,
                4.0,
                3.0,
            ));
        } else {
            painter.hline(lay.x0..=end, y, stroke);
        }
        if lay.x1 - lay.x0 > 60.0 {
            painter.text(
                Pos2::new(lay.x1 - 2.0, y - 1.0),
                Align2::RIGHT_BOTTOM,
                label,
                font.clone(),
                color(spec.color.unwrap_or(fallback)).gamma_multiply(0.9),
            );
        }
    };
    line(&settings.poc, profile.poc_price(), POC, "POC", false);
    line(&settings.vah, profile.vah(), EDGE, "VAH", true);
    line(&settings.val, profile.val(), EDGE, "VAL", true);
    // The developing levels: a step line over the session, each point as of the end of its candle.
    let steps = |value: fn(&crate::model::market_profile::svp::Developing) -> f64| -> Vec<Pos2> {
        let mut points = Vec::with_capacity(profile.developing.len() * 2);
        let mut previous_y: Option<f32> = None;
        for point in &profile.developing {
            let (x, y) = (geo.x_at(point.time_ms), geo.y(value(point)));
            if let Some(py) = previous_y {
                points.push(Pos2::new(x, py));
            }
            points.push(Pos2::new(x, y));
            previous_y = Some(y);
        }
        points
    };
    if settings.developing_poc {
        let stroke = Stroke::new(1.25, tint(settings.poc.color.unwrap_or(POC), 0.9));
        painter.add(Shape::line(steps(|p| p.poc), stroke));
    }
    if settings.developing_value_area {
        let stroke = Stroke::new(1.0, tint(EDGE, 0.9));
        painter.add(Shape::line(steps(|p| p.vah), stroke));
        painter.add(Shape::line(steps(|p| p.val), stroke));
    }
}

/// Where the data came from, or why there is none: bottom left, small.
fn paint_status(frame: &SeriesFrame, set: Option<&SvpSet>) {
    let Some(message) = status_text(frame, set) else {
        return;
    };
    frame.painter.text(
        Pos2::new(frame.plot.left() + 8.0, frame.plot.bottom() - 8.0),
        Align2::LEFT_BOTTOM,
        message,
        FontId::proportional(10.5),
        theme::text_secondary(),
    );
}

pub fn status_text(frame: &SeriesFrame, set: Option<&SvpSet>) -> Option<String> {
    if let Some(SetStatus::Unsupported(reason)) = set.map(|set| set.status) {
        return Some(format!(
            "Session volume profile unavailable: {}",
            reason.explanation()
        ));
    }
    let Some(set) = set else {
        return Some(match frame.profiles {
            Some(feed) if feed.error.is_some() => {
                format!(
                    "Session volume profile: {}",
                    feed.error.as_deref().unwrap_or("")
                )
            }
            _ => "Loading order-flow data…".to_owned(),
        });
    };
    if set.refused > 0 {
        let reason = set.first_refusal.map_or("", |reason| reason.explanation());
        return Some(format!("{} sessions not shown. {reason}", set.refused));
    }
    if set.profiles.is_empty() {
        return Some("No intraday candles with volume for these bars yet".to_owned());
    }
    let estimated = set
        .profiles
        .iter()
        .filter(|p| !p.quality.is_exact())
        .count();
    let split = set
        .profiles
        .iter()
        .filter(|p| p.quality == crate::model::market_profile::quality::DataQuality::ExchangeSplit)
        .count();
    Some(format!(
        "{} session{} from {}-minute candles{}",
        set.profiles.len(),
        if set.profiles.len() == 1 { "" } else { "s" },
        set.candle_minutes,
        if split == set.profiles.len() {
            ", exchange buy/sell split".to_owned()
        } else if estimated > 0 {
            ", up/down estimated from the candles".to_owned()
        } else {
            String::new()
        }
    ))
}

/// Draws `bars` with `set` as a session volume profile into a picture named `name` in the gallery
/// folder (the live tests' way of showing what real data looks like).
#[cfg(test)]
pub(crate) fn gallery_picture(
    name: &str,
    bars: &[crate::model::Kline],
    set: &std::sync::Arc<SvpSet>,
) {
    use crate::model::PriceFormat;
    use crate::model::chart_look::Palette;
    use crate::model::chart_type::{ChartStyle, ChartType};
    use crate::model::market_profile::feed::ProfileFeed;
    use crate::ui::chart_card::PriceScale;
    use crate::ui::render_tests::{Screen, gallery_dir};
    if gallery_dir().is_none() || bars.is_empty() {
        return;
    }
    let feed = ProfileFeed {
        svp: Some(set.clone()),
        ..ProfileFeed::default()
    };
    let (width, height) = (1500.0_f32, 900.0_f32);
    let slot = width / bars.len() as f32;
    let (low, high) = bars.iter().fold((f64::MAX, f64::MIN), |(l, h), bar| {
        (l.min(bar.low), h.max(bar.high))
    });
    let pad = (high - low) * 0.05;
    let mut style = ChartStyle {
        chart_type: ChartType::SessionVolumeProfile,
        ..ChartStyle::default()
    };
    style.svp.developing_poc = true;
    style.svp.developing_value_area = true;
    style.svp.box_color = Some([128, 128, 128, 255]);
    let mut screen = Screen::new(eframe::egui::vec2(width, height));
    let mut last = None;
    for _ in 0..2 {
        last = Some(screen.run(Vec::new(), |ui| {
            let plot = Rect::from_min_size(Pos2::ZERO, eframe::egui::vec2(width, height));
            ui.painter().rect_filled(plot, 0.0, theme::chrome());
            let scale = PriceScale::linear(low - pad, high + pad, 0.0, height);
            let x_for = |index: usize| slot * (index as f32 + 0.5);
            paint(&SeriesFrame {
                painter: ui.painter(),
                plot,
                bars,
                synthetic: None,
                box_size: None,
                previous_close: None,
                slot_width: slot,
                x_for: &x_for,
                scale: &scale,
                palette: Palette::new([38, 166, 154], [239, 83, 80]),
                style: &style,
                chart_type: ChartType::SessionVolumeProfile,
                recolor: None,
                price_format: PriceFormat::AUTO,
                profiles: Some(&feed),
            });
        }));
    }
    screen.save(name, last.expect("a frame"));
}

// MARK: Pointer

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit<'a> {
    pub profile: &'a SvpProfile,
    pub row: i64,
}

pub fn hit<'a>(
    geo: &Geo,
    set: &'a SvpSet,
    settings: &SvpSettings,
    pointer: Pos2,
) -> Option<Hit<'a>> {
    if !geo.plot.contains(pointer) || geo.bars.is_empty() {
        return None;
    }
    for profile in &set.profiles {
        let lay = layout(geo, profile, settings);
        if pointer.x < lay.x0 || pointer.x > lay.x1 {
            continue;
        }
        let row = profile.grid.row_of(geo.scale.price(pointer.y))?;
        profile.row_at(row)?;
        return Some(Hit { profile, row });
    }
    None
}

fn clock(time_ms: i64) -> String {
    let minutes = time_ms.div_euclid(60_000).rem_euclid(1_440);
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

/// The tooltip for the row under `pointer`: its price range and volume (up, down, total, delta), where
/// it sits against the value area, then the session's own figures and where its numbers came from.
pub fn hover(
    frame: &SeriesFrame,
    pointer: Pos2,
) -> Option<crate::ui::pine_chart_render::HoverTarget> {
    let set = frame.profiles?.svp.as_deref()?;
    let geo = Geo::of(frame);
    let hit = hit(&geo, set, &frame.style.svp, pointer)?;
    let profile = hit.profile;
    let grid = profile.grid;
    let row = profile.row_at(hit.row)?;
    let format = |price: f64| frame.price_format.format(price);
    let volume = super::footprint::format_volume;
    let (top, bottom) = (geo.y(grid.top(hit.row)), geo.y(grid.bottom(hit.row)));
    let mut lines = vec![
        format!(
            "{} – {}",
            format(grid.bottom(hit.row)),
            format(grid.top(hit.row))
        ),
        format!("Volume {}", volume(row.total())),
        format!("Up {}   Down {}", volume(row.up), volume(row.down)),
        format!("Delta {}", super::footprint::format_signed(row.delta())),
    ];
    let index = (hit.row - profile.first_row()) as usize;
    if index == profile.poc {
        lines.push("Point of control".to_owned());
    } else if (profile.value_area.0..=profile.value_area.1).contains(&index) {
        lines.push("In the value area".to_owned());
    }
    lines.push(format!(
        "Session {} – {} UTC{}",
        clock(profile.start_ms),
        clock(profile.end_ms),
        if profile.forming { " (forming)" } else { "" }
    ));
    lines.push(format!(
        "POC {}, VAH {}, VAL {}",
        format(profile.poc_price()),
        format(profile.vah()),
        format(profile.val())
    ));
    lines.push(format!(
        "Volume {} (up {}, down {})",
        volume(profile.total()),
        volume(profile.up()),
        volume(profile.down())
    ));
    lines.push(profile.quality.label().to_owned());
    Some(crate::ui::pine_chart_render::HoverTarget {
        rect: Rect::from_min_max(
            Pos2::new(pointer.x - 3.0, top.min(bottom)),
            Pos2::new(pointer.x + 3.0, top.max(bottom)),
        ),
        text: lines.join("\n"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thin_rows_are_merged_by_average_and_tall_ones_are_left_alone() {
        use crate::model::chart_type::RowsLayout;
        use crate::model::market_profile::svp::{Row, SvpProfile};
        let _ = RowsLayout::NumberOfRows;
        let grid = crate::model::market_profile::price_grid::PriceGrid::new(1.0, 1).unwrap();
        let profile = SvpProfile {
            start_ms: 0,
            end_ms: 1,
            grid,
            rows: (0..10)
                .map(|i| Row {
                    row: i,
                    up: 10.0,
                    down: if i % 2 == 0 { 2.0 } else { 4.0 },
                })
                .collect(),
            poc: 0,
            value_area: (0, 9),
            developing: Vec::new(),
            quality: crate::model::market_profile::quality::DataQuality::Intrabar,
            high: 10.0,
            low: 0.0,
            forming: false,
        };
        // Rows 4 points tall are drawn one by one.
        assert_eq!(buckets(&profile, 4.0, 2.0).len(), 10);
        // Rows half a point tall are merged four at a time: 10 rows -> 3 buckets (4, 4, 2).
        let merged = buckets(&profile, 0.5, 2.0);
        assert_eq!(merged.len(), 3);
        assert_eq!((merged[0].0, merged[0].1), (0, 3));
        assert!(
            (merged[0].2 - 10.0).abs() < 1e-9,
            "the average up volume, not the sum"
        );
        assert!((merged[0].3 - 3.0).abs() < 1e-9);
        assert_eq!((merged[2].0, merged[2].1), (8, 9));
    }

    // MARK: Drawing

    use crate::model::chart_look::Palette;
    use crate::model::chart_type::{ChartStyle, ChartType};
    use crate::model::market_profile::feed::ProfileFeed;
    use crate::model::{ChartAppearance, Kline, PriceFormat};
    use crate::ui::chart_card::PriceScale;
    use crate::ui::chart_style_preview::{sample_candles, sample_svp};
    use eframe::egui;

    fn bars() -> Vec<Kline> {
        sample_candles()[..24].to_vec()
    }

    fn style() -> ChartStyle {
        ChartStyle {
            chart_type: ChartType::SessionVolumeProfile,
            ..ChartStyle::default()
        }
    }

    fn feed(style: &ChartStyle) -> ProfileFeed {
        sample_svp(&ChartAppearance {
            style: *style,
            ..ChartAppearance::default()
        })
    }

    fn setup(width: f32, height: f32) -> (Rect, PriceScale, f32) {
        (
            Rect::from_min_max(Pos2::ZERO, Pos2::new(width, height)),
            PriceScale::linear(95.0, 125.0, 0.0, height),
            width / 24.0,
        )
    }

    fn paint_with(
        feed: Option<&ProfileFeed>,
        style: &ChartStyle,
        width: f32,
        height: f32,
    ) -> (usize, Vec<String>) {
        let bars = bars();
        let (plot, scale, slot) = setup(width, height);
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let x_for = |index: usize| slot * (index as f32 + 0.5);
            paint(&SeriesFrame {
                painter: ui.painter(),
                plot,
                bars: &bars,
                synthetic: None,
                box_size: None,
                previous_close: None,
                slot_width: slot,
                x_for: &x_for,
                scale: &scale,
                palette: Palette::new([0, 200, 0], [200, 0, 0]),
                style,
                chart_type: ChartType::SessionVolumeProfile,
                recolor: None,
                price_format: PriceFormat::AUTO,
                profiles: feed,
            });
        });
        output.textures_delta.clear();
        let shapes = std::mem::take(&mut output.shapes);
        let words = shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::epaint::Shape::Text(text) => Some(text.galley.text().to_owned()),
                _ => None,
            })
            .collect();
        (shapes.len(), words)
    }

    #[test]
    fn a_profile_draws_its_histogram_and_lines_and_says_what_it_is_built_from() {
        let style = style();
        let feed = feed(&style);
        let (shapes, words) = paint_with(Some(&feed), &style, 1200.0, 800.0);
        assert!(shapes > 30, "rows, lines: {shapes}");
        assert!(words.iter().any(|w| w == "POC"), "{words:?}");
        assert!(
            words
                .iter()
                .any(|w| w.contains("1 session") && w.contains("5-minute candles")),
            "{words:?}"
        );
    }

    #[test]
    fn no_feed_draws_candles_and_says_it_is_loading_and_a_failed_load_says_why() {
        let (shapes, words) = paint_with(None, &style(), 600.0, 300.0);
        assert!(shapes > 0);
        assert!(words.iter().any(|w| w.contains("Loading")), "{words:?}");
        let failed = ProfileFeed {
            error: Some("Network error: offline".into()),
            ..ProfileFeed::default()
        };
        let (_, words) = paint_with(Some(&failed), &style(), 600.0, 300.0);
        assert!(words.iter().any(|w| w.contains("offline")), "{words:?}");
    }

    #[test]
    fn every_volume_mode_placement_and_option_paints_without_panicking() {
        let base = style();
        for volume in [SvpVolume::UpDown, SvpVolume::Total, SvpVolume::Delta] {
            for placement in [VolumePlacement::Left, VolumePlacement::Right] {
                for extra in [false, true] {
                    let mut style = base;
                    style.svp.volume = volume;
                    style.svp.placement = placement;
                    style.svp.values = extra;
                    style.svp.developing_poc = extra;
                    style.svp.developing_value_area = extra;
                    style.svp.box_color = extra.then_some([128, 128, 128, 255]);
                    style.svp.poc.extend = extra;
                    let feed = feed(&style);
                    for (w, h) in [(1200.0, 900.0), (300.0, 100.0)] {
                        let (shapes, _) = paint_with(Some(&feed), &style, w, h);
                        assert!(shapes > 0, "{volume:?} {placement:?} {extra}");
                    }
                }
            }
        }
    }

    #[test]
    fn the_histogram_grows_from_the_placed_edge_and_stays_inside_its_share_of_the_session() {
        let bars = bars();
        let (plot, scale, slot) = setup(2400.0, 800.0);
        let x_for = |index: usize| slot * (index as f32 + 0.5);
        let geo = Geo {
            plot,
            bars: &bars,
            x_for: &x_for,
            slot_width: slot,
            scale: &scale,
        };
        let mut style = style();
        let feed = feed(&style);
        let profile = &feed.svp.as_ref().unwrap().profiles[0];
        let left = layout(&geo, profile, &style.svp);
        assert!(!left.leftwards && (left.base - left.x0).abs() < 1e-3);
        assert!(
            (left.length - 0.3 * (left.x1 - left.x0)).abs() < 1e-2,
            "30 % of the session"
        );
        style.svp.placement = VolumePlacement::Right;
        style.svp.width_percent = 50;
        let right = layout(&geo, profile, &style.svp);
        assert!(right.leftwards && (right.base - right.x1).abs() < 1e-3);
        assert!((right.length - 0.5 * (right.x1 - right.x0)).abs() < 1e-2);
    }

    #[test]
    fn the_hover_describes_the_row_and_the_session_and_nothing_outside_a_profile() {
        let style = style();
        let feed = feed(&style);
        let bars = bars();
        let (plot, scale, slot) = setup(2400.0, 800.0);
        let x_for = |index: usize| slot * (index as f32 + 0.5);
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let frame = SeriesFrame {
                painter: ui.painter(),
                plot,
                bars: &bars,
                synthetic: None,
                box_size: None,
                previous_close: None,
                slot_width: slot,
                x_for: &x_for,
                scale: &scale,
                palette: Palette::new([0, 200, 0], [200, 0, 0]),
                style: &style,
                chart_type: ChartType::SessionVolumeProfile,
                recolor: None,
                price_format: PriceFormat::AUTO,
                profiles: Some(&feed),
            };
            let profile = &feed.svp.as_ref().unwrap().profiles[0];
            let pointer = Pos2::new(200.0, scale.y(profile.poc_price()));
            let target = hover(&frame, pointer).expect("over the point of control row");
            for expected in [
                "Volume",
                "Up ",
                "Delta",
                "Point of control",
                "Session 00:00",
                "POC",
            ] {
                assert!(
                    target.text.contains(expected),
                    "{expected}: {}",
                    target.text
                );
            }
            assert!(hover(&frame, Pos2::new(-5.0, 10.0)).is_none());
            let bare = SeriesFrame {
                profiles: None,
                ..frame
            };
            assert!(hover(&bare, pointer).is_none());
        });
        output.textures_delta.clear();
    }

    /// Pictures of the profile at the sizes and settings worth a look.
    /// `DEGENVIEW_GALLERY_DIR=/tmp/g cargo test --lib gallery_of_the_svp -- --ignored`.
    #[test]
    #[ignore = "writes images for a person to look at"]
    fn gallery_of_the_svp() {
        use crate::ui::render_tests::{Screen, gallery_dir};
        if gallery_dir().is_none() {
            return;
        }
        let bars = bars();
        type Tweak = fn(&mut ChartStyle);
        let cases: [(&str, f32, f32, Tweak); 5] = [
            ("svp-wide", 900.0, 640.0, |_| {}),
            ("svp-narrow", 300.0, 160.0, |_| {}),
            ("svp-right-developing", 900.0, 640.0, |s| {
                s.svp.placement = VolumePlacement::Right;
                s.svp.developing_poc = true;
                s.svp.developing_value_area = true;
                s.svp.box_color = Some([128, 128, 128, 255]);
            }),
            ("svp-delta-values", 900.0, 640.0, |s| {
                s.svp.volume = SvpVolume::Delta;
                s.svp.values = true;
                s.svp.row_size = 40;
            }),
            ("svp-total-wide", 900.0, 640.0, |s| {
                s.svp.volume = SvpVolume::Total;
                s.svp.width_percent = 70;
            }),
        ];
        for (name, width, height, tweak) in cases {
            let mut style = style();
            tweak(&mut style);
            let feed = feed(&style);
            let mut screen = Screen::new(egui::vec2(width, height));
            let mut last = None;
            for _ in 0..2 {
                last = Some(screen.run(Vec::new(), |ui| {
                    let (plot, scale, slot) = setup(width, height);
                    ui.painter().rect_filled(plot, 0.0, theme::chrome());
                    let x_for = |index: usize| slot * (index as f32 + 0.5);
                    paint(&SeriesFrame {
                        painter: ui.painter(),
                        plot,
                        bars: &bars,
                        synthetic: None,
                        box_size: None,
                        previous_close: None,
                        slot_width: slot,
                        x_for: &x_for,
                        scale: &scale,
                        palette: Palette::new([38, 166, 154], [239, 83, 80]),
                        style: &style,
                        chart_type: ChartType::SessionVolumeProfile,
                        recolor: None,
                        price_format: PriceFormat::AUTO,
                        profiles: Some(&feed),
                    });
                }));
            }
            screen.save(name, last.expect("a frame"));
        }
    }
}

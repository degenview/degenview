use super::composition::{
    BRANDING_HEIGHT, HEADER_HEIGHT, HEADER_PAD_SHARE, MIN_PADDING, PADDING_SHARE, dimensions,
    layout, plot_in, summary,
};
use super::framing::Base;
use super::studio::{Area, AspectRatio, Background, StudioConfig, Style};
use super::{MAX_PIXELS, MAX_SIDE_PX, SnapshotDimensions, SnapshotScale};
use crate::model::chart::viewport::{HorizontalViewport, PriceRange};
use eframe::egui::{Rect, Vec2, pos2, vec2};

fn config(style: Style, size: AspectRatio, background: Background) -> StudioConfig {
    let mut config = StudioConfig::new(Area::FullChart);
    config.style = style;
    config.size = size;
    config.background = background;
    config
}

fn styled(size: AspectRatio, background: Background) -> StudioConfig {
    config(Style::Styled, size, background)
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() <= 1e-3 * a.abs().max(b.abs()).max(1.0)
}

const CAPTURE: Vec2 = vec2(1200.0, 800.0);

// MARK: Proportions

#[test]
fn the_chart_style_is_the_capture_whatever_size_says() {
    for size in AspectRatio::ALL {
        let got = layout(CAPTURE, &config(Style::Chart, size, Background::None));
        assert_eq!(got.size, CAPTURE, "{size:?}");
        assert_eq!(got.chart, Rect::from_min_size(pos2(0.0, 0.0), CAPTURE));
        assert_eq!(got.padding, 0.0);
        assert_eq!((got.header, got.branding), (None, None));
    }
}

#[test]
fn styled_original_keeps_the_capture_as_it_is() {
    let got = layout(CAPTURE, &styled(AspectRatio::Original, Background::None));
    assert_eq!(got.size, CAPTURE);
    assert_eq!(got.chart.size(), CAPTURE);
}

#[test]
fn every_preset_gives_exactly_its_proportions_and_the_captures_area() {
    for size in AspectRatio::ALL {
        let Some(ratio) = size.ratio() else { continue };
        let got = layout(CAPTURE, &styled(size, Background::None));
        assert!(
            near(got.size.x / got.size.y, ratio),
            "{size:?}: {:?}",
            got.size
        );
        assert_eq!(got.chart.size(), got.size, "no margin, no stretch");
        let (area, wanted) = (got.size.x * got.size.y, CAPTURE.x * CAPTURE.y);
        assert!(
            (area / wanted - 1.0).abs() < 0.03,
            "{size:?} keeps the area: {area} against {wanted}"
        );
    }
}

#[test]
fn a_preset_grows_one_side_and_shrinks_the_other() {
    let wide = layout(CAPTURE, &styled(AspectRatio::Wide16x9, Background::None)).size;
    assert!(wide.x > CAPTURE.x && wide.y < CAPTURE.y, "{wide:?}");
    let square = layout(CAPTURE, &styled(AspectRatio::Square, Background::None)).size;
    assert!(square.x < CAPTURE.x && square.y > CAPTURE.y, "{square:?}");
    let tall = layout(
        CAPTURE,
        &styled(AspectRatio::Portrait9x16, Background::None),
    )
    .size;
    assert!(tall.x < square.x && tall.y > square.y, "{tall:?}");
}

#[test]
fn a_preset_is_whole_points_so_its_proportions_hold_in_pixels() {
    for size in AspectRatio::ALL.into_iter().filter(|s| s.ratio().is_some()) {
        for background in Background::ALL {
            let got = layout(CAPTURE, &styled(size, background)).size;
            assert_eq!(got.x, got.x.round(), "{size:?} {background:?}");
            assert_eq!(got.y, got.y.round(), "{size:?} {background:?}");
            let (a, b) = size.terms().expect("a preset");
            assert_eq!(
                got.x as u32 * b,
                got.y as u32 * a,
                "{size:?} {background:?}"
            );
        }
    }
}

// MARK: Padding

#[test]
fn a_backdrop_adds_six_percent_of_the_shorter_side_on_every_side() {
    for background in Background::ALL.into_iter().filter(|b| b.has_frame()) {
        let got = layout(
            CAPTURE,
            &config(Style::Chart, AspectRatio::Original, background),
        );
        let short = got.size.x.min(got.size.y);
        assert!(
            near(got.padding, PADDING_SHARE as f32 * short),
            "{background:?}: {} of {short}",
            got.padding
        );
        assert!(
            near(got.chart.width(), CAPTURE.x) && near(got.chart.height(), CAPTURE.y),
            "the chart is not resized"
        );
        let expected = CAPTURE + vec2(2.0, 2.0) * got.padding;
        assert!(near(got.size.x, expected.x) && near(got.size.y, expected.y));
        assert_eq!(got.chart.min, pos2(got.padding, got.padding));
    }
}

#[test]
fn no_backdrop_no_padding() {
    for size in AspectRatio::ALL {
        assert_eq!(
            layout(CAPTURE, &styled(size, Background::None)).padding,
            0.0
        );
    }
}

#[test]
fn the_padding_is_never_less_than_the_minimum() {
    let small = vec2(200.0, 100.0);
    let got = layout(
        small,
        &config(Style::Chart, AspectRatio::Original, Background::Dots),
    );
    assert_eq!(got.padding, MIN_PADDING as f32);
    assert_eq!(got.size, small + vec2(48.0, 48.0));

    let preset = layout(small, &styled(AspectRatio::Square, Background::Dots));
    assert_eq!(preset.padding, MIN_PADDING as f32);
}

#[test]
fn a_preset_on_a_backdrop_hits_the_proportions_with_the_margin_in() {
    for size in AspectRatio::ALL.into_iter().filter(|s| s.ratio().is_some()) {
        for capture in [
            vec2(1200.0, 800.0),
            vec2(300.0, 200.0),
            vec2(5000.0, 2000.0),
        ] {
            let got = layout(capture, &styled(size, Background::Spectrum));
            let ratio = size.ratio().expect("a preset");
            assert!(near(got.size.x / got.size.y, ratio), "{size:?} {capture:?}");
            let margin =
                (PADDING_SHARE as f32 * got.size.x.min(got.size.y)).max(MIN_PADDING as f32);
            assert!(near(got.padding, margin), "{size:?} {capture:?}");
            assert!(near(got.chart.width(), got.size.x - 2.0 * got.padding));
            assert!(near(got.chart.height(), got.size.y - 2.0 * got.padding));
            let (chart, wanted) = (got.chart.area(), capture.x * capture.y);
            assert!(
                (chart / wanted - 1.0).abs() < 0.06,
                "{size:?} {capture:?}: the chart has {chart} against {wanted}"
            );
        }
    }
}

#[test]
fn a_tiny_capture_on_a_backdrop_still_leaves_the_chart_room() {
    for size in AspectRatio::ALL {
        let got = layout(vec2(24.0, 24.0), &styled(size, Background::Radiant));
        assert!(
            got.chart.width() >= 1.0 && got.chart.height() >= 1.0,
            "{size:?}"
        );
        assert!(got.size.is_finite());
    }
}

#[test]
fn a_capture_that_is_not_a_size_does_not_break_the_layout() {
    for capture in [
        vec2(0.0, 0.0),
        vec2(-5.0, 300.0),
        vec2(f32::NAN, f32::INFINITY),
    ] {
        for size in AspectRatio::ALL {
            let got = layout(capture, &styled(size, Background::Dots));
            assert!(got.size.is_finite() && got.chart.is_finite(), "{capture:?}");
            assert!(got.chart.width() >= 1.0 && got.chart.height() >= 1.0);
        }
    }
}

// MARK: Header and branding

#[test]
fn styled_reserves_a_title_strip_across_the_top_of_the_chart() {
    let got = layout(
        vec2(800.0, 600.0),
        &styled(AspectRatio::Landscape3x2, Background::Dots),
    );
    let header = got.header.expect("a title block");
    assert_eq!(header.min, got.chart.min);
    assert_eq!(header.width(), got.chart.width());
    assert!(header.height() >= HEADER_HEIGHT.0 && header.height() <= HEADER_HEIGHT.1);
    assert!(near(header.height(), 0.15 * got.chart.height()));
    assert_eq!(
        got.body(),
        Rect::from_min_max(pos2(got.chart.min.x, header.max.y), got.chart.max)
    );
}

#[test]
fn the_title_strip_is_clamped() {
    let small = layout(
        vec2(300.0, 200.0),
        &styled(AspectRatio::Original, Background::None),
    );
    assert_eq!(small.header.expect("header").height(), HEADER_HEIGHT.0);
    let large = layout(
        vec2(4000.0, 3000.0),
        &styled(AspectRatio::Original, Background::None),
    );
    assert_eq!(large.header.expect("header").height(), HEADER_HEIGHT.1);
    let tiny = layout(
        vec2(100.0, 60.0),
        &styled(AspectRatio::Original, Background::None),
    );
    assert_eq!(
        tiny.header.expect("header").height(),
        30.0,
        "half the chart at most"
    );
}

#[test]
fn the_chart_style_has_no_header_and_the_body_is_the_chart() {
    let got = layout(
        CAPTURE,
        &config(Style::Chart, AspectRatio::Original, Background::Dots),
    );
    assert_eq!(got.header, None);
    assert_eq!(got.body(), got.chart);
}

#[test]
fn the_mark_sits_at_the_right_end_of_the_title_strip_centred_on_it() {
    let got = layout(
        CAPTURE,
        &styled(AspectRatio::Landscape3x2, Background::Radiant),
    );
    let mark = got.branding.expect("the mark");
    let header = got.header.expect("header");
    assert!(header.contains_rect(mark), "{mark:?} in {header:?}");
    assert!(mark.height() >= BRANDING_HEIGHT.0 && mark.height() <= BRANDING_HEIGHT.1);
    assert!(near(mark.center().y, header.center().y));
    // Right-aligned with the margin the title has on the left.
    let pad = header.height() * HEADER_PAD_SHARE;
    assert!(near(header.max.x - mark.max.x, pad));
    assert!(!mark.intersects(got.body()), "never over market data");
}

#[test]
fn the_mark_is_left_out_when_switched_off_or_not_styled() {
    let mut off = styled(AspectRatio::Landscape3x2, Background::None);
    off.include.branding = false;
    assert_eq!(layout(CAPTURE, &off).branding, None);
    assert_eq!(
        layout(CAPTURE, &off).header.map(|_| ()),
        Some(()),
        "header stays"
    );

    let mut chart = config(Style::Chart, AspectRatio::Original, Background::None);
    chart.include.branding = true;
    assert_eq!(layout(CAPTURE, &chart).branding, None);
}

#[test]
fn the_mark_has_no_place_in_a_strip_too_narrow_to_clear_the_logo() {
    let got = layout(
        vec2(90.0, 300.0),
        &styled(AspectRatio::Original, Background::None),
    );
    assert_eq!(got.branding, None);
    assert!(got.header.is_some(), "the title stays");
    for size in AspectRatio::ALL {
        let got = layout(CAPTURE, &styled(size, Background::Dots));
        assert!(got.branding.is_some(), "{size:?} has room for both");
    }
}

// MARK: Pixels

#[test]
fn the_image_is_the_composition_at_the_scale() {
    let composition = layout(
        CAPTURE,
        &config(Style::Chart, AspectRatio::Original, Background::None),
    );
    let one = dimensions(&composition, SnapshotScale::X1).expect("fits");
    assert_eq!(
        (one.width, one.height, one.pixels_per_point),
        (1200, 800, 1.0)
    );
    let two = dimensions(&composition, SnapshotScale::X2).expect("fits");
    assert_eq!(
        (two.width, two.height, two.pixels_per_point),
        (2400, 1600, 2.0)
    );
}

#[test]
fn a_composition_that_is_too_large_is_drawn_at_a_smaller_scale() {
    let composition = layout(
        vec2(6000.0, 3000.0),
        &config(Style::Chart, AspectRatio::Original, Background::None),
    );
    let got = dimensions(&composition, SnapshotScale::X2).expect("fits smaller");
    assert!(got.pixels_per_point < 2.0 && got.pixels_per_point >= 1.3);
    assert!(got.width <= MAX_SIDE_PX && got.height <= MAX_SIDE_PX);
    assert!(u64::from(got.width) * u64::from(got.height) <= MAX_PIXELS);
    assert!(
        got.width >= 8190,
        "the longest side uses the limit: {got:?}"
    );
}

#[test]
fn a_composition_that_even_a_quarter_scale_cannot_hold_is_refused() {
    let composition = layout(
        vec2(40_000.0, 10_000.0),
        &config(Style::Chart, AspectRatio::Original, Background::None),
    );
    assert_eq!(dimensions(&composition, SnapshotScale::X1), None);
    assert_eq!(dimensions(&composition, SnapshotScale::X2), None);
}

#[test]
fn the_backdrop_counts_toward_the_limit() {
    // The chart alone fits at 1×; the margin around it (6% of the shorter side) does not.
    let framed = layout(
        vec2(8_150.0, 2_000.0),
        &config(Style::Chart, AspectRatio::Original, Background::Dots),
    );
    let got = dimensions(&framed, SnapshotScale::X1).expect("fits");
    assert!(
        got.width <= MAX_SIDE_PX && got.pixels_per_point < 1.0,
        "{got:?}"
    );
}

// MARK: Summary

#[test]
fn the_summary_reads_like_the_footer() {
    let dims = SnapshotDimensions {
        pixels_per_point: 2.0,
        width: 3884,
        height: 2557,
    };
    assert_eq!(
        summary(Area::Portion, Style::Styled, &dims),
        "Selected portion · Styled · 3,884 × 2,557 · PNG"
    );
    assert_eq!(
        summary(Area::FullChart, Style::Chart, &dims),
        "Full chart · Chart · 3,884 × 2,557 · PNG"
    );
    assert_eq!(
        summary(Area::EntireLayout, Style::Styled, &dims),
        "Entire layout · Styled · 3,884 × 2,557 · PNG"
    );
}

#[test]
fn the_summary_groups_thousands_only_where_there_are_some() {
    let text = |width, height| {
        summary(
            Area::FullChart,
            Style::Chart,
            &SnapshotDimensions {
                pixels_per_point: 1.0,
                width,
                height,
            },
        )
    };
    assert_eq!(text(1, 12), "Full chart · Chart · 1 × 12 · PNG");
    assert_eq!(text(999, 1000), "Full chart · Chart · 999 × 1,000 · PNG");
    assert_eq!(text(8192, 100), "Full chart · Chart · 8,192 × 100 · PNG");
    assert_eq!(
        text(1_234_567, 123),
        "Full chart · Chart · 1,234,567 × 123 · PNG"
    );
}

#[test]
fn the_summary_of_a_real_layout_matches_its_dimensions() {
    let composition = layout(
        CAPTURE,
        &styled(AspectRatio::Landscape3x2, Background::None),
    );
    let dims = dimensions(&composition, SnapshotScale::X2).expect("fits");
    let text = summary(Area::Portion, Style::Styled, &dims);
    assert!(text.starts_with("Selected portion · Styled · "), "{text}");
    assert!(
        text.contains(&format!("{} × {}", group(dims.width), group(dims.height))),
        "{text}"
    );
}

fn group(n: u32) -> String {
    if n < 1000 {
        n.to_string()
    } else {
        format!("{},{:03}", n / 1000, n % 1000)
    }
}

// MARK: The plot inside the composition

fn base_of(plot: Vec2) -> Base {
    Base::from_view(
        &HorizontalViewport {
            visible: 100,
            right_offset: 0.0,
        },
        PriceRange::new(100.0, 200.0),
        false,
        500,
        10,
        plot,
    )
}

#[test]
fn the_plot_is_the_same_share_of_the_body_as_it_was_of_the_capture() {
    // A 1200 x 800 capture whose plot was 1100 x 700 (axes and a time scale took the rest).
    let base = base_of(vec2(1100.0, 700.0));
    let composition = layout(
        CAPTURE,
        &config(Style::Chart, AspectRatio::Original, Background::None),
    );
    let plot = plot_in(&composition, &base, CAPTURE);
    assert!(near(plot.x, 1100.0) && near(plot.y, 700.0), "{plot:?}");

    // The same capture as a Styled 16:9 image: the body is another shape, the shares stay.
    let wide = layout(
        CAPTURE,
        &styled(AspectRatio::Wide16x9, Background::Spectrum),
    );
    let body = wide.body().size();
    let plot = plot_in(&wide, &base, CAPTURE);
    assert!(near(plot.x, body.x * 1100.0 / 1200.0), "{plot:?}");
    assert!(near(plot.y, body.y * 700.0 / 800.0), "{plot:?}");
    assert!(plot.x < body.x && plot.y < body.y);
}

#[test]
fn a_capture_with_nothing_to_compare_gives_the_whole_body() {
    let composition = layout(CAPTURE, &styled(AspectRatio::Square, Background::None));
    let body = composition.body().size();
    for base in [base_of(Vec2::ZERO), base_of(vec2(f32::NAN, 10.0))] {
        assert_eq!(plot_in(&composition, &base, Vec2::ZERO), body);
        assert_eq!(
            plot_in(&composition, &base, vec2(f32::INFINITY, 1.0)).x,
            body.x
        );
    }
    // A plot reported larger than its capture is held to the capture.
    let over = plot_in(&composition, &base_of(vec2(2_000.0, 2_000.0)), CAPTURE);
    assert_eq!(over, body);
}

//! The composer: that a preview and an export are one painting at two scales, what the pixels of
//! each backdrop are, what Include and Styled add and take away, how big the image is, how
//! the view is framed, how a layout is put together and that the live chart is never touched.

use super::*;
use crate::model::ChartCanvas;
use crate::model::chart::snapshot::composition::summary;
use crate::model::chart::snapshot::framing::Framing;
use crate::model::chart::snapshot::studio::{Area, AspectRatio, Background, Style};
use crate::model::chart::snapshot::{MAX_PIXELS, MAX_SIDE_PX};
use crate::ui::chart::card::studio_fixture::Fixture;
use crate::ui::chart::card::{SnapshotFrame, SnapshotGeometry};
use crate::ui::snapshot_studio::capture::{Capture, ChartCapture, LayoutCapture};
use crate::ui::snapshot_studio::fixtures::{
    CHART_ID, Flat, chart_capture, close, first_difference, flatten, light_canvas, paint_shapes,
    parts, pixel, plain, rasterized, single, styled, texts,
};
use eframe::egui::{Pos2, Vec2, pos2, vec2};

/// How far apart, in points, the same shape may be at two scales: text is laid out for the
/// pixels it lands on, so a label's width differs by a fraction of a point.
const TOLERANCE: f32 = 2.0;

fn fill_of(canvas: &ChartCanvas) -> Color32 {
    styled::top_color(CanvasStyle::resolve(canvas).fill)
}

fn painted(
    capture: &Capture,
    cards: &Cards<'_>,
    config: &StudioConfig,
    pixels_per_point: f32,
) -> Vec<eframe::egui::epaint::ClippedShape> {
    let size = composition::layout(capture.size(), config).size;
    paint_shapes(size, pixels_per_point, |painter, rect| {
        paint(painter, rect, capture, cards, config).expect("painted");
    })
}

fn image(capture: &Capture, cards: &Cards<'_>, config: &StudioConfig) -> eframe::egui::ColorImage {
    let (scene, dimensions) =
        scene(capture, cards, config, Target::Export(SnapshotScale::X1)).expect("a scene");
    let image = rasterized(&scene);
    assert_eq!(
        image.size,
        [dimensions.width as usize, dimensions.height as usize]
    );
    image
}

// MARK: One painting

#[test]
fn the_preview_and_the_export_paint_the_same_shapes() {
    let fixture = Fixture::busy(ChartCanvas::default(), 300);
    let capture = single(&fixture);
    let cards = |id: u64| (id == CHART_ID).then(|| fixture.card());
    for config in [
        styled(AspectRatio::Landscape3x2, Background::Spectrum),
        styled(AspectRatio::Square, Background::Dots),
        styled(AspectRatio::Original, Background::None),
        plain(),
    ] {
        let preview = flatten(&painted(&capture, &cards, &config, 1.0));
        let export = flatten(&painted(&capture, &cards, &config, 2.0));
        assert!(preview.len() > 50, "something was painted");
        assert_eq!(
            first_difference(&preview, &export, TOLERANCE),
            None,
            "{config:?}"
        );
    }
}

#[test]
fn a_preview_is_the_scene_an_export_is_at_the_scale_that_fits_its_box() {
    let fixture = Fixture::plain(ChartCanvas::default(), 300);
    let capture = single(&fixture);
    let cards = |id: u64| (id == CHART_ID).then(|| fixture.card());
    let config = styled(AspectRatio::Landscape3x2, Background::Radiant);
    let size = composition::layout(capture.size(), &config).size;
    let (_, small) = scene(
        &capture,
        &cards,
        &config,
        Target::Preview { max_px: (600, 600) },
    )
    .expect("a preview");
    assert!(small.width <= 600 && small.height <= 600);
    assert!(
        small.width >= 599 || small.height >= 599,
        "it fills its box"
    );
    assert!(small.pixels_per_point < 1.0);
    let aspect = |width: u32, height: u32| width as f32 / height as f32;
    assert!((aspect(small.width, small.height) - size.x / size.y).abs() < 0.02);
    // A box larger than the image at 2× is not filled by enlarging past 2×.
    let (_, roomy) = scene(
        &capture,
        &cards,
        &config,
        Target::Preview {
            max_px: (20_000, 20_000),
        },
    )
    .expect("a preview");
    assert_eq!(roomy.pixels_per_point, 2.0);
    let (_, export) =
        scene(&capture, &cards, &config, Target::Export(SnapshotScale::X2)).expect("an export");
    assert_eq!(roomy, export, "the same scale, the same image");
}

// MARK: The backdrops

#[test]
fn without_a_backdrop_the_chart_is_the_whole_image() {
    let canvas = ChartCanvas::default();
    let fixture = Fixture::plain(canvas, 300);
    let capture = single(&fixture);
    let cards = |id: u64| (id == CHART_ID).then(|| fixture.card());
    let image = image(&capture, &cards, &plain());
    assert_eq!(image.size, [760, 520], "the capture's own size");
    assert_eq!(pixel(&image, 0, 0), fill_of(&canvas));
    assert_eq!(pixel(&image, 759, 0), fill_of(&canvas));
    assert_eq!(pixel(&image, 0, 519), fill_of(&canvas));
}

#[test]
fn every_backdrop_is_what_it_is_defined_to_be_in_its_margin_and_is_opaque() {
    let fixture = Fixture::plain(ChartCanvas::default(), 300);
    let capture = single(&fixture);
    let cards = |id: u64| (id == CHART_ID).then(|| fixture.card());
    for background in &Background::ALL[1..] {
        let config = styled(AspectRatio::Original, *background);
        let image = image(&capture, &cards, &config);
        assert!(
            image.pixels.iter().all(|pixel| pixel.a() == 255),
            "{background:?} is opaque"
        );
        let (width, height) = (image.size[0], image.size[1]);
        for (x, y) in [
            (1, 1),
            (width - 2, 1),
            (1, height - 2),
            (width - 2, height - 2),
        ] {
            let uv = vec2(
                (x as f32 + 0.5) / width as f32,
                (y as f32 + 0.5) / height as f32,
            );
            let expected = background::color_at(*background, uv);
            let found = pixel(&image, x, y);
            assert!(
                close(found, expected, 2),
                "{background:?} at ({x}, {y}): {found:?} against {expected:?}"
            );
        }
    }
}

#[test]
fn each_gradient_differs_from_the_others_and_changes_across_the_image() {
    let colors: Vec<Vec<Color32>> = Background::ALL[2..]
        .iter()
        .map(|background| {
            [
                vec2(0.05, 0.05),
                vec2(0.95, 0.05),
                vec2(0.5, 0.5),
                vec2(0.05, 0.95),
                vec2(0.95, 0.95),
            ]
            .iter()
            .map(|uv| background::color_at(*background, *uv))
            .collect()
        })
        .collect();
    for (index, one) in colors.iter().enumerate() {
        assert!(
            one.iter().any(|color| !close(*color, one[0], 24)),
            "backdrop {index} is not one flat colour"
        );
        for other in &colors[index + 1..] {
            assert_ne!(one, other);
        }
    }
}

#[test]
fn the_dots_are_on_a_grid_in_the_margin_and_the_rest_is_the_field() {
    let fixture = Fixture::plain(ChartCanvas::default(), 300);
    let capture = single(&fixture);
    let cards = |id: u64| (id == CHART_ID).then(|| fixture.card());
    let config = styled(AspectRatio::Original, Background::Dots);
    let image = image(&capture, &cards, &config);
    let field = background::color_at(Background::Dots, Vec2::ZERO);
    let dot = match background::swatch(Background::Dots) {
        background::SwatchLook::Dots { dot, .. } => dot,
        other => panic!("{other:?}"),
    };
    assert_eq!(pixel(&image, 1, 1), field, "the corner is the field");
    let padding = composition::layout(capture.size(), &config).padding as usize;
    let margin: Vec<Color32> = (0..padding - 4)
        .flat_map(|y| (0..image.size[0]).map(move |x| (x, y)))
        .map(|(x, y)| pixel(&image, x, y))
        .collect();
    assert!(margin.contains(&dot), "a dot, in its own colour");
    let dots = margin.iter().filter(|color| **color == dot).count();
    assert!(
        dots < margin.len() / 20,
        "dots are sparse: {dots} of {}",
        margin.len()
    );
}

#[test]
fn the_chart_is_rounded_and_the_corner_joins_the_margin_without_a_seam() {
    let canvas = ChartCanvas::default();
    let fixture = Fixture::plain(canvas, 300);
    let capture = single(&fixture);
    let cards = |id: u64| (id == CHART_ID).then(|| fixture.card());
    for background in [Background::Spectrum, Background::Luminous, Background::Dots] {
        let mut config = plain();
        config.background = background;
        let composition = composition::layout(capture.size(), &config);
        let image = image(&capture, &cards, &config);
        let surround = Surround::new(
            &composition,
            Rect::from_min_size(Pos2::ZERO, composition.size),
            background,
        );
        let edge = composition.padding as usize;
        // Inside the square of the chart's corner but outside its curve.
        for (x, y) in [(edge, edge), (edge + 1, edge + 1), (edge + 2, edge)] {
            let found = pixel(&image, x, y);
            let expected = surround.surface_at(pos2(x as f32 + 0.5, y as f32 + 0.5));
            assert!(
                close(found, expected, 3),
                "{background:?} corner pixel ({x}, {y}): {found:?} against {expected:?}"
            );
            if background != Background::Dots {
                assert!(
                    !close(found, fill_of(&canvas), 12),
                    "not the chart's colour"
                );
            }
        }
        // Past the curve it is the chart.
        let inside = pixel(&image, edge + 14, edge + 14);
        assert!(close(inside, fill_of(&canvas), 12), "{inside:?}");
    }
}

// MARK: Include and Styled

#[test]
fn indicators_drawings_and_branding_are_each_left_out_when_switched_off() {
    let fixture = Fixture::busy(ChartCanvas::default(), 300);
    let capture = single(&fixture);
    let cards = |id: u64| (id == CHART_ID).then(|| fixture.card());
    let everything = styled(AspectRatio::Landscape3x2, Background::Luminous);
    let shapes = |config: &StudioConfig| painted(&capture, &cards, config, 1.0);
    let all = shapes(&everything);

    let mut no_indicators = everything;
    no_indicators.include.indicators = false;
    let without = shapes(&no_indicators);
    let words = |shapes: &[_]| texts(shapes);
    let lost: Vec<String> = words(&all)
        .into_iter()
        .filter(|word| !words(&without).contains(word))
        .collect();
    assert!(
        !lost.is_empty() && flatten(&without).len() < flatten(&all).len(),
        "the indicators' labels and plots are gone: {lost:?}"
    );

    let mut no_drawings = everything;
    no_drawings.include.drawings = false;
    assert!(
        flatten(&shapes(&no_drawings)).len() < flatten(&all).len(),
        "the trend line is gone"
    );

    assert!(words(&all).contains(&"DegenView".to_owned()));
    let mut no_branding = everything;
    no_branding.include.branding = false;
    let unbranded = words(&shapes(&no_branding));
    assert!(!unbranded.contains(&"DegenView".to_owned()));
    assert_eq!(
        flatten(&shapes(&no_branding)).len() + 2,
        flatten(&all).len(),
        "the mark is its picture and its name, nothing else"
    );
}

#[test]
fn a_drawing_that_is_not_there_changes_nothing() {
    let fixture = Fixture::plain(ChartCanvas::default(), 300);
    let capture = single(&fixture);
    let cards = |id: u64| (id == CHART_ID).then(|| fixture.card());
    let mut off = plain();
    off.include.drawings = false;
    assert_eq!(
        first_difference(
            &flatten(&painted(&capture, &cards, &plain(), 1.0)),
            &flatten(&painted(&capture, &cards, &off, 1.0)),
            0.0,
        ),
        None
    );
}

#[test]
fn the_styled_look_names_the_market_and_the_chart_look_does_not() {
    let fixture = Fixture::plain(ChartCanvas::default(), 300);
    let capture = single(&fixture);
    let cards = |id: u64| (id == CHART_ID).then(|| fixture.card());
    let styled_words = texts(&painted(
        &capture,
        &cards,
        &styled(AspectRatio::Original, Background::None),
        1.0,
    ));
    assert!(
        styled_words.contains(&"BTC/USDT".to_owned()),
        "{styled_words:?}"
    );
    assert!(styled_words.contains(&"BINANCE · 1h".to_owned()));
    let chart_words = texts(&painted(&capture, &cards, &plain(), 1.0));
    assert!(!chart_words.contains(&"BTC/USDT".to_owned()));
    assert!(!chart_words.contains(&"DegenView".to_owned()));
    // The header is inside the strip the layout gave it.
    let config = styled(AspectRatio::Original, Background::None);
    let header = composition::layout(capture.size(), &config)
        .header
        .expect("a header");
    let title = flatten(&painted(&capture, &cards, &config, 1.0))
        .into_iter()
        .find_map(|flat| match flat {
            Flat::Text { text, pos, .. } if text == "BTC/USDT" => Some(pos),
            _ => None,
        })
        .expect("the title");
    assert!(header.contains(title), "{title:?} in {header:?}");
}

#[test]
fn the_mark_is_in_the_title_strip_beside_the_title_and_never_over_the_chart() {
    let fixture = Fixture::busy(ChartCanvas::default(), 300);
    let capture = single(&fixture);
    let cards = |id: u64| (id == CHART_ID).then(|| fixture.card());
    for size in AspectRatio::ALL {
        let config = styled(size, Background::Illuminated);
        let composition = composition::layout(capture.size(), &config);
        let header = composition.header.expect("header");
        let reserved = composition
            .branding
            .unwrap_or_else(|| panic!("{size:?}: no room for the mark in {composition:?}"));
        let shapes = flatten(&painted(&capture, &cards, &config, 1.0));
        let at = |word: &str| {
            shapes.iter().find_map(|flat| match flat {
                Flat::Text { text, pos, .. } if text == word => Some(*pos),
                _ => None,
            })
        };
        // The market's name is whole in every size, and the mark never pushes it.
        let title = at("BTC/USDT").expect("the whole title");
        if let Some(mark) = at("DegenView") {
            assert!(header.contains(mark), "{size:?}: {mark:?} in {header:?}");
            assert!(mark.x > reserved.min.x, "{size:?}: right of the icon");
            assert!(mark.x > title.x, "{size:?}: beside the title");
            assert!(
                mark.y < composition.body().min.y,
                "{size:?}: above the body"
            );
        }
    }
}

#[test]
fn the_mark_gives_way_to_the_title_first_as_its_icon_and_then_altogether() {
    let fixture = Fixture::plain(ChartCanvas::default(), 300);
    let cards = |id: u64| (id == CHART_ID).then(|| fixture.card());
    let config = styled(AspectRatio::Portrait9x16, Background::None);
    let with_title = |title: &str| {
        let mut parts = parts(&fixture, CHART_ID);
        parts.title = title.to_owned();
        let capture =
            Capture::Single(ChartCapture::new(fixture.card(), parts, 1.0, None).expect("captured"));
        let shapes = flatten(&painted(&capture, &cards, &config, 1.0));
        let words = texts(&painted(&capture, &cards, &config, 1.0));
        (words, shapes.len())
    };
    let (short, with_icon) = with_title("BTC");
    assert!(short.contains(&"BTC".to_owned()) && short.contains(&"DegenView".to_owned()));
    // A name that leaves room for the icon only: no wordmark, the icon is still drawn.
    let (middling, icon_alone) = with_title("BTC/USDT Perp");
    assert!(
        middling.contains(&"BTC/USDT Perp".to_owned()),
        "{middling:?}"
    );
    assert!(!middling.contains(&"DegenView".to_owned()));
    assert_eq!(
        icon_alone + 1,
        with_icon,
        "the icon is drawn, the wordmark is not"
    );
    // A name wider than the strip alone (it is cut there) leaves no room: the mark is gone.
    let long = "A market whose name is far longer than any title strip".repeat(2);
    let (cut, nothing) = with_title(&long);
    assert!(!cut.contains(&"DegenView".to_owned()));
    assert_eq!(nothing + 1, icon_alone, "no icon either");
}

#[test]
fn the_starting_prices_fill_the_plot_in_every_size_so_the_last_price_is_inside_it() {
    let fixture = Fixture::busy(ChartCanvas::default(), 300);
    let capture = single(&fixture);
    let Capture::Single(chart) = &capture else {
        panic!("a single chart");
    };
    let last = fixture.candles().last().expect("candles").close;
    for size in AspectRatio::ALL {
        let config = styled(size, Background::Spectrum);
        let view = view_of(&capture, &fixture, &config).view;
        assert!(
            (view.minimum - chart.base().price.minimum).abs() < 1e-6
                && (view.maximum - chart.base().price.maximum).abs() < 1e-6,
            "{size:?}: {} to {}",
            view.minimum,
            view.maximum
        );
        assert!(
            view.minimum < last && last < view.maximum,
            "{size:?}: {last}"
        );
    }
}

// MARK: Size

#[test]
fn an_export_is_as_big_as_the_composition_says_and_the_footer_says_so() {
    let fixture = Fixture::plain(ChartCanvas::default(), 300);
    let capture = single(&fixture);
    let cards = |id: u64| (id == CHART_ID).then(|| fixture.card());
    for size in AspectRatio::ALL {
        for scale in SnapshotScale::ALL {
            let config = styled(size, Background::Spectrum);
            let composition = composition::layout(capture.size(), &config);
            let expected = composition::dimensions(&composition, scale).expect("fits");
            let (scene, dimensions) =
                scene(&capture, &cards, &config, Target::Export(scale)).expect("a scene");
            assert_eq!(dimensions, expected);
            let image = rasterized(&scene);
            assert_eq!(
                image.size,
                [expected.width as usize, expected.height as usize]
            );
            let line = summary(Area::FullChart, Style::Styled, &dimensions).replace(',', "");
            assert!(
                line.contains(&format!("{} × {}", dimensions.width, dimensions.height))
                    && line.ends_with("· PNG"),
                "{line}"
            );
        }
    }
}

#[test]
fn a_composition_too_large_for_the_scale_is_drawn_at_a_smaller_one_or_refused() {
    let huge = Fixture::plain(ChartCanvas::default(), 300).sized(vec2(6000.0, 4000.0));
    let capture = single(&huge);
    let cards = |id: u64| (id == CHART_ID).then(|| huge.card());
    let (_, dimensions) = scene(
        &capture,
        &cards,
        &plain(),
        Target::Export(SnapshotScale::X2),
    )
    .expect("drawn smaller");
    assert!(dimensions.pixels_per_point < 2.0, "{dimensions:?}");
    assert!(dimensions.width <= MAX_SIDE_PX && dimensions.height <= MAX_SIDE_PX);
    assert!(u64::from(dimensions.width) * u64::from(dimensions.height) <= MAX_PIXELS);

    let enormous = Fixture::plain(ChartCanvas::default(), 300).sized(vec2(40_000.0, 100.0));
    let capture = single(&enormous);
    let cards = |id: u64| (id == CHART_ID).then(|| enormous.card());
    assert_eq!(
        scene(
            &capture,
            &cards,
            &plain(),
            Target::Export(SnapshotScale::X1)
        )
        .err(),
        Some(ComposeError::TooLarge)
    );
    assert_eq!(
        scene(
            &capture,
            &cards,
            &plain(),
            Target::Preview { max_px: (0, 100) }
        )
        .err(),
        Some(ComposeError::TooLarge)
    );
}

// MARK: Framing

fn view_of(capture: &Capture, fixture: &Fixture, config: &StudioConfig) -> SnapshotGeometry {
    let Capture::Single(chart) = capture else {
        panic!("a single chart");
    };
    let size = composition::layout(capture.size(), config).size;
    let mut found = None;
    paint_shapes(size, 1.0, |painter, rect| {
        found = Some(
            chart::paint(
                painter,
                rect,
                chart,
                fixture.card(),
                config,
                &config.framing,
            )
            .expect("painted"),
        );
    });
    found.expect("a geometry")
}

#[test]
fn a_dial_at_twice_shows_half_the_bars_or_half_the_prices() {
    let fixture = Fixture::plain(ChartCanvas::default(), 300);
    let capture = single(&fixture);
    let start = view_of(&capture, &fixture, &plain()).view;
    // The slots across the plot: the bars it holds and the free ones past the newest.
    let slots = |view: &crate::ui::chart::card::ViewGeometry| {
        f64::from(view.plot.width() / view.slot_width)
    };

    let mut wide = plain();
    wide.framing = Framing {
        width: 2.0,
        ..Framing::default()
    };
    let view = view_of(&capture, &fixture, &wide).view;
    assert!(
        (slots(&view) - slots(&start) / 2.0).abs() <= 1.0,
        "{} bars against {}",
        slots(&view),
        slots(&start)
    );
    assert!((view.slot_width - start.slot_width * 2.0).abs() / start.slot_width < 0.03);

    let mut tall = plain();
    tall.framing = Framing {
        height: 2.0,
        ..Framing::default()
    };
    let view = view_of(&capture, &fixture, &tall).view;
    let span = |view: &crate::ui::chart::card::ViewGeometry| view.maximum - view.minimum;
    assert!(
        (span(&view) / span(&start) - 0.5).abs() < 0.03,
        "{} against {}",
        span(&view),
        span(&start)
    );

    let mut zoomed_out = plain();
    zoomed_out.framing = Framing {
        width: 0.5,
        ..Framing::default()
    };
    let view = view_of(&capture, &fixture, &zoomed_out).view;
    assert!((slots(&view) - slots(&start) * 2.0).abs() <= 1.0);
}

#[test]
fn the_view_starts_as_the_chart_was() {
    let fixture = Fixture::plain(ChartCanvas::default(), 300);
    let capture = single(&fixture);
    let Capture::Single(chart) = &capture else {
        panic!("a single chart");
    };
    let live = fixture
        .card()
        .snapshot_geometry(&chart.input(SnapshotFrame::default()), chart.size(), 1.0)
        .expect("measured")
        .view;
    let framed = view_of(&capture, &fixture, &plain()).view;
    assert_eq!((framed.start, framed.end), (live.start, live.end));
    assert!((framed.lead - live.lead).abs() < 1e-3);
    assert!((framed.minimum - live.minimum).abs() < 1e-6 * live.minimum.abs().max(1.0));
    assert!((framed.maximum - live.maximum).abs() < 1e-6 * live.maximum.abs().max(1.0));
}

// MARK: Layouts

fn side_by_side(left: &Fixture, right: &Fixture) -> Capture {
    let width = left.card().snapshot_size().x;
    let height = left.card().snapshot_size().y;
    Capture::Layout(LayoutCapture {
        charts: vec![
            (
                chart_capture(left, 1, None),
                Rect::from_min_size(Pos2::ZERO, vec2(width, height)),
            ),
            (
                chart_capture(right, 2, None),
                Rect::from_min_size(pos2(width, 0.0), vec2(width, height)),
            ),
        ],
        size: vec2(2.0 * width, height),
        crop: None,
        focused: 1,
    })
}

#[test]
fn a_layout_paints_each_chart_in_its_rectangle_with_a_hairline_between() {
    let (left, right) = (
        Fixture::plain(ChartCanvas::default(), 300),
        Fixture::plain(ChartCanvas::default(), 200).with_indicators(),
    );
    let capture = side_by_side(&left, &right);
    let cards = |id: u64| match id {
        1 => Some(left.card()),
        2 => Some(right.card()),
        _ => None,
    };
    let shapes = flatten(&painted(&capture, &cards, &plain(), 1.0));
    let vertical = |x: f32| {
        shapes
            .iter()
            .filter(|flat| {
                matches!(flat, Flat::Line(points) if points.len() == 2
                    && (points[0].x - x).abs() < 0.01 && (points[1].x - x).abs() < 0.01
                    && (points[0].y - points[1].y).abs() > 500.0)
            })
            .count()
    };
    assert!(vertical(760.0) >= 1, "a divider where the charts meet");
    assert_eq!(vertical(0.0) + vertical(1520.0), 0, "none on the outline");

    let image = image(&capture, &cards, &plain());
    assert_eq!(image.size, [1520, 520]);
    let [r, g, b] = left.card().appearance().bullish;
    let up = Color32::from_rgb(r, g, b);
    let candles = |from: usize, to: usize| {
        (0..520)
            .flat_map(|y| (from..to).map(move |x| (x, y)))
            .filter(|(x, y)| close(pixel(&image, *x, *y), up, 3))
            .count()
    };
    assert!(
        candles(0, 760) > 1_000 && candles(760, 1520) > 1_000,
        "both charts drew candles"
    );
}

#[test]
fn the_gutter_between_charts_is_the_canvas_and_never_the_backdrop() {
    let (left, right) = (
        Fixture::plain(ChartCanvas::default(), 300),
        Fixture::plain(ChartCanvas::default(), 300),
    );
    let Capture::Layout(mut layout) = side_by_side(&left, &right) else {
        panic!("a layout");
    };
    // The workspace leaves a gutter between its charts.
    let gutter = 12.0;
    layout.charts[1].1 = layout.charts[1].1.translate(vec2(gutter, 0.0));
    layout.size.x += gutter;
    let width = layout.charts[0].1.width();
    let capture = Capture::Layout(layout);
    let cards = |id: u64| match id {
        1 => Some(left.card()),
        2 => Some(right.card()),
        _ => None,
    };
    for background in &Background::ALL[1..] {
        let config = styled(AspectRatio::Original, *background);
        let body = composition::layout(capture.size(), &config).body();
        let scale = body.width() / capture.size().x;
        let image = image(&capture, &cards, &config);
        let x = (body.min.x + (width + gutter / 2.0) * scale) as usize;
        let y = body.center().y as usize;
        let found = pixel(&image, x, y);
        let canvas = fill_of(&ChartCanvas::default());
        assert!(
            close(found, canvas, 3),
            "{background:?} gutter at ({x}, {y}): {found:?} against {canvas:?}"
        );
    }
}

#[test]
fn a_portion_of_a_layout_is_the_layout_cut_and_moved() {
    let (left, right) = (
        Fixture::plain(ChartCanvas::default(), 300),
        Fixture::plain(ChartCanvas::default(), 300),
    );
    let Capture::Layout(mut layout) = side_by_side(&left, &right) else {
        panic!("a layout");
    };
    layout.crop = Some(Rect::from_min_max(pos2(500.0, 100.0), pos2(1000.0, 400.0)));
    let capture = Capture::Layout(layout);
    assert_eq!(capture.size(), vec2(500.0, 300.0));
    let cards = |id: u64| match id {
        1 => Some(left.card()),
        2 => Some(right.card()),
        _ => None,
    };
    let shapes = flatten(&painted(&capture, &cards, &plain(), 1.0));
    // The meeting of the charts, at 760, is 260 into the cut.
    assert!(
        shapes.iter().any(|flat| matches!(flat, Flat::Line(points)
            if points.len() == 2 && (points[0].x - 260.0).abs() < 0.01
                && (points[1].x - 260.0).abs() < 0.01)),
        "the divider moved with the cut"
    );
    assert_eq!(image(&capture, &cards, &plain()).size, [500, 300]);
}

// MARK: Nothing changes

#[test]
fn composing_changes_nothing_in_the_live_chart() {
    let fixture = Fixture::busy(ChartCanvas::default(), 300);
    let before = fixture.fingerprint();
    let capture = single(&fixture);
    let cards = |id: u64| (id == CHART_ID).then(|| fixture.card());
    for config in [
        plain(),
        styled(AspectRatio::Wide16x9, Background::Radiant),
        styled(AspectRatio::Portrait9x16, Background::Dots),
    ] {
        for target in [
            Target::Preview { max_px: (800, 600) },
            Target::Export(SnapshotScale::X2),
        ] {
            drop(scene(&capture, &cards, &config, target).expect("a scene"));
        }
    }
    assert_eq!(fixture.fingerprint(), before);
}

#[test]
fn a_chart_that_is_gone_cannot_be_drawn() {
    let fixture = Fixture::plain(ChartCanvas::default(), 300);
    let capture = single(&fixture);
    let nowhere = |_: u64| None;
    assert_eq!(
        scene(
            &capture,
            &nowhere,
            &plain(),
            Target::Export(SnapshotScale::X1)
        )
        .err(),
        Some(ComposeError::Chart(SnapshotError::Empty))
    );
}

#[test]
fn a_light_canvas_is_headed_in_dark_ink() {
    let fixture = Fixture::plain(light_canvas(), 300);
    let capture = single(&fixture);
    let cards = |id: u64| (id == CHART_ID).then(|| fixture.card());
    let config = styled(AspectRatio::Original, Background::None);
    let shapes = flatten(&painted(&capture, &cards, &config, 1.0));
    let title = shapes
        .iter()
        .find_map(|flat| match flat {
            Flat::Text { text, color, .. } if text == "BTC/USDT" => Some(*color),
            _ => None,
        })
        .expect("the title");
    assert!(title.r() < 60, "{title:?}");
}

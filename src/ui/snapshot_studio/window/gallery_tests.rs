//! The whole window written as PNGs for a person to look at. Run by hand:
//! `DEGENVIEW_GALLERY_DIR=<dir> cargo test --lib gallery_of_the_studio_window -- --ignored`.

use super::rig::{Rig, Scenario, Shown, picture};
use crate::model::ChartCanvas;
use crate::model::chart::snapshot::SnapshotScale;
use crate::model::chart::snapshot::composition;
use crate::model::chart::snapshot::framing::Framing;
use crate::model::chart::snapshot::studio::{Area, AspectRatio, Background, StudioConfig, Style};
use crate::ui::chart::card::studio_fixture::Fixture;
use crate::ui::render_tests::gallery_dir;
use crate::ui::snapshot_studio::capture::{Capture, ChartCapture, LayoutCapture};
use crate::ui::snapshot_studio::compose::Cards;
use crate::ui::snapshot_studio::fixtures::{CHART_ID, chart_capture, light_canvas, parts, single};
use crate::ui::theme::RestoreDefaultTheme;
use eframe::egui::{Pos2, Rect, Vec2, pos2, vec2};

const DEFAULT: Vec2 = vec2(1440.0, 900.0);
/// The chart window's smallest size.
const MINIMUM: Vec2 = vec2(960.0, 620.0);

const AREAS_WITH_LAYOUT: [Area; 3] = [Area::FullChart, Area::EntireLayout, Area::Portion];

struct Case {
    name: &'static str,
    config: StudioConfig,
    loading: bool,
    problem: Option<&'static str>,
    layout: bool,
    long_symbol: bool,
    bare_chart: bool,
}

fn case(name: &'static str, config: StudioConfig) -> Case {
    Case {
        name,
        config,
        loading: false,
        problem: None,
        layout: false,
        long_symbol: false,
        bare_chart: false,
    }
}

fn styled(background: Background, size: AspectRatio) -> StudioConfig {
    let mut config = StudioConfig::new(Area::Portion);
    config.background = background;
    config.size = size;
    config
}

fn cases() -> Vec<Case> {
    let mut chart = StudioConfig::new(Area::FullChart);
    chart.set_style(Style::Chart);
    let mut framed = styled(Background::Radiant, AspectRatio::Wide16x9);
    framed.framing = Framing {
        width: 1.28,
        height: 0.8,
        dx: -6.0,
        dy: 0.1,
    };
    let mut layout = StudioConfig::new(Area::EntireLayout);
    layout.background = Background::Luminous;
    vec![
        Case {
            loading: true,
            ..case(
                "loading",
                styled(Background::Spectrum, AspectRatio::Landscape3x2),
            )
        },
        case(
            "styled-spectrum",
            styled(Background::Spectrum, AspectRatio::Landscape3x2),
        ),
        case(
            "styled-none",
            styled(Background::None, AspectRatio::Landscape3x2),
        ),
        case("chart-style", chart),
        case("framing-changed", framed),
        case(
            "portrait-dots",
            styled(Background::Dots, AspectRatio::Portrait9x16),
        ),
        Case {
            layout: true,
            ..case("entire-layout", layout)
        },
        Case {
            long_symbol: true,
            ..case(
                "long-symbol",
                styled(Background::Illuminated, AspectRatio::Landscape3x2),
            )
        },
        Case {
            bare_chart: true,
            ..case(
                "nothing-to-include",
                styled(Background::None, AspectRatio::Landscape3x2),
            )
        },
        Case {
            problem: Some("This image is too large to draw. Try a different size or background."),
            ..case(
                "too-large",
                styled(Background::Spectrum, AspectRatio::Square),
            )
        },
    ]
}

fn layout_capture(left: &Fixture, right: &Fixture) -> Capture {
    let size = left.card().snapshot_size();
    Capture::Layout(LayoutCapture {
        charts: vec![
            (
                chart_capture(left, 1, None),
                Rect::from_min_size(Pos2::ZERO, size),
            ),
            (
                chart_capture(right, 2, None),
                Rect::from_min_size(pos2(size.x, 0.0), size),
            ),
        ],
        size: vec2(2.0 * size.x, size.y),
        crop: None,
        focused: 0,
    })
}

#[test]
#[ignore = "writes images for a person to look at"]
fn gallery_of_the_studio_window() {
    if gallery_dir().is_none() {
        return;
    }
    let _restore = RestoreDefaultTheme;
    for (screen_name, size) in [("default", DEFAULT), ("minimum", MINIMUM)] {
        for light in [false, true] {
            let canvas = if light {
                light_canvas()
            } else {
                ChartCanvas::default()
            };
            let busy = Fixture::busy(canvas, 300);
            let right = Fixture::plain(canvas, 200).with_a_drawing();
            let bare = Fixture::plain(canvas, 300);
            for case in cases() {
                let tag = format!(
                    "{}-{screen_name}-{}",
                    case.name,
                    if light { "light" } else { "dark" }
                );
                if let Ok(filter) = std::env::var("DEGENVIEW_GALLERY_FILTER")
                    && !filter.split(',').any(|part| tag.contains(part))
                {
                    continue;
                }
                let (fixture, long) = if case.bare_chart {
                    (&bare, false)
                } else {
                    (&busy, case.long_symbol)
                };
                let capture = if case.layout {
                    layout_capture(&busy, &right)
                } else if long {
                    let mut chart_parts = parts(fixture, CHART_ID);
                    chart_parts.title =
                        "Bitcoin / TetherUS perpetual futures contract, quarterly".to_owned();
                    chart_parts.symbol = "BTCUSDT_QUARTERLY_CONTRACT".to_owned();
                    Capture::Single(
                        ChartCapture::new(fixture.card(), chart_parts, 1.0, None)
                            .expect("a capture"),
                    )
                } else {
                    single(fixture)
                };
                let cards: &Cards<'_> = &|id| match id {
                    1 => Some(busy.card()),
                    2 => Some(right.card()),
                    _ => None,
                };
                let mut scenario = Scenario::new(case.config);
                scenario.reframable = !case.layout;
                if case.layout {
                    scenario.areas = &AREAS_WITH_LAYOUT;
                }
                scenario.has_indicators = capture.has_indicators();
                scenario.has_drawings = capture.has_drawings();
                let composition = composition::layout(capture.size(), &scenario.config);
                if let Some(dimensions) = composition::dimensions(&composition, SnapshotScale::X2) {
                    scenario.summary = Ok(composition::summary(
                        scenario.config.area,
                        scenario.config.style,
                        &dimensions,
                    ));
                }
                if let Some(problem) = case.problem {
                    scenario.summary = Err(problem.to_owned());
                    scenario.can_export = false;
                }
                let mut rig = Rig::new(size, light);
                let (output, _) = rig.settle(&scenario);
                // The picture's texture lives until the frame is written.
                let _kept;
                if case.loading {
                    scenario.preview = Shown::Loading;
                } else if let Some(problem) = case.problem {
                    scenario.preview = Shown::Failed(problem);
                } else {
                    let (handle, image) = picture(
                        &rig.screen,
                        &capture,
                        cards,
                        &scenario.config,
                        output.preview_pixels,
                    );
                    _kept = handle;
                    scenario.preview = Shown::Picture(image);
                }
                let (_, full) = rig.settle(&scenario);
                rig.screen.save(
                    &format!(
                        "studio-{}-{screen_name}-{}",
                        case.name,
                        if light { "light" } else { "dark" }
                    ),
                    full,
                );
            }
        }
    }
}

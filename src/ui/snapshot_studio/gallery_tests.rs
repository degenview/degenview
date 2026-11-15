//! The Studio's images written as PNGs for a person to look at. Run by hand:
//! `DEGENVIEW_GALLERY_DIR=<dir> cargo test --lib gallery_of_the_studio -- --ignored`.

use super::capture::{Capture, LayoutCapture};
use super::compose::{self, Target};
use super::fixtures::{CHART_ID, chart_capture, light_canvas, rasterized, single, styled};
use crate::model::ChartCanvas;
use crate::model::chart::snapshot::studio::{Area, AspectRatio, Background, StudioConfig, Style};
use crate::service::snapshot::export;
use crate::ui::chart::card::studio_fixture::Fixture;
use crate::ui::render_tests::gallery_dir;
use eframe::egui::{Pos2, Rect, pos2, vec2};

/// The widest the pictures are drawn, so a gallery of seventy is quick to write and to look at.
const PICTURE_PX: (u32, u32) = (1500, 1500);

fn write(
    dir: &std::path::Path,
    name: &str,
    capture: &Capture,
    cards: &compose::Cards<'_>,
    config: &StudioConfig,
) {
    let (scene, _) = compose::scene(
        capture,
        cards,
        config,
        Target::Preview { max_px: PICTURE_PX },
    )
    .expect("a scene");
    let png = export::encode_png(&rasterized(&scene)).expect("encoded");
    std::fs::write(dir.join(format!("{name}.png")), png).expect("written");
}

#[test]
#[ignore = "writes images for a person to look at"]
fn gallery_of_the_studio_compositions() {
    let Some(dir) = gallery_dir() else {
        return;
    };
    for (look, canvas) in [("dark", ChartCanvas::default()), ("light", light_canvas())] {
        let fixture = Fixture::busy(canvas, 300);
        let capture = single(&fixture);
        let cards = |id: u64| (id == CHART_ID).then(|| fixture.card());
        for background in Background::ALL {
            let name = background.label().to_lowercase();
            let mut chart = StudioConfig::new(Area::FullChart);
            chart.set_style(Style::Chart);
            chart.background = background;
            write(
                &dir,
                &format!("{look}-chart-{name}"),
                &capture,
                &cards,
                &chart,
            );
            for size in AspectRatio::ALL {
                let config = styled(size, background);
                let size = size.label().replace([':'], "x").to_lowercase();
                write(
                    &dir,
                    &format!("{look}-styled-{size}-{name}"),
                    &capture,
                    &cards,
                    &config,
                );
            }
        }
    }
}

#[test]
#[ignore = "writes images for a person to look at"]
fn gallery_of_the_studio_layouts_and_portions() {
    let Some(dir) = gallery_dir() else {
        return;
    };
    let (left, right) = (
        Fixture::plain(ChartCanvas::default(), 300).with_a_drawing(),
        Fixture::busy(ChartCanvas::default(), 200),
    );
    let (width, height) = (left.card().snapshot_size().x, left.card().snapshot_size().y);
    let layout = |crop: Option<Rect>| {
        Capture::Layout(LayoutCapture {
            charts: vec![
                (
                    chart_capture(&left, 1, None),
                    Rect::from_min_size(Pos2::ZERO, vec2(width, height)),
                ),
                (
                    chart_capture(&right, 2, None),
                    Rect::from_min_size(pos2(width, 0.0), vec2(width, height)),
                ),
            ],
            size: vec2(2.0 * width, height),
            crop,
            focused: 0,
        })
    };
    let cards = |id: u64| match id {
        1 => Some(left.card()),
        2 => Some(right.card()),
        _ => None,
    };
    let whole = layout(None);
    write(&dir, "layout-chart", &whole, &cards, &{
        let mut config = StudioConfig::new(Area::EntireLayout);
        config.set_style(Style::Chart);
        config
    });
    write(
        &dir,
        "layout-styled",
        &whole,
        &cards,
        &styled(AspectRatio::Wide16x9, Background::Spectrum),
    );
    let cut = layout(Some(Rect::from_min_max(
        pos2(500.0, 100.0),
        pos2(1100.0, 460.0),
    )));
    let mut portion = styled(AspectRatio::Landscape3x2, Background::Luminous);
    portion.area = Area::Portion;
    write(&dir, "layout-portion-styled", &cut, &cards, &portion);

    // A portion of one chart, framed tighter and moved.
    let selection = Rect::from_min_max(pos2(220.0, 90.0), pos2(560.0, 330.0));
    let capture = Capture::Single(chart_capture(&left, 1, Some(selection)));
    write(&dir, "portion-chart", &capture, &cards, &{
        let mut config = StudioConfig::new(Area::Portion);
        config.set_style(Style::Chart);
        config
    });
    let mut framed = styled(AspectRatio::Square, Background::Radiant);
    framed.area = Area::Portion;
    framed.set_style(Style::Styled);
    write(&dir, "portion-styled", &capture, &cards, &framed);
}

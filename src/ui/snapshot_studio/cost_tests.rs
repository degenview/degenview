//! What a preview and an export cost, as the Studio will pay it. Run with
//! `cargo test --release studio_cost -- --ignored --nocapture`.

use super::compose::{self, Target};
use super::fixtures::{CHART_ID, rasterized, single, styled};
use crate::model::ChartCanvas;
use crate::model::chart::snapshot::SnapshotScale;
use crate::model::chart::snapshot::studio::{AspectRatio, Background};
use crate::service::snapshot::export;
use crate::ui::chart::card::studio_fixture::Fixture;
use std::time::Instant;

/// Prints how long each stage takes for a 500-candle chart with indicators, a script pane and a
/// drawing: the preview (about 1600 × 1066 pixels, on the UI thread: the scene; on the worker:
/// the raster) and an export at 2× in 3:2 on Spectrum (scene, raster, PNG).
#[test]
#[ignore = "a measurement, not a check"]
fn studio_cost() {
    let fixture = Fixture::busy(ChartCanvas::default(), 500);
    let started = Instant::now();
    let capture = single(&fixture);
    eprintln!("freeze {:?} (UI thread, once)", started.elapsed());
    let cards = |id: u64| (id == CHART_ID).then(|| fixture.card());
    let config = styled(AspectRatio::Landscape3x2, Background::Spectrum);

    for run in 0..2 {
        let started = Instant::now();
        let (scene, dimensions) = compose::scene(
            &capture,
            &cards,
            &config,
            Target::Preview {
                max_px: (1600, 1066),
            },
        )
        .expect("a scene");
        let composed = started.elapsed();
        let started = Instant::now();
        let image = rasterized(&scene);
        let rasterizing = started.elapsed();
        eprintln!(
            "preview {}×{} (run {run}): compose+tessellate {composed:?} (UI thread), rasterize {rasterizing:?} ({} px)",
            dimensions.width,
            dimensions.height,
            image.pixels.len(),
        );
    }

    let started = Instant::now();
    let (scene, dimensions) =
        compose::scene(&capture, &cards, &config, Target::Export(SnapshotScale::X2))
            .expect("a scene");
    let composed = started.elapsed();
    let started = Instant::now();
    let image = rasterized(&scene);
    let rasterizing = started.elapsed();
    let started = Instant::now();
    let png = export::encode_png(&image).expect("encoded");
    eprintln!(
        "export {}×{}: compose+tessellate {composed:?}, rasterize {rasterizing:?}, png {:?}, {} KiB png, {} MiB raw",
        dimensions.width,
        dimensions.height,
        started.elapsed(),
        png.len() / 1024,
        image.pixels.len() * 4 / (1024 * 1024),
    );
}

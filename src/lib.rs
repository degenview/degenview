#[cfg(test)]
mod alloc_count;
mod app;
mod format;
mod host;
mod mcp;
mod model;
mod native_bridge;
mod pine;
mod platform;
mod script_manager;
mod service;
mod store;
mod sync;
mod ui;
mod workspace;

use app::DegenViewApp;
use eframe::egui;

pub use host::sync_host::{EventSink, TransportFactory};
pub use sync::native::{CloudKitFactory, NativeApi};

pub fn run_gui() -> Result<(), Box<dyn std::error::Error>> {
    run_gui_with(None)
}

/// Runs the app. `sync` connects iCloud Sync to CloudKit; builds without it pass `None` and the
/// feature stays out of Settings.
pub fn run_gui_with(
    sync: Option<std::sync::Arc<dyn TransportFactory>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let store = store::open_default()?;
    // A finished update leaves the previous app in a hidden folder until the new one has started.
    service::work_queue::submit(service::self_update::cleanup_stale);
    // macOS rounds and insets the square art itself (`inset_macos_icon`); elsewhere the file is
    // drawn as it is, so it comes with its corners (`tools/round-icon.py`).
    #[cfg(target_os = "macos")]
    let icon = inset_macos_icon(eframe::icon_data::from_png_bytes(include_bytes!(
        "../assets/brand/DegenView-512.png"
    ))?);
    #[cfg(not(target_os = "macos"))]
    let icon = ui::window_chrome::app_icon();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("DegenView")
            .with_app_id(ui::window_chrome::APP_ID)
            // The surface is shared by every window, and eframe reads whether it may be
            // transparent from this first one (`ui::window_corners`).
            .with_transparent(ui::window_chrome::rounded_corners())
            .with_icon(icon)
            .with_inner_size([1.0, 1.0])
            .with_position([-10_000.0, -10_000.0]),
        ..Default::default()
    };

    Ok(eframe::run_native(
        "DegenView",
        options,
        Box::new(move |creation| {
            Ok(Box::new(DegenViewApp::new(
                creation,
                store.clone(),
                sync.clone(),
            )))
        }),
    )?)
}

pub fn run_alert_agent() -> Result<(), Box<dyn std::error::Error>> {
    host::alert_runtime::run_alert_agent()
}

/// Fraction of the canvas the icon art fills. AppKit draws `applicationIconImage` as-is, so the
/// standard macOS margin (824pt squircle in a 1024pt canvas) must be baked in.
#[cfg(target_os = "macos")]
const MACOS_ICON_ART_FRACTION: f32 = 824.0 / 1024.0;

/// Scales the art into the standard macOS icon grid: rounded corners plus transparent margin.
#[cfg(target_os = "macos")]
fn inset_macos_icon(icon: egui::IconData) -> egui::IconData {
    let width = icon.width as usize;
    let height = icon.height as usize;
    if width == 0 || height == 0 || icon.rgba.len() != width * height * 4 {
        return icon;
    }

    let art_width = ((width as f32 * MACOS_ICON_ART_FRACTION).round() as usize).clamp(1, width);
    let art_height = ((height as f32 * MACOS_ICON_ART_FRACTION).round() as usize).clamp(1, height);
    let mut art = egui::IconData {
        rgba: resample_rgba(&icon.rgba, width, height, art_width, art_height),
        width: art_width as u32,
        height: art_height as u32,
    };
    round_macos_icon_corners(&mut art);

    let left = (width - art_width) / 2;
    let top = (height - art_height) / 2;
    let mut rgba = vec![0u8; width * height * 4];
    for y in 0..art_height {
        let src = y * art_width * 4;
        let dst = ((top + y) * width + left) * 4;
        rgba[dst..dst + art_width * 4].copy_from_slice(&art.rgba[src..src + art_width * 4]);
    }
    egui::IconData {
        rgba,
        width: icon.width,
        height: icon.height,
    }
}

/// Box-filter resample (area average) — adequate for the one-off icon downscale.
#[cfg(target_os = "macos")]
fn resample_rgba(src: &[u8], sw: usize, sh: usize, dw: usize, dh: usize) -> Vec<u8> {
    let mut out = vec![0u8; dw * dh * 4];
    for dy in 0..dh {
        let y0 = dy * sh / dh;
        let y1 = ((dy + 1) * sh / dh).max(y0 + 1).min(sh);
        for dx in 0..dw {
            let x0 = dx * sw / dw;
            let x1 = ((dx + 1) * sw / dw).max(x0 + 1).min(sw);
            let mut sum = [0u32; 4];
            for y in y0..y1 {
                for x in x0..x1 {
                    let p = &src[(y * sw + x) * 4..(y * sw + x) * 4 + 4];
                    // Premultiply so transparent pixels do not bleed colour.
                    let a = p[3] as u32;
                    sum[0] += p[0] as u32 * a;
                    sum[1] += p[1] as u32 * a;
                    sum[2] += p[2] as u32 * a;
                    sum[3] += a;
                }
            }
            let count = ((y1 - y0) * (x1 - x0)) as u32;
            let o = (dy * dw + dx) * 4;
            for c in 0..3 {
                out[o + c] = sum[c].checked_div(sum[3]).unwrap_or(0) as u8;
            }
            out[o + 3] = (sum[3] / count) as u8;
        }
    }
    out
}

#[cfg(target_os = "macos")]
fn round_macos_icon_corners(icon: &mut egui::IconData) {
    let width = icon.width as usize;
    let height = icon.height as usize;
    if width == 0 || height == 0 || icon.rgba.len() != width * height * 4 {
        return;
    }

    let half_width = width as f32 / 2.0;
    let half_height = height as f32 / 2.0;
    let radius = (width.min(height) as f32 * 0.223)
        .min(half_width)
        .min(half_height);
    for y in 0..height {
        let qy = ((y as f32 + 0.5) - half_height).abs() - (half_height - radius);
        for x in 0..width {
            let qx = ((x as f32 + 0.5) - half_width).abs() - (half_width - radius);
            let outside_x = qx.max(0.0);
            let outside_y = qy.max(0.0);
            let distance = outside_x.hypot(outside_y) + qx.max(qy).min(0.0) - radius;
            let coverage = (0.5 - distance).clamp(0.0, 1.0);
            let alpha = &mut icon.rgba[(y * width + x) * 4 + 3];
            *alpha = (*alpha as f32 * coverage).round() as u8;
        }
    }
}

/// The window icon off macOS, the Flatpak's and the tarball's: the square master with its corners
/// in the alpha channel (`tools/round-icon.py`), since nothing there rounds it.
#[cfg(test)]
mod rounded_icon_tests {
    #[test]
    fn the_rounded_icon_has_clear_corners_and_a_solid_middle() {
        let icon = eframe::icon_data::from_png_bytes(include_bytes!(
            "../assets/brand/DegenView-512-rounded.png"
        ))
        .unwrap();
        let (w, h) = (icon.width as usize, icon.height as usize);
        assert_eq!((w, h), (512, 512));
        let alpha = |x: usize, y: usize| icon.rgba[(y * w + x) * 4 + 3];
        for (x, y) in [(0, 0), (w - 1, 0), (0, h - 1), (w - 1, h - 1)] {
            assert_eq!(alpha(x, y), 0, "corner ({x}, {y})");
        }
        // The edges' midpoints and the art itself are untouched; the radius is 22.3% of the side.
        for (x, y) in [(w / 2, 0), (0, h / 2), (w / 2, h / 2), (w - 1, h / 2)] {
            assert_eq!(alpha(x, y), 255, "({x}, {y})");
        }
        let radius = (w as f32 * 0.223) as usize;
        assert_eq!(
            alpha(radius + 4, 0),
            255,
            "the straight edge starts at the radius"
        );
        assert!(
            alpha(radius / 4, radius / 4) < 255,
            "the arc cuts the corner"
        );
    }
}

#[cfg(all(test, target_os = "macos"))]
mod icon_tests {
    use super::*;

    #[test]
    fn dock_icon_keeps_standard_margin_and_rounded_corners() {
        let icon =
            eframe::icon_data::from_png_bytes(include_bytes!("../assets/brand/DegenView-512.png"))
                .unwrap();
        let (w, h) = (icon.width as usize, icon.height as usize);
        let icon = inset_macos_icon(icon);
        assert_eq!((icon.width as usize, icon.height as usize), (w, h));
        let alpha = |x: usize, y: usize| icon.rgba[(y * w + x) * 4 + 3];
        let margin = ((w as f32 * (1.0 - MACOS_ICON_ART_FRACTION)) / 2.0).round() as usize;

        assert_eq!(alpha(w / 2, margin - 2), 0, "margin must be transparent");
        assert_eq!(alpha(w / 2, margin + 2), 255, "art starts after the margin");
        assert_eq!(alpha(w / 2, h / 2), 255);
        assert_eq!(alpha(margin + 1, margin + 1), 0, "art corner is rounded");
        assert_eq!(alpha(0, 0), 0);
    }
}

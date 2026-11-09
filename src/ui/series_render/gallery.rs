//! Pictures of the three order-flow painters in the light theme and at a narrow width, for a person
//! to look at: `DEGENVIEW_GALLERY_DIR=/tmp/g cargo test --lib series_render::gallery -- --ignored`.
//! (The dark pictures are each painter's own gallery.)

use super::SeriesFrame;
use crate::model::chart_look::Palette;
use crate::model::chart_type::{ChartStyle, ChartType};
use crate::model::market_profile::feed::ProfileFeed;
use crate::model::{ChartAppearance, Kline, PriceFormat};
use crate::ui::chart_card::PriceScale;
use crate::ui::chart_style_preview::{sample_candles, sample_footprint, sample_svp, sample_tpo};
use crate::ui::render_tests::{Screen, gallery_dir};
use crate::ui::theme::{self, AppTheme};
use eframe::egui::{self, Pos2, Rect};

#[test]
#[ignore = "writes images for a person to look at"]
fn gallery_of_the_order_flow_types_in_the_light_theme() {
    if gallery_dir().is_none() {
        return;
    }
    let _restore = theme::RestoreDefaultTheme;
    for (chart_type, name, slot_bars) in [
        (ChartType::VolumeFootprint, "footprint", 6usize),
        (ChartType::TimePriceOpportunity, "tpo", 24),
        (ChartType::SessionVolumeProfile, "svp", 24),
    ] {
        let style = ChartStyle {
            chart_type,
            ..ChartStyle::default()
        };
        let appearance = ChartAppearance {
            style,
            ..ChartAppearance::default()
        };
        let (bars, feed): (Vec<Kline>, ProfileFeed) = match chart_type {
            ChartType::VolumeFootprint => {
                let all = sample_candles();
                (all[all.len() - 6..].to_vec(), sample_footprint(&appearance))
            }
            ChartType::TimePriceOpportunity => {
                (sample_candles()[..24].to_vec(), sample_tpo(&appearance))
            }
            _ => (sample_candles()[..24].to_vec(), sample_svp(&appearance)),
        };
        let (width, height) = (900.0_f32, 640.0_f32);
        let mut screen = Screen::new(egui::vec2(width, height));
        theme::apply(&screen.ctx, AppTheme::Light);
        let slot = width / slot_bars as f32;
        let mut last = None;
        for _ in 0..2 {
            last = Some(screen.run(Vec::new(), |ui| {
                theme::sync(ui.ctx());
                let plot = Rect::from_min_size(Pos2::ZERO, egui::vec2(width, height));
                ui.painter().rect_filled(plot, 0.0, theme::chrome());
                let scale = PriceScale::linear(95.0, 125.0, 0.0, height);
                let x_for = |index: usize| slot * (index as f32 + 0.5);
                super::paint(&SeriesFrame {
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
                    chart_type,
                    recolor: None,
                    price_format: PriceFormat::AUTO,
                    profiles: Some(&feed),
                });
            }));
        }
        screen.save(&format!("{name}-light"), last.expect("a frame"));
    }
}

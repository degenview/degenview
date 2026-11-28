//! A script's point on the chart: a line at its time or its price, a handle at the point both
//! name, and their values on the axes.

use super::super::crosshair::paint_value_tag;
use super::super::*;
use crate::pine::inputs::points::PointValue;

/// Half the side of a line's square handle, and the radius of a point's round one.
const HANDLE: f32 = 4.5;

/// Where a point's price is read and its marker drawn.
pub(super) struct Surface {
    /// The price chart or a script pane, without the price axis.
    pub rect: Rect,
    pub scale: PriceScale,
}

impl ChartCard {
    pub(super) fn paint_point(
        &self,
        painter: &egui::Painter,
        surface: &Surface,
        projection: &Projection,
        layout: &frame::FrameLayout,
        point: PointValue,
    ) {
        let rect = surface.rect;
        let x = point.time_ms.map(|time_ms| projection.x(time_ms));
        let y = point.price.map(|price| surface.scale.y(price));
        let inside = painter.with_clip_rect(rect);
        let line = Stroke::new(1.0, theme::text_secondary());
        let handle = |centre: Pos2, round: bool| {
            let stroke = Stroke::new(1.5, theme::text_primary());
            if round {
                inside.circle(centre, HANDLE, theme::surface(), stroke);
            } else {
                let square = Rect::from_center_size(centre, Vec2::splat(2.0 * HANDLE));
                inside.rect(
                    square,
                    2.0,
                    theme::surface(),
                    stroke,
                    egui::StrokeKind::Middle,
                );
            }
        };
        match (x, y) {
            (Some(x), Some(y)) => handle(Pos2::new(x, y), true),
            (Some(x), None) => {
                inside.vline(x, rect.y_range(), line);
                handle(Pos2::new(x, rect.center().y), false);
            }
            (None, Some(y)) => {
                inside.hline(rect.x_range(), y, line);
                handle(Pos2::new(rect.center().x, y), false);
            }
            (None, None) => {}
        }
        if let Some((x, time_ms)) = x.zip(point.time_ms)
            && rect.x_range().contains(x)
        {
            self.paint_time_tag(painter, layout.plot, layout.rect, x, time_ms, &layout.style);
        }
        if let Some((y, price)) = y.zip(point.price)
            && rect.y_range().contains(y)
        {
            let label = surface
                .scale
                .tag_label(self.appearance.price_format(), price);
            let axis = (layout.plot.right(), layout.rect.right());
            paint_value_tag(painter, axis, y, &label, &layout.style);
        }
    }
}

//! The picture at the head of each Settings page: a card and a few marks in the section's colour
//! that say what the page is about, with no disc behind them (the flat style, `flat` on
//! `Art`) — the same recipe as the Pine Studio's empty tabs (`drawer_intro`), one small scene per page. Painter code only, so it follows the theme.
//!
//! The Swift app has a plain icon badge here; these pictures are an addition, not a port.

use crate::ui::icons::Icon;
use crate::ui::settings_window::SettingsSection;
use crate::ui::{kit, theme};
use eframe::egui::{
    self, Color32, CornerRadius, Pos2, Rect, Shape, Stroke, StrokeKind, Ui, Vec2, pos2, vec2,
};

pub use crate::ui::art::SIZE;
use crate::ui::art::{Art, CARD, ghost};

/// Draws `section`'s picture in `rect` (a [`SIZE`] box).
pub fn paint(ui: &Ui, rect: Rect, section: SettingsSection) {
    let art = Art {
        ui,
        painter: ui.painter(),
        tint: section.tint(),
        centre: rect.center(),
        flat: true,
    };
    match section {
        SettingsSection::Appearance => art.appearance(),
        SettingsSection::PineStudio => art.pine_studio(),
        SettingsSection::Alpaca => art.alpaca(),
        SettingsSection::Brokers => art.brokers(),
        SettingsSection::CoinMarketCap => art.coinmarketcap(),
        SettingsSection::Ai => art.ai(),
        SettingsSection::Notifications => art.notifications(),
        SettingsSection::Webhooks => art.webhooks(),
        SettingsSection::Snapshots => art.snapshots(),
        SettingsSection::Mcp => art.mcp(),
        SettingsSection::Sync => art.sync(),
        SettingsSection::Storage => art.storage(),
        SettingsSection::About => art.about(),
    }
}

impl Art<'_> {
    /// Light and dark halves of one card, a chart across both, and a row of colour choices.
    fn appearance(&self) {
        let centre = self.centre - vec2(0.0, 5.0);
        self.disc(centre);
        let card = Rect::from_center_size(centre, CARD);
        self.card_at(card, theme::chrome());
        let inner = card.shrink(1.5);
        let light = Color32::from_rgb(244, 244, 247);
        let dark = Color32::from_rgb(38, 38, 44);
        let (left, right) = (
            Rect::from_min_max(inner.min, pos2(inner.center().x, inner.max.y)),
            Rect::from_min_max(pos2(inner.center().x, inner.min.y), inner.max),
        );
        self.painter.rect_filled(
            left,
            CornerRadius {
                nw: 8,
                sw: 8,
                ne: 0,
                se: 0,
            },
            light,
        );
        self.painter.rect_filled(
            right,
            CornerRadius {
                nw: 0,
                sw: 0,
                ne: 8,
                se: 8,
            },
            dark,
        );
        // Candles that run across the join, each in the half it sits on.
        for (index, (top, bottom, up)) in [
            (8.0, 17.0, false),
            (13.0, 25.0, false),
            (9.0, 20.0, true),
            (4.0, 15.0, true),
            (11.0, 17.0, false),
            (6.0, 14.0, true),
        ]
        .into_iter()
        .enumerate()
        {
            let x = inner.left() + 8.0 + index as f32 * 8.6;
            let y = inner.top() + 12.0;
            let color = if up { kit::GREEN } else { kit::RED };
            self.painter
                .vline(x, y + top - 3.0..=y + bottom + 3.0, Stroke::new(1.1, color));
            self.painter.rect_filled(
                Rect::from_min_max(pos2(x - 2.3, y + top), pos2(x + 2.3, y + bottom)),
                1.2,
                color,
            );
        }
        self.painter.vline(
            inner.center().x,
            inner.y_range(),
            Stroke::new(1.0, Color32::from_gray(128).gamma_multiply(0.5)),
        );
        // The choices below, one of them taken.
        let y = card.bottom() + 9.0;
        let colors = [self.tint, kit::ORANGE, kit::GREEN, theme::ACCENT];
        for (index, color) in colors.into_iter().enumerate() {
            let x = centre.x - 16.5 + index as f32 * 11.0;
            self.painter.circle_filled(pos2(x, y), 3.4, color);
            if index == 0 {
                self.painter.circle_stroke(
                    pos2(x, y),
                    5.4,
                    Stroke::new(1.3, theme::text_secondary()),
                );
            }
        }
    }

    /// Source lines in colour with a gutter and a caret, and the curly braces on a chip.
    fn pine_studio(&self) {
        let card = self.scene();
        let gutter = card.left() + 15.0;
        self.painter.vline(
            gutter,
            card.top() + 6.0..=card.bottom() - 6.0,
            Stroke::new(1.0, theme::ink(18)),
        );
        let rows: [(f32, [(f32, Color32); 2]); 4] = [
            (
                0.0,
                [
                    (10.0, theme::syntax::keyword()),
                    (14.0, theme::syntax::function()),
                ],
            ),
            (
                1.0,
                [
                    (8.0, theme::syntax::variable()),
                    (15.0, theme::syntax::number()),
                ],
            ),
            (
                1.0,
                [
                    (12.0, theme::syntax::function()),
                    (9.0, theme::syntax::string()),
                ],
            ),
            (0.0, [(7.0, theme::syntax::keyword()), (10.0, ghost())]),
        ];
        for (index, (indent, parts)) in rows.into_iter().enumerate() {
            let y = card.top() + 11.0 + index as f32 * 10.0;
            self.line(pos2(card.left() + 7.0, y), 4.0, ghost().gamma_multiply(0.7));
            let mut x = gutter + 5.0 + indent * 6.0;
            for (length, color) in parts {
                self.line(pos2(x, y), length, color.gamma_multiply(0.9));
                x += length + 3.0;
            }
            if index == 3 {
                self.painter.vline(
                    x,
                    y - 4.0..=y + 4.0,
                    Stroke::new(1.5, theme::text_primary()),
                );
            }
        }
        let chip = Rect::from_center_size(card.right_bottom() + vec2(-3.0, -1.0), vec2(24.0, 15.0));
        self.chip(chip);
        self.glyph(Icon::Curlybraces, chip.center(), 9.0, Color32::WHITE);
    }

    /// A run of candles with a price ladder, and a ticker chip hanging off the corner.
    fn alpaca(&self) {
        let card = self.scene();
        let base = card.bottom() - 8.0;
        // Each candle's body as heights above the baseline, rising to the right.
        let candles = [
            (7.0, 14.0, true),
            (9.0, 15.0, false),
            (11.0, 21.0, true),
            (17.0, 27.0, true),
            (23.0, 33.0, true),
        ];
        for (index, (low, high, up)) in candles.into_iter().enumerate() {
            let x = card.left() + 10.0 + index as f32 * 8.6;
            let color = if up { kit::GREEN } else { kit::RED };
            self.painter.vline(
                x,
                base - high - 3.0..=base - low + 3.0,
                Stroke::new(1.1, color),
            );
            self.painter.rect_filled(
                Rect::from_min_max(pos2(x - 2.4, base - high), pos2(x + 2.4, base - low)),
                1.2,
                color,
            );
        }
        // The price ladder.
        for index in 0..3 {
            let y = card.top() + 12.0 + index as f32 * 14.0;
            self.painter.hline(
                card.right() - 9.0..=card.right() - 5.0,
                y,
                Stroke::new(1.4, ghost()),
            );
        }
        let chip = Rect::from_min_size(card.left_top() + vec2(-7.0, -5.0), vec2(30.0, 12.0));
        self.chip(chip);
        self.painter
            .circle_filled(chip.left_center() + vec2(6.0, 0.0), 2.3, Color32::WHITE);
        self.painter.hline(
            chip.left() + 11.0..=chip.right() - 6.0,
            chip.center().y,
            Stroke::new(2.0, Color32::WHITE.gamma_multiply(0.85)),
        );
    }

    /// Two order lines across a chart, a buy under the price and a sell above it, and a chip with
    /// the swap arrows.
    fn brokers(&self) {
        let card = self.scene();
        let sell = card.top() + 14.0;
        let buy = card.top() + 31.0;
        for (y, color) in [(sell, kit::RED), (buy, kit::GREEN)] {
            self.painter.hline(
                card.left() + 9.0..=card.right() - 22.0,
                y,
                Stroke::new(1.3, color),
            );
            let pill = Rect::from_center_size(pos2(card.right() - 14.0, y), vec2(14.0, 7.0));
            self.painter.rect_filled(pill, 3.0, color);
        }
        // The price between them.
        let mid = (sell + buy) / 2.0;
        self.painter.hline(
            card.left() + 9.0..=card.right() - 22.0,
            mid,
            Stroke::new(1.6, ghost()),
        );
        let chip = Rect::from_min_size(card.left_top() + vec2(-7.0, -5.0), vec2(30.0, 12.0));
        self.chip(chip);
        self.glyph(Icon::ArrowLeftRight, chip.center(), 9.0, Color32::WHITE);
    }

    /// A gauge from red to green with its needle, and a coin.
    fn coinmarketcap(&self) {
        let card = self.scene();
        let pivot = pos2(card.center().x, card.bottom() - 14.0);
        let pi = std::f32::consts::PI;
        for (index, color) in [kit::RED, kit::ORANGE, theme::YELLOW, kit::GREEN]
            .into_iter()
            .enumerate()
        {
            let from = pi + pi * index as f32 / 4.0 + 0.04;
            let to = pi + pi * (index + 1) as f32 / 4.0 - 0.04;
            self.arc(
                pivot,
                20.0,
                from,
                to,
                Stroke::new(5.0, color.gamma_multiply(0.9)),
            );
        }
        let angle = pi + pi * 0.72;
        let tip = pivot + vec2(angle.cos(), angle.sin()) * 15.0;
        self.painter
            .line_segment([pivot, tip], Stroke::new(2.0, theme::text_primary()));
        self.painter
            .circle_filled(pivot, 3.4, theme::text_primary());
        self.painter.circle_filled(pivot, 1.3, theme::chrome());
        // A coin on the corner.
        let coin = card.right_top() + vec2(-2.0, 2.0);
        self.painter.circle_filled(coin, 8.5, self.tint);
        self.painter.circle_stroke(
            coin,
            5.6,
            Stroke::new(1.2, Color32::WHITE.gamma_multiply(0.7)),
        );
        self.painter.vline(
            coin.x,
            coin.y - 2.8..=coin.y + 2.8,
            Stroke::new(1.5, Color32::WHITE),
        );
    }

    /// A prompt, the change it asked for as a diff, and sparkles.
    fn ai(&self) {
        let card = self.scene();
        let prompt = Rect::from_min_size(
            card.left_top() + vec2(7.0, 7.0),
            vec2(card.width() - 14.0, 12.0),
        );
        self.painter.rect_filled(prompt, 6.0, theme::well_fill());
        self.line(prompt.left_center() + vec2(6.0, 0.0), 22.0, ghost());
        self.painter.vline(
            prompt.left() + 32.0,
            prompt.center().y - 3.5..=prompt.center().y + 3.5,
            Stroke::new(1.5, self.tint),
        );
        for (index, (color, minus)) in [(kit::RED, true), (kit::GREEN, false)]
            .into_iter()
            .enumerate()
        {
            let row = Rect::from_min_size(
                card.left_top() + vec2(7.0, 24.0 + index as f32 * 12.0),
                vec2(card.width() - 14.0, 9.0),
            );
            self.painter
                .rect_filled(row, 3.0, color.gamma_multiply(0.18));
            let mark = row.left_center() + vec2(5.0, 0.0);
            self.painter
                .hline(mark.x - 2.0..=mark.x + 2.0, mark.y, Stroke::new(1.5, color));
            if !minus {
                self.painter
                    .vline(mark.x, mark.y - 2.0..=mark.y + 2.0, Stroke::new(1.5, color));
            }
            self.line(
                row.left_center() + vec2(13.0, 0.0),
                if minus { 24.0 } else { 30.0 },
                color.gamma_multiply(0.8),
            );
        }
        self.star(card.right_top() + vec2(0.0, 0.0), 8.5, self.tint);
        self.star(
            card.left_bottom() + vec2(-3.0, -2.0),
            4.8,
            self.tint.gamma_multiply(0.8),
        );
    }

    /// A banner with its badge, and the sound it makes.
    fn notifications(&self) {
        let card = self.scene();
        for index in 0..3 {
            self.line(
                card.left_top() + vec2(8.0, 40.0 + index as f32 * 4.0 - 2.0),
                [40.0, 28.0, 34.0][index],
                theme::ink(12),
            );
        }
        let banner = Rect::from_min_size(
            card.left_top() + vec2(5.0, 9.0),
            vec2(card.width() - 10.0, 22.0),
        );
        self.card_at(banner, theme::chrome());
        self.painter
            .rect_filled(banner.shrink(1.0), 8.0, theme::ink(5));
        self.painter
            .circle_filled(banner.left_center() + vec2(9.0, 0.0), 5.0, self.tint);
        self.glyph(
            Icon::Bell,
            banner.left_center() + vec2(9.0, 0.0),
            6.4,
            Color32::WHITE,
        );
        self.line(
            banner.left_center() + vec2(18.0, -3.5),
            19.0,
            theme::text_secondary().gamma_multiply(0.8),
        );
        self.line(banner.left_center() + vec2(18.0, 3.5), 12.0, ghost());
        let badge = banner.right_top() + vec2(-1.0, 1.0);
        self.painter.circle_filled(badge, 5.6, kit::RED);
        self.painter
            .circle_stroke(badge, 5.6, Stroke::new(1.5, theme::chrome()));
        self.painter.circle_filled(badge, 1.6, Color32::WHITE);
        // The sound.
        let origin = card.left_center() + vec2(-5.0, 4.0);
        let pi = std::f32::consts::PI;
        for (radius, alpha) in [(6.0, 0.95), (11.0, 0.55)] {
            self.arc(
                origin,
                radius,
                pi - 0.75,
                pi + 0.75,
                Stroke::new(2.0, self.tint.gamma_multiply(alpha)),
            );
        }
    }

    /// A message leaving a source for an endpoint, as a payload on a dashed path.
    fn webhooks(&self) {
        let card = self.scene();
        let y = card.center().y;
        let source = pos2(card.left() + 13.0, y);
        let endpoint = Rect::from_center_size(pos2(card.right() - 14.0, y), vec2(16.0, 20.0));
        self.painter.circle_filled(source, 6.5, self.tint);
        self.painter.circle_filled(source, 2.4, Color32::WHITE);
        self.painter.rect_filled(endpoint, 4.0, theme::well_fill());
        self.painter.rect_stroke(
            endpoint,
            4.0,
            Stroke::new(1.3, theme::text_secondary()),
            StrokeKind::Inside,
        );
        for offset in [-5.0, 0.0, 5.0] {
            self.painter.hline(
                endpoint.left() + 4.0..=endpoint.right() - 4.0,
                endpoint.center().y + offset,
                Stroke::new(1.3, ghost()),
            );
        }
        let from = source + vec2(9.0, 0.0);
        let to = pos2(endpoint.left() - 5.0, y);
        self.painter.extend(Shape::dashed_line(
            &[from, to],
            Stroke::new(1.6, self.tint.gamma_multiply(0.85)),
            3.0,
            2.5,
        ));
        self.arrow_head(to + vec2(1.5, 0.0), vec2(1.0, 0.0), 6.0, self.tint);
        let payload =
            Rect::from_center_size(pos2((from.x + to.x) / 2.0, y - 11.0), vec2(20.0, 12.0));
        self.chip(payload);
        self.glyph(Icon::Curlybraces, payload.center(), 8.0, Color32::WHITE);
    }

    /// A card in a viewfinder with its copy behind it, and the shutter.
    fn snapshots(&self) {
        let centre = self.centre - vec2(3.0, 1.0);
        self.disc(centre);
        let card = Rect::from_center_size(centre, CARD - vec2(6.0, 4.0));
        self.card_at(card.translate(vec2(7.0, 5.0)), theme::well_fill());
        self.card_at(card, theme::chrome());
        let points = [0.62, 0.5, 0.58, 0.38, 0.46, 0.22, 0.3];
        let area = card.shrink2(vec2(11.0, 10.0));
        let curve: Vec<Pos2> = points
            .iter()
            .enumerate()
            .map(|(index, level)| {
                pos2(
                    area.left() + area.width() * index as f32 / (points.len() - 1) as f32,
                    area.top() + area.height() * level,
                )
            })
            .collect();
        let mut fill = curve.clone();
        fill.push(pos2(area.right(), area.bottom()));
        fill.push(pos2(area.left(), area.bottom()));
        self.painter.add(Shape::convex_polygon(
            fill,
            self.tint.gamma_multiply(0.16),
            Stroke::NONE,
        ));
        self.painter
            .add(Shape::line(curve, Stroke::new(2.0, self.tint)));
        // The viewfinder.
        let frame = card.shrink(5.0);
        let reach = 6.0;
        let stroke = Stroke::new(1.8, theme::text_secondary());
        for (corner, dx, dy) in [
            (frame.left_top(), 1.0, 1.0),
            (frame.right_top(), -1.0, 1.0),
            (frame.left_bottom(), 1.0, -1.0),
            (frame.right_bottom(), -1.0, -1.0),
        ] {
            self.painter
                .line_segment([corner, corner + vec2(reach * dx, 0.0)], stroke);
            self.painter
                .line_segment([corner, corner + vec2(0.0, reach * dy)], stroke);
        }
        let shutter = card.right_bottom() + vec2(8.0, 4.0);
        self.painter.circle_filled(shutter, 8.0, self.tint);
        self.painter
            .circle_stroke(shutter, 5.2, Stroke::new(1.4, Color32::WHITE));
        self.painter.circle_filled(shutter, 2.4, Color32::WHITE);
    }

    /// A hub holding the server, with the tools it reaches.
    fn mcp(&self) {
        let card = self.scene();
        let hub = card.center() - vec2(0.0, 2.0);
        let nodes = [
            (-150.0_f32, Icon::GaugeHigh),
            (-30.0, Icon::Curlybraces),
            (90.0, Icon::BellBadge),
        ];
        for (degrees, icon) in nodes {
            let angle = degrees.to_radians();
            let at = hub + vec2(angle.cos(), angle.sin()) * 19.5;
            self.painter
                .line_segment([hub, at], Stroke::new(1.8, self.tint.gamma_multiply(0.7)));
            self.painter.circle_filled(at, 6.6, theme::chrome());
            self.painter
                .circle_stroke(at, 6.6, Stroke::new(1.4, theme::text_secondary()));
            self.glyph(icon, at, 8.0, theme::text_primary());
        }
        self.painter.circle_filled(hub, 8.5, self.tint);
        self.glyph(Icon::LockFill, hub, 9.0, Color32::WHITE);
    }

    /// Two ends of a sync around a cloud, and the arrows that go between them.
    fn sync(&self) {
        let card = self.scene();
        let middle = card.center();
        let cloud = self.tint;
        for (dx, dy, radius) in [(-7.5, 2.0, 5.6), (-0.5, -2.5, 7.4), (7.0, 1.5, 6.0)] {
            self.painter
                .circle_filled(middle + vec2(dx, dy), radius, cloud);
        }
        self.painter.rect_filled(
            Rect::from_min_max(middle + vec2(-7.5, 1.0), middle + vec2(7.0, 7.5)),
            CornerRadius::same(3),
            cloud,
        );
        let pi = std::f32::consts::PI;
        let stroke = Stroke::new(2.0, theme::text_secondary());
        for turn in [0.0, pi] {
            let from = turn + pi * 1.12;
            let to = turn + pi * 1.8;
            let points = self.arc(middle, 20.5, from, to, stroke);
            if let [.., before, last] = points.as_slice() {
                self.arrow_head(*last, *last - *before, 7.0, theme::text_secondary());
            }
        }
    }

    /// Stacked drives, and how full they are.
    fn storage(&self) {
        let card = self.scene();
        for index in 0..3 {
            let drive = Rect::from_min_size(
                card.left_top() + vec2(10.0, 7.0 + index as f32 * 11.0),
                vec2(card.width() - 20.0, 8.0),
            );
            self.painter.rect_filled(
                drive,
                4.0,
                self.tint.gamma_multiply([0.95, 0.7, 0.45][index]),
            );
            self.painter.circle_filled(
                drive.right_center() - vec2(5.0, 0.0),
                1.5,
                Color32::WHITE.gamma_multiply(0.9),
            );
            self.painter.hline(
                drive.left() + 5.0..=drive.left() + 17.0,
                drive.center().y,
                Stroke::new(1.6, Color32::WHITE.gamma_multiply(0.55)),
            );
        }
        let track = Rect::from_min_size(
            pos2(card.left() + 10.0, card.bottom() - 11.0),
            vec2(card.width() - 20.0, 4.5),
        );
        self.painter.rect_filled(track, 2.25, theme::well_fill());
        let used = Rect::from_min_size(track.min, vec2(track.width() * 0.62, track.height()));
        self.painter.rect_filled(used, 2.25, kit::GREEN);
        self.painter.rect_filled(
            Rect::from_min_max(pos2(used.right() - 8.0, used.top()), used.max),
            2.25,
            kit::ORANGE,
        );
    }

    /// The app icon.
    fn about(&self) {
        self.disc(self.centre);
        let icon = Rect::from_center_size(self.centre - vec2(0.0, 3.0), Vec2::splat(46.0));
        self.painter.add(
            egui::Shadow {
                offset: [0, 3],
                blur: 8,
                spread: 0,
                color: Color32::from_black_alpha(60),
            }
            .as_shape(icon, 11.0),
        );
        egui::Image::new(egui::include_image!("../../assets/brand/app_icon.png"))
            .fit_to_exact_size(icon.size())
            .corner_radius(11.0)
            .paint_at(self.ui, icon);
        let chip = Rect::from_center_size(icon.center_bottom() + vec2(0.0, 2.0), vec2(30.0, 11.0));
        self.chip(chip);
        self.line(
            chip.left_center() + vec2(6.0, 0.0),
            18.0,
            Color32::WHITE.gamma_multiply(0.85),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::theme::AppTheme;

    fn drawn(section: SettingsSection, app_theme: AppTheme) -> (Rect, Vec<Rect>) {
        let ctx = egui::Context::default();
        crate::ui::theme::install_offscreen(&ctx);
        crate::ui::theme::apply(&ctx, app_theme);
        let rect = Rect::from_min_size(pos2(40.0, 40.0), SIZE);
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(200.0, 160.0))),
                ..Default::default()
            },
            |ui| paint(ui, rect, section),
        );
        output.textures_delta.clear();
        let bounds = output
            .shapes
            .iter()
            .map(|clipped| clipped.shape.visual_bounding_rect())
            .filter(|bounds| bounds.is_finite() && bounds.width() > 0.0)
            .collect();
        (rect, bounds)
    }

    #[test]
    fn every_section_draws_a_picture_inside_its_box() {
        let _theme = crate::ui::theme::RestoreDefaultTheme;
        for section in SettingsSection::ALL {
            for app_theme in [AppTheme::Dark, AppTheme::Light] {
                let (rect, bounds) = drawn(section, app_theme);
                assert!(
                    bounds.len() >= 5,
                    "{section:?} drew {} shapes",
                    bounds.len()
                );
                for shape in bounds {
                    assert!(
                        rect.expand(9.0).contains_rect(shape),
                        "{section:?} {app_theme:?}: {shape:?} is outside {rect:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_pictures_differ_between_sections() {
        let _theme = crate::ui::theme::RestoreDefaultTheme;
        let sizes: Vec<usize> = SettingsSection::ALL
            .into_iter()
            .map(|section| drawn(section, AppTheme::Dark).1.len())
            .collect();
        assert!(
            sizes.iter().collect::<std::collections::HashSet<_>>().len() > 6,
            "{sizes:?}"
        );
    }

    /// Pictures for a person to look at. Run by hand:
    /// `DEGENVIEW_GALLERY_DIR=… cargo test --lib settings_art -- --ignored`.
    #[test]
    #[ignore = "writes images for a person to look at"]
    fn gallery_of_the_settings_pictures() {
        use crate::ui::render_tests::{Screen, gallery_dir};
        if gallery_dir().is_none() {
            return;
        }
        let _theme = crate::ui::theme::RestoreDefaultTheme;
        for (name, app_theme) in [("dark", AppTheme::Dark), ("light", AppTheme::Light)] {
            let columns = 4.0;
            let cell = vec2(130.0, 112.0);
            let mut screen = Screen::new(vec2(cell.x * columns, cell.y * 3.0));
            egui_extras::install_image_loaders(&screen.ctx);
            crate::ui::theme::apply(&screen.ctx, app_theme);
            let mut last = None;
            for _ in 0..12 {
                std::thread::sleep(std::time::Duration::from_millis(20));
                last = Some(screen.run(Vec::new(), |ui| {
                    let full = ui.max_rect();
                    ui.painter()
                        .rect_filled(full, 0.0, crate::ui::kit::window());
                    for (index, section) in SettingsSection::ALL.into_iter().enumerate() {
                        let column = (index % 4) as f32;
                        let row = (index / 4) as f32;
                        let rect = Rect::from_center_size(
                            full.min + vec2(cell.x * (column + 0.5), cell.y * (row + 0.5)),
                            SIZE,
                        );
                        paint(ui, rect, section);
                    }
                }));
            }
            screen.save(&format!("settings-art-{name}"), last.expect("a frame"));
        }
    }
}

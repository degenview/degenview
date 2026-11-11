//! What the Portfolio tab says while it has nothing to show: a picture for each page, in the
//! style of the Pine Studio's Logs, Problems and Profile introductions, and a smaller one for
//! a panel that is waiting on prices. Painter code over the shared [`Art`] helpers, so every
//! picture follows the theme.

use crate::ui::art::{Art, ghost};
use crate::ui::icons::Icon;
use crate::ui::pine::logs_view::{centered, intro, step};
use crate::ui::{kit, theme};
use eframe::egui::{
    self, Align2, Color32, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, pos2, vec2,
};

const COIN: Color32 = Color32::from_rgb(247, 147, 26);
const COIN_BLUE: Color32 = Color32::from_rgb(98, 126, 234);
const COIN_PURPLE: Color32 = Color32::from_rgb(191, 90, 242);

/// The pictures. The first five head a page, the rest fill a panel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Scene {
    /// No portfolio yet.
    Portfolios,
    Overview,
    Holdings,
    Transactions,
    Statistics,
    /// A donut waiting for priced holdings.
    Allocation,
    /// Bars around a zero line waiting for priced holdings.
    Results,
    /// A value curve waiting for history.
    History,
}

impl Scene {
    #[cfg(test)]
    pub const ALL: [Scene; 8] = [
        Scene::Portfolios,
        Scene::Overview,
        Scene::Holdings,
        Scene::Transactions,
        Scene::Statistics,
        Scene::Allocation,
        Scene::Results,
        Scene::History,
    ];

    fn tint(self) -> Color32 {
        match self {
            Scene::Portfolios | Scene::Overview | Scene::Allocation => kit::ACCENT,
            Scene::Holdings => COIN_PURPLE,
            Scene::Transactions => kit::ORANGE,
            Scene::Statistics | Scene::Results | Scene::History => kit::GREEN,
        }
    }
}

/// Draws `scene` in `rect` (an [`crate::ui::art::SIZE`] box).
pub fn paint(ui: &Ui, rect: Rect, scene: Scene) {
    let art = Art {
        ui,
        painter: ui.painter(),
        tint: scene.tint(),
        centre: rect.center(),
        flat: false,
    };
    match scene {
        Scene::Portfolios => art.pf_portfolios(),
        Scene::Overview => art.pf_overview(),
        Scene::Holdings => art.pf_holdings(),
        Scene::Transactions => art.pf_transactions(),
        Scene::Statistics | Scene::Results => art.pf_results(scene == Scene::Results),
        Scene::Allocation => art.pf_allocation(),
        Scene::History => art.pf_history(),
    }
}

impl Art<'_> {
    /// A pill over a card's corner holding a white glyph, in the scene's colour.
    fn pf_badge(&self, centre: Pos2, icon: Icon) {
        let rect = Rect::from_center_size(centre, vec2(28.0, 21.0));
        self.chip(rect);
        self.painter.rect_stroke(
            rect,
            rect.height() / 2.0,
            Stroke::new(1.5, theme::chrome()),
            StrokeKind::Outside,
        );
        self.glyph(icon, rect.center(), 12.0, Color32::WHITE);
    }

    /// A briefcase with the plus that makes the first portfolio.
    fn pf_portfolios(&self) {
        let card = self.scene();
        self.glyph(
            Icon::Briefcase,
            card.center() + vec2(0.0, 1.0),
            26.0,
            theme::text_secondary(),
        );
        self.pf_badge(card.right_top() + vec2(2.0, -2.0), Icon::Plus);
        self.star(
            self.centre + vec2(-44.0, -22.0),
            4.6,
            theme::text_tertiary(),
        );
        self.star(self.centre + vec2(43.0, 26.0), 3.4, theme::text_tertiary());
    }

    /// A donut and the three figures beside it, the Overview's own parts.
    fn pf_overview(&self) {
        let card = self.scene();
        let ring = pos2(card.left() + 22.0, card.center().y);
        self.pf_ring(ring, 11.5, 5.0, 1.0);
        let x = card.left() + 40.0;
        self.line(pos2(x, card.center().y - 9.0), 12.0, ghost());
        self.line(
            pos2(x, card.center().y),
            15.0,
            kit::GREEN.gamma_multiply(0.85),
        );
        self.line(pos2(x, card.center().y + 9.0), 9.0, ghost());
        self.pf_badge(card.right_top() + vec2(2.0, -2.0), Icon::PieChart);
    }

    /// A donut cut in three colours; `fade` dims it for a panel that has no data.
    fn pf_ring(&self, centre: Pos2, radius: f32, width: f32, strength: f32) {
        let start = -std::f32::consts::FRAC_PI_2;
        let colours = [kit::ACCENT, kit::ORANGE, kit::GREEN];
        let shares = [0.46, 0.30, 0.24];
        let gap = 0.16;
        let mut from = start;
        for (colour, share) in colours.into_iter().zip(shares) {
            let to = from + share * std::f32::consts::TAU;
            self.arc(
                centre,
                radius,
                from + gap / 2.0,
                to - gap / 2.0,
                Stroke::new(width, colour.gamma_multiply(strength)),
            );
            from = to;
        }
    }

    /// Three positions: a coin, its name and its size.
    fn pf_holdings(&self) {
        let card = self.scene();
        let coins = [COIN, COIN_BLUE, COIN_PURPLE];
        let sizes = [0.62, 0.40, 0.24];
        for (index, (colour, size)) in coins.into_iter().zip(sizes).enumerate() {
            let y = card.top() + 13.0 + index as f32 * 14.0;
            self.painter
                .circle_filled(pos2(card.left() + 12.0, y), 4.2, colour);
            self.line(pos2(card.left() + 21.0, y), 15.0, ghost());
            let bar = Rect::from_min_size(
                pos2(card.right() - 9.0 - 14.0 * size - 3.0, y - 2.0),
                vec2(14.0 * size + 3.0, 4.0),
            );
            self.painter
                .rect_filled(bar, 2.0, kit::ACCENT.gamma_multiply(0.7));
        }
        self.pf_badge(card.right_top() + vec2(2.0, -2.0), Icon::Layers);
    }

    /// A ledger: a buy, a sell, a buy, each with its amount.
    fn pf_transactions(&self) {
        let card = self.scene();
        let kinds = [(true, 0.5), (false, 0.34), (true, 0.42)];
        for (index, (buy, size)) in kinds.into_iter().enumerate() {
            let y = card.top() + 13.0 + index as f32 * 14.0;
            let colour = if buy { kit::GREEN } else { kit::RED };
            self.painter.circle_filled(
                pos2(card.left() + 12.0, y),
                5.0,
                colour.gamma_multiply(0.22),
            );
            self.glyph(
                if buy {
                    Icon::ArrowDownRight
                } else {
                    Icon::ArrowUpRight
                },
                pos2(card.left() + 12.0, y),
                7.0,
                colour,
            );
            self.line(pos2(card.left() + 22.0, y), 14.0, ghost());
            self.line(
                pos2(card.right() - 9.0 - 20.0 * size, y),
                20.0 * size,
                colour.gamma_multiply(0.7),
            );
        }
        self.pf_badge(card.right_top() + vec2(2.0, -2.0), Icon::Exchange);
    }

    /// Gains to the right of a zero line, losses to the left. `plain` draws it without the
    /// badge, as a panel's picture.
    fn pf_results(&self, plain: bool) {
        let card = self.scene();
        let zero = card.left() + 29.0;
        self.painter.vline(
            zero,
            card.top() + 7.0..=card.bottom() - 7.0,
            Stroke::new(1.0, theme::text_tertiary().gamma_multiply(0.7)),
        );
        let bars = [(0.62, true), (0.34, false), (0.86, true)];
        for (index, (length, gain)) in bars.into_iter().enumerate() {
            let y = card.top() + 13.0 + index as f32 * 14.0;
            let colour = if gain { kit::GREEN } else { kit::RED };
            let reach = 30.0 * length;
            let bar = if gain {
                Rect::from_min_size(pos2(zero, y - 3.0), vec2(reach, 6.0))
            } else {
                Rect::from_min_size(pos2(zero - reach * 0.8, y - 3.0), vec2(reach * 0.8, 6.0))
            };
            self.painter
                .rect_filled(bar, 2.0, colour.gamma_multiply(0.85));
        }
        if !plain {
            self.pf_badge(card.right_top() + vec2(2.0, -2.0), Icon::BarChart);
        }
    }

    /// A donut drawn faintly: where the allocation will go.
    fn pf_allocation(&self) {
        self.disc(self.centre);
        let card = egui::Rect::from_center_size(self.centre, crate::ui::art::CARD);
        self.card_at(card, theme::chrome());
        self.pf_ring(card.center(), 15.0, 6.0, 0.55);
        self.line(
            card.center() - vec2(5.0, 0.0),
            10.0,
            theme::text_tertiary().gamma_multiply(0.7),
        );
    }

    /// A value curve climbing over a dashed baseline: the history that is still to be built.
    fn pf_history(&self) {
        let card = self.scene();
        let area = card.shrink2(vec2(9.0, 11.0));
        self.painter.add(Shape::dashed_line(
            &[
                pos2(area.left(), area.bottom() - 4.0),
                pos2(area.right(), area.bottom() - 4.0),
            ],
            Stroke::new(1.0, theme::text_tertiary()),
            2.5,
            3.0,
        ));
        let rise = [0.0, 0.18, 0.1, 0.34, 0.28, 0.55, 0.5, 0.82];
        let points: Vec<Pos2> = rise
            .iter()
            .enumerate()
            .map(|(index, level)| {
                pos2(
                    area.left() + area.width() * index as f32 / (rise.len() - 1) as f32,
                    area.bottom() - 4.0 - (area.height() - 8.0) * level,
                )
            })
            .collect();
        let last = points[points.len() - 1];
        // The area under the curve, fading down to the baseline.
        let mut mesh = egui::Mesh::default();
        for point in &points {
            mesh.colored_vertex(*point, kit::GREEN.gamma_multiply(0.22));
            mesh.colored_vertex(
                pos2(point.x, area.bottom() - 4.0),
                kit::GREEN.gamma_multiply(0.02),
            );
        }
        for step in 0..points.len() as u32 - 1 {
            let base = step * 2;
            mesh.add_triangle(base, base + 1, base + 2);
            mesh.add_triangle(base + 1, base + 3, base + 2);
        }
        self.painter.add(Shape::mesh(mesh));
        self.painter
            .add(Shape::line(points, Stroke::new(2.0, kit::GREEN)));
        self.painter.circle_filled(last, 3.0, kit::GREEN);
        self.painter
            .circle_stroke(last, 3.0, Stroke::new(1.5, theme::chrome()));
    }
}

/// A picture, a title and one line of explanation in the middle of `area`: what a panel says
/// while it waits for data.
pub fn compact(ui: &Ui, area: Rect, scene: Scene, title: &str, detail: &str) {
    let heading = kit::galley(
        ui,
        title,
        kit::size::HEADLINE,
        kit::SEMIBOLD,
        theme::text_primary(),
        area.width() - 24.0,
    );
    let body = kit::galley(
        ui,
        detail,
        kit::size::SUBHEADLINE,
        kit::REGULAR,
        theme::text_secondary(),
        (area.width() - 32.0).min(320.0),
    );
    let picture = crate::ui::art::SIZE;
    let total = picture.y + 8.0 + heading.size().y + 4.0 + body.size().y;
    let top = area.center().y - total / 2.0;
    paint(
        ui,
        Rect::from_center_size(pos2(area.center().x, top + picture.y / 2.0), picture),
        scene,
    );
    let mut y = top + picture.y + 8.0;
    for galley in [heading, body] {
        ui.painter().galley(
            pos2(area.center().x - galley.size().x / 2.0, y),
            galley.clone(),
            Color32::PLACEHOLDER,
        );
        y += galley.size().y + 4.0;
    }
}

/// How a sample row starts.
enum Lead {
    Coin(Color32, &'static str),
    Trade(bool),
    Icon(Icon, Color32),
}

/// A row of the page the introduction is about, on a well: a leading mark, two lines of text
/// and two lines of figures.
fn sample(
    ui: &mut Ui,
    width: f32,
    lead: Lead,
    (title, subtitle): (&str, &str),
    (figure, caption, tone): (&str, &str, Color32),
) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, 50.0), Sense::hover());
    let painter = ui.painter().clone();
    painter.rect_filled(rect, 8.0, theme::well_fill());
    let mark = pos2(rect.left() + 24.0, rect.center().y);
    match lead {
        Lead::Coin(colour, letter) => {
            painter.circle_filled(mark, 12.0, colour);
            painter.text(
                mark,
                Align2::CENTER_CENTER,
                letter,
                egui::FontId::proportional(11.0),
                Color32::WHITE,
            );
        }
        Lead::Trade(buy) => {
            let colour = if buy { kit::GREEN } else { kit::RED };
            painter.circle_filled(mark, 12.0, colour.gamma_multiply(0.18));
            crate::ui::icons::paint(
                &painter,
                Rect::from_center_size(mark, vec2(13.0, 13.0)),
                if buy {
                    Icon::ArrowDownRight
                } else {
                    Icon::ArrowUpRight
                },
                colour,
            );
        }
        Lead::Icon(icon, colour) => {
            let badge = Rect::from_center_size(mark, vec2(26.0, 26.0));
            painter.rect_filled(badge, 7.0, colour.gamma_multiply(0.16));
            crate::ui::icons::paint(
                &painter,
                Rect::from_center_size(mark, vec2(14.0, 14.0)),
                icon,
                colour,
            );
        }
    }
    let right = rect.right() - 12.0;
    let figures = [
        kit::galley(
            ui,
            figure,
            kit::size::SUBHEADLINE,
            kit::SEMIBOLD,
            tone,
            120.0,
        ),
        kit::galley(
            ui,
            caption,
            kit::size::CAPTION,
            kit::REGULAR,
            theme::text_secondary(),
            120.0,
        ),
    ];
    let figures_width = figures[0].size().x.max(figures[1].size().x);
    let block = figures[0].size().y + 1.0 + figures[1].size().y;
    let mut y = rect.center().y - block / 2.0;
    for galley in figures {
        painter.galley(
            pos2(right - galley.size().x, y),
            galley.clone(),
            Color32::PLACEHOLDER,
        );
        y += galley.size().y + 1.0;
    }
    let text_left = rect.left() + 44.0;
    let room = (right - figures_width - 10.0 - text_left).max(20.0);
    let lines = [
        kit::galley(
            ui,
            title,
            kit::size::SUBHEADLINE,
            kit::SEMIBOLD,
            theme::text_primary(),
            room,
        ),
        kit::galley(
            ui,
            subtitle,
            kit::size::CAPTION,
            kit::REGULAR,
            theme::text_secondary(),
            room,
        ),
    ];
    let block = lines[0].size().y + 1.0 + lines[1].size().y;
    let mut y = rect.center().y - block / 2.0;
    for galley in lines {
        painter.galley(pos2(text_left, y), galley.clone(), Color32::PLACEHOLDER);
        y += galley.size().y + 1.0;
    }
}

fn muted(text: &str) -> egui::RichText {
    kit::text(
        text,
        kit::size::SUBHEADLINE,
        kit::REGULAR,
        theme::text_secondary(),
    )
}

/// The introduction to one page of a portfolio that has nothing in it yet: what the page is for,
/// three steps, an example row, and `actions` (the buttons that fill it) under the words.
pub fn page(ui: &mut Ui, scene: Scene, actions: impl FnOnce(&mut Ui)) {
    let (heading, subheading, steps): (&str, &str, [&str; 3]) = match scene {
        Scene::Portfolios => (
            "Track everything you hold",
            "Create a portfolio to get started.",
            [
                "Name it and pick the currency it reports in.",
                "Add transactions, or import a CSV or CoinMarketCap export.",
                "Value, profit and allocation follow the market.",
            ],
        ),
        Scene::Overview => (
            "Your portfolio at a glance",
            "Add a transaction to see what it is worth.",
            [
                "Add a buy or sell, or import your history.",
                "Prices stream in for every asset you hold.",
                "Value, profit and allocation fill this page.",
            ],
        ),
        Scene::Holdings => (
            "Every position in one table",
            "Nothing held yet.",
            [
                "Buy an asset, or import a ledger.",
                "Each holding lists its quantity, cost, value and profit.",
                "Double-click a row for its transactions.",
            ],
        ),
        Scene::Transactions => (
            "A ledger of every trade",
            "No transactions yet.",
            [
                "Add one by hand, or import many at once.",
                "Search and filter by asset, type or date.",
                "Edit a row to correct a price or a fee.",
            ],
        ),
        Scene::Statistics => (
            "How your portfolio has performed",
            "Statistics need a transaction to start from.",
            [
                "Add the buys and sells you have made.",
                "Cost basis, profit and fees are worked out for you.",
                "Each asset's result is charted against the rest.",
            ],
        ),
        Scene::Allocation | Scene::Results | Scene::History => {
            unreachable!("a panel picture has no page")
        }
    };
    centered(ui, |ui| {
        intro(
            ui,
            heading,
            subheading,
            |ui| {
                for (index, text) in steps.into_iter().enumerate() {
                    step(ui, index + 1, |ui| {
                        ui.label(muted(text));
                    });
                }
            },
            |ui, width| example(ui, width, scene),
            |ui, rect| paint(ui, rect, scene),
            |ui| {
                ui.add_space(16.0);
                ui.spacing_mut().item_spacing.y = 8.0;
                actions(ui);
            },
        );
    });
}

/// The row each page will hold, with made-up figures.
fn example(ui: &mut Ui, width: f32, scene: Scene) {
    let green = kit::GREEN;
    match scene {
        Scene::Portfolios => sample(
            ui,
            width,
            Lead::Icon(Icon::Briefcase, kit::ACCENT),
            ("Main", "3 assets"),
            ("$12,480.20", "USD", theme::text_primary()),
        ),
        Scene::Overview => sample(
            ui,
            width,
            Lead::Icon(Icon::PieChart, kit::ACCENT),
            ("Total value", "Live market value"),
            ("$12,480.20", "\u{25B2} 3.21%", green),
        ),
        Scene::Holdings => sample(
            ui,
            width,
            Lead::Coin(COIN, "B"),
            ("BTC", "Bitcoin"),
            ("$8,120.50", "65.1%", theme::text_primary()),
        ),
        Scene::Transactions => sample(
            ui,
            width,
            Lead::Trade(true),
            ("Buy BTC", "0.25 at $64,200"),
            ("$16,050.00", "Mar 4, 2026", theme::text_primary()),
        ),
        Scene::Statistics => sample(
            ui,
            width,
            Lead::Icon(Icon::Banknote, green),
            ("Cost Basis", "What you put in"),
            ("$9,860.00", "+$2,620.20", green),
        ),
        Scene::Allocation | Scene::Results | Scene::History => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::art::test_support;

    #[test]
    fn every_picture_draws_inside_its_box_in_both_themes() {
        test_support::assert_all_inside(&Scene::ALL, paint);
    }

    #[test]
    #[ignore = "writes PNGs for a person to look at"]
    fn gallery_pictures() {
        test_support::gallery("portfolio-empty", &Scene::ALL, paint);
    }
}

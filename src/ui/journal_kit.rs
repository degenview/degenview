//! The Trade Journal's look: one flat card, one pill, one chip strip, one set of toggles and a few
//! small marks, shared by every page so they read as one product.
//!
//! The rules are the app's Flat style (`docs/agent-guides/ui-style.md`): a faint fill and a
//! hairline at [`kit::CARD_RADIUS`], bare icons, colour only for state and data, no gradients and
//! no shadows. Colour comes through [`Sign`]: the chart colours (`theme::UP` / `DOWN`, as the
//! Portfolio uses them) for marks and fills, the softened tone for words, so both are legible in
//! the dark and the light theme. Nothing here knows a trade: pages hand it text and numbers.

use eframe::egui::{
    self, Align, Color32, CursorIcon, Frame, Layout, Margin, Mesh, Rect, Response, Sense, Shape,
    Stroke, StrokeKind, Ui, pos2, vec2,
};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use super::icons::{self, Icon};
use super::kit::{self, MEDIUM, REGULAR, SEMIBOLD, Tone, size};
use super::theme;

/// The gap between cards, in both directions.
pub const GAP: f32 = 12.0;
/// A card's inner margin (horizontal, vertical).
const MARGIN: (i8, i8) = (16, 14);

// MARK: Colour

/// Whether a figure gained, lost or is flat. The one place the journal decides its colours.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Sign {
    Gain,
    Loss,
    Flat,
}

impl Sign {
    pub fn of(value: f64) -> Self {
        if value > 0.0 {
            Self::Gain
        } else if value < 0.0 {
            Self::Loss
        } else {
            Self::Flat
        }
    }

    pub fn of_decimal(value: Decimal) -> Self {
        Self::of(value.to_f64().unwrap_or(0.0))
    }

    /// For a mark or a fill: a line, a bar, a ring, a dot.
    pub fn data(self) -> Color32 {
        match self {
            Self::Gain => theme::UP,
            Self::Loss => theme::DOWN,
            Self::Flat => theme::text_tertiary(),
        }
    }

    /// For words: the softened tone on a dark window, the deep one on a light window.
    pub fn text(self) -> Color32 {
        match self {
            Self::Gain => Tone::Good.text(),
            Self::Loss => Tone::Bad.text(),
            Self::Flat => theme::text_primary(),
        }
    }

    pub fn tone(self) -> Tone {
        match self {
            Self::Gain => Tone::Good,
            Self::Loss => Tone::Bad,
            Self::Flat => Tone::Neutral,
        }
    }
}

// MARK: Cards

/// What a card says about itself. Everything but the title is optional.
pub struct Card<'a> {
    pub title: &'a str,
    pub subtitle: Option<&'a str>,
    /// A definition shown on hover beside the title.
    pub info: Option<&'a str>,
    /// A state tint (the hero card takes its sign's colour).
    pub tint: Option<Sign>,
    pub min_height: Option<f32>,
}

impl<'a> Card<'a> {
    pub fn new(title: &'a str) -> Self {
        Self {
            title,
            subtitle: None,
            info: None,
            tint: None,
            min_height: None,
        }
    }

    pub fn subtitle(mut self, subtitle: &'a str) -> Self {
        self.subtitle = Some(subtitle);
        self
    }

    pub fn info(mut self, info: &'a str) -> Self {
        self.info = Some(info);
        self
    }

    pub fn min_height(mut self, height: f32) -> Self {
        self.min_height = Some(height);
        self
    }
}

/// The hairline round a card: a little lighter than egui's own widgets so a card is plainly an
/// object on either theme.
pub fn border(hovered: bool) -> Color32 {
    theme::ink(if hovered { 40 } else { 30 })
}

/// The flat frame every journal card uses.
pub fn card_frame(tint: Option<Sign>) -> Frame {
    let (fill, stroke) = match tint {
        // A state tint is the tone's own translucent wash and hairline.
        Some(sign) if sign != Sign::Flat => (sign.tone().fill(), sign.tone().edge()),
        _ => (theme::card_fill(false), border(false)),
    };
    Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, stroke))
        .corner_radius(kit::CARD_RADIUS)
        .inner_margin(Margin::symmetric(MARGIN.0, MARGIN.1))
}

/// A card across the available width. `right` fills the header's right end (toggles, a link);
/// `body` is the rest. The header is skipped when `spec.title` is empty.
pub fn card<R>(
    ui: &mut Ui,
    spec: &Card<'_>,
    right: impl FnOnce(&mut Ui),
    body: impl FnOnce(&mut Ui) -> R,
) -> R {
    let frame = card_frame(spec.tint);
    let width = ui.available_width() - frame.total_margin().sum().x;
    frame
        .show(ui, |ui| {
            ui.set_width(width.max(0.0));
            if let Some(height) = spec.min_height {
                ui.set_min_height((height - frame.total_margin().sum().y).max(0.0));
            }
            if !spec.title.is_empty() {
                header(ui, spec, right);
                ui.add_space(10.0);
            }
            body(ui)
        })
        .inner
}

fn header(ui: &mut Ui, spec: &Card<'_>, right: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.label(kit::text(
                    spec.title,
                    size::BODY + 1.0,
                    SEMIBOLD,
                    theme::text_primary(),
                ));
                if let Some(info) = spec.info {
                    info_mark(ui, info);
                }
            });
            if let Some(subtitle) = spec.subtitle {
                ui.label(kit::text(
                    subtitle,
                    size::CAPTION + 1.0,
                    REGULAR,
                    theme::text_secondary(),
                ));
            }
        });
        // Level with the title when there is no subtitle under it, else with the title's row.
        let align = if spec.subtitle.is_some() {
            Align::Min
        } else {
            Align::Center
        };
        ui.with_layout(Layout::right_to_left(align), right);
    });
}

/// A bare "i" that explains a figure on hover.
pub fn info_mark(ui: &mut Ui, text: &str) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
    let color = if response.hovered() {
        theme::text_primary()
    } else {
        theme::text_tertiary()
    };
    icons::paint(ui.painter(), rect, Icon::InfoCircle, color);
    response.on_hover_text(text)
}

/// What an editor says about its autosave: the error (in red, with the retry implied), "Saving",
/// or a "Saved" pill that the caller shows for a moment. Nothing when there is nothing to say.
pub fn save_status(ui: &mut Ui, error: Option<&str>, saving: bool, saved: bool) {
    if let Some(error) = error {
        ui.label(kit::text(
            error,
            size::CAPTION + 1.0,
            REGULAR,
            Sign::Loss.text(),
        ));
    } else if saving {
        ui.label(kit::text(
            "Saving\u{2026}",
            size::CAPTION + 1.0,
            REGULAR,
            theme::text_tertiary(),
        ));
    } else if saved {
        pill(ui, "Saved", Tone::Good);
    }
}

/// A hairline across the available width: between the rows of a table inside a card.
pub fn divider(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().hline(
        rect.x_range(),
        rect.center().y,
        Stroke::new(1.0, theme::ink(14)),
    );
}

/// A figure's small label above its value, as the KPI cards and the chips write it.
pub fn label(ui: &mut Ui, text: &str) -> Response {
    ui.label(kit::text(
        text,
        size::CAPTION + 1.0,
        MEDIUM,
        theme::text_secondary(),
    ))
}

/// The big number of a KPI card.
pub fn value(ui: &mut Ui, text: &str, color: Color32) -> Response {
    ui.label(kit::text(text, 26.0, SEMIBOLD, color))
}

/// A line of small text under a value.
pub fn note(ui: &mut Ui, text: impl Into<String>, color: Color32) -> Response {
    ui.label(kit::text(text, size::CAPTION + 1.0, REGULAR, color))
}

// MARK: Delta

/// "+5 vs prior period" with a bare arrow, in the colour of the change; nothing when either side
/// is missing. `current` and `previous` are in the figure's own unit; `decimals` is the precision
/// shown.
pub fn delta_text(current: f64, previous: f64, decimals: usize, unit: &str) -> (String, Sign) {
    let change = current - previous;
    let rounded = (change * 10f64.powi(decimals as i32)).round() / 10f64.powi(decimals as i32);
    let sign = Sign::of(rounded);
    (format!("{rounded:+.decimals$}{unit} vs prior period"), sign)
}

/// A delta line: a bare arrow (none when flat) and the words.
pub fn delta_line(ui: &mut Ui, text: &str, sign: Sign) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        let icon = match sign {
            Sign::Gain => Some(Icon::ArrowUp),
            Sign::Loss => Some(Icon::ArrowDown),
            Sign::Flat => None,
        };
        let color = match sign {
            Sign::Flat => theme::text_tertiary(),
            other => other.text(),
        };
        if let Some(icon) = icon {
            let (rect, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
            icons::paint(ui.painter(), rect, icon, color);
        }
        ui.label(kit::text(text, size::CAPTION + 1.0, MEDIUM, color));
    });
}

// MARK: Chips

/// One small figure in the strip under the headline cards.
pub struct Chip {
    pub icon: Icon,
    pub label: &'static str,
    pub value: String,
    pub sign: Sign,
    pub tip: &'static str,
}

const CHIP_HEIGHT: f32 = 54.0;
const CHIP_MIN_WIDTH: f32 = 128.0;

/// How many columns `n` equal cells of at least `min_width` take in `width`, so the rows come out
/// even (11 chips in two rows are 6 and 5, never 10 and 1).
pub fn balanced_columns(n: usize, width: f32, min_width: f32, gap: f32) -> usize {
    if n == 0 {
        return 1;
    }
    let fit = (((width + gap) / (min_width + gap)).floor() as usize).clamp(1, n);
    let rows = n.div_ceil(fit);
    n.div_ceil(rows)
}

/// The chips as a strip of small cards, a bare icon in each.
pub fn chip_strip(ui: &mut Ui, chips: &[Chip]) {
    if chips.is_empty() {
        return;
    }
    let width = ui.available_width();
    let columns = balanced_columns(chips.len(), width, CHIP_MIN_WIDTH, GAP / 2.0);
    for row in chips.chunks(columns) {
        // Each row fills the width, so a short last row is wider cells, not a gap.
        let cell = (width - GAP / 2.0 * (row.len() as f32 - 1.0)) / row.len() as f32;
        let (band, _) = ui.allocate_exact_size(vec2(width, CHIP_HEIGHT), Sense::hover());
        for (index, chip) in row.iter().enumerate() {
            let left = band.left() + index as f32 * (cell + GAP / 2.0);
            let rect = Rect::from_min_size(pos2(left, band.top()), vec2(cell, CHIP_HEIGHT));
            paint_chip(ui, rect, chip);
        }
        ui.add_space(GAP / 2.0);
    }
    ui.add_space(GAP / 2.0);
}

fn paint_chip(ui: &Ui, rect: Rect, chip: &Chip) {
    let painter = ui.painter();
    painter.rect_filled(rect, kit::CARD_RADIUS, theme::card_fill(false));
    painter.rect_stroke(
        rect,
        kit::CARD_RADIUS,
        Stroke::new(1.0, border(false)),
        StrokeKind::Inside,
    );
    let icon = Rect::from_center_size(pos2(rect.left() + 22.0, rect.center().y), vec2(16.0, 16.0));
    icons::paint(painter, icon, chip.icon, theme::text_secondary());
    let text_left = rect.left() + 40.0;
    let room = (rect.right() - 10.0 - text_left).max(10.0);
    let label = kit::galley(
        ui,
        chip.label,
        size::CAPTION + 0.5,
        REGULAR,
        theme::text_secondary(),
        room,
    );
    let value = kit::galley(
        ui,
        &chip.value,
        size::BODY + 1.0,
        SEMIBOLD,
        chip.sign.text(),
        room,
    );
    let block = label.size().y + 2.0 + value.size().y;
    let top = rect.center().y - block / 2.0;
    painter.galley(pos2(text_left, top), label, theme::text_secondary());
    painter.galley(
        pos2(text_left, top + block - value.size().y),
        value,
        chip.sign.text(),
    );
    ui.interact(rect, ui.id().with(("chip", chip.label)), Sense::hover())
        .on_hover_text(chip.tip);
}

// MARK: Pills, toggles, links

/// A tinted capsule with a hairline and no dot: LONG, SHORT, WIN, LOSS, OPEN, THIN.
pub fn pill(ui: &mut Ui, text: &str, tone: Tone) -> Response {
    let galley = kit::galley(
        ui,
        text,
        size::CAPTION + 0.5,
        SEMIBOLD,
        tone.text(),
        f32::INFINITY,
    );
    let size = vec2(galley.size().x + 16.0, galley.size().y + 6.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    let radius = size.y / 2.0;
    let painter = ui.painter();
    painter.rect_filled(rect, radius, tone.fill());
    painter.rect_stroke(
        rect,
        radius,
        Stroke::new(1.0, tone.edge()),
        StrokeKind::Inside,
    );
    painter.galley(rect.min + vec2(8.0, 3.0), galley, tone.text());
    response
}

/// Two labelled states of one flag as a segmented control ("USD | %", "Equity | Cumulative").
pub fn flag_segments(ui: &mut Ui, id: egui::Id, off: &str, on: &str, flag: &mut bool) {
    kit::segmented(ui, id, &[(false, off), (true, on)], flag);
}

const LINK_DROP: f32 = 3.0;

/// A link: gray, underlined, a pointing hand. Never blue.
pub fn link(ui: &mut Ui, text: &str) -> Response {
    let galley = kit::galley(
        ui,
        text,
        size::CAPTION + 1.0,
        MEDIUM,
        theme::text_secondary(),
        f32::INFINITY,
    );
    // A few points of room above the text, so in a card header it sits level with the title.
    let (rect, response) =
        ui.allocate_exact_size(galley.size() + vec2(0.0, LINK_DROP), Sense::click());
    let color = if response.hovered() {
        theme::text_primary()
    } else {
        theme::text_secondary()
    };
    let painter = ui.painter();
    let text = Rect::from_min_size(rect.min + vec2(0.0, LINK_DROP), galley.size());
    painter.line_segment(
        [text.left_bottom(), text.right_bottom()],
        Stroke::new(1.0, color),
    );
    painter.galley(text.min, galley, color);
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// The wash behind a list row: the selection tint, else a faint one under the pointer.
pub fn paint_row(ui: &Ui, rect: Rect, selected: bool, hovered: bool) {
    let fill = if selected {
        theme::selected_tint(hovered)
    } else if hovered {
        theme::ink(10)
    } else {
        return;
    };
    ui.painter().rect_filled(rect, 8.0, fill);
}

// MARK: Marks

/// At most `max` of `values`, evenly spaced, always keeping the first and the last.
pub fn downsample(values: &[f64], max: usize) -> Vec<f64> {
    if max < 2 || values.len() <= max {
        return values.to_vec();
    }
    (0..max)
        .map(|i| values[i * (values.len() - 1) / (max - 1)])
        .collect()
}

/// The area between a polyline and the horizontal `floor`, in one plain colour (no gradient).
pub fn area_fill(painter: &egui::Painter, points: &[egui::Pos2], floor: f32, color: Color32) {
    if points.len() < 2 {
        return;
    }
    let mut mesh = Mesh::default();
    for point in points {
        mesh.colored_vertex(*point, color);
        mesh.colored_vertex(pos2(point.x, floor), color);
    }
    for index in 0..points.len() as u32 - 1 {
        let base = index * 2;
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base + 1, base + 3, base + 2);
    }
    painter.add(Shape::mesh(mesh));
}

/// A small trend line with a flat area under it, `size` big. Nothing for fewer than two values.
pub fn sparkline(ui: &mut Ui, size: egui::Vec2, values: &[f64], sign: Sign) {
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let values = downsample(values, (size.x.max(2.0) as usize).min(160));
    if values.len() < 2 {
        return;
    }
    let low = values.iter().copied().fold(f64::INFINITY, f64::min);
    let high = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let span = (high - low).max(f64::EPSILON);
    let inner = rect.shrink2(vec2(1.0, 3.0));
    let points: Vec<egui::Pos2> = values
        .iter()
        .enumerate()
        .map(|(i, v)| {
            pos2(
                inner.left() + inner.width() * i as f32 / (values.len() - 1) as f32,
                inner.bottom() - inner.height() * ((v - low) / span) as f32,
            )
        })
        .collect();
    let painter = ui.painter_at(rect);
    let color = sign.data();
    area_fill(
        &painter,
        &points,
        rect.bottom(),
        theme::with_alpha(color, 0.10),
    );
    painter.add(Shape::line(points, Stroke::new(1.6, color)));
}

/// A solid pill with dark text at `anchor` (its right edge, vertically centred): the last value on
/// a chart.
pub fn paint_value_badge(
    ui: &Ui,
    painter: &egui::Painter,
    anchor: egui::Pos2,
    text: &str,
    fill: Color32,
) {
    let galley = kit::galley(
        ui,
        text,
        size::CAPTION + 0.5,
        SEMIBOLD,
        Color32::from_rgb(8, 14, 10),
        f32::INFINITY,
    );
    let size = vec2(galley.size().x + 12.0, galley.size().y + 6.0);
    let rect = Rect::from_min_size(pos2(anchor.x - size.x, anchor.y - size.y / 2.0), size);
    painter.rect_filled(rect, 5.0, fill);
    painter.galley(rect.min + vec2(6.0, 3.0), galley, Color32::BLACK);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eleven_chips_make_two_even_rows_until_they_all_fit_in_one() {
        assert_eq!(balanced_columns(11, 1368.0, 128.0, 6.0), 6);
        assert_eq!(balanced_columns(11, 1500.0, 128.0, 6.0), 11);
        assert_eq!(balanced_columns(11, 700.0, 128.0, 6.0), 4);
        assert_eq!(balanced_columns(11, 200.0, 128.0, 6.0), 1);
        assert_eq!(balanced_columns(0, 900.0, 128.0, 6.0), 1);
        // Rows are even: 11 over 4 columns is 3 rows, which 4 columns fill 4+4+3.
        let columns = balanced_columns(11, 700.0, 128.0, 6.0);
        assert!(11usize.div_ceil(columns) <= 3);
    }

    #[test]
    fn downsampling_keeps_the_ends_and_never_grows() {
        let values: Vec<f64> = (0..1000).map(f64::from).collect();
        let small = downsample(&values, 50);
        assert_eq!(small.len(), 50);
        assert_eq!(small[0], 0.0);
        assert_eq!(small[49], 999.0);
        assert_eq!(downsample(&values[..10], 50), values[..10].to_vec());
        assert_eq!(downsample(&values, 1), values);
    }

    #[test]
    fn a_delta_says_which_way_and_how_much() {
        let (text, sign) = delta_text(67.0, 62.0, 0, " pts");
        assert_eq!(text, "+5 pts vs prior period");
        assert_eq!(sign, Sign::Gain);
        let (text, sign) = delta_text(1.2, 1.5, 1, "");
        assert_eq!(text, "-0.3 vs prior period");
        assert_eq!(sign, Sign::Loss);
        // A change that rounds to nothing is flat, not a coloured "+0".
        let (text, sign) = delta_text(1.001, 1.0, 1, "");
        assert_eq!(text, "+0.0 vs prior period");
        assert_eq!(sign, Sign::Flat);
    }

    #[test]
    fn a_sign_follows_the_value_and_zero_is_flat() {
        assert_eq!(Sign::of(0.5), Sign::Gain);
        assert_eq!(Sign::of(-0.5), Sign::Loss);
        assert_eq!(Sign::of(0.0), Sign::Flat);
        assert_eq!(Sign::of_decimal(Decimal::new(-1, 2)), Sign::Loss);
        assert_eq!(Sign::of(f64::NAN), Sign::Flat);
    }

    #[test]
    fn a_tint_is_one_flat_colour() {
        let tinted = card_frame(Some(Sign::Gain));
        let plain = card_frame(None);
        assert_ne!(tinted.fill, plain.fill);
        assert_eq!(tinted.fill, Tone::Good.fill());
        assert_eq!(card_frame(Some(Sign::Flat)).fill, plain.fill);
    }

    fn run(content: impl FnMut(&mut Ui)) {
        let ctx = egui::Context::default();
        theme::install_offscreen(&ctx);
        let mut content = content;
        for _ in 0..2 {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 600.0))),
                    ..Default::default()
                },
                |ui| content(ui),
            );
            out.textures_delta.clear();
        }
    }

    #[test]
    fn every_piece_draws_in_both_themes() {
        for light in [false, true] {
            let chips: Vec<Chip> = (0..11)
                .map(|i| Chip {
                    icon: Icon::Clock,
                    label: "Label",
                    value: format!("+{i}.00 USD"),
                    sign: if i % 2 == 0 { Sign::Gain } else { Sign::Loss },
                    tip: "tip",
                })
                .collect();
            run(|ui| {
                ui.ctx().set_theme(if light {
                    egui::Theme::Light
                } else {
                    egui::Theme::Dark
                });
                theme::sync(ui.ctx());
                card(
                    ui,
                    &Card::new("Title").subtitle("Sub").info("What it is"),
                    |ui| {
                        let _ = link(ui, "View all");
                    },
                    |ui| {
                        sparkline(ui, vec2(120.0, 32.0), &[1.0, 3.0, 2.0, 5.0], Sign::Gain);
                        sparkline(ui, vec2(120.0, 32.0), &[1.0], Sign::Flat);
                        let _ = pill(ui, "LONG", Tone::Good);
                        delta_line(ui, "+5 vs prior period", Sign::Gain);
                    },
                );
                chip_strip(ui, &chips);
            });
        }
    }

    /// The journal is flat and never blue: no gradient mesh outside this module's flat fill, no
    /// lit card edge, no accent colour. A source scan, so a later page cannot drift back.
    #[test]
    fn the_journal_pages_stay_flat_and_never_blue() {
        let pages = [
            ("journal_analytics", include_str!("journal_analytics.rs")),
            ("journal_calendar", include_str!("journal_calendar.rs")),
            ("journal_charts", include_str!("journal_charts.rs")),
            ("journal_compare", include_str!("journal_compare.rs")),
            ("journal_daily", include_str!("journal_daily.rs")),
            ("journal_detail", include_str!("journal_detail.rs")),
            ("journal_import", include_str!("journal_import.rs")),
            ("journal_overview", include_str!("journal_overview.rs")),
            ("journal_playbooks", include_str!("journal_playbooks.rs")),
            ("journal_shots", include_str!("journal_shots.rs")),
            ("journal_table", include_str!("journal_table.rs")),
        ];
        for (name, source) in pages {
            let code = source.split("#[cfg(test)]").next().unwrap_or(source);
            for banned in ["Mesh", "paint_card_highlight", "ACCENT", "card_highlight"] {
                assert!(!code.contains(banned), "{name} uses {banned}");
            }
        }
    }
}

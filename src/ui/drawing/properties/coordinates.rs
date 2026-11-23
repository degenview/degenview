//! The Coordinates page: each anchor's time and price, and where a pinned table sits.

use super::rows::Page;
use super::{CONTROL_WIDTH, CoordinateText, DrawingProperties, MAX_LISTED};
use crate::format;
use crate::i18n::{tr, tr_n};
use crate::model::{Anchor, Geometry, Tool};
use crate::ui::chart::settings::style::{number_field, number_field_sized};
use crate::ui::icons::Icon;
use crate::ui::kit::date_picker;
use crate::ui::{kit, settings::card, theme};
use eframe::egui::{Id, Ui};

/// The time picker's width: room for "Oct 9, 2026 14:03" and the calendar glyph.
const TIME_FIELD_WIDTH: f32 = 150.0;

/// The price field has the stepper beside it, and the card's 14 pt gap, so it is narrower.
const PRICE_FIELD_WIDTH: f32 = CONTROL_WIDTH - 18.0 - 16.0 - 14.0;

/// The label of anchor `index` of a drawing by `tool`.
pub(super) fn anchor_label(tool: Tool, index: usize) -> String {
    let named: &[&str] = match tool {
        Tool::Rectangle
        | Tool::Ellipse
        | Tool::Arrow
        | Tool::ArrowMarker
        | Tool::TrendLine
        | Tool::Ray
        | Tool::InfoLine
        | Tool::ExtendedLine
        | Tool::TrendAngle
        | Tool::FibRetracement => &[tr!("Point 1"), tr!("Point 2")],
        Tool::FibExtension => &[
            tr!("Start of move"),
            tr!("End of move"),
            tr!("End of retracement"),
        ],
        Tool::FibChannel => &[tr!("Point 1"), tr!("Point 2"), tr!("Channel width")],
        Tool::FibTimeZone | Tool::FibSpeedFan | Tool::FibSpeedArcs => {
            &[tr!("Point 1"), tr!("Point 2")]
        }
        Tool::FibCircles => &[tr!("Point 1"), tr!("Point 2")],
        Tool::FibSpiral => &[tr!("Center"), tr!("Point on the spiral")],
        Tool::FibWedge => &[tr!("Apex"), tr!("Radius and first arm"), tr!("Second arm")],
        Tool::Pitchfan => &[tr!("Point 1"), tr!("Point 2"), tr!("Point 3")],
        Tool::GannBox | Tool::GannSquareFixed | Tool::GannSquare => {
            &[tr!("Point 1"), tr!("Point 2")]
        }
        Tool::GannFan => &[tr!("Origin"), tr!("Point on the 1x1 angle")],
        Tool::FibTrendTime => &[tr!("Point 1"), tr!("Point 2"), tr!("Point 3")],
        Tool::HorizontalLine => &[tr!("Price level")],
        Tool::HorizontalRay | Tool::Crossline => &[tr!("Anchor")],
        Tool::VerticalLine => &[tr!("Time")],
        Tool::ParallelChannel => &[tr!("Point 1"), tr!("Point 2"), tr!("Offset point")],
        Tool::FlatChannel => &[tr!("Point 1"), tr!("Point 2"), tr!("Level")],
        Tool::DisjointChannel => &[
            tr!("Point 1"),
            tr!("Point 2"),
            tr!("Point 3"),
            tr!("End of second edge"),
        ],
        Tool::RegressionTrend => &[tr!("Start bar"), tr!("End bar")],
        Tool::Pitchfork
        | Tool::SchiffPitchfork
        | Tool::ModifiedSchiffPitchfork
        | Tool::InsidePitchfork => &[tr!("Point 1"), tr!("Point 2"), tr!("Point 3")],
        Tool::RotatedRectangle => &[tr!("Point 1"), tr!("Point 2"), tr!("Width point")],
        Tool::Circle => &[tr!("Center"), tr!("Radius point")],
        Tool::Arc => &[tr!("Start"), tr!("End"), tr!("Curvature point")],
        Tool::Curve => &[tr!("Start"), tr!("End"), tr!("Control point")],
        Tool::DoubleCurve => &[
            tr!("Start"),
            tr!("End"),
            tr!("Control point 1"),
            tr!("Control point 2"),
        ],
        Tool::ArrowMarkUp | Tool::ArrowMarkDown => &[tr!("Anchor")],
        Tool::Text => &[tr!("Top-left corner")],
        Tool::Note | Tool::PriceNote | Tool::Callout => &[tr!("Anchor"), tr!("Label position")],
        Tool::Signpost => &[tr!("Anchor"), tr!("Label height")],
        Tool::Comment | Tool::PriceLabel | Tool::FlagMark | Tool::Pin => &[tr!("Anchor")],
        Tool::XabcdPattern | Tool::CypherPattern => {
            &[tr!("X"), tr!("A"), tr!("B"), tr!("C"), tr!("D")]
        }
        Tool::AbcdPattern | Tool::TrianglePattern => &[tr!("A"), tr!("B"), tr!("C"), tr!("D")],
        Tool::HeadAndShoulders => &[
            tr!("Start"),
            tr!("Left shoulder"),
            tr!("Left neckline"),
            tr!("Head"),
            tr!("Right neckline"),
            tr!("Right shoulder"),
            tr!("End"),
        ],
        Tool::ThreeDrivesPattern => &[
            tr!("Start"),
            tr!("Drive 1"),
            tr!("Retracement A"),
            tr!("Drive 2"),
            tr!("Retracement C"),
            tr!("Drive 3"),
        ],
        Tool::ElliottImpulse => &[
            tr!("Start"),
            tr!("Wave 1"),
            tr!("Wave 2"),
            tr!("Wave 3"),
            tr!("Wave 4"),
            tr!("Wave 5"),
        ],
        Tool::ElliottCorrection => &[tr!("Start"), tr!("Wave A"), tr!("Wave B"), tr!("Wave C")],
        Tool::ElliottTriangle => &[
            tr!("Start"),
            tr!("Wave A"),
            tr!("Wave B"),
            tr!("Wave C"),
            tr!("Wave D"),
            tr!("Wave E"),
        ],
        Tool::ElliottDoubleCombo => &[tr!("Start"), tr!("Wave W"), tr!("Wave X"), tr!("Wave Y")],
        Tool::ElliottTripleCombo => &[
            tr!("Start"),
            tr!("Wave W"),
            tr!("Wave X"),
            tr!("Wave Y"),
            tr!("Second wave X"),
            tr!("Wave Z"),
        ],
        Tool::LongPosition | Tool::ShortPosition => &[tr!("Entry"), tr!("Target"), tr!("Stop")],
        Tool::PositionForecast => &[tr!("Source"), tr!("Target")],
        Tool::BarsPattern => &[tr!("First bar's close")],
        Tool::GhostFeed => &[tr!("Start"), tr!("End")],
        Tool::Sector => &[tr!("Center"), tr!("Radius point"), tr!("End of arc")],
        Tool::AnchoredVwap | Tool::AnchoredVolumeProfile => &[tr!("Start bar")],
        Tool::FixedRangeVolumeProfile => &[tr!("Left edge"), tr!("Right edge")],
        Tool::PriceRange | Tool::DateRange | Tool::DateAndPriceRange => {
            &[tr!("Point 1"), tr!("Point 2")]
        }
        Tool::CyclicLines => &[tr!("First line"), tr!("Second line")],
        Tool::TimeCycles => &[tr!("Start"), tr!("End of first cycle")],
        Tool::SineLine => &[tr!("First extreme"), tr!("Next extreme")],
        _ => &[],
    };
    named.get(index).map_or_else(
        || tr!("Point {number}", number = index + 1),
        |name| (*name).to_owned(),
    )
}

impl DrawingProperties {
    /// Where a table pinned to the screen is, as percentages of the plot. A table at a chart point
    /// is placed by its anchor like any other drawing; pinning and unpinning are on the chart,
    /// which knows where the corner is.
    fn pinned_table_position(&mut self, ui: &mut Ui, page: &mut Page) {
        let Some(Geometry::Table(table)) = self.draft.geometry() else {
            return;
        };
        let Some(place) = table.screen else {
            return;
        };
        page.section(
            ui,
            tr!("Position on screen"),
            Some(tr!("The table stays here however the chart is scrolled.")),
        );
        let mut edited = place;
        for (title, id, value) in [
            (tr!("Horizontal"), "table-pin-x", &mut edited.x),
            (tr!("Vertical"), "table-pin-y", &mut edited.y),
        ] {
            if let Some(percent) = card::card(
                ui,
                Icon::ArrowLeftRight,
                title,
                Some(tr!(
                    "Percent of the chart's width or height, from its top-left corner."
                )),
                |ui| {
                    number_field(
                        ui,
                        Id::new((id, self.draft.id())),
                        f64::from(*value) * 100.0,
                        1.0,
                        0.0..=100.0,
                        1,
                    )
                },
            ) {
                *value = (percent / 100.0) as f32;
            }
        }
        if edited != place
            && let Some(Geometry::Table(table)) = self.draft.geometry_mut()
        {
            table.screen = Some(edited.clamped());
        }
    }

    pub(super) fn coordinates_page(&mut self, ui: &mut Ui) {
        let mut page = Page::new(true);
        self.pinned_table_position(ui, &mut page);
        let anchors = self.draft.anchors();
        let shown = anchors.len().min(MAX_LISTED);
        for (index, &anchor) in anchors.iter().enumerate().take(shown) {
            page.section(ui, &anchor_label(self.tool, index), None);
            let mut edited = anchor;
            // Some anchors are read for one of their two numbers only (a level has no time, a
            // regression no price): the other is where a handle was left and is not offered.
            let (reads_time, reads_price) = self
                .draft
                .geometry()
                .map_or((true, true), |geometry| geometry.reads(index));
            let decimals = price_decimals(anchor.price);
            let step = 10f64.powi(-(decimals as i32));
            let price_id = Id::new(("drawing-price", self.draft.id(), index));
            if reads_price
                && let Some(price) = card::card(ui, Icon::Tag, tr!("Price"), None, |ui| {
                    number_field_sized(
                        ui,
                        price_id,
                        anchor.price,
                        step,
                        -1e12..=1e12,
                        decimals,
                        PRICE_FIELD_WIDTH,
                    )
                })
            {
                edited.price = price;
            }
            let time_id = self.time_id(index);
            let state = &mut self.coordinates[index];
            if reads_time && let Some(time) = time_row(ui, time_id, state) {
                edited.time_ms = time;
            }
            if edited.time_ms != anchor.time_ms || edited.price != anchor.price {
                self.move_anchor(index, edited);
            }
        }
        if anchors.len() > shown {
            ui.label(kit::text(
                tr_n!(
                    anchors.len() - shown,
                    "{count} more point: drag it on the chart.",
                    "{count} more points: drag them on the chart."
                ),
                kit::size::SUBHEADLINE,
                kit::REGULAR,
                theme::text_secondary(),
            ));
        }
    }

    pub(super) fn move_anchor(&mut self, index: usize, anchor: Anchor) {
        let mut candidate = self.draft.clone();
        if let Some(slot) = candidate.anchors_mut().into_iter().nth(index) {
            *slot = anchor;
        }
        self.draft = candidate;
        if let Some(text) = self.coordinates.get_mut(index) {
            text.time = format::timestamp(anchor.time_ms);
        }
    }
}

/// The decimals a price is edited with: enough that a typical tick is visible.
fn price_decimals(price: f64) -> usize {
    let magnitude = price.abs();
    if magnitude >= 1000.0 {
        2
    } else if magnitude >= 1.0 {
        4
    } else if magnitude >= 0.01 {
        6
    } else {
        8
    }
}

/// The time of one anchor, UTC: a field that opens a calendar and a time stepper. The text only
/// changes when the user picks, so an anchor nobody touched keeps its exact time.
fn time_row(ui: &mut Ui, id: Id, state: &mut CoordinateText) -> Option<i64> {
    card::card_reserving(
        ui,
        Icon::Clock,
        tr!("Time (UTC)"),
        None,
        TIME_FIELD_WIDTH,
        |ui| {
            date_picker::show(
                ui,
                id,
                &mut state.time,
                0,
                TIME_FIELD_WIDTH,
                date_picker::Look::Compact,
                false,
            )
            .then(|| format::parse_timestamp(&state.time))
            .flatten()
        },
    )
}

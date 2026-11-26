//! Matching names from somewhere else to markets: the band that says how many have one, and a row
//! per name with what it maps to and the actions that state allows. The CoinMarketCap and
//! TradingView imports draw both, so they look and behave the same.

use crate::i18n::{tr, tr_n};
use crate::model::DataSource;
use crate::ui::icons::{self, Icon};
use crate::ui::kit::loading::LoadingIndicator;
use crate::ui::kit::sheet_parts::{card_background, pill};
use crate::ui::kit::{self, source_logo};
use crate::ui::theme;
use eframe::egui::{
    self, Color32, CursorIcon, Id, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2,
};

const BAND_HEIGHT: f32 = 76.0;
const ROW_HEIGHT: f32 = 60.0;
/// Where a row's market starts, from its left edge, when its name is short.
pub const TARGET_LEFT: f32 = 196.0;

// MARK: Tally

/// How the names stand: the one summary the band draws.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Tally {
    pub total: usize,
    /// Matched to a market.
    pub matched: usize,
    /// Left out on purpose.
    pub skipped: usize,
    /// Still being searched for.
    pub resolving: usize,
    /// Neither matched nor skipped: left out unless the user matches them.
    pub unmatched: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TallyKind {
    Matched,
    Searching,
    Unmatched,
    Skipped,
}

impl TallyKind {
    pub fn color(self) -> Color32 {
        match self {
            Self::Matched => theme::UP,
            Self::Searching => theme::text_secondary(),
            Self::Unmatched => theme::REPLAY,
            Self::Skipped => theme::text_tertiary(),
        }
    }
}

/// The big figure of the band: how many of the names have a market.
pub fn matched_title(tally: &Tally) -> String {
    tr!(
        "{matched} of {total}",
        matched = tally.matched,
        total = tally.total
    )
}

/// The legend under the bar; only the groups that have names.
pub fn tally_legend(tally: &Tally) -> Vec<(TallyKind, String)> {
    [
        (TallyKind::Matched, tally.matched),
        (TallyKind::Searching, tally.resolving),
        (TallyKind::Unmatched, tally.unmatched),
        (TallyKind::Skipped, tally.skipped),
    ]
    .into_iter()
    .filter(|(_, count)| *count > 0)
    .map(|(kind, count)| {
        let text = match kind {
            TallyKind::Matched => tr_n!(count, "{count} matched", "{count} matched"),
            TallyKind::Searching => tr!("{count} searching", count = count),
            TallyKind::Unmatched => tr!("{count} to match", count = count),
            TallyKind::Skipped => tr_n!(count, "{count} skipped", "{count} skipped"),
        };
        (kind, text)
    })
    .collect()
}

/// How many names have a market, as a figure over `caption` and one bar split by state, with its
/// legend.
pub fn band(ui: &mut Ui, tally: &Tally, caption: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), BAND_HEIGHT), Sense::hover());
    card_background(ui, rect, None);
    let inner = rect.shrink2(vec2(18.0, 14.0));
    let primary = theme::text_primary();
    let secondary = theme::text_secondary();

    let figure = kit::galley(
        ui,
        &matched_title(tally),
        22.0,
        kit::SEMIBOLD,
        primary,
        140.0,
    );
    let caption = kit::galley(
        ui,
        caption,
        kit::size::CAPTION + 0.5,
        kit::REGULAR,
        secondary,
        140.0,
    );
    let block = figure.size().y + 1.0 + caption.size().y;
    let top = rect.center().y - block / 2.0;
    let figure_width = figure.size().x.max(caption.size().x);
    ui.painter().galley(
        pos2(inner.left(), top),
        figure.clone(),
        Color32::PLACEHOLDER,
    );
    ui.painter().galley(
        pos2(inner.left(), top + figure.size().y + 1.0),
        caption,
        Color32::PLACEHOLDER,
    );

    // The bar and its legend, to the right of a hairline.
    let left = inner.left() + figure_width.max(96.0) + 22.0;
    ui.painter().vline(
        left - 12.0,
        rect.top() + 16.0..=rect.bottom() - 16.0,
        Stroke::new(1.0, theme::separator().gamma_multiply(0.7)),
    );
    let bar = Rect::from_min_size(
        pos2(left, inner.top() + 8.0),
        vec2(inner.right() - left, 8.0),
    );
    ui.painter().rect_filled(bar, 4.0, theme::quaternary(1.0));
    if tally.total > 0 {
        let mut x = bar.left();
        let segments = [
            (TallyKind::Matched, tally.matched),
            (TallyKind::Searching, tally.resolving),
            (TallyKind::Unmatched, tally.unmatched),
            (TallyKind::Skipped, tally.skipped),
        ];
        let visible: Vec<_> = segments.iter().filter(|(_, count)| *count > 0).collect();
        let gap = 2.0;
        let usable = bar.width() - gap * visible.len().saturating_sub(1) as f32;
        for (kind, count) in visible {
            let width = usable * *count as f32 / tally.total as f32;
            let piece = Rect::from_min_size(pos2(x, bar.top()), vec2(width, bar.height()));
            ui.painter().rect_filled(piece, 4.0, kind.color());
            x += width + gap;
        }
    }
    let mut x = left;
    let legend_y = bar.bottom() + 14.0;
    for (kind, label) in tally_legend(tally) {
        ui.painter()
            .circle_filled(pos2(x + 4.0, legend_y), 3.5, kind.color());
        let text = kit::galley(
            ui,
            &label,
            kit::size::CAPTION + 0.5,
            kit::REGULAR,
            secondary,
            200.0,
        );
        ui.painter().galley(
            pos2(x + 14.0, legend_y - text.size().y / 2.0),
            text.clone(),
            Color32::PLACEHOLDER,
        );
        x += 14.0 + text.size().x + 18.0;
    }
}

// MARK: Heading

/// A section's title with an optional status at its trailing edge, and a line of explanation.
pub fn section_heading(ui: &mut Ui, title: &str, detail: &str, busy: Option<&str>) {
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::hover());
    let heading = kit::galley(
        ui,
        title,
        kit::size::HEADLINE + 2.0,
        kit::SEMIBOLD,
        theme::text_primary(),
        row.width(),
    );
    ui.painter().galley(
        pos2(row.left(), row.center().y - heading.size().y / 2.0),
        heading,
        Color32::PLACEHOLDER,
    );
    if let Some(busy) = busy {
        let label = kit::galley(
            ui,
            busy,
            kit::size::CAPTION + 0.5,
            kit::REGULAR,
            theme::text_secondary(),
            300.0,
        );
        let left = row.right() - label.size().x;
        ui.painter().galley(
            pos2(left, row.center().y - label.size().y / 2.0),
            label,
            Color32::PLACEHOLDER,
        );
        LoadingIndicator::new().size(13.0).paint_at(
            ui,
            Rect::from_center_size(pos2(left - 14.0, row.center().y), Vec2::splat(13.0)),
        );
    }
    ui.add_space(3.0);
    ui.add(
        egui::Label::new(kit::text(
            detail,
            kit::size::SUBHEADLINE,
            kit::REGULAR,
            theme::text_secondary(),
        ))
        .wrap(),
    );
    ui.add_space(12.0);
}

// MARK: Row

#[derive(Clone, Copy, PartialEq)]
enum ButtonStyle {
    /// The step that moves the row forward.
    Prominent,
    Bordered,
    /// A quiet text button.
    Quiet,
}

/// A 26 pt button ending at `right`, centred on `middle`; returns its left edge and whether it
/// was clicked. Painted and hit-tested by hand so a row stays one fixed-height block.
fn row_button(
    ui: &Ui,
    id: Id,
    right: f32,
    middle: f32,
    label: &str,
    style: ButtonStyle,
) -> (f32, bool) {
    let color = match style {
        // The filled button is the inverse of the sheet: light on dark, dark on light.
        ButtonStyle::Prominent => theme::chrome(),
        ButtonStyle::Bordered => theme::text_primary(),
        ButtonStyle::Quiet => theme::text_secondary(),
    };
    let text = kit::galley(
        ui,
        label,
        kit::size::SUBHEADLINE + 1.0,
        kit::MEDIUM,
        color,
        200.0,
    );
    let rect = Rect::from_min_max(
        pos2(right - text.size().x - 24.0, middle - 13.0),
        pos2(right, middle + 13.0),
    );
    let response = ui
        .interact(rect, id, Sense::click())
        .on_hover_cursor(CursorIcon::PointingHand);
    let response = crate::ui::accessibility::label(response, label);
    let hovered = response.hovered();
    match style {
        ButtonStyle::Prominent => {
            let fill = theme::text_primary();
            ui.painter().rect_filled(
                rect,
                7.0,
                if hovered {
                    fill.gamma_multiply(0.85)
                } else {
                    fill
                },
            );
        }
        ButtonStyle::Bordered => {
            ui.painter().rect_filled(
                rect,
                7.0,
                theme::quaternary(if hovered { 0.9 } else { 0.6 }),
            );
            ui.painter().rect_stroke(
                rect,
                7.0,
                Stroke::new(1.0, theme::separator()),
                StrokeKind::Inside,
            );
        }
        ButtonStyle::Quiet => {
            if hovered {
                ui.painter().rect_filled(rect, 7.0, theme::quaternary(0.6));
            }
        }
    }
    let color = if style == ButtonStyle::Quiet && hovered {
        theme::text_primary()
    } else {
        color
    };
    ui.painter().galley_with_override_text_color(
        pos2(
            rect.center().x - text.size().x / 2.0,
            rect.center().y - text.size().y / 2.0,
        ),
        text,
        color,
    );
    (rect.left(), response.clicked())
}

/// A row's bordered button on its own, for the rows under a mapping list that act the same way:
/// it ends at `right`, centred on `middle`. Returns its left edge and whether it was clicked.
pub fn bordered_button(ui: &Ui, id: Id, right: f32, middle: f32, label: &str) -> (f32, bool) {
    row_button(ui, id, right, middle, label, ButtonStyle::Bordered)
}

/// A name's initials on a neutral disc, for one with no market yet.
pub fn placeholder_icon(ui: &Ui, rect: Rect, name: &str) {
    ui.painter()
        .circle_filled(rect.center(), rect.width() / 2.0, theme::quaternary(1.0));
    let initials: String = name.chars().take(2).collect();
    let text = kit::galley(
        ui,
        &initials.to_uppercase(),
        13.0,
        kit::SEMIBOLD,
        theme::text_secondary(),
        rect.width(),
    );
    ui.painter().galley(
        rect.center() - text.size() / 2.0,
        text,
        Color32::PLACEHOLDER,
    );
}

/// What a row maps to.
pub enum RowState<'a> {
    /// Being searched for.
    Searching(&'a str),
    Matched {
        source: DataSource,
        ticker: &'a str,
        detail: &'a str,
        /// How it was matched, and its colour.
        badge: Option<(&'a str, Color32)>,
    },
    Skipped {
        note: &'a str,
    },
    Unmatched {
        note: &'a str,
    },
}

/// What was clicked on a row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RowAction {
    /// Pick a market (or another one).
    Match,
    Skip,
    /// Take the choice back.
    Reset,
}

pub struct Row<'a> {
    /// The name as it was exported.
    pub name: &'a str,
    /// What uses it.
    pub caption: &'a str,
    pub state: RowState<'a>,
    /// Where the market starts, from the row's left edge: [`TARGET_LEFT`], or more for long
    /// names.
    pub target_left: f32,
}

/// One name and its market, with the actions the state allows. `icon` paints the name's picture.
pub fn row(
    ui: &mut Ui,
    id: Id,
    row: &Row<'_>,
    icon: impl FnOnce(&mut Ui, Rect),
) -> Option<RowAction> {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::hover());
    let outline =
        matches!(row.state, RowState::Unmatched { .. }).then(|| theme::REPLAY.gamma_multiply(0.45));
    card_background(ui, rect, outline);
    let middle = rect.center().y;
    let primary = theme::text_primary();
    let secondary = theme::text_secondary();

    let picture = Rect::from_center_size(pos2(rect.left() + 32.0, middle), Vec2::splat(36.0));
    icon(ui, picture);
    // The name column ends short of the arrow before the market.
    let name_room = row.target_left - 96.0;
    let name = kit::galley(
        ui,
        row.name,
        kit::size::BODY + 1.0,
        kit::SEMIBOLD,
        primary,
        name_room,
    );
    let caption = kit::galley(
        ui,
        row.caption,
        kit::size::CAPTION + 0.5,
        kit::REGULAR,
        secondary,
        name_room,
    );
    let stack = name.size().y + 2.0 + caption.size().y;
    let top = middle - stack / 2.0;
    let text_left = picture.right() + 12.0;
    ui.painter()
        .galley(pos2(text_left, top), name.clone(), Color32::PLACEHOLDER);
    ui.painter().galley(
        pos2(text_left, top + name.size().y + 2.0),
        caption,
        Color32::PLACEHOLDER,
    );
    icons::paint(
        ui.painter(),
        Rect::from_center_size(
            pos2(rect.left() + row.target_left - 22.0, middle),
            Vec2::splat(11.0),
        ),
        Icon::ChevronRight,
        theme::text_tertiary(),
    );

    // The actions, from the trailing edge in.
    let buttons: &[(RowAction, &str, ButtonStyle)] = match row.state {
        RowState::Searching(_) => &[],
        RowState::Matched { .. } => &[
            (
                RowAction::Match,
                tr!("Change\u{2026}"),
                ButtonStyle::Bordered,
            ),
            (RowAction::Reset, tr!("Reset"), ButtonStyle::Quiet),
        ],
        RowState::Skipped { .. } => &[
            (
                RowAction::Match,
                tr!("Match\u{2026}"),
                ButtonStyle::Bordered,
            ),
            (RowAction::Reset, tr!("Reset"), ButtonStyle::Quiet),
        ],
        RowState::Unmatched { .. } => &[
            (
                RowAction::Match,
                tr!("Match\u{2026}"),
                ButtonStyle::Prominent,
            ),
            (RowAction::Skip, tr!("Skip"), ButtonStyle::Quiet),
        ],
    };
    let mut clicked = None;
    let mut right = rect.right() - 12.0;
    for (index, (action, label, style)) in buttons.iter().enumerate() {
        let end = right - if index > 0 { 6.0 } else { 0.0 };
        let (left, was_clicked) = row_button(ui, id.with(index), end, middle, label, *style);
        right = left;
        if was_clicked {
            clicked = Some(*action);
        }
    }

    // What the name maps to, between the arrow and the actions.
    let target = rect.left() + row.target_left;
    let room = (right - 14.0 - target).max(60.0);
    let line = |ui: &Ui, text: &str, top: f32| {
        let line = kit::galley(
            ui,
            text,
            kit::size::CAPTION + 0.5,
            kit::REGULAR,
            secondary,
            room,
        );
        ui.painter().galley(
            pos2(target, top - line.size().y / 2.0),
            line,
            Color32::PLACEHOLDER,
        );
    };
    match &row.state {
        RowState::Searching(label) => {
            LoadingIndicator::new().size(14.0).paint_at(
                ui,
                Rect::from_center_size(pos2(target + 7.0, middle), Vec2::splat(14.0)),
            );
            let label = kit::galley(
                ui,
                label,
                kit::size::BODY,
                kit::REGULAR,
                secondary,
                room - 24.0,
            );
            ui.painter().galley(
                pos2(target + 24.0, middle - label.size().y / 2.0),
                label,
                Color32::PLACEHOLDER,
            );
        }
        RowState::Matched {
            source,
            ticker,
            detail,
            badge,
        } => {
            let logo = Rect::from_center_size(pos2(target + 11.0, middle), Vec2::splat(22.0));
            source_logo::paint(ui, *source, logo);
            let badge_room = badge.map_or(0.0, |(label, _)| {
                kit::galley(
                    ui,
                    label,
                    kit::size::CAPTION,
                    kit::SEMIBOLD,
                    secondary,
                    200.0,
                )
                .size()
                .x + 22.0
            });
            let ticker = kit::galley(
                ui,
                ticker,
                kit::size::BODY,
                kit::MEDIUM,
                primary,
                (room - 34.0 - badge_room).max(60.0),
            );
            let detail = kit::galley(
                ui,
                detail,
                kit::size::CAPTION + 0.5,
                kit::REGULAR,
                secondary,
                room - 34.0,
            );
            let stack = ticker.size().y + 2.0 + detail.size().y;
            let top = middle - stack / 2.0;
            let ticker_right = target + 34.0 + ticker.size().x;
            ui.painter().galley(
                pos2(target + 34.0, top),
                ticker.clone(),
                Color32::PLACEHOLDER,
            );
            ui.painter().galley(
                pos2(target + 34.0, top + ticker.size().y + 2.0),
                detail,
                Color32::PLACEHOLDER,
            );
            if let Some((label, tint)) = badge {
                pill(
                    ui,
                    ticker_right + 8.0,
                    top + ticker.size().y / 2.0,
                    label,
                    *tint,
                );
            }
        }
        RowState::Skipped { note } => {
            pill(
                ui,
                target,
                middle - 9.0,
                tr!("Skipped"),
                theme::text_tertiary(),
            );
            line(ui, note, middle + 9.0);
        }
        RowState::Unmatched { note } => {
            pill(ui, target, middle - 9.0, tr!("Not matched"), theme::REPLAY);
            line(ui, note, middle + 9.0);
        }
    }
    clicked
}

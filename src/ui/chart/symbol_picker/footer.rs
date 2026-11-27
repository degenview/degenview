//! The key hints along the foot of the picker, and what Return does in words.

use super::offer::{Offer, Primary};
use super::{Item, PADDING, PickKind, pick_name};
use crate::i18n::tr;
use crate::service::SearchResult;
use crate::ui::chart::picker_kit::widgets;
use crate::ui::icons::{self, Icon};
use crate::ui::kit::hints::{Hint, WIDE_HINT_GAP, spaced_hint_row};
use crate::ui::kit::{self, MEDIUM, REGULAR};
use crate::ui::theme;
use eframe::egui::{self, Color32, Rect, Ui, Vec2, pos2, vec2};

/// What the footer shows this frame.
pub(super) struct Footer<'a> {
    /// What the highlighted row offers.
    pub offer: Offer,
    /// What Return does, in words.
    pub return_hint: String,
    /// The label of the button that charts the ticked choices together, once there are any.
    pub ticked: Option<String>,
    /// What the last pick added to a watchlist.
    pub added: Option<&'a str>,
}

/// The keys that work here, and the button for ticked choices, whose press is returned. A narrow
/// picker drops the least needed hints rather than run past its edge.
pub(super) fn show(ui: &mut Ui, area: Rect, footer: Footer) -> bool {
    let mut pressed = false;
    let mut hints = vec![
        Hint::new(&["↑", "↓"], tr!("Navigate")),
        Hint::new(&["↵"], footer.return_hint),
    ];
    hints.extend(footer.offer.others().map(|kind| {
        if kind == PickKind::Overlay {
            Hint::new(&["⌥", "↵"], tr!("Compare"))
        } else {
            Hint::new(&["⇧", "↵"], tr!("Add to this tab"))
        }
    }));
    hints.push(Hint::new(&["Tab"], tr!("Switch tab")));
    hints.push(Hint::new(&["Esc"], tr!("Close")));

    // What sits at the right edge: the button for ticked choices, else what was just added.
    let mut right = area.right() - PADDING;
    if let Some(label) = footer.ticked {
        let width = kit::galley(
            ui,
            &label,
            13.0,
            MEDIUM,
            Color32::PLACEHOLDER,
            f32::INFINITY,
        )
        .size()
        .x + 32.0;
        let width = width.max(76.0);
        let button = Rect::from_min_size(
            pos2(right - width, area.center().y - 14.0),
            vec2(width, 28.0),
        );
        let mut cell = ui.new_child(egui::UiBuilder::new().max_rect(button));
        pressed = widgets::sheet_button(&mut cell, &label, true, true).clicked();
        right = button.left() - 16.0;
    } else if let Some(added) = footer.added {
        let galley = kit::galley(
            ui,
            &tr!("Added {market}", market = added),
            13.0,
            REGULAR,
            theme::text_secondary(),
            (area.width() * 0.4).max(80.0),
        );
        let left = right - galley.size().x;
        ui.painter().galley(
            pos2(left, area.center().y - galley.size().y / 2.0),
            galley,
            theme::text_secondary(),
        );
        icons::paint(
            ui.painter(),
            Rect::from_center_size(pos2(left - 14.0, area.center().y), Vec2::splat(14.0)),
            Icon::Check,
            theme::UP,
        );
        right = left - 30.0;
    }

    let row = Rect::from_min_max(
        pos2(area.left() + PADDING, area.top()),
        pos2(right, area.bottom()),
    );
    spaced_hint_row(
        ui,
        row,
        hints,
        13.0,
        WIDE_HINT_GAP,
        &[
            tr!("Compare"),
            tr!("Add to this tab"),
            tr!("Switch tab"),
            tr!("Navigate"),
        ],
    );
    pressed
}

/// What Return does on the highlighted row, in the footer's words. `replacing` is the focused
/// chart's market and `list` the watchlist being added to.
pub(super) fn return_hint(
    offer: Offer,
    item: Option<&Item>,
    replacing: Option<&str>,
    list: &str,
) -> String {
    match offer.primary {
        Primary::Fold { open: true } => tr!("Hide choices").to_owned(),
        Primary::Fold { open: false } => tr!("Show choices").to_owned(),
        Primary::Watch if !offer.enabled => tr!("Already in {list}", list = list),
        Primary::Watch => match item.and_then(Item::market) {
            Some(market) => tr!(
                "Add {market} to {list}",
                market = market_label(market),
                list = list
            ),
            None => tr!("Add to {list}", list = list),
        },
        Primary::Replace => tr!("Replace {market}", market = replacing.unwrap_or_default()),
        Primary::Compare => match replacing {
            Some(current) => tr!("Compare with {market}", market = current),
            None => tr!("Compare").to_owned(),
        },
        Primary::AddChart => tr!("Add chart").to_owned(),
        Primary::AddCard => tr!("Add card").to_owned(),
    }
}

/// The longest market name the footer's Return hint spells out; an event's title is longer.
const HINT_NAME_CHARS: usize = 28;

/// A market as the footer names it before it is picked: its venue and its name, as the row has them.
pub(super) fn market_label(result: &SearchResult) -> String {
    // A lone "Yes" says nothing: a prediction market is named by its event.
    let name = match &result.event {
        Some(event) if !event.title.is_empty() => event.title.clone(),
        _ => pick_name(result),
    };
    let name = if name.chars().count() > HINT_NAME_CHARS {
        let cut: String = name.chars().take(HINT_NAME_CHARS - 1).collect();
        format!("{}…", cut.trim_end())
    } else {
        name
    };
    format!("{} · {name}", super::lookup::short_name(result.source))
}

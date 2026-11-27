//! The button at the foot of the preview: what Return does with the shown market, and a chevron
//! that opens what else can be done with it.

use super::super::PickKind;
use super::super::list::{ADD_KEY, COMPARE_KEY, REPLACE_KEY};
use super::super::offer::{Offer, Primary};
use crate::i18n::tr;
use crate::ui::icons::{self, Icon};
use crate::ui::kit::{self, MEDIUM, size};
use crate::ui::theme;
use eframe::egui::{self, Id, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};

/// The chevron's share of the button.
const CHEVRON: f32 = 38.0;
const RADIUS: f32 = 8.0;

/// One thing to do with the shown market.
#[derive(Clone, Debug)]
pub struct Action {
    pub label: &'static str,
    pub icon: Icon,
    pub shortcut: &'static str,
    pub kind: PickKind,
}

/// The button's actions: the one it does, whether that can be done now, and the others.
#[derive(Clone, Debug)]
pub struct Actions {
    pub main: Action,
    pub enabled: bool,
    pub others: Vec<Action>,
}

impl Actions {
    /// The button for `offer`, whose main action is the mode's `default_kind`. An event is only
    /// folded open, so it has none.
    pub fn of(offer: &Offer, default_kind: PickKind) -> Option<Self> {
        let (label, icon) = match offer.primary {
            Primary::Fold { .. } => return None,
            Primary::Replace => (tr!("Replace chart"), Icon::Exchange),
            Primary::AddChart => (tr!("Add chart"), Icon::Plus),
            Primary::AddCard => (tr!("Add card"), Icon::Plus),
            Primary::Compare => (tr!("Compare"), Icon::Compare),
            Primary::Watch => (tr!("Add to watchlist"), Icon::Plus),
        };
        let others = offer
            .others()
            .map(|kind| {
                let (label, icon, shortcut) = if kind == PickKind::Overlay {
                    (tr!("Compare"), Icon::Compare, COMPARE_KEY)
                } else {
                    (tr!("Add to this tab"), Icon::Plus, ADD_KEY)
                };
                Action {
                    label,
                    icon,
                    shortcut,
                    kind,
                }
            })
            .collect();
        Some(Self {
            main: Action {
                label,
                icon,
                shortcut: REPLACE_KEY,
                kind: default_kind,
            },
            enabled: offer.enabled,
            others,
        })
    }
}

/// Draws the button in `rect`; returns what was asked for.
pub fn show(ui: &mut Ui, rect: Rect, actions: &Actions) -> Option<PickKind> {
    let chevron_width = if actions.others.is_empty() {
        0.0
    } else {
        CHEVRON
    };
    let main = Rect::from_min_max(rect.min, pos2(rect.right() - chevron_width, rect.bottom()));
    let chevron = Rect::from_min_max(pos2(main.right(), rect.top()), rect.max);
    let sense = if actions.enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let response = ui.interact(main, Id::new("symbol_picker_action"), sense);
    let response = crate::ui::accessibility::label(response, actions.main.label);
    let more = (chevron_width > 0.0).then(|| {
        let more = ui.interact(chevron, Id::new("symbol_picker_more"), Sense::click());
        crate::ui::accessibility::label(more, tr!("More actions"))
    });

    // One neutral shape, with the half under the pointer a step stronger.
    ui.painter().rect_filled(rect, RADIUS, kit::primary(0.16));
    if response.hovered() && actions.enabled {
        ui.painter().rect_filled(main, RADIUS, theme::ink(18));
    }
    if more.as_ref().is_some_and(egui::Response::hovered) {
        ui.painter().rect_filled(chevron, RADIUS, theme::ink(18));
    }
    ui.painter().rect_stroke(
        rect,
        RADIUS,
        Stroke::new(1.0, kit::primary(0.14)),
        StrokeKind::Inside,
    );

    let color = if actions.enabled {
        theme::text_primary()
    } else {
        theme::text_tertiary()
    };
    let label = kit::galley(
        ui,
        actions.main.label,
        size::BODY + 1.0,
        MEDIUM,
        color,
        (main.width() - 40.0).max(40.0),
    );
    let group = 14.0 + 8.0 + label.size().x;
    let left = main.center().x - group / 2.0;
    icons::paint(
        ui.painter(),
        Rect::from_center_size(pos2(left + 7.0, main.center().y), Vec2::splat(14.0)),
        actions.main.icon,
        color,
    );
    ui.painter().galley(
        pos2(left + 22.0, main.center().y - label.size().y / 2.0),
        label,
        color,
    );

    let mut chosen = (response.clicked() && actions.enabled).then_some(actions.main.kind);
    if let Some(more) = more {
        ui.painter().vline(
            chevron.left(),
            chevron.shrink2(vec2(0.0, 7.0)).y_range(),
            Stroke::new(1.0, theme::separator()),
        );
        icons::paint(
            ui.painter(),
            Rect::from_center_size(chevron.center(), Vec2::splat(11.0)),
            Icon::ChevronDown,
            theme::text_secondary(),
        );
        let rows: Vec<(&str, Option<&str>)> = actions
            .others
            .iter()
            .map(|action| (action.label, Some(action.shortcut)))
            .collect();
        kit::fitted_menu_rows(ui, &more, egui::RectAlign::TOP_END, &rows, |ui| {
            kit::menu_scope(ui, 0.0);
            for action in &actions.others {
                if kit::menu_item_with_shortcut(
                    ui,
                    action.icon,
                    action.label,
                    Some(action.shortcut),
                    true,
                    false,
                ) {
                    chosen = Some(action.kind);
                    ui.close();
                }
            }
        });
    }
    chosen
}

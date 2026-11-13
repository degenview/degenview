//! The flat bar every tab kind carries along its top: the chart's toolbar, the Portfolio's and the
//! Pine Studio's. Borderless buttons that tint on hover, hairline separators between groups, and
//! labels that fall back to icons when the window is too narrow for them. A bar is two runs of
//! [`Item`]s, one from each edge; what the buttons do is the caller's: a button carries an action
//! `A` that [`show`] returns when it is clicked, and a key `M` for the popup it owns, which the
//! caller draws through the `popup` callback once the button is.
//!
//! One engine keeps every bar the same height ([`HEIGHT`]), fill (`theme::chrome()`), spacing and
//! tint, so a tab does not change the window's chrome when it is switched to.

use crate::ui::icons::{self, Icon};
use crate::ui::kit::loading::LoadingIndicator;
use crate::ui::kit::{self, MEDIUM, REGULAR, size};
use crate::ui::theme;
use eframe::egui::{
    self, Color32, Galley, Id, Painter, Rect, Response, Sense, Ui, Vec2, pos2, vec2,
};
use std::sync::Arc;

#[cfg(test)]
mod tests;

pub const HEIGHT: f32 = 44.0;
/// A button's height.
const BUTTON: f32 = 30.0;
const RADIUS: f32 = 6.0;
const ICON: f32 = 16.0;
/// Space inside a button, either side of its content.
const PAD: f32 = 8.0;
/// Between a button's icon and its label.
const ICON_GAP: f32 = 6.0;
/// Between two buttons of one group.
const GAP: f32 = 2.0;
/// Either side of a separator's hairline.
const SEPARATOR_MARGIN: f32 = 7.0;
const SEPARATOR_HEIGHT: f32 = 18.0;
pub const EDGE: f32 = 10.0;
/// The most the two clusters may crowd each other before the right one starts to drop buttons.
const CLUSTER_GAP: f32 = 16.0;
/// Long labels are cut so the bar keeps its shape.
const NAME_MAX_WIDTH: f32 = 180.0;
const LABEL_SIZE: f32 = size::BODY;

/// Paints a picture of the caller's own, in the colour it is given, into a rectangle.
pub type GlyphPainter = Arc<dyn Fn(&Painter, Rect, Color32)>;

/// What a button draws before its label.
#[derive(Clone)]
pub enum Glyph {
    Icon(Icon),
    /// A picture of the caller's own, painted in the button's colour into a 16 point square.
    Custom(GlyphPainter),
}

pub struct Button<A, M> {
    id: Id,
    pub glyph: Option<Glyph>,
    pub label: Option<String>,
    /// Quieter text after the label.
    pub suffix: Option<String>,
    pub chevron: bool,
    pub tip: String,
    /// What assistive tech reads, when it differs from the tooltip.
    pub access: Option<String>,
    pub enabled: bool,
    /// Lit: the thing it toggles is on.
    pub active: bool,
    /// Work is going on behind it: the candle indicator runs where its icon is.
    pub busy: bool,
    /// The colour of its content while lit.
    pub tint: Option<Color32>,
    /// The label stays when the bar is too narrow for labels: it is what the button says.
    pub keep_label: bool,
    /// Quiet text until hovered or lit (a timeframe that is not the current one).
    pub quiet: bool,
    pub weight: f32,
    pub min_width: f32,
    /// The popup the button owns; the caller draws it.
    pub menu: Option<M>,
    pub action: Option<A>,
}

impl<A, M> Button<A, M> {
    /// `id` names the button within its bar; the bar's own id scopes it.
    pub fn new(id: impl std::hash::Hash + std::fmt::Debug, tip: impl Into<String>) -> Self {
        Self {
            id: Id::new(id),
            glyph: None,
            label: None,
            suffix: None,
            chevron: false,
            tip: tip.into(),
            access: None,
            enabled: true,
            active: false,
            busy: false,
            tint: None,
            keep_label: false,
            quiet: false,
            weight: MEDIUM,
            min_width: 0.0,
            menu: None,
            action: None,
        }
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.glyph = Some(Glyph::Icon(icon));
        self
    }

    /// Text beside the glyph; it is dropped before the glyph when space runs out. A button with
    /// no glyph always keeps its text.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn on_click(mut self, action: A) -> Self {
        self.action = Some(action);
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    pub fn busy(mut self, busy: bool) -> Self {
        self.busy = busy;
        self
    }

    /// A pull-down: `menu` is handed to the popup callback once the button is drawn.
    pub fn with_menu(mut self, menu: M) -> Self {
        self.menu = Some(menu);
        self
    }

    /// A chevron after the label, for a pull-down that shows the choice made in it.
    pub fn chevron(mut self) -> Self {
        self.chevron = true;
        self
    }
}

pub enum Item<A, M> {
    Button(Button<A, M>),
    Separator,
}

/// A button measured for one frame: its label is laid out and its width known.
pub struct Measured<'a, A, M> {
    button: &'a Button<A, M>,
    label: Option<Arc<Galley>>,
    suffix: Option<Arc<Galley>>,
    width: f32,
}

pub enum Slot<'a, A, M> {
    Button(Measured<'a, A, M>),
    Separator,
}

impl<A, M> Slot<'_, A, M> {
    /// The room the slot takes, without the gap after it.
    fn span(&self) -> f32 {
        match self {
            Slot::Button(measured) => measured.width,
            Slot::Separator => 1.0 + 2.0 * SEPARATOR_MARGIN,
        }
    }

    fn gap_after(&self) -> f32 {
        match self {
            Slot::Button(_) => GAP,
            Slot::Separator => 0.0,
        }
    }
}

fn measure<'a, A, M>(ui: &Ui, items: &'a [Item<A, M>], labels: bool) -> Vec<Slot<'a, A, M>> {
    items
        .iter()
        .map(|item| match item {
            Item::Separator => Slot::Separator,
            Item::Button(button) => {
                let shown = button.glyph.is_none() || labels || button.keep_label;
                let text = |value: &Option<String>, weight: f32| {
                    value.as_ref().filter(|_| shown).map(|value| {
                        // The colour is chosen when the button is painted.
                        kit::galley(
                            ui,
                            value,
                            LABEL_SIZE,
                            weight,
                            Color32::PLACEHOLDER,
                            NAME_MAX_WIDTH,
                        )
                    })
                };
                let label = text(&button.label, button.weight);
                let suffix = text(&button.suffix, REGULAR);
                let content = content_width(button, label.as_deref(), suffix.as_deref());
                Slot::Button(Measured {
                    button,
                    label,
                    suffix,
                    width: (content + 2.0 * PAD).max(button.min_width),
                })
            }
        })
        .collect()
}

fn content_width<A, M>(
    button: &Button<A, M>,
    label: Option<&Galley>,
    suffix: Option<&Galley>,
) -> f32 {
    let mut width = 0.0;
    if button.glyph.is_some() {
        width += ICON;
    }
    if let Some(label) = label {
        width += label.size().x
            + if button.glyph.is_some() {
                ICON_GAP
            } else {
                0.0
            };
        if let Some(suffix) = suffix {
            width += 4.0 + suffix.size().x;
        }
        if button.chevron {
            width += 4.0 + 9.0;
        }
    } else if button.chevron {
        width += 9.0;
    }
    width
}

fn run_width<A, M>(slots: &[Slot<A, M>]) -> f32 {
    slots
        .iter()
        .map(|slot| slot.span() + slot.gap_after())
        .sum()
}

/// Whether both runs fit side by side in a bar `width` wide.
fn fits<A, M>(left: &[Slot<A, M>], right: &[Slot<A, M>], width: f32) -> bool {
    run_width(left) + run_width(right) <= width - 2.0 * EDGE - CLUSTER_GAP
}

/// Where each slot starts, from the bar's left edge: the left run from the edge, the right run
/// from the other edge inwards, so what is dropped for lack of room is the right run's first slot.
fn arrange<'a, 'b, A, M>(
    width: f32,
    left: &'b [Slot<'a, A, M>],
    right: &'b [Slot<'a, A, M>],
) -> Vec<(f32, &'b Slot<'a, A, M>)> {
    let mut placed = Vec::new();
    let mut x = EDGE;
    for slot in left {
        placed.push((x, slot));
        x += slot.span() + slot.gap_after();
    }
    let mut edge = width - EDGE;
    for slot in right.iter().rev() {
        let at = edge - slot.span();
        if at < x + CLUSTER_GAP {
            break;
        }
        placed.push((at, slot));
        edge = at - slot.gap_after();
    }
    placed
}

/// The two runs of a bar, measured.
struct Runs<'a, A, M> {
    /// The labels are shown (there is room for them); only the tests ask.
    #[cfg_attr(not(test), allow(dead_code))]
    labels: bool,
    left: Vec<Slot<'a, A, M>>,
    right: Vec<Slot<'a, A, M>>,
}

/// The two runs measured for a bar `width` wide: labels go first when the two ends meet, then
/// the right run drops what does not fit (in [`arrange`]).
fn lay_out<'a, A, M>(
    ui: &Ui,
    left: &'a [Item<A, M>],
    right: &'a [Item<A, M>],
    width: f32,
) -> Runs<'a, A, M> {
    let wide = (measure(ui, left, true), measure(ui, right, true));
    if fits(&wide.0, &wide.1, width) {
        Runs {
            labels: true,
            left: wide.0,
            right: wide.1,
        }
    } else {
        Runs {
            labels: false,
            left: measure(ui, left, false),
            right: measure(ui, right, false),
        }
    }
}

/// One button as laid out: where it starts and ends along the bar, and what a click asks for.
#[cfg(test)]
pub struct Placed<A> {
    pub from: f32,
    pub to: f32,
    pub action: Option<A>,
}

/// Where every button lands in a bar `width` wide, and whether the labels showed.
#[cfg(test)]
pub fn geometry<A: Copy, M>(
    ui: &Ui,
    left: &[Item<A, M>],
    right: &[Item<A, M>],
    width: f32,
) -> (bool, Vec<Placed<A>>) {
    let runs = lay_out(ui, left, right, width);
    let placed = arrange(width, &runs.left, &runs.right)
        .into_iter()
        .filter_map(|(x, slot)| match slot {
            Slot::Button(measured) => Some(Placed {
                from: x,
                to: x + measured.width,
                action: measured.button.action,
            }),
            Slot::Separator => None,
        })
        .collect();
    (runs.labels, placed)
}

/// Draws the bar over `ui`'s rectangle and returns what was clicked. `popup` draws a button's
/// pull-down (`Button::menu`) right after the button, and may add actions of its own.
pub fn show<A: Copy, M: Copy>(
    ui: &mut Ui,
    left: &[Item<A, M>],
    right: &[Item<A, M>],
    mut popup: impl FnMut(&mut Ui, M, &Response, &mut Vec<A>),
) -> Vec<A> {
    let mut actions = Vec::new();
    let bar = ui.max_rect();
    ui.painter().rect_filled(bar, 0.0, theme::chrome());
    let y = bar.center().y - BUTTON / 2.0;
    let runs = lay_out(ui, left, right, bar.width());
    for (x, slot) in arrange(bar.width(), &runs.left, &runs.right) {
        paint_slot(ui, slot, bar.left() + x, y, &mut popup, &mut actions);
    }
    actions
}

fn paint_slot<A: Copy, M: Copy>(
    ui: &mut Ui,
    slot: &Slot<A, M>,
    x: f32,
    y: f32,
    popup: &mut impl FnMut(&mut Ui, M, &Response, &mut Vec<A>),
    actions: &mut Vec<A>,
) {
    let measured = match slot {
        Slot::Separator => {
            ui.painter().rect_filled(
                Rect::from_center_size(
                    pos2(x + SEPARATOR_MARGIN + 0.5, y + BUTTON / 2.0),
                    vec2(1.0, SEPARATOR_HEIGHT),
                ),
                0.0,
                theme::separator(),
            );
            return;
        }
        Slot::Button(measured) => measured,
    };
    let response = paint_button(ui, measured, x, y);
    let button = measured.button;
    if response.clicked()
        && button.enabled
        && let Some(action) = button.action
    {
        actions.push(action);
    }
    if button.enabled
        && let Some(menu) = button.menu
    {
        popup(ui, menu, &response, actions);
    }
}

/// One flat button: nothing at rest, a faint tint on hover, a stronger one while pressed or lit.
fn paint_button<A, M>(ui: &mut Ui, measured: &Measured<A, M>, x: f32, y: f32) -> Response {
    let button = measured.button;
    let rect = Rect::from_min_size(pos2(x, y), vec2(measured.width, BUTTON));
    let sense = if button.enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let response = ui.interact(rect, ui.id().with(button.id), sense);
    let tip = if button.tip.is_empty() {
        button.label.clone().unwrap_or_default()
    } else {
        button.tip.clone()
    };
    let response = crate::ui::accessibility::label(
        response.on_hover_text(&tip),
        button.access.clone().unwrap_or(tip),
    );
    let menu_open = button.menu.is_some()
        && egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&response));
    let lit = button.active || menu_open;
    let hovered = button.enabled && response.hovered();
    let pressed = button.enabled && response.is_pointer_button_down_on();
    let tint = if pressed {
        0.16
    } else if lit && hovered {
        0.15
    } else if hovered {
        0.09
    } else if lit {
        0.11
    } else {
        0.0
    };
    let painter = ui.painter();
    if tint > 0.0 {
        painter.rect_filled(rect, RADIUS, kit::primary(tint));
    }
    let color = if !button.enabled {
        theme::text_tertiary()
    } else if let (Some(tint), true) = (button.tint, lit) {
        tint
    } else if button.quiet && !hovered && !lit {
        theme::text_secondary()
    } else {
        theme::text_primary()
    };
    let content = content_width(
        button,
        measured.label.as_deref(),
        measured.suffix.as_deref(),
    );
    let mut at = rect.center().x - content / 2.0;
    let middle = rect.center().y;
    if let Some(glyph) = &button.glyph {
        let glyph_rect = Rect::from_min_size(pos2(at, middle - ICON / 2.0), Vec2::splat(ICON));
        if button.busy {
            // The icon gives way to the candle indicator; it asks for the next frame itself.
            LoadingIndicator::new()
                .size(ICON)
                .color(color)
                .paint_at(ui, glyph_rect);
        } else {
            match glyph {
                Glyph::Icon(icon) => icons::paint(painter, glyph_rect, *icon, color),
                Glyph::Custom(paint) => paint(painter, glyph_rect, color),
            }
        }
        at += ICON
            + if measured.label.is_some() {
                ICON_GAP
            } else {
                0.0
            };
    }
    if let Some(label) = &measured.label {
        let width = label.size().x;
        painter.galley(
            pos2(at, middle - label.size().y / 2.0),
            label.clone(),
            color,
        );
        at += width;
        if let Some(suffix) = &measured.suffix {
            at += 4.0;
            painter.galley(
                pos2(at, middle - suffix.size().y / 2.0),
                suffix.clone(),
                if button.enabled {
                    theme::text_secondary()
                } else {
                    theme::text_tertiary()
                },
            );
            at += suffix.size().x;
        }
        if button.chevron {
            at += 4.0;
        }
    }
    if button.chevron {
        icons::paint(
            painter,
            Rect::from_min_size(pos2(at, middle - 4.5), Vec2::splat(9.0)),
            Icon::ChevronDown,
            theme::text_secondary(),
        );
    }
    response
}

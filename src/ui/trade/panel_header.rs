//! The top row of the orders panel, the same for every venue:
//!
//! `[venue ▾] [account ▾] │ tabs ……… [New order] [eye] [⋯] [gear] [✕]`
//!
//! The venue pill says where orders go, the account chip which account of it, the tabs are the
//! venue's own sections, and the buttons on the right are the same few everywhere (a venue adds
//! entries to the `⋯` menu, not buttons). Below `COMPACT_WIDTH` the tabs drop to a second row.

use eframe::egui::{self, Id, Sense, Vec2, pos2, vec2};

use crate::trading::model::{BackendState, VenueInfo};
use crate::ui::accessibility;
use crate::ui::icons::{self, Icon};
use crate::ui::kit;
use crate::ui::theme;
use crate::ui::trade::venue_pill::{self, PillAction, PillSize};

/// Width below which the tabs drop to a second row instead of being cut off.
pub const COMPACT_WIDTH: f32 = 900.0;

/// Every control in the header is this tall, so a row of them shares one top and bottom edge.
pub const CONTROL_HEIGHT: f32 = 28.0;
/// The corner radius they share. The tab bar's track is this too (`kit::tab_bar_slim`).
pub const CONTROL_RADIUS: f32 = 6.0;

/// What the panel says about the venue it is showing.
#[derive(Clone, Debug)]
pub struct VenueBar {
    pub info: VenueInfo,
    pub state: BackendState,
    /// The venues the user enabled, for the pill's menu.
    pub enabled: Vec<VenueInfo>,
}

/// One row of the account chip's menu.
#[derive(Clone, Debug)]
pub struct ChipRow {
    pub label: String,
    pub selected: bool,
}

/// The account switcher: a quiet plate with the account's name, a small code beside it, and
/// (when the venue has more than one account, or lets one be added) a menu.
#[derive(Clone, Debug, Default)]
pub struct Chip {
    pub label: String,
    /// The currency, set beside the name in a smaller, quieter type.
    pub sub: Option<String>,
    pub tip: String,
    pub rows: Vec<ChipRow>,
    /// The last row, apart from the others ("New Account…").
    pub add: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChipPick {
    Row(usize),
    Add,
}

/// An entry of the `⋯` menu.
#[derive(Clone, Debug)]
pub struct MoreItem {
    pub icon: Icon,
    pub label: String,
    pub danger: bool,
    /// A hairline above it, between groups.
    pub rule_above: bool,
}

/// What the panel asks of its host, the same for every venue.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PanelEvent {
    Close,
    /// "New order": the order sheet takes it from here.
    NewOrder,
    /// Open Settings on this venue.
    OpenSettings,
    /// The pill's menu: another venue, or "Add broker…".
    Venue(PillAction),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HeaderEvent {
    Panel(PanelEvent),
    Chip(ChipPick),
    /// The eye: show or hide this venue's orders and positions on the charts.
    ToggleOverlay,
    /// The `⋯` menu's entry, by its index.
    More(usize),
}

pub struct Header<'a, T> {
    pub bar: &'a VenueBar,
    pub chip: &'a Chip,
    pub tabs: &'a [kit::TabItem<T>],
    /// `Some(showing)` when the venue draws its orders on the charts and the user may hide them.
    pub overlay: Option<bool>,
    /// Whether "New order" can be used now.
    pub can_order: bool,
    pub more: &'a [MoreItem],
    /// The tooltip of the gear.
    pub settings_tip: &'a str,
}

/// The quiet plate behind the account chip: the tab bar track's fill and hairline, lifted a step
/// on hover and while the menu is open.
pub fn plate(painter: &egui::Painter, rect: egui::Rect, hovered: bool, down: bool) {
    painter.rect_filled(rect, CONTROL_RADIUS, theme::quaternary(0.6));
    if down {
        painter.rect_filled(rect, CONTROL_RADIUS, kit::primary(0.1));
    } else if hovered {
        painter.rect_filled(rect, CONTROL_RADIUS, kit::primary(0.05));
    }
    painter.rect_stroke(
        rect,
        CONTROL_RADIUS,
        egui::Stroke::new(1.0, kit::hairline()),
        egui::StrokeKind::Inside,
    );
}

/// An icon button the height of the header's other controls: it lights up on hover and, when
/// `active`, sits on a raised plate.
pub fn glyph_response(
    ui: &mut egui::Ui,
    id: Id,
    icon: Icon,
    active: bool,
    label: &str,
) -> egui::Response {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(CONTROL_HEIGHT), Sense::hover());
    let response = ui
        .interact(rect, id, Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let response = accessibility::label(response, label);
    let fill = if active {
        theme::selected_tint(false)
    } else if response.is_pointer_button_down_on() {
        kit::primary(0.12)
    } else if response.hovered() {
        kit::primary(0.08)
    } else {
        egui::Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, CONTROL_RADIUS, fill);
    icons::paint(
        ui.painter(),
        egui::Rect::from_center_size(rect.center(), Vec2::splat(14.0)),
        icon,
        if active {
            theme::text_primary()
        } else {
            theme::text_secondary()
        },
    );
    response.on_hover_text(label)
}

/// An icon-only button; `label` is its tooltip.
pub fn icon_button(ui: &mut egui::Ui, icon: Icon, label: &str, active: bool) -> egui::Response {
    glyph_response(ui, Id::new(("trade-icon", label)), icon, active, label)
}

/// A hairline between the identity and the sections it holds, shorter than the controls.
fn rule(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(1.0, 16.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, theme::separator());
}

/// One header row, `height` tall, its children centred on it.
fn row(ui: &mut egui::Ui, height: f32, add: impl FnOnce(&mut egui::Ui)) {
    let size = vec2(ui.available_width(), height);
    ui.allocate_ui_with_layout(size, egui::Layout::left_to_right(egui::Align::Center), add);
}

/// Draws the header and returns what the user did. `tab` is the selected section.
pub fn show<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    header: &Header<T>,
    tab: &mut T,
) -> Vec<HeaderEvent> {
    let mut events = Vec::new();
    let compact = ui.available_width() < COMPACT_WIDTH;
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(12, 9))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(10.0, 0.0);
            if compact {
                row(ui, CONTROL_HEIGHT, |ui| {
                    identity(ui, header, &mut events);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        buttons(ui, header, &mut events);
                    });
                });
                ui.add_space(8.0);
                egui::ScrollArea::horizontal()
                    .id_salt("trade-tabs")
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                    .show(ui, |ui| tabs(ui, header, tab));
            } else {
                row(ui, CONTROL_HEIGHT, |ui| {
                    identity(ui, header, &mut events);
                    rule(ui);
                    tabs(ui, header, tab);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        buttons(ui, header, &mut events);
                    });
                });
            }
        });
    events
}

fn identity<T>(ui: &mut egui::Ui, header: &Header<T>, events: &mut Vec<HeaderEvent>) {
    if let Some(action) = venue_pill::show(
        ui,
        &header.bar.info,
        &header.bar.state,
        &header.bar.enabled,
        PillSize::Panel,
    ) {
        events.push(HeaderEvent::Panel(PanelEvent::Venue(action)));
    }
    if let Some(pick) = chip(ui, header.chip) {
        events.push(HeaderEvent::Chip(pick));
    }
}

/// The account switcher plate. Returns the row chosen in its menu.
fn chip(ui: &mut egui::Ui, chip: &Chip) -> Option<ChipPick> {
    if chip.label.is_empty() {
        return None;
    }
    let has_menu = !chip.rows.is_empty() || chip.add.is_some();
    let name = kit::galley(
        ui,
        &chip.label,
        13.0,
        kit::SEMIBOLD,
        theme::text_primary(),
        170.0,
    );
    let sub = chip.sub.as_ref().map(|sub| {
        kit::galley(
            ui,
            sub,
            10.0,
            kit::SEMIBOLD,
            theme::text_secondary(),
            f32::INFINITY,
        )
    });
    let width = 11.0
        + name.size().x
        + sub.as_ref().map_or(0.0, |sub| 7.0 + sub.size().x)
        + if has_menu { 7.0 + 9.0 } else { 0.0 }
        + 10.0;
    let sense = if has_menu {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(vec2(width, CONTROL_HEIGHT), sense);
    let response = accessibility::label(response, &chip.tip);
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&response));
    plate(
        ui.painter(),
        rect,
        has_menu && response.hovered(),
        open || response.is_pointer_button_down_on(),
    );
    let mut x = rect.left() + 11.0;
    let name_width = name.size().x;
    ui.painter().galley(
        pos2(x, rect.center().y - name.size().y / 2.0),
        name,
        theme::text_primary(),
    );
    x += name_width;
    if let Some(sub) = sub {
        x += 7.0;
        let sub_width = sub.size().x;
        ui.painter().galley(
            pos2(x, rect.center().y - sub.size().y / 2.0),
            sub,
            theme::text_secondary(),
        );
        x += sub_width;
    }
    if !has_menu {
        return None;
    }
    icons::paint(
        ui.painter(),
        egui::Rect::from_center_size(pos2(x + 7.0 + 4.5, rect.center().y), Vec2::splat(9.0)),
        Icon::ChevronDown,
        theme::text_secondary(),
    );
    let response = response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(&chip.tip);
    let labels: Vec<&str> = chip
        .rows
        .iter()
        .map(|row| row.label.as_str())
        .chain(chip.add.as_deref())
        .collect();
    let mut picked = None;
    kit::fitted_menu(
        ui,
        &response,
        egui::RectAlign::BOTTOM_START,
        &labels,
        |ui| {
            for (index, row) in chip.rows.iter().enumerate() {
                if kit::menu_item_marked(ui, Icon::Check, &row.label, row.selected) {
                    picked = Some(ChipPick::Row(index));
                    ui.close();
                }
            }
            if let Some(add) = &chip.add {
                if !chip.rows.is_empty() {
                    kit::menu_separator(ui);
                }
                if kit::menu_item(ui, Icon::Plus, add, true, false) {
                    picked = Some(ChipPick::Add);
                    ui.close();
                }
            }
        },
    );
    picked
}

fn tabs<T: Copy + PartialEq>(ui: &mut egui::Ui, header: &Header<T>, tab: &mut T) {
    kit::tab_bar_slim(ui, Id::new("trade-tab-bar"), header.tabs, tab);
}

fn buttons<T>(ui: &mut egui::Ui, header: &Header<T>, events: &mut Vec<HeaderEvent>) {
    ui.spacing_mut().item_spacing.x = 2.0;
    // Right-to-left, so the last one added sits leftmost.
    if icon_button(ui, Icon::Xmark, "Close the trading panel", false).clicked() {
        events.push(HeaderEvent::Panel(PanelEvent::Close));
    }
    if icon_button(ui, Icon::Gear, header.settings_tip, false).clicked() {
        events.push(HeaderEvent::Panel(PanelEvent::OpenSettings));
    }
    if !header.more.is_empty() {
        let more = glyph_response(ui, Id::new("trade-more"), Icon::Ellipsis, false, "More");
        let labels: Vec<&str> = header.more.iter().map(|item| item.label.as_str()).collect();
        kit::fitted_menu(ui, &more, egui::RectAlign::BOTTOM_END, &labels, |ui| {
            for (index, item) in header.more.iter().enumerate() {
                if item.rule_above {
                    kit::menu_separator(ui);
                }
                if kit::menu_item(ui, item.icon, &item.label, true, item.danger) {
                    events.push(HeaderEvent::More(index));
                    ui.close();
                }
            }
        });
    }
    if let Some(showing) = header.overlay {
        let icon = if showing { Icon::Eye } else { Icon::EyeSlash };
        let label = if showing {
            "Hide trading on charts"
        } else {
            "Show trading on charts"
        };
        if icon_button(ui, icon, label, showing).clicked() {
            events.push(HeaderEvent::ToggleOverlay);
        }
    }
    ui.add_space(6.0);
    if kit::sheet_button(ui, "New order", true, header.can_order).clicked() {
        events.push(HeaderEvent::Panel(PanelEvent::NewOrder));
    }
}

/// A bar for a test or a gallery picture, for a venue of `kind` called `name`.
#[cfg(test)]
pub(crate) fn sample_bar(
    venue: crate::model::broker::venue::TradingVenue,
    kind: crate::trading::model::VenueKind,
    name: &str,
) -> VenueBar {
    let info = VenueInfo {
        venue,
        name: name.to_owned(),
        kind,
        summary: String::new(),
        needs_no_credentials: true,
    };
    VenueBar {
        enabled: vec![info.clone()],
        info,
        state: BackendState::Ready,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::broker::venue::TradingVenue;
    use crate::trading::model::VenueKind;

    fn height_at(width: f32, kind: VenueKind) -> f32 {
        let ctx = egui::Context::default();
        let bar = sample_bar(TradingVenue::Paper, kind, "Coinbase");
        let chip = Chip {
            label: "Default portfolio".to_owned(),
            ..Chip::default()
        };
        let tabs = [
            kit::TabItem {
                value: 0,
                title: "Orders",
                icon: Icon::List,
                count: 2,
            },
            kit::TabItem {
                value: 1,
                title: "History",
                icon: Icon::Clock,
                count: 0,
            },
        ];
        let mut tab = 0;
        let mut height = 0.0;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                vec2(width, 400.0),
            )),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            let before = ui.cursor().top();
            show(
                ui,
                &Header {
                    bar: &bar,
                    chip: &chip,
                    tabs: &tabs,
                    overlay: None,
                    can_order: true,
                    more: &[],
                    settings_tip: "Settings",
                },
                &mut tab,
            );
            height = ui.cursor().top() - before;
        });
        output.textures_delta.clear();
        height
    }

    /// The venue, the account and the tabs share one row: a venue is not given a row of its own.
    #[test]
    fn the_header_is_one_row_when_wide_and_two_when_narrow_for_every_kind_of_venue() {
        for kind in [VenueKind::Live, VenueKind::Test, VenueKind::Simulated] {
            let wide = height_at(1_200.0, kind);
            assert!(
                (CONTROL_HEIGHT..=CONTROL_HEIGHT + 24.0).contains(&wide),
                "{kind:?}: {wide}"
            );
            let narrow = height_at(600.0, kind);
            assert!(narrow > wide + CONTROL_HEIGHT - 4.0, "{kind:?}: {narrow}");
        }
    }
}

//! The card of compared markets that shows under the OHLC box while the pointer is on the chart:
//! one row per market (colour, name, the value at the crosshair), and on a hovered row the market's
//! eye, settings and remove. It replaces the chips the markets used to have next to the ticker.
//!
//! The card is shown while the crosshair is, and stays while the pointer is on the card itself
//! (which takes the pointer from the plot, so the crosshair goes) or while a market's settings
//! popover is up. For that it remembers, per chart, the bar and the place it was last drawn at.
//!
//! Nothing here edits a chart: every control answers with a [`CardAction`].

use super::overlays::{LineState, OverlayFrame, OverlayLine};
use super::{ChartCard, READOUT_MARGIN, READOUT_PAD, READOUT_ROW_GAP, View, truncated};
use crate::model::overlay::feed::OverlayStatus;
use crate::model::overlay::{Move, OverlayCommand, OverlayId, OverlayScale, ScaleChoice};
use crate::ui::chart::indicator_strip::{self, Action};
use crate::ui::icons::{self, Icon};
use crate::ui::{chart::settings::overlay_popover, kit, theme};
use eframe::egui::{
    self, Align2, Color32, Context, FontId, Id, Painter, Pos2, Rect, Sense, Stroke, StrokeKind, Ui,
    Vec2, vec2,
};

/// The space between two control buttons. A button is as tall as a row, so the card is as tall as
/// the OHLC box above it when it holds one market.
const CONTROL_GAP: f32 = 2.0;
/// The card's rows are at most this many characters of a market's name.
const NAME_CHARS: usize = 18;

/// What the card remembers between frames of one chart.
#[derive(Clone, Copy)]
struct Memo {
    /// The pass the crosshair last drew the card in (`Context::cumulative_pass_nr`).
    pass: u64,
    /// The bar the crosshair was on, among the bars in view.
    index: usize,
    /// Where the card's top sits: under the OHLC box.
    top: f32,
    /// The card as last drawn, to know when the pointer is on it.
    rect: Option<Rect>,
}

fn memo_id(chart_id: u64) -> Id {
    Id::new(("overlay-card", chart_id))
}

/// The crosshair is on `index`: the card shows there, `top` under the readout above it.
pub(super) fn remember(ctx: &Context, chart_id: u64, index: usize, top: f32) {
    let rect = ctx
        .data(|data| data.get_temp::<Memo>(memo_id(chart_id)))
        .and_then(|memo| memo.rect);
    // Read before taking the data lock: the context's own lock is not reentrant.
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|data| {
        data.insert_temp(
            memo_id(chart_id),
            Memo {
                pass,
                index,
                top,
                rect,
            },
        );
    });
}

/// What a control of the card asks for.
pub(super) type CardAction = Action;

/// What the card reports after a frame.
#[derive(Default)]
pub(super) struct CardResult {
    pub action: Option<CardAction>,
    /// The pointer is on the card, so the plot under it sees none.
    pub on_card: bool,
}

struct Row<'a> {
    line: &'a OverlayLine<'a>,
    name: String,
    value: String,
    /// Not drawn: hidden by the user, or cannot be on this chart.
    dim: bool,
}

impl ChartCard {
    /// Draws the card and answers its controls. `plot` is the part of the plot under the header.
    pub(super) fn overlay_card(
        &self,
        ui: &mut Ui,
        chart_id: u64,
        plot: Rect,
        view: &View,
        frame: &OverlayFrame,
    ) -> CardResult {
        if frame.lines.is_empty() {
            return CardResult::default();
        }
        let ctx = ui.ctx().clone();
        let memo = ctx.data(|data| data.get_temp::<Memo>(memo_id(chart_id)));
        let open = overlay_popover::open(&ctx).filter(|(chart, _)| *chart == chart_id);
        let shown = memo.is_some_and(|memo| {
            memo.pass == ctx.cumulative_pass_nr()
                || memo.rect.is_some_and(|rect| ui.rect_contains_pointer(rect))
                || open.is_some()
        });
        let Some(memo) = memo.filter(|_| shown) else {
            self.forget_card(&ctx, chart_id);
            return CardResult::default();
        };

        let painter = ui.painter().clone();
        let mode = view.scale.mode;
        let format = self.appearance.price_format();
        let rows: Vec<Row> = frame
            .lines
            .iter()
            .map(|line| {
                let dim = line.state != LineState::Drawn
                    && !matches!(
                        line.state,
                        LineState::Waiting | LineState::NoOverlap | LineState::NoBaseline
                    );
                let value = if line.state == LineState::Hidden {
                    "hidden".to_owned()
                } else {
                    line.values
                        .get(memo.index)
                        .copied()
                        .flatten()
                        .map_or_else(|| "—".to_owned(), |value| line.text(value, mode, format))
                };
                Row {
                    line,
                    name: truncated(&line.overlay.name(), NAME_CHARS),
                    value,
                    dim,
                }
            })
            .collect();

        let font = FontId::monospace(10.0);
        let width_of = |text: &str| {
            painter
                .layout_no_wrap(text.to_owned(), font.clone(), Color32::WHITE)
                .size()
                .x
        };
        // A row is as tall as the OHLC box's text, so the card is as tall as that box with one
        // market on it, and a control button is as tall as its row.
        let row_h = painter
            .layout_no_wrap("0".into(), font.clone(), Color32::WHITE)
            .size()
            .y;
        let controls_width = 3.0 * row_h + 2.0 * CONTROL_GAP;
        let pad = READOUT_PAD;
        let stride = row_h + READOUT_ROW_GAP;
        let room = (plot.bottom() - memo.top - 8.0 - 2.0 * pad.y + READOUT_ROW_GAP) / stride;
        let fit = room.floor().max(1.0) as usize;
        let more = rows.len().saturating_sub(fit);
        let shown_rows = if more > 0 {
            fit.saturating_sub(1).max(1)
        } else {
            rows.len()
        };
        let more = rows.len() - shown_rows;
        let name_width = rows
            .iter()
            .take(shown_rows)
            .map(|row| width_of(&row.name))
            .fold(0.0, f32::max);
        let value_width = rows
            .iter()
            .take(shown_rows)
            .map(|row| width_of(&row.value))
            .fold(0.0, f32::max);
        // The room for the controls is only there while the pointer is on the card (or a settings
        // popover is up): at rest the card is as wide as its text. The pointer is on the narrow card
        // before it widens and on the wide one until it leaves, so the width cannot flip-flop.
        let pointer_on_card = memo.rect.is_some_and(|rect| ui.rect_contains_pointer(rect));
        let on_card = pointer_on_card || open.is_some();
        let controls_room = if on_card { 8.0 + controls_width } else { 0.0 };
        let lines = shown_rows + usize::from(more > 0);
        let inner = Vec2::new(
            12.0 + name_width + 8.0 + value_width + controls_room,
            lines as f32 * row_h + lines.saturating_sub(1) as f32 * READOUT_ROW_GAP,
        );
        let origin = Pos2::new(plot.left() + READOUT_MARGIN, memo.top + 4.0);
        let card = Rect::from_min_size(origin, inner + pad * 2.0);
        ctx.data_mut(|data| {
            data.insert_temp(
                memo_id(chart_id),
                Memo {
                    rect: Some(card),
                    ..memo
                },
            );
        });

        painter.rect(
            card,
            4.0,
            theme::chrome().gamma_multiply(0.94),
            Stroke::new(1.0, theme::border()),
            StrokeKind::Inside,
        );
        // The card takes the pointer: a press on it must not pan the chart under it.
        let _ = ui.interact(
            card,
            memo_id(chart_id).with("card"),
            Sense::click_and_drag(),
        );

        let mut action = None;
        let count = self.overlay_list().len();
        let own_scales: Vec<(OverlayId, String)> = self
            .overlay_list()
            .iter()
            .filter_map(|overlay| match overlay.scale {
                OverlayScale::Own(group) if group == overlay.id => {
                    Some((overlay.id, overlay.name()))
                }
                _ => None,
            })
            .collect();
        for (n, row) in rows.iter().take(shown_rows).enumerate() {
            let overlay = row.line.overlay;
            let id = overlay.id;
            let top = origin.y + pad.y + n as f32 * stride;
            let row_rect = Rect::from_min_size(
                Pos2::new(origin.x + 2.0, top - READOUT_ROW_GAP / 2.0),
                vec2(card.width() - 4.0, row_h + READOUT_ROW_GAP),
            );
            let response = ui
                .interact(
                    row_rect,
                    memo_id(chart_id).with(("row", id)),
                    Sense::click(),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            let popover_here = open == Some((chart_id, id));
            let hovered = ui.rect_contains_pointer(row_rect) || popover_here;
            if hovered {
                painter.rect_filled(
                    row_rect,
                    3.0,
                    theme::ink(if popover_here { 18 } else { 10 }),
                );
            }
            self.paint_row(
                &painter,
                row,
                &font,
                (origin.x + pad.x, top),
                (name_width, row_h),
            );
            if hovered {
                action = action.or(self.row_controls(
                    ui,
                    chart_id,
                    row_rect,
                    overlay.visible,
                    id,
                    row_h,
                ));
            }
            if response.clicked() && action.is_none() {
                action = Some(Action::ConfigureOverlay(id, row_rect));
            }
            let note = self.row_note(row);
            response.clone().on_hover_text(note);
            let first = n == 0;
            let last = n + 1 == count;
            let scale = overlay.scale;
            let visible = overlay.visible;
            let others: Vec<_> = own_scales
                .iter()
                .filter(|(other, _)| *other != id)
                .cloned()
                .collect();
            let labels = menu_labels(visible, &others);
            let rows_for_menu: Vec<(&str, Option<&str>)> =
                labels.iter().map(|label| (label.as_str(), None)).collect();
            let mut chosen = None;
            kit::fitted_context_menu(ui, &response, &rows_for_menu, |ui| {
                chosen = row_menu(ui, id, visible, scale, first, last, &others, row_rect);
            });
            action = action.or(chosen);
        }
        if more > 0 {
            let y = origin.y + pad.y + shown_rows as f32 * stride + row_h / 2.0;
            painter.text(
                Pos2::new(origin.x + pad.x, y),
                Align2::LEFT_CENTER,
                format!("+{more} more"),
                font,
                theme::text_secondary(),
            );
        }
        CardResult {
            action,
            on_card: pointer_on_card,
        }
    }

    /// The card goes: its place is forgotten so a pointer near it later does not keep it.
    fn forget_card(&self, ctx: &Context, chart_id: u64) {
        if let Some(memo) = ctx.data(|data| data.get_temp::<Memo>(memo_id(chart_id)))
            && memo.rect.is_some()
        {
            ctx.data_mut(|data| data.insert_temp(memo_id(chart_id), Memo { rect: None, ..memo }));
        }
    }

    /// Dot, name and value of one row.
    fn paint_row(
        &self,
        painter: &Painter,
        row: &Row,
        font: &FontId,
        (left, top): (f32, f32),
        (name_width, row_h): (f32, f32),
    ) {
        let mid = top + row_h / 2.0;
        let color = if row.dim {
            theme::text_tertiary()
        } else {
            row.line.color()
        };
        painter.circle_filled(Pos2::new(left + 3.0, mid), 3.0, color);
        painter.text(
            Pos2::new(left + 12.0, mid),
            Align2::LEFT_CENTER,
            &row.name,
            font.clone(),
            if row.dim {
                theme::text_secondary()
            } else {
                theme::text_primary()
            },
        );
        painter.text(
            Pos2::new(left + 12.0 + name_width + 8.0, mid),
            Align2::LEFT_CENTER,
            &row.value,
            font.clone(),
            color,
        );
    }

    /// Eye, settings and remove at the right end of a hovered row.
    fn row_controls(
        &self,
        ui: &mut Ui,
        chart_id: u64,
        row: Rect,
        visible: bool,
        id: OverlayId,
        control: f32,
    ) -> Option<CardAction> {
        let mut action = None;
        let controls = [
            (
                if visible { Icon::Eye } else { Icon::EyeSlash },
                if visible { "Hide" } else { "Show" },
                Action::Overlay(OverlayCommand::SetVisible(id, !visible)),
            ),
            (Icon::Sliders, "Settings", Action::ConfigureOverlay(id, row)),
            (
                Icon::Xmark,
                "Remove",
                Action::Overlay(OverlayCommand::Remove(id)),
            ),
        ];
        let mut x = row.right() - 6.0 - (3.0 * control + 2.0 * CONTROL_GAP);
        for (n, (icon, label, wanted)) in controls.into_iter().enumerate() {
            let rect = Rect::from_min_size(
                Pos2::new(x, row.center().y - control / 2.0),
                Vec2::splat(control),
            );
            x += control + CONTROL_GAP;
            let response = ui.interact(
                rect,
                memo_id(chart_id).with(("control", id, n)),
                Sense::click(),
            );
            let response = crate::ui::accessibility::label(response, label)
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(label);
            let fill = if response.is_pointer_button_down_on() {
                theme::ink(26)
            } else if response.hovered() {
                theme::ink(14)
            } else {
                Color32::TRANSPARENT
            };
            let painter = ui.painter();
            painter.rect_filled(rect, 5.0, fill);
            icons::paint(
                painter,
                Rect::from_center_size(rect.center(), Vec2::splat(control * 0.75)),
                icon,
                theme::text_secondary(),
            );
            if response.clicked() {
                action = Some(wanted);
            }
        }
        action
    }

    /// What a row's tooltip says: the market, where it comes from and how its data is doing.
    fn row_note(&self, row: &Row) -> String {
        let overlay = row.line.overlay;
        let mut note = format!(
            "{} · {}",
            overlay.name(),
            overlay.instrument.source.display_name()
        );
        let status = match row.line.state {
            LineState::Waiting => Some("waiting to load".to_owned()),
            LineState::NoOverlap => Some("no data in view".to_owned()),
            LineState::NoBaseline => Some("nothing to start from yet".to_owned()),
            LineState::Blocked(reason) => Some(reason.to_lowercase()),
            LineState::Unsupported => Some("not available on price-based chart types".to_owned()),
            LineState::Hidden | LineState::Drawn => None,
        };
        if let Some(status) = status {
            note.push_str(" — ");
            note.push_str(&status);
        }
        if let Some(OverlayStatus::Error(message)) =
            self.overlay_feed(overlay.id).map(|feed| &feed.status)
        {
            note.push_str(&format!(" — could not load: {message}"));
        }
        note
    }
}

/// Every label a row's menu can show, for measuring it: the menu is as wide as its longest row.
fn menu_labels(visible: bool, others: &[(OverlayId, String)]) -> Vec<String> {
    let mut labels: Vec<String> = [
        if visible { "Hide" } else { "Show" },
        "Settings…",
        "Chart's scale",
        "Its own scale",
        "Bring to front",
        "Bring forward",
        "Send backward",
        "Send to back",
        "Remove",
    ]
    .map(str::to_owned)
    .to_vec();
    labels.extend(others.iter().map(|(_, name)| format!("Scale of {name}")));
    labels
}

/// The right-click menu of a row: show or hide, its settings, its scale and place in the drawing
/// order, and remove.
#[allow(clippy::too_many_arguments)]
fn row_menu(
    ui: &mut Ui,
    id: OverlayId,
    visible: bool,
    scale: OverlayScale,
    first: bool,
    last: bool,
    others: &[(OverlayId, String)],
    row: Rect,
) -> Option<CardAction> {
    let mut action = None;
    ui.spacing_mut().item_spacing.y = 0.0;
    let (icon, label) = if visible {
        (Icon::EyeSlash, "Hide")
    } else {
        (Icon::Eye, "Show")
    };
    if kit::menu_item(ui, icon, label, true, false) {
        action = Some(Action::Overlay(OverlayCommand::SetVisible(id, !visible)));
        ui.close();
    }
    if kit::menu_item(ui, Icon::Sliders, "Settings…", true, false) {
        action = Some(Action::ConfigureOverlay(id, row));
        ui.close();
    }
    let mut send = |ui: &mut Ui, command: OverlayCommand| {
        action = Some(Action::Overlay(command));
        ui.close();
    };
    kit::menu_separator(ui);
    indicator_strip::menu_heading(ui, "Scale");
    let own = scale == OverlayScale::Own(id);
    let shared = matches!(scale, OverlayScale::Own(group) if group != id);
    if kit::menu_item_marked(
        ui,
        Icon::Check,
        "Chart's scale",
        scale == OverlayScale::Main,
    ) {
        send(ui, OverlayCommand::SetScale(id, ScaleChoice::Main));
    }
    if kit::menu_item_marked(ui, Icon::Check, "Its own scale", own) {
        send(ui, OverlayCommand::SetScale(id, ScaleChoice::Own));
    }
    for (other, name) in others {
        let marked = shared && scale == OverlayScale::Own(*other);
        if kit::menu_item_marked(ui, Icon::Check, &format!("Scale of {name}"), marked) {
            send(
                ui,
                OverlayCommand::SetScale(id, ScaleChoice::SharedWith(*other)),
            );
        }
    }
    kit::menu_separator(ui);
    indicator_strip::menu_heading(ui, "Visual order");
    for (label, to, enabled) in [
        ("Bring to front", Move::ToFront, !last),
        ("Bring forward", Move::Forward, !last),
        ("Send backward", Move::Backward, !first),
        ("Send to back", Move::ToBack, !first),
    ] {
        if kit::menu_item(ui, Icon::ArrowUp, label, enabled, false) {
            send(ui, OverlayCommand::Move(id, to));
        }
    }
    kit::menu_separator(ui);
    if kit::menu_item(ui, Icon::Trash, "Remove", true, true) {
        send(ui, OverlayCommand::Remove(id));
    }
    action
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_menu_is_as_wide_as_its_longest_row() {
        let ctx = egui::Context::default();
        crate::ui::theme::install_offscreen(&ctx);
        let width = |others: &[(OverlayId, String)]| {
            let labels = menu_labels(true, others);
            let rows: Vec<(&str, Option<&str>)> =
                labels.iter().map(|l| (l.as_str(), None)).collect();
            let mut width = 0.0;
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                width = kit::menu_width_for_rows(ui, &rows, 0.0);
            });
            output.textures_delta.clear();
            width
        };
        let long = [(
            OverlayId(9),
            "A very long decentralised pair name / WETH".to_owned(),
        )];
        assert!(
            menu_labels(true, &long)
                .iter()
                .any(|l| l.starts_with("Scale of A very long"))
        );
        assert!(
            width(&long) > width(&[]) + 40.0,
            "a long row widens the menu"
        );
        assert!(
            width(&[]) < 180.0 + 24.0,
            "and a short one is not padded to a guess"
        );
        assert!(menu_labels(true, &[]).contains(&"Hide".to_owned()));
        assert!(menu_labels(false, &[]).contains(&"Show".to_owned()));
    }
}

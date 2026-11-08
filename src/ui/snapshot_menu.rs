//! The menu under the toolbar's camera button: what to do with a picture of the focused chart.
//!
//! The button is the toolbar's and reports the row picked; the workspace takes the snapshot of
//! the focused chart. A row that cannot work says why instead of doing nothing: the hosted
//! ones need a host in Settings, and every row waits while a snapshot is being made.

use super::icons::Icon;
use super::kit;
use crate::model::snapshot::SnapshotAction;
use eframe::egui::{self, Vec2};

/// What the menu needs to know besides the chart.
pub struct SnapshotMenu<'a> {
    /// A snapshot is already being made.
    pub busy: bool,
    /// Why the rows that upload cannot be used, if they cannot.
    pub hosting_unavailable: Option<&'a str>,
    /// The shortcut each row shows, if it has one that works.
    pub shortcut: &'a dyn Fn(SnapshotAction) -> Option<String>,
}

/// What the menu reports when a row is picked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    /// Take a snapshot and do this with it.
    Take(SnapshotAction),
    /// Open Settings on the Snapshots page.
    Configure,
}

/// The row after the separator, which opens the Snapshots page of Settings.
const CONFIGURE_LABEL: &str = "Configure Snapshots…";

const fn icon(action: SnapshotAction) -> Icon {
    match action {
        SnapshotAction::Download => Icon::Download,
        SnapshotAction::CopyImage => Icon::CopyDoc,
        SnapshotAction::CopyLink => Icon::Link,
        SnapshotAction::OpenInTab => Icon::ArrowUpRightSquare,
        SnapshotAction::ShareOnX => Icon::XLogo,
    }
}

/// Why `action` cannot be used now, or `None` when it can.
fn unavailable(action: SnapshotAction, menu: &SnapshotMenu) -> Option<String> {
    if menu.busy {
        return Some("A snapshot is already being made.".to_owned());
    }
    action
        .is_hosted()
        .then(|| menu.hosting_unavailable.map(str::to_owned))
        .flatten()
}

/// The menu under the toolbar's camera button `response`, opened by a click on it. Returns the
/// row picked this frame.
pub fn menu(ui: &mut egui::Ui, response: &egui::Response, menu: &SnapshotMenu) -> Option<Pick> {
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(response));
    let mut picked = None;
    if !(open || response.clicked()) {
        return picked;
    }
    // As wide as the rows and their shortcuts need, worked out only while the menu is open.
    let hints: Vec<Option<String>> = SnapshotAction::ALL
        .into_iter()
        .map(|action| {
            unavailable(action, menu)
                .is_none()
                .then(|| (menu.shortcut)(action))
                .flatten()
        })
        .collect();
    let rows: Vec<(&str, Option<&str>)> = SnapshotAction::ALL
        .into_iter()
        .zip(&hints)
        .map(|(action, hint)| (action.label(), hint.as_deref()))
        .chain([(CONFIGURE_LABEL, None)])
        .collect();
    let width = kit::menu_width_for_rows(ui, &rows, 0.0);
    egui::Popup::menu(response)
        .frame(kit::menu_frame())
        .width(width)
        .show(|ui| {
            ui.set_width(width - kit::MENU_FRAME_MARGIN);
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            kit::menu_heading(ui, "Chart snapshot");
            for action in SnapshotAction::ALL {
                let blocked = unavailable(action, menu);
                let shortcut = if blocked.is_none() {
                    (menu.shortcut)(action)
                } else {
                    None
                };
                let row = kit::menu_row(
                    ui,
                    kit::MenuRow {
                        icon: Some(icon(action)),
                        shortcut: shortcut.as_deref(),
                        enabled: blocked.is_none(),
                        ..kit::MenuRow::new(action.label())
                    },
                );
                if let Some(reason) = &blocked {
                    row.on_hover_text(reason);
                } else if row.clicked() {
                    picked = Some(Pick::Take(action));
                    ui.close();
                }
            }
            // Never blocked: it is where a missing host is fixed.
            kit::menu_separator(ui);
            let configure = kit::menu_row(
                ui,
                kit::MenuRow {
                    icon: Some(Icon::Sliders),
                    ..kit::MenuRow::new(CONFIGURE_LABEL)
                },
            );
            if configure.clicked() {
                picked = Some(Pick::Configure);
                ui.close();
            }
        });
    picked
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::Rect;

    fn none(_: SnapshotAction) -> Option<String> {
        None
    }

    #[test]
    fn every_row_has_its_own_icon_and_label() {
        let icons: Vec<_> = SnapshotAction::ALL.into_iter().map(icon).collect();
        for (i, a) in icons.iter().enumerate() {
            assert!(icons.iter().skip(i + 1).all(|b| a != b), "{a:?} repeats");
        }
        let labels: Vec<_> = SnapshotAction::ALL.into_iter().map(|a| a.label()).collect();
        assert_eq!(
            labels,
            [
                "Download image",
                "Copy image",
                "Copy link",
                "Open in new tab",
                "Tweet image"
            ]
        );
    }

    #[test]
    fn only_the_hosted_rows_wait_for_a_host() {
        let menu = SnapshotMenu {
            busy: false,
            hosting_unavailable: Some("Set a host"),
            shortcut: &none,
        };
        let blocked: Vec<_> = SnapshotAction::ALL
            .into_iter()
            .map(|action| unavailable(action, &menu).is_some())
            .collect();
        assert_eq!(blocked, [false, false, true, true, true]);
        let ready = SnapshotMenu {
            hosting_unavailable: None,
            ..menu
        };
        assert!(
            SnapshotAction::ALL
                .into_iter()
                .all(|action| unavailable(action, &ready).is_none())
        );
    }

    #[test]
    fn every_row_waits_while_a_snapshot_is_made() {
        let menu = SnapshotMenu {
            busy: true,
            hosting_unavailable: None,
            shortcut: &none,
        };
        assert!(
            SnapshotAction::ALL
                .into_iter()
                .all(|action| unavailable(action, &menu).is_some())
        );
    }

    /// A 27-point camera button, as the toolbar draws one, with the menu under it.
    fn button(ui: &mut egui::Ui, snapshot: &SnapshotMenu) -> Option<Pick> {
        let (_, response) = ui.allocate_exact_size(Vec2::splat(27.0), egui::Sense::click());
        menu(ui, &response, snapshot)
    }

    /// One frame of a button in a row, with the pointer events given: where the button is and
    /// what it reported.
    fn frame(
        ctx: &egui::Context,
        menu: &SnapshotMenu,
        events: Vec<egui::Event>,
    ) -> (Rect, Option<Pick>) {
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(400.0, 400.0),
            )),
            events,
            ..Default::default()
        };
        let (mut rect, mut picked) = (Rect::NOTHING, None);
        let mut out = ctx.run_ui(raw, |ui| {
            ui.horizontal(|ui| {
                let at = ui.next_widget_position();
                picked = button(ui, menu);
                rect = Rect::from_min_size(at, egui::vec2(27.0, 27.0));
            });
        });
        out.textures_delta.clear();
        (rect, picked)
    }

    fn click(position: egui::Pos2) -> Vec<egui::Event> {
        let button = |pressed| egui::Event::PointerButton {
            pos: position,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        vec![
            egui::Event::PointerMoved(position),
            button(true),
            button(false),
        ]
    }

    /// The menu drawn headlessly: its button opens it, and a row is a click away.
    #[test]
    fn the_menu_opens_from_the_button_and_a_click_picks_a_row() {
        let ctx = egui::Context::default();
        let shortcut =
            |action: SnapshotAction| (action == SnapshotAction::Download).then(|| "⌥⌘S".to_owned());
        let menu = SnapshotMenu {
            busy: false,
            hosting_unavailable: None,
            shortcut: &shortcut,
        };
        let (button_rect, _) = frame(&ctx, &menu, Vec::new());
        assert!(button_rect.width() > 0.0);
        frame(&ctx, &menu, click(button_rect.center()));
        let (_, picked) = frame(&ctx, &menu, Vec::new());
        assert_eq!(picked, None, "opening the menu picks nothing");
        // The first row sits under the heading, below the button.
        let row = egui::pos2(
            button_rect.left() + 40.0,
            button_rect.bottom() + 6.0 + 24.0 + 12.0,
        );
        let (_, first) = frame(&ctx, &menu, click(row));
        let (_, second) = frame(&ctx, &menu, Vec::new());
        assert_eq!(first.or(second), Some(Pick::Take(SnapshotAction::Download)));
    }

    /// The last row, after the separator, asks for the Snapshots page of Settings, and it works
    /// while a snapshot is being made and when no host is set.
    #[test]
    fn the_last_row_configures_snapshots_even_when_the_others_wait() {
        let ctx = egui::Context::default();
        let menu = SnapshotMenu {
            busy: true,
            hosting_unavailable: Some("Set a host"),
            shortcut: &none,
        };
        let (button_rect, _) = frame(&ctx, &menu, Vec::new());
        frame(&ctx, &menu, click(button_rect.center()));
        frame(&ctx, &menu, Vec::new());
        // Heading, five rows, then the separator, each row 24 points tall.
        let row = egui::pos2(
            button_rect.left() + 40.0,
            button_rect.bottom() + 6.0 + 24.0 + 5.0 * 24.0 + 9.0 + 12.0,
        );
        let (_, first) = frame(&ctx, &menu, click(row));
        let (_, second) = frame(&ctx, &menu, Vec::new());
        assert_eq!(first.or(second), Some(Pick::Configure));
    }
}

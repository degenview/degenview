//! The sheet that opens when the Image tool is armed: where a picture is chosen. It offers the
//! system's file panel and the clipboard, and says it can be dropped on the chart instead. It
//! only asks: the workspace reads the picture, checks it and stores it (off the UI thread), and the
//! sheet shows what it is doing and why a picture was refused.

use crate::model::link_card::{Link, LinkError, LinkKind};
use crate::ui::icons::{self, Icon};
use crate::ui::kit::{self, REGULAR, size};
use crate::ui::{sheet, theme};
use eframe::egui::{self, Align2, Id, Key, Rect, Sense, Stroke, vec2};

const WIDTH: f32 = 420.0;

/// What the user asked of the image sheet this frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImageSheetAction {
    None,
    /// Open the file panel.
    Choose,
    /// Take the picture on the clipboard.
    Paste,
    /// Close the sheet and put the tool down.
    Close,
}

/// What the sheet has to say.
pub struct ImageSheet<'a> {
    /// A picture is being read: nothing else can be started.
    pub working: bool,
    /// Why the last picture was refused.
    pub error: Option<&'a str>,
    /// Whether a drag with files is over the window right now.
    pub dragging: bool,
}

/// Shows the sheet.
pub fn show_image(ctx: &egui::Context, view: &ImageSheet) -> ImageSheetAction {
    let mut action = ImageSheetAction::None;
    sheet::show(ctx, Id::new("image-sheet"), WIDTH, |ui| {
        ui.add_space(f32::from(sheet::INSET));
        sheet::inset(ui, |ui| {
            sheet::header(
                ui,
                Icon::ImageTool,
                "Add image",
                "A PNG or JPEG, scaled to fit and kept with your drawings",
                kit::ACCENT,
            );
            ui.add_space(18.0);
            drop_zone(ui, view);
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                if kit::push_button(ui, Some(Icon::ImageTool), "Choose file…", !view.working)
                    .clicked()
                {
                    action = ImageSheetAction::Choose;
                }
                if kit::push_button(
                    ui,
                    Some(Icon::Clipboard),
                    "Paste from clipboard",
                    !view.working,
                )
                .clicked()
                {
                    action = ImageSheetAction::Paste;
                }
            });
            ui.add_space(10.0);
            let (text, color) = match (view.working, view.error) {
                (true, _) => (Some("Reading the picture…"), theme::text_secondary()),
                (false, Some(error)) => (Some(error), kit::RED),
                (false, None) => (None, theme::text_secondary()),
            };
            if let Some(text) = text {
                ui.label(kit::text(text, size::SUBHEADLINE, REGULAR, color));
            }
        });
        ui.add_space(18.0);
        sheet::footer(ui, |ui| {
            if kit::sheet_button(ui, "Cancel", false, true).clicked() {
                action = ImageSheetAction::Close;
            }
        });
    });
    if !view.working && ctx.input(|input| input.key_pressed(Key::Escape)) {
        action = ImageSheetAction::Close;
    }
    action
}

/// What the user asked of the link sheet this frame.
#[derive(Clone, Debug, PartialEq)]
pub enum LinkSheetAction {
    None,
    /// Use this link.
    Confirm(Link),
    /// Close the sheet and put the tool down.
    Close,
}

/// What the link sheet shows, and the text the user is typing, which it edits.
pub struct LinkSheet<'a> {
    pub kind: LinkKind,
    pub text: &'a mut String,
    /// The sheet replaces the link of a drawing rather than starting a new one.
    pub replacing: bool,
}

/// The link a kind accepts, or why not. Empty text is not an error to show yet.
pub fn check_link(kind: LinkKind, text: &str) -> Result<Link, Option<LinkError>> {
    if text.trim().is_empty() {
        return Err(None);
    }
    match kind {
        LinkKind::Post => Link::parse_post(text),
        LinkKind::Idea => Link::parse_idea(text),
    }
    .map_err(Some)
}

/// Shows the sheet that asks for the link of a Post or an Idea. The link is checked as it is
/// typed; nothing is contacted.
pub fn show_link(ctx: &egui::Context, view: LinkSheet) -> LinkSheetAction {
    let mut action = LinkSheetAction::None;
    let (title, subtitle, hint) = match (view.kind, view.replacing) {
        (LinkKind::Post, false) => (
            "Add post",
            "Paste the link of a post on X. It is placed at the time it was posted.",
            "https://x.com/name/status/…",
        ),
        (LinkKind::Idea, false) => (
            "Add idea",
            "Paste the link of a published TradingView idea.",
            "https://www.tradingview.com/chart/…",
        ),
        (LinkKind::Post, true) => (
            "Replace link",
            "Paste the link of another post on X.",
            "https://x.com/name/status/…",
        ),
        (LinkKind::Idea, true) => (
            "Replace link",
            "Paste the link of another TradingView idea.",
            "https://www.tradingview.com/chart/…",
        ),
    };
    let checked = check_link(view.kind, view.text);
    sheet::show(ctx, Id::new("link-sheet"), WIDTH, |ui| {
        ui.add_space(f32::from(sheet::INSET));
        sheet::inset(ui, |ui| {
            let icon = match view.kind {
                LinkKind::Post => Icon::XLogo,
                LinkKind::Idea => Icon::Lightbulb,
            };
            sheet::header(ui, icon, title, subtitle, kit::ACCENT);
            ui.add_space(18.0);
            let field = Id::new("link-sheet-field");
            let focused = ui.memory(|memory| memory.has_focus(field));
            kit::field_frame(focused).show(ui, |ui| {
                let response = ui.add(
                    egui::TextEdit::singleline(view.text)
                        .id(field)
                        .frame(egui::Frame::NONE)
                        .hint_text(hint)
                        .char_limit(crate::model::link_card::MAX_URL_BYTES)
                        .desired_width(f32::INFINITY),
                );
                if !response.has_focus() && !ui.memory(|memory| memory.has_focus(field)) {
                    response.request_focus();
                }
            });
            ui.add_space(10.0);
            let (text, color) = match &checked {
                Ok(link) => (
                    format!("{} · {}", link.kind().service(), link.headline()),
                    theme::text_secondary(),
                ),
                Err(Some(error)) => (error.message().to_owned(), kit::RED),
                Err(None) => (
                    "Nothing is contacted: the card shows what the link says.".to_owned(),
                    theme::text_secondary(),
                ),
            };
            ui.label(kit::text(&text, size::SUBHEADLINE, REGULAR, color));
        });
        ui.add_space(18.0);
        sheet::footer(ui, |ui| {
            let label = if view.replacing { "Replace" } else { "Add" };
            if kit::sheet_button(ui, label, true, checked.is_ok()).clicked()
                && let Ok(link) = &checked
            {
                action = LinkSheetAction::Confirm(link.clone());
            }
            if kit::sheet_button(ui, "Cancel", false, true).clicked() {
                action = LinkSheetAction::Close;
            }
        });
    });
    if let (LinkSheetAction::None, Ok(link)) = (&action, &checked)
        && ctx.input(|input| input.key_pressed(Key::Enter))
    {
        action = LinkSheetAction::Confirm(link.clone());
    }
    if ctx.input(|input| input.key_pressed(Key::Escape)) {
        action = LinkSheetAction::Close;
    }
    action
}

/// What the user answered to the question of fetching previews.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConsentAction {
    None,
    Allow,
    Cancel,
}

/// Asks once per service whether its link may be sent to it to fetch a preview.
pub fn show_consent(ctx: &egui::Context, kind: LinkKind) -> ConsentAction {
    let mut action = ConsentAction::None;
    sheet::show(ctx, Id::new("link-consent-sheet"), WIDTH, |ui| {
        ui.add_space(f32::from(sheet::INSET));
        sheet::inset(ui, |ui| {
            let icon = match kind {
                LinkKind::Post => Icon::XLogo,
                LinkKind::Idea => Icon::Lightbulb,
            };
            sheet::header(
                ui,
                icon,
                &format!("Fetch previews from {}?", kind.service()),
                "Only when you ask, for the one link you chose",
                kit::ACCENT,
            );
            ui.add_space(14.0);
            ui.label(kit::text(
                format!(
                    "To show its title and text, DegenView sends this link to {} and keeps the few lines it answers with on the card. Nothing else is sent: no account, no cookies, nothing about your charts.",
                    kind.fetch_host()
                ),
                size::SUBHEADLINE,
                REGULAR,
                theme::text_secondary(),
            ));
        });
        ui.add_space(18.0);
        sheet::footer(ui, |ui| {
            if kit::sheet_button(ui, "Allow", true, true).clicked() {
                action = ConsentAction::Allow;
            }
            if kit::sheet_button(ui, "Cancel", false, true).clicked() {
                action = ConsentAction::Cancel;
            }
        });
    });
    if ctx.input(|input| input.key_pressed(Key::Escape)) {
        action = ConsentAction::Cancel;
    }
    action
}

/// A dashed box that says where a picture can be dropped.
fn drop_zone(ui: &mut egui::Ui, view: &ImageSheet) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, 96.0), Sense::hover());
    let color = if view.dragging {
        kit::ACCENT
    } else {
        theme::text_tertiary()
    };
    if view.dragging {
        ui.painter()
            .rect_filled(rect, 10.0, theme::with_alpha(kit::ACCENT, 0.10));
    }
    let path = [
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
    ];
    ui.painter().extend(egui::Shape::dashed_line(
        &[path[0], path[1], path[2], path[3], path[0]],
        Stroke::new(1.0, color),
        6.0,
        4.0,
    ));
    icons::paint(
        ui.painter(),
        Rect::from_center_size(rect.center() - vec2(0.0, 16.0), vec2(26.0, 26.0)),
        Icon::ImageTool,
        color,
    );
    ui.painter().text(
        rect.center() + vec2(0.0, 18.0),
        Align2::CENTER_CENTER,
        if view.dragging {
            "Drop to add the picture"
        } else {
            "Or drop a picture on the chart"
        },
        egui::FontId::proportional(13.0),
        if view.dragging {
            kit::ACCENT
        } else {
            theme::text_secondary()
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sheet_accepts_only_the_link_of_its_own_kind() {
        let post = "https://x.com/degenview/status/1750000000000000000";
        let idea = "https://www.tradingview.com/chart/BTCUSD/AbCdEf12-A-sample-idea/";
        assert!(check_link(LinkKind::Post, post).is_ok());
        assert!(check_link(LinkKind::Idea, idea).is_ok());
        assert!(check_link(LinkKind::Post, idea).is_err());
        assert!(check_link(LinkKind::Idea, post).is_err());
        assert_eq!(
            check_link(LinkKind::Post, "  "),
            Err(None),
            "nothing typed is not an error"
        );
        assert_eq!(
            check_link(LinkKind::Post, "ftp://x.com/a/status/1"),
            Err(Some(LinkError::Scheme))
        );
    }

    fn frame(ctx: &egui::Context, events: Vec<egui::Event>, mut run: impl FnMut(&egui::Context)) {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(800.0, 600.0))),
                events,
                ..Default::default()
            },
            |ui| run(ui.ctx()),
        );
        output.textures_delta.clear();
    }

    fn enter() -> Vec<egui::Event> {
        vec![egui::Event::Key {
            key: Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }]
    }

    #[test]
    fn enter_confirms_a_good_link_and_does_nothing_for_a_bad_one() {
        let ctx = egui::Context::default();
        crate::ui::theme::install_offscreen(&ctx);
        let mut good = "https://x.com/degenview/status/1750000000000000000".to_owned();
        let mut action = LinkSheetAction::None;
        frame(&ctx, enter(), |ctx| {
            action = show_link(
                ctx,
                LinkSheet {
                    kind: LinkKind::Post,
                    text: &mut good,
                    replacing: false,
                },
            );
        });
        assert!(matches!(action, LinkSheetAction::Confirm(_)), "{action:?}");
        let mut bad = "not a link".to_owned();
        frame(&ctx, enter(), |ctx| {
            action = show_link(
                ctx,
                LinkSheet {
                    kind: LinkKind::Post,
                    text: &mut bad,
                    replacing: false,
                },
            );
        });
        assert_eq!(action, LinkSheetAction::None);
    }

    #[test]
    fn escape_closes_the_link_sheet_and_the_consent_question() {
        let ctx = egui::Context::default();
        crate::ui::theme::install_offscreen(&ctx);
        let escape = || {
            vec![egui::Event::Key {
                key: Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }]
        };
        let mut text = String::new();
        let mut action = LinkSheetAction::None;
        frame(&ctx, escape(), |ctx| {
            action = show_link(
                ctx,
                LinkSheet {
                    kind: LinkKind::Idea,
                    text: &mut text,
                    replacing: false,
                },
            );
        });
        assert_eq!(action, LinkSheetAction::Close);
        let mut answer = ConsentAction::None;
        frame(&ctx, escape(), |ctx| {
            answer = show_consent(ctx, LinkKind::Idea);
        });
        assert_eq!(answer, ConsentAction::Cancel);
        frame(&ctx, Vec::new(), |ctx| {
            answer = show_consent(ctx, LinkKind::Post);
        });
        assert_eq!(answer, ConsentAction::None, "it waits for an answer");
    }
}

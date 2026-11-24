//! Settings ▸ About (Swift `AboutSettingsView`): the app icon, name and version, the license, the
//! update channel and a copyable build summary, the project links, and the copyright line.

use crate::build_info::{BUILD_ID, VERSION};
use crate::format::civil_from_days;
use crate::host::update::{Phase, UpdateHost, UpdateView};
use crate::i18n::tr;
use crate::service::update::check::Channel;
use crate::ui::icons::{self, Icon};
use crate::ui::kit::loading::LoadingIndicator;
use crate::ui::window::{update_indicator, update_sheet};
use crate::ui::{kit, theme};
use eframe::egui::{self, Color32, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};
use std::time::{Duration, UNIX_EPOCH};

pub const NAME: &str = "DegenView";
pub const LICENSE: &str = "GPL-3.0-only";
pub const REPOSITORY_URL: &str = "https://github.com/degenview/degenview";
pub const DONATION_URL: &str = crate::ui::window::tab_bar::COFFEE_URL;
pub const DOCS_URL: &str = crate::ui::window::tab_bar::DOCS_URL;

const MAX_WIDTH: f32 = 480.0;
const ROW_HEIGHT: f32 = 44.0;
const ROW_PADDING: f32 = 14.0;
const BADGE: f32 = 26.0;
const BADGE_SPACING: f32 = 12.0;
/// How long "Copied" shows after the build summary goes to the clipboard.
const COPIED_FOR: Duration = Duration::from_millis(1_600);

/// When the running executable was built, as epoch milliseconds: its modification time, unless
/// that is older than the build itself (`build.rs` stamps it), as in a Flatpak, which installs
/// every file dated 1 Jan 1970.
fn build_time_ms() -> Option<i64> {
    let modified = std::env::current_exe()
        .ok()
        .and_then(|path| path.metadata().ok())
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .and_then(|since| i64::try_from(since.as_millis()).ok());
    pick_build_time(modified, env!("DEGENVIEW_BUILD_TIME_MS").parse().ok())
}

/// The file's time when it is at least as new as the stamp from compiling, else the stamp. A copy
/// made after the build keeps its later time; a build that never re-ran `build.rs` has an older
/// stamp than the file, so the file wins.
fn pick_build_time(modified: Option<i64>, compiled: Option<i64>) -> Option<i64> {
    match (modified, compiled) {
        (Some(modified), Some(compiled)) if modified < compiled => Some(compiled),
        (Some(modified), _) => Some(modified),
        (None, compiled) => compiled,
    }
}

/// "Oct 11, 2026" in the viewer's time zone.
pub fn date_label(ms: i64) -> String {
    let local = ms + crate::service::system::timezone::local_utc_offset_secs(ms) * 1_000;
    let (year, month, day) = civil_from_days(local.div_euclid(86_400_000));
    crate::i18n::datetime::date(year, month, day)
}

/// The header's second line: "Version 0.1.0 - Oct 11, 2026".
pub fn header_subtitle(build_ms: Option<i64>) -> String {
    match build_ms {
        Some(ms) => tr!(
            "Version {version} - {date}",
            version = VERSION,
            date = date_label(ms)
        ),
        None => tr!("Version {version}", version = VERSION),
    }
}

/// One line for bug reports: "DegenView 0.1.0 (59d2aa59f) · built Oct 11, 2026 at 02:14".
pub fn clipboard_summary(build_ms: Option<i64>) -> String {
    match build_ms {
        Some(ms) => {
            let local = ms + crate::service::system::timezone::local_utc_offset_secs(ms) * 1_000;
            let seconds = local.div_euclid(1_000).rem_euclid(86_400);
            format!(
                "{NAME} {VERSION} ({BUILD_ID}) · built {} at {:02}:{:02}",
                date_label(ms),
                seconds / 3_600,
                seconds % 3_600 / 60
            )
        }
        None => format!("{NAME} {VERSION} ({BUILD_ID})"),
    }
}

/// What the page asks of its caller.
#[derive(Debug, Default, PartialEq)]
pub struct Outcome {
    /// An installed update asks the app to quit so it can reopen as the new version.
    pub quit: bool,
    /// The user picked another update channel.
    pub channel: Option<Channel>,
}

pub fn show(ui: &mut Ui, updates: &UpdateHost) -> Outcome {
    let build = build_time_ms();
    let area = ui.max_rect();
    let mut outcome = Outcome::default();
    egui::ScrollArea::vertical()
        .id_salt("settings-about")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let width = (area.width() - 56.0).min(MAX_WIDTH);
            ui.add_space(28.0);
            ui.vertical_centered(|ui| {
                ui.allocate_ui_with_layout(
                    vec2(width, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_width(width);
                        ui.spacing_mut().item_spacing = Vec2::ZERO;
                        header(ui, build);
                        ui.add_space(20.0);
                        outcome.quit = update_status_card(ui, updates, &mut outcome.channel);
                        if updates.channel() == Channel::ReleaseCandidate {
                            ui.add_space(10.0);
                            channel_warning(ui);
                        }
                        ui.add_space(24.0);
                        group(ui, |ui| {
                            row(
                                ui,
                                Icon::DocText,
                                Color32::from_rgb(48, 176, 199),
                                tr!("License"),
                                |ui| {
                                    ui.label(kit::text(
                                        LICENSE,
                                        kit::size::SUBHEADLINE + 1.0,
                                        kit::REGULAR,
                                        theme::text_secondary(),
                                    ));
                                },
                            );
                            divider(ui);
                            row(
                                ui,
                                Icon::InfoCircle,
                                Color32::from_rgb(94, 92, 230),
                                tr!("Build Info"),
                                |ui| {
                                    copy_button(ui, build);
                                    ui.add_space(6.0);
                                    ui.label(
                                        kit::text(
                                            BUILD_ID,
                                            kit::size::CAPTION + 1.0,
                                            kit::REGULAR,
                                            theme::text_tertiary(),
                                        )
                                        .monospace(),
                                    );
                                },
                            );
                        });
                        ui.add_space(24.0);
                        group(ui, |ui| {
                            link_row(
                                ui,
                                egui::include_image!("../../../assets/brand/docs.svg"),
                                Color32::from_rgb(24, 23, 23),
                                tr!("Documentation"),
                                tr!("User guide for every feature"),
                                DOCS_URL,
                            );
                            divider(ui);
                            link_row(
                                ui,
                                egui::include_image!("../../../assets/brand/github.svg"),
                                Color32::from_rgb(24, 23, 23),
                                "GitHub",
                                tr!("Source code, issues and releases"),
                                REPOSITORY_URL,
                            );
                            divider(ui);
                            link_row(
                                ui,
                                egui::include_image!("../../../assets/brand/buymeacoffee.png"),
                                Color32::WHITE,
                                tr!("Support DegenView"),
                                tr!("Buy me a coffee. It keeps development going"),
                                DONATION_URL,
                            );
                        });
                        ui.add_space(24.0);
                        let copyright = kit::galley(
                            ui,
                            tr!("Copyright © 2026 Nico Oelgart. Licensed under GPL-3.0-only."),
                            kit::size::CAPTION + 0.5,
                            kit::REGULAR,
                            theme::text_tertiary(),
                            width,
                        );
                        let (line, _) =
                            ui.allocate_exact_size(vec2(width, copyright.size().y), Sense::hover());
                        ui.painter().galley(
                            pos2(line.center().x - copyright.size().x / 2.0, line.top()),
                            copyright,
                            theme::text_tertiary(),
                        );
                    },
                );
            });
            ui.add_space(28.0);
        });
    outcome
}

/// What the card says and how it is tinted for an update phase.
#[derive(Debug, PartialEq)]
struct Card {
    headline: String,
    detail: String,
    tint: Color32,
    glyph: Glyph,
}

#[derive(Debug, PartialEq)]
enum Glyph {
    Spinner,
    Icon(Icon),
    Mark(&'static str),
}

const NEUTRAL: Color32 = Color32::from_rgb(142, 142, 147);
const UP_TO_DATE: Color32 = Color32::from_rgb(48, 176, 112);

fn card(view: &UpdateView) -> Card {
    let (headline, detail, tint, glyph) = match &view.phase {
        Phase::Unchecked | Phase::Checking => (
            tr!("Checking for updates…").to_owned(),
            tr!("You're on version {version}.", version = VERSION),
            NEUTRAL,
            Glyph::Spinner,
        ),
        Phase::UpToDate => (
            tr!("DegenView is up to date").to_owned(),
            tr!(
                "You're on the latest version, {version}.",
                version = VERSION
            ),
            UP_TO_DATE,
            Glyph::Icon(Icon::Check),
        ),
        Phase::Available { version, note, .. } => (
            tr!("DegenView {version} is available", version = version),
            note.clone()
                .unwrap_or_else(|| tr!("You're on version {version}.", version = VERSION)),
            kit::ORANGE,
            Glyph::Icon(Icon::ArrowUp),
        ),
        Phase::Installing(progress) => (
            tr!("Updating DegenView…").to_owned(),
            progress.stage.label().to_owned(),
            kit::ORANGE,
            Glyph::Spinner,
        ),
        Phase::Ready { version } => (
            tr!("DegenView {version} is ready", version = version),
            tr!("Restart DegenView to finish updating.").to_owned(),
            kit::ORANGE,
            Glyph::Icon(Icon::Cycle),
        ),
        Phase::InstallFailed {
            version, message, ..
        } => (
            tr!("Couldn't update to DegenView {version}", version = version),
            message.clone(),
            kit::RED,
            Glyph::Mark("!"),
        ),
        Phase::CheckFailed(message) => (
            tr!("Couldn't check for updates").to_owned(),
            message.clone(),
            kit::RED,
            Glyph::Mark("!"),
        ),
    };
    Card {
        headline,
        detail,
        tint,
        glyph,
    }
}

/// The one button the card offers, if the phase calls for one.
#[derive(Debug, PartialEq)]
enum Action {
    /// Opens the sheet that lists what the update changes; installing, or for a build that cannot
    /// install, opening the release page, is that sheet's answer.
    Review,
    /// Starts the failed install of this version again: the changes were reviewed the first time.
    Install(String),
    Restart,
    CheckAgain,
}

fn primary_action(view: &UpdateView) -> Option<(&'static str, Action)> {
    match &view.phase {
        Phase::Available {
            installable: true, ..
        } => Some((tr!("Update…"), Action::Review)),
        Phase::Available { .. } => Some((tr!("View Release…"), Action::Review)),
        Phase::Ready { .. } => Some((tr!("Restart to Update"), Action::Restart)),
        Phase::InstallFailed { version, .. } => {
            Some((tr!("Try Again"), Action::Install(version.clone())))
        }
        Phase::CheckFailed(_) => Some((tr!("Try Again"), Action::CheckAgain)),
        Phase::Unchecked | Phase::Checking | Phase::UpToDate | Phase::Installing(_) => None,
    }
}

/// The release page, for the footer's "Release Notes" link.
fn release_link(view: &UpdateView) -> Option<&str> {
    match &view.phase {
        Phase::Available { url, .. } | Phase::InstallFailed { url, .. } => Some(url),
        _ => None,
    }
}

/// "just now", "12 min ago", "3 h ago".
fn ago(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    match seconds {
        0..60 => tr!("just now").to_owned(),
        60..3_600 => tr!("{minutes} min ago", minutes = seconds / 60),
        3_600..86_400 => tr!("{hours} h ago", hours = seconds / 3_600),
        _ => tr!("{days} d ago", days = seconds / 86_400),
    }
}

/// The footer's right side: when it last looked, then the way to look again. Nothing while an
/// install is under way or waiting for its restart (a new check would just be refused).
fn footer_status(view: &UpdateView) -> Option<String> {
    if matches!(view.phase, Phase::Installing(_) | Phase::Ready { .. }) {
        return None;
    }
    if view.checking {
        return Some(tr!("Checking…").to_owned());
    }
    Some(match view.checked_ago {
        Some(elapsed) => tr!("Checked {ago}", ago = ago(elapsed)),
        None => String::new(),
    })
}

/// The update card: one tinted block with the state, at most one button, and a quiet footer for
/// the rest. True when an installed update asks the app to quit so it can reopen as the new
/// version.
fn update_status_card(ui: &mut Ui, host: &UpdateHost, channel: &mut Option<Channel>) -> bool {
    let view = host.view_for(ui.ctx());
    if view.phase == Phase::Unchecked && !view.checking {
        // The first visit before the launch check ran.
        host.check_now(ui.ctx());
    }
    let busy = view.checking || matches!(view.phase, Phase::Installing(_));
    if busy {
        ui.ctx().request_repaint_after(Duration::from_millis(250));
    }
    let card = card(&view);
    let action = primary_action(&view);
    let mut quit = false;

    let status = footer_status(&view);
    // The channel can change until an install starts; after that a switch would only confuse.
    let pick_channel = !matches!(view.phase, Phase::Installing(_) | Phase::Ready { .. });
    let has_footer = status.is_some() || release_link(&view).is_some() || pick_channel;
    egui::Frame::new()
        .fill(card.tint.gamma_multiply(0.10))
        .stroke(Stroke::new(1.0, card.tint.gamma_multiply(0.28)))
        .corner_radius(14.0)
        // The footer is a slim row: it sits closer to the card's edge than the body does.
        .inner_margin(egui::Margin {
            bottom: if has_footer { 6 } else { 16 },
            ..egui::Margin::same(16)
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                badge_glyph(ui, &card);
                ui.add_space(10.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some((label, action)) = &action {
                        let clicked =
                            kit::tinted_button(ui, label, card.tint, true, false, 0.0).clicked();
                        if clicked {
                            match action {
                                Action::Review => {
                                    host.review(ui.ctx().viewport_id());
                                    ui.ctx().request_repaint();
                                }
                                Action::Install(version) => {
                                    host.install(ui.ctx(), version);
                                }
                                Action::CheckAgain => host.check_now(ui.ctx()),
                                Action::Restart => quit = host.restart().is_ok(),
                            }
                        }
                        ui.add_space(12.0);
                    }
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.label(kit::text(
                            card.headline.as_str(),
                            kit::size::BODY + 1.0,
                            kit::SEMIBOLD,
                            theme::text_primary(),
                        ));
                        ui.add(
                            egui::Label::new(kit::text(
                                card.detail.as_str(),
                                kit::size::SUBHEADLINE + 1.0,
                                kit::REGULAR,
                                theme::text_secondary(),
                            ))
                            .wrap(),
                        );
                    });
                });
            });
            if let Phase::Installing(progress) = &view.phase {
                install_progress(ui, progress.fraction, card.tint);
            }
            if has_footer {
                footer(
                    ui,
                    host,
                    &view,
                    &card,
                    status.as_deref(),
                    pick_channel.then_some(channel),
                );
            }
        });
    update_sheet::show_review(ui.ctx(), host, &view);
    quit
}

/// The 44 pt round badge: a spinner while busy, otherwise the state's glyph.
fn badge_glyph(ui: &mut Ui, card: &Card) {
    let (badge, _) = ui.allocate_exact_size(Vec2::splat(44.0), Sense::hover());
    ui.painter().circle_filled(badge.center(), 22.0, card.tint);
    match &card.glyph {
        Glyph::Spinner => {
            ui.scope_builder(egui::UiBuilder::new().max_rect(badge), |ui| {
                ui.centered_and_justified(|ui| {
                    // White, like the glyphs the other states put on this disc.
                    ui.add(LoadingIndicator::new().size(26.0).color(Color32::WHITE));
                });
            });
        }
        Glyph::Icon(icon) => icons::paint(
            ui.painter(),
            Rect::from_center_size(badge.center(), Vec2::splat(20.0)),
            *icon,
            Color32::WHITE,
        ),
        Glyph::Mark(mark) => {
            ui.painter().text(
                badge.center(),
                egui::Align2::CENTER_CENTER,
                mark,
                egui::FontId::proportional(22.0),
                Color32::WHITE,
            );
        }
    }
}

/// The install's bar, with the percent and the stage's name beneath it.
fn install_progress(ui: &mut Ui, fraction: f32, tint: Color32) {
    ui.add_space(14.0);
    let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), 6.0), Sense::hover());
    update_indicator::paint_progress(ui.painter(), bar, fraction, tint, bar.height() / 2.0);
    ui.add_space(6.0);
    let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), 14.0), Sense::hover());
    let percent = kit::galley(
        ui,
        &format!("{}%", update_indicator::percent(fraction)),
        kit::size::CAPTION + 0.5,
        kit::MEDIUM,
        theme::text_secondary(),
        line.width(),
    );
    ui.painter().galley(
        pos2(line.right() - percent.size().x, line.top()),
        percent,
        theme::text_secondary(),
    );
}

/// A hairline, then the channel picker and "Release Notes" on the left and the check status on the
/// right. `channel` is where a pick is reported; `None` leaves the picker out.
fn footer(
    ui: &mut Ui,
    host: &UpdateHost,
    view: &UpdateView,
    card: &Card,
    status: Option<&str>,
    channel: Option<&mut Option<Channel>>,
) {
    ui.add_space(8.0);
    let (rule, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().hline(
        rule.x_range(),
        rule.center().y,
        Stroke::new(1.0, card.tint.gamma_multiply(0.22)),
    );
    ui.add_space(4.0);
    let room = ui.available_width();
    // The parts on the left, in order, and how wide each is in the language in use.
    let mut parts = Vec::new();
    let mut widths = Vec::new();
    if channel.is_some() {
        parts.push(Part::Picker);
        widths.push(kit::segments_width(ui, &channel_segments()));
    }
    if let Some(url) = release_link(view) {
        parts.push(Part::Link(url));
        widths.push(text_link_width(ui, tr!("Release Notes")));
    }
    let status_width = status.map(|status| check_status_width(ui, status));
    let mut picked = channel;
    for row in footer_rows(&widths, status_width, room) {
        // A fixed height: a right-to-left layout would otherwise take whatever is left of the page.
        ui.allocate_ui_with_layout(
            vec2(room, 24.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                // The gaps between the parts are the ones `footer_rows` counted: nothing is added to them.
                ui.scope(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    for (place, part) in parts[row.left.clone()].iter().enumerate() {
                        if place > 0 {
                            ui.add_space(PART_GAP);
                        }
                        match part {
                            Part::Picker => {
                                let mut choice = host.channel();
                                kit::segmented(
                                    ui,
                                    egui::Id::new("about-update-channel"),
                                    &Channel::ALL.map(|channel| (channel, channel.label())),
                                    &mut choice,
                                );
                                if choice != host.channel()
                                    && let Some(picked) = picked.as_deref_mut()
                                {
                                    *picked = Some(choice);
                                }
                            }
                            Part::Link(url) => {
                                if text_link(
                                    ui,
                                    tr!("Release Notes"),
                                    tr!("Open the release page on GitHub"),
                                )
                                .clicked()
                                {
                                    ui.ctx().open_url(egui::OpenUrl::new_tab(*url));
                                }
                            }
                        }
                    }
                });
                if let Some(status) = status.filter(|_| row.status) {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        check_status(ui, host, view, status);
                    });
                }
            },
        );
    }
}

/// What sits on the left of the footer.
enum Part<'a> {
    /// The update channel picker.
    Picker,
    /// "Release Notes", with the address it opens.
    Link(&'a str),
}

/// The room between two parts on the left of the footer, and the least between them and the status.
const PART_GAP: f32 = 14.0;
const CHECK_GAP: f32 = 12.0;

/// A row of the footer: which of its left parts (the picker, then the link) it holds, and whether the
/// check status sits at its right end.
#[derive(Clone, Debug, PartialEq)]
struct FooterRow {
    left: std::ops::Range<usize>,
    status: bool,
}

/// Flows the footer's parts into rows by how wide each is in the language in use: the left parts
/// (`left`, in order) share a row while they fit in `room`, and the status goes at the right end of the
/// last of them when it fits there, else on a row of its own. Nothing is placed past `room`.
fn footer_rows(left: &[f32], status: Option<f32>, room: f32) -> Vec<FooterRow> {
    let mut rows = Vec::new();
    let mut start = 0;
    let mut used = 0.0_f32;
    for (index, width) in left.iter().enumerate() {
        if index > start && used + PART_GAP + width > room {
            rows.push(FooterRow {
                left: start..index,
                status: false,
            });
            start = index;
            used = *width;
        } else {
            used = if index == start {
                *width
            } else {
                used + PART_GAP + width
            };
        }
    }
    let beside = !left.is_empty() && status.is_some_and(|width| used + CHECK_GAP + width <= room);
    if !left.is_empty() {
        rows.push(FooterRow {
            left: start..left.len(),
            status: beside,
        });
    }
    if status.is_some() && !beside {
        rows.push(FooterRow {
            left: left.len()..left.len(),
            status: true,
        });
    }
    rows
}

fn channel_segments() -> [kit::Segment<'static, Channel>; 2] {
    Channel::ALL.map(|channel| kit::Segment::new(channel, channel.label()))
}

/// How wide [`check_status`] is: the refresh icon and, when there is one, the words and their gap.
fn check_status_width(ui: &Ui, status: &str) -> f32 {
    let words = if status.is_empty() {
        0.0
    } else {
        2.0 + ui.spacing().item_spacing.x + check_status_galley(ui, status).size().x
    };
    22.0 + ui.spacing().item_spacing.x + words
}

fn check_status_galley(ui: &Ui, status: &str) -> std::sync::Arc<egui::Galley> {
    kit::galley(
        ui,
        status,
        kit::size::CAPTION + 1.0,
        kit::REGULAR,
        theme::text_secondary(),
        f32::INFINITY,
    )
}

/// The refresh icon with the words of the last check to its left. Right to left: the icon is the
/// rightmost. While a check runs it stays, dimmed, so the row does not shift.
fn check_status(ui: &mut Ui, host: &UpdateHost, view: &UpdateView, status: &str) {
    if refresh_button(ui, view.checking).clicked() {
        host.check_now(ui.ctx());
    }
    if !status.is_empty() {
        ui.add_space(2.0);
        ui.label(kit::text(
            status,
            kit::size::CAPTION + 1.0,
            kit::REGULAR,
            theme::text_secondary(),
        ));
    }
}

/// The caution under the update card while the Release Candidate channel is on: an amber panel
/// with a triangle, a short title and the plain-language reason.
fn channel_warning(ui: &mut Ui) {
    egui::Frame::new()
        .fill(kit::ORANGE.gamma_multiply(0.10))
        .stroke(Stroke::new(1.0, kit::ORANGE.gamma_multiply(0.34)))
        .corner_radius(10.0)
        .inner_margin(egui::Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                let (glyph, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                icons::paint(
                    ui.painter(),
                    Rect::from_center_size(glyph.center(), Vec2::splat(16.0)),
                    Icon::Triangle,
                    kit::ORANGE,
                );
                ui.add_space(10.0);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 3.0;
                    ui.label(kit::text(
                        tr!("Pre-release channel"),
                        kit::size::SUBHEADLINE + 1.0,
                        kit::SEMIBOLD,
                        theme::text_primary(),
                    ));
                    ui.add(
                        egui::Label::new(kit::text(
                            tr!("Release candidates preview the next version and may be less stable."),
                            kit::size::SUBHEADLINE + 1.0,
                            kit::REGULAR,
                            theme::text_secondary(),
                        ))
                        .wrap(),
                    );
                });
            });
        });
}

/// The footer's "check again": a small refresh icon, accent coloured on hover. `busy` dims it and
/// ignores clicks while a check is already running.
fn refresh_button(ui: &mut Ui, busy: bool) -> egui::Response {
    let sense = if busy { Sense::hover() } else { Sense::click() };
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(22.0), sense);
    let response = crate::ui::accessibility::label(response, tr!("Check Again"))
        .on_hover_text(tr!("Look for a newer version"));
    let hot = !busy && response.hovered();
    if hot {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        ui.painter()
            .rect_filled(rect, 6.0, theme::selected_tint(false));
    }
    let color = if busy {
        theme::text_tertiary()
    } else if hot {
        theme::text_primary()
    } else {
        theme::text_secondary()
    };
    icons::paint(
        ui.painter(),
        Rect::from_center_size(rect.center(), Vec2::splat(13.0)),
        Icon::Cycle,
        color,
    );
    response
}

fn link_galley(ui: &Ui, label: &str) -> std::sync::Arc<egui::Galley> {
    kit::galley(
        ui,
        label,
        kit::size::CAPTION + 1.0,
        kit::MEDIUM,
        Color32::PLACEHOLDER,
        f32::INFINITY,
    )
}

/// How wide [`text_link`] is for `label`.
fn text_link_width(ui: &Ui, label: &str) -> f32 {
    link_galley(ui, label).size().x
}

/// A small text-only button: accent coloured, underlined on hover.
fn text_link(ui: &mut Ui, label: &str, hint: &str) -> egui::Response {
    let galley = link_galley(ui, label);
    let (rect, response) =
        ui.allocate_exact_size(vec2(galley.size().x, galley.size().y + 4.0), Sense::click());
    let response = crate::ui::accessibility::label(response, label)
        .on_hover_text(hint)
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let color = if response.is_pointer_button_down_on() {
        kit::ACCENT.gamma_multiply(0.7)
    } else {
        kit::ACCENT
    };
    let top = rect.center().y - galley.size().y / 2.0;
    if response.hovered() {
        let y = top + galley.size().y;
        ui.painter()
            .hline(rect.x_range(), y, Stroke::new(1.0, color));
    }
    ui.painter().galley(pos2(rect.left(), top), galley, color);
    response
}

fn header(ui: &mut Ui, build: Option<i64>) {
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 72.0), Sense::hover());
    let icon = Rect::from_min_size(row.min, Vec2::splat(72.0));
    ui.painter().add(
        egui::Shadow {
            offset: [0, 3],
            blur: 14,
            spread: 0,
            color: Color32::from_black_alpha(56),
        }
        .as_shape(icon.shrink(8.0), 14.0),
    );
    // Rounded like a macOS app icon.
    egui::Image::new(egui::include_image!("../../../assets/brand/app_icon.png"))
        .fit_to_exact_size(icon.size())
        .corner_radius(16.0)
        .paint_at(ui, icon);
    let left = icon.right() + 16.0;
    let name = kit::galley(
        ui,
        NAME,
        28.0,
        kit::BOLD,
        theme::text_primary(),
        row.right() - left,
    );
    let subtitle = kit::galley(
        ui,
        &header_subtitle(build),
        kit::size::SUBHEADLINE + 1.0,
        kit::REGULAR,
        theme::text_secondary(),
        row.right() - left,
    );
    let top = row.center().y - (name.size().y + 3.0 + subtitle.size().y) / 2.0;
    let name_height = name.size().y;
    ui.painter()
        .galley(pos2(left, top), name, theme::text_primary());
    ui.painter().galley(
        pos2(left, top + name_height + 3.0),
        subtitle,
        theme::text_secondary(),
    );
}

/// A rounded, inset-list card holding rows separated by dividers.
fn group(ui: &mut Ui, content: impl FnOnce(&mut Ui)) {
    kit::card_frame(false)
        .inner_margin(egui::Margin::ZERO)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            content(ui);
        });
}

/// The hairline between rows, inset to where their text begins.
fn divider(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().hline(
        (rect.left() + ROW_PADDING + BADGE + BADGE_SPACING)..=rect.right(),
        rect.center().y,
        Stroke::new(1.0, kit::hairline()),
    );
}

fn badge(ui: &Ui, rect: Rect, tint: Color32) {
    ui.painter().rect_filled(rect, 7.0, tint);
    ui.painter().rect_filled(
        Rect::from_min_max(rect.min, pos2(rect.max.x, rect.center().y)),
        egui::CornerRadius {
            nw: 7,
            ne: 7,
            sw: 0,
            se: 0,
        },
        Color32::from_white_alpha(28),
    );
}

/// One line: a tinted badge, a title, and `trailing` at the right.
fn row(ui: &mut Ui, icon: Icon, tint: Color32, title: &str, trailing: impl FnOnce(&mut Ui)) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::hover());
    let tile = Rect::from_min_size(
        pos2(rect.left() + ROW_PADDING, rect.center().y - BADGE / 2.0),
        Vec2::splat(BADGE),
    );
    badge(ui, tile, tint);
    icons::paint(
        ui.painter(),
        Rect::from_center_size(tile.center(), Vec2::splat(13.0)),
        icon,
        Color32::WHITE,
    );
    let label = kit::galley(
        ui,
        title,
        kit::size::SUBHEADLINE + 1.0,
        kit::REGULAR,
        theme::text_primary(),
        200.0,
    );
    ui.painter().galley(
        pos2(
            tile.right() + BADGE_SPACING,
            rect.center().y - label.size().y / 2.0,
        ),
        label,
        theme::text_primary(),
    );
    let mut right = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(vec2(ROW_PADDING, 0.0)))
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    trailing(&mut right);
}

/// "Copy" the build summary for a bug report; it reads "Copied" for a moment after.
fn copy_button(ui: &mut Ui, build: Option<i64>) {
    let copied_at = egui::Id::new("about-copied-at");
    let now = ui.input(|input| input.time);
    let copied = ui
        .data(|data| data.get_temp::<f64>(copied_at))
        .is_some_and(|at| now - at < COPIED_FOR.as_secs_f64());
    let (icon, label, color) = if copied {
        (Icon::Check, tr!("Copied"), kit::GREEN)
    } else {
        (Icon::CopyDoc, tr!("Copy"), kit::ACCENT)
    };
    let text = kit::galley(
        ui,
        label,
        kit::size::CAPTION + 0.5,
        kit::MEDIUM,
        color,
        80.0,
    );
    let size = vec2(12.0 + 5.0 + text.size().x + 12.0, 22.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let response = crate::ui::accessibility::label(response, tr!("Copy build info"))
        .on_hover_text(tr!("Copy the version and build date for a bug report"))
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.hovered() {
        ui.painter().rect_filled(rect, 5.0, kit::primary(0.06));
    }
    icons::paint(
        ui.painter(),
        Rect::from_min_size(
            pos2(rect.left() + 6.0, rect.center().y - 6.0),
            Vec2::splat(12.0),
        ),
        icon,
        color,
    );
    ui.painter().galley(
        pos2(rect.left() + 23.0, rect.center().y - text.size().y / 2.0),
        text,
        color,
    );
    if response.clicked() {
        ui.ctx().copy_text(clipboard_summary(build));
        ui.data_mut(|data| data.insert_temp(copied_at, now));
        ui.ctx().request_repaint_after(COPIED_FOR);
    } else if copied {
        ui.ctx().request_repaint_after(Duration::from_millis(200));
    }
}

/// A whole-row link: the brand logo on its tile, title and hint, highlighted on hover.
fn link_row(
    ui: &mut Ui,
    logo: egui::ImageSource<'static>,
    tile: Color32,
    title: &str,
    hint: &str,
    url: &str,
) {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT + 6.0), Sense::click());
    let response = crate::ui::accessibility::label(
        response,
        tr!("{title}, {hint}", title = title, hint = hint),
    )
    .on_hover_text(url)
    .on_hover_cursor(egui::CursorIcon::PointingHand);
    let hovered = response.hovered();
    if hovered {
        ui.painter().rect_filled(rect, 0.0, theme::quaternary(0.7));
    }
    let badge_rect = Rect::from_min_size(
        pos2(rect.left() + ROW_PADDING, rect.center().y - BADGE / 2.0),
        Vec2::splat(BADGE),
    );
    ui.painter().rect_filled(badge_rect, 7.0, tile);
    ui.painter().rect_stroke(
        badge_rect,
        7.0,
        Stroke::new(0.5, kit::primary(0.2)),
        StrokeKind::Inside,
    );
    egui::Image::new(logo)
        .fit_to_exact_size(badge_rect.shrink(4.0).size())
        .paint_at(ui, badge_rect.shrink(4.0));
    let left = badge_rect.right() + BADGE_SPACING;
    let width = rect.right() - 40.0 - left;
    let name = kit::galley(
        ui,
        title,
        kit::size::SUBHEADLINE + 1.0,
        kit::REGULAR,
        theme::text_primary(),
        width,
    );
    let detail = kit::galley(
        ui,
        hint,
        kit::size::CAPTION + 0.5,
        kit::REGULAR,
        theme::text_secondary(),
        width,
    );
    let top = rect.center().y - (name.size().y + 1.0 + detail.size().y) / 2.0;
    let name_height = name.size().y;
    ui.painter()
        .galley(pos2(left, top), name, theme::text_primary());
    ui.painter().galley(
        pos2(left, top + name_height + 1.0),
        detail,
        theme::text_secondary(),
    );
    icons::paint(
        ui.painter(),
        Rect::from_center_size(
            pos2(rect.right() - ROW_PADDING - 5.0, rect.center().y),
            Vec2::splat(10.0),
        ),
        Icon::ArrowUpRight,
        if hovered {
            theme::text_primary()
        } else {
            theme::text_tertiary()
        },
    );
    if response.clicked() {
        ui.ctx().open_url(egui::OpenUrl::new_tab(url));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn a_file_dated_before_the_build_gives_way_to_the_stamp() {
        let built = 1_791_684_840_000;
        // A Flatpak install: every file is dated the epoch.
        assert_eq!(pick_build_time(Some(0), Some(built)), Some(built));
        // The usual case, and a copy made later, keep the file's time.
        assert_eq!(
            pick_build_time(Some(built + 5), Some(built)),
            Some(built + 5)
        );
        // A stale stamp (build.rs did not re-run) never wins over a newer file.
        assert_eq!(pick_build_time(Some(built), Some(built - 9)), Some(built));
        assert_eq!(pick_build_time(None, Some(built)), Some(built));
        assert_eq!(pick_build_time(Some(built), None), Some(built));
        assert_eq!(pick_build_time(None, None), None);
    }

    #[test]
    fn the_version_lines_read_like_swift() {
        // 2026-10-11 02:14 UTC; tests run in UTC.
        let ms = 1_791_684_840_000;
        assert_eq!(date_label(ms), "Oct 11, 2026");
        assert_eq!(
            header_subtitle(Some(ms)),
            format!("Version {VERSION} - Oct 11, 2026")
        );
        assert_eq!(header_subtitle(None), format!("Version {VERSION}"));
        assert_eq!(
            clipboard_summary(None),
            format!("DegenView {VERSION} ({BUILD_ID})")
        );
        assert_eq!(
            clipboard_summary(Some(ms)),
            format!("DegenView {VERSION} ({BUILD_ID}) · built Oct 11, 2026 at 02:14")
        );
    }

    #[test]
    fn the_page_draws_headlessly() {
        let ctx = egui::Context::default();
        egui_extras::install_image_loaders(&ctx);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            assert_eq!(show(ui, &UpdateHost::default()), Outcome::default());
        });
        output.textures_delta.clear();
    }

    fn view(phase: Phase) -> UpdateView {
        UpdateView {
            phase,
            checking: false,
            checked_ago: Some(Duration::from_secs(5)),
        }
    }

    fn available(installable: bool) -> Phase {
        Phase::Available {
            version: "9.9.9".to_owned(),
            url: "https://example.test/r".to_owned(),
            installable,
            note: (!installable).then(|| "Download it manually.".to_owned()),
            notes: Arc::default(),
            coverage: Default::default(),
        }
    }

    #[test]
    fn an_available_update_offers_exactly_one_button() {
        let installable = view(available(true));
        assert_eq!(
            primary_action(&installable),
            Some(("Update…", Action::Review))
        );
        // Release Notes moves to the footer, so it isn't a second button.
        assert_eq!(release_link(&installable), Some("https://example.test/r"));

        // A build that cannot install gets the same review: the sheet's View Release opens the page.
        let manual = view(available(false));
        assert_eq!(
            primary_action(&manual),
            Some(("View Release…", Action::Review))
        );
        assert_eq!(release_link(&manual), Some("https://example.test/r"));
        assert_eq!(card(&manual).detail, "Download it manually.");
    }

    #[test]
    fn busy_and_settled_phases_offer_no_button() {
        for phase in [
            Phase::Unchecked,
            Phase::Checking,
            Phase::UpToDate,
            Phase::Installing(crate::service::update::install::Progress::new(
                crate::service::update::install::Stage::Downloading,
            )),
        ] {
            assert_eq!(primary_action(&view(phase)), None);
        }
        assert_eq!(
            primary_action(&view(Phase::Ready {
                version: "9.9.9".to_owned()
            })),
            Some(("Restart to Update", Action::Restart))
        );
        assert_eq!(
            primary_action(&view(Phase::CheckFailed("offline".to_owned()))),
            Some(("Try Again", Action::CheckAgain))
        );
    }

    #[test]
    fn the_channel_warning_draws_headlessly() {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), channel_warning);
        output.textures_delta.clear();
    }

    #[test]
    fn the_card_says_what_happened() {
        let failed = card(&view(Phase::InstallFailed {
            version: "9.9.9".to_owned(),
            url: String::new(),
            message: "Checksum mismatch.".to_owned(),
        }));
        assert_eq!(failed.headline, "Couldn't update to DegenView 9.9.9");
        assert_eq!(failed.detail, "Checksum mismatch.");
        assert_eq!(failed.tint, kit::RED);
        assert_eq!(card(&view(Phase::UpToDate)).tint, UP_TO_DATE);
        assert_eq!(card(&view(Phase::Checking)).glyph, Glyph::Spinner);
    }

    #[test]
    fn the_footer_reports_the_last_check_and_hides_while_installing() {
        assert_eq!(
            footer_status(&view(Phase::UpToDate)).as_deref(),
            Some("Checked just now")
        );
        let mut checking = view(available(true));
        checking.checking = true;
        assert_eq!(footer_status(&checking).as_deref(), Some("Checking…"));
        assert_eq!(
            footer_status(&view(Phase::Ready {
                version: "9.9.9".to_owned()
            })),
            None
        );
        assert_eq!(ago(Duration::from_secs(59)), "just now");
        assert_eq!(ago(Duration::from_secs(12 * 60)), "12 min ago");
        assert_eq!(ago(Duration::from_secs(3 * 3_600)), "3 h ago");
        assert_eq!(ago(Duration::from_secs(2 * 86_400)), "2 d ago");
    }

    fn press(key: egui::Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    /// The About card over `host`, one frame per entry of `frames`, each with its events. True
    /// when any frame asked the platform to open a page.
    fn run_card(
        screen: &mut crate::ui::render_tests::Screen,
        host: &UpdateHost,
        frames: Vec<Vec<egui::Event>>,
    ) -> bool {
        let mut opened_page = false;
        for events in frames {
            let output = screen.run(events, |ui| {
                update_status_card(ui, host, &mut None);
            });
            opened_page |= output
                .platform_output
                .commands
                .iter()
                .any(|command| matches!(command, egui::OutputCommand::OpenUrl(_)));
        }
        opened_page
    }

    #[test]
    fn the_return_that_presses_update_does_not_also_confirm_the_sheet() {
        let host = UpdateHost::at_phase(&available(true));
        let mut screen = crate::ui::render_tests::Screen::new(Vec2::new(700.0, 760.0));
        let root = egui::ViewportId::ROOT;
        // The keyboard reaches the button, then one Return presses it and opens the sheet.
        run_card(
            &mut screen,
            &host,
            vec![vec![], vec![press(egui::Key::Tab)], vec![]],
        );
        run_card(&mut screen, &host, vec![vec![press(egui::Key::Enter)]]);
        assert!(
            host.review_open_in(root),
            "the button opened the sheet: {:?}",
            host.view().phase
        );
        assert!(
            matches!(host.view().phase, Phase::Available { .. }),
            "the same key must not start the install: {:?}",
            host.view().phase
        );
        run_card(&mut screen, &host, vec![vec![]; 4]);
        assert!(matches!(host.view().phase, Phase::Available { .. }));
        // A fresh Return, with the sheet already showing, confirms.
        run_card(&mut screen, &host, vec![vec![press(egui::Key::Enter)]]);
        assert!(matches!(host.view().phase, Phase::Installing(_)));
    }

    #[test]
    fn a_manual_update_opens_the_sheet_instead_of_the_browser() {
        let host = UpdateHost::at_phase(&available(false));
        let mut screen = crate::ui::render_tests::Screen::new(Vec2::new(700.0, 760.0));
        run_card(
            &mut screen,
            &host,
            vec![vec![], vec![press(egui::Key::Tab)], vec![]],
        );
        let opened_page = run_card(&mut screen, &host, vec![vec![press(egui::Key::Enter)]]);
        assert!(host.review_open_in(egui::ViewportId::ROOT));
        assert!(!opened_page, "only the sheet's View Release opens the page");
    }

    #[test]
    fn every_card_state_draws_headlessly() {
        let ctx = egui::Context::default();
        egui_extras::install_image_loaders(&ctx);
        let progress = crate::service::update::install::Progress {
            stage: crate::service::update::install::Stage::Downloading,
            fraction: 0.4,
        };
        for phase in [
            Phase::Unchecked,
            Phase::Checking,
            Phase::UpToDate,
            Phase::CheckFailed("GitHub is rate limiting requests.".to_owned()),
            available(true),
            available(false),
            Phase::Installing(progress),
            Phase::Ready {
                version: "9.9.9".to_owned(),
            },
            Phase::InstallFailed {
                version: "9.9.9".to_owned(),
                url: "https://example.test/r".to_owned(),
                message: "The download does not match its published checksum.".to_owned(),
            },
        ] {
            let host = UpdateHost::at_phase(&phase);
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                let mut channel = None;
                assert!(!update_status_card(ui, &host, &mut channel), "{phase:?}");
                assert_eq!(channel, None);
            });
            output.textures_delta.clear();
        }
    }

    /// The picker and the words of the last check share a row only when both fit on it.
    #[test]
    fn the_footer_never_prints_a_part_over_another_or_past_its_row() {
        use crate::i18n::{Locale, with_locale};
        use crate::ui::render_tests::Screen;
        use crate::ui::trade::pseudo_text::{overlapping, painted, painted_boxes};
        with_locale(Locale::PSEUDO, || {
            let host = UpdateHost::default();
            // Up to date has no link; an available update and a failed install show "Release Notes".
            for phase in [Phase::UpToDate, available(true)] {
                let view = view(phase);
                let card = card(&view);
                let status = footer_status(&view).expect("a status");
                // Every width the card can have: each part fits beside the others in some and not
                // in others.
                for width in (300..=900).step_by(10).map(|width| width as f32) {
                    let mut screen = Screen::new(vec2(width, 300.0));
                    let mut last = None;
                    for _ in 0..3 {
                        last = Some(screen.run(Vec::new(), |ui| {
                            let mut picked = None;
                            footer(ui, &host, &view, &card, Some(&status), Some(&mut picked));
                        }));
                    }
                    let output = last.expect("a frame");
                    let words = painted(&output);
                    assert!(words.contains(&status), "{width}: the status is shown");
                    assert_eq!(
                        words.contains(&tr!("Release Notes").to_owned()),
                        release_link(&view).is_some(),
                        "{width}: the link is shown when there is one"
                    );
                    let overlapping = overlapping(&output);
                    assert!(overlapping.is_empty(), "{width}: {overlapping:?}");
                    for (text, rect) in painted_boxes(&output) {
                        // The picker alone is wider than the narrowest widths: it is not wrapped.
                        if text.contains("Çåñðîðåţé") {
                            continue;
                        }
                        assert!(
                            rect.right() <= width,
                            "{width}: {text:?} runs past: {rect:?}"
                        );
                    }
                }
            }
        });
    }

    #[test]
    fn the_footer_flows_its_parts_into_rows_by_their_width() {
        let row = |left: std::ops::Range<usize>, status| FooterRow { left, status };
        // Everything fits: one row, the status at its end.
        assert_eq!(
            footer_rows(&[200.0, 80.0], Some(100.0), 600.0),
            vec![row(0..2, true)]
        );
        // The status does not fit beside the parts: a row of its own below them.
        assert_eq!(
            footer_rows(&[200.0, 80.0], Some(100.0), 360.0),
            vec![row(0..2, false), row(2..2, true)]
        );
        // The link does not fit beside the picker: it starts the second row, with the status when
        // that fits there.
        assert_eq!(
            footer_rows(&[200.0, 120.0], Some(100.0), 300.0),
            vec![row(0..1, false), row(1..2, true)]
        );
        // ...and the status drops again when it does not fit beside the link either.
        assert_eq!(
            footer_rows(&[200.0, 120.0], Some(200.0), 300.0),
            vec![row(0..1, false), row(1..2, false), row(2..2, true)]
        );
        // No picker: the link and the status share a row.
        assert_eq!(
            footer_rows(&[80.0], Some(100.0), 300.0),
            vec![row(0..1, true)]
        );
        // Nothing on the left: the status alone, and nothing at all with no status.
        assert_eq!(footer_rows(&[], Some(100.0), 300.0), vec![row(0..0, true)]);
        assert_eq!(footer_rows(&[], None, 300.0), Vec::<FooterRow>::new());
        // No status: the parts alone.
        assert_eq!(
            footer_rows(&[200.0, 80.0], None, 600.0),
            vec![row(0..2, false)]
        );
        // Nothing is placed in a row past its room: every row's parts and gaps fit, except a single
        // part that is wider than the room by itself.
        for room in (100..700).step_by(7).map(|room| room as f32) {
            let widths = [180.0, 90.0];
            for row in footer_rows(&widths, Some(110.0), room) {
                if row.left.len() > 1 {
                    let used: f32 = widths[row.left.clone()].iter().sum::<f32>() + PART_GAP;
                    assert!(used <= room, "{room}: {row:?}");
                }
            }
        }
    }
}

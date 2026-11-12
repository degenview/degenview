//! The update mark at the right end of the tab strip, and the progress bar painter Settings ▸
//! About shares with it. Everything here draws an `UpdateView`; the host runs the update.

use crate::host::update_host::{Phase, UpdateView};
use crate::ui::icons::{self, Icon};
use crate::ui::{kit, theme};
use eframe::egui::{
    self, Color32, CornerRadius, Galley, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2,
};
use std::sync::Arc;

const HEIGHT: f32 = 22.0;
/// Square-ish like the tab strip's other small controls, not a capsule.
const RADIUS: f32 = 6.0;
const PADDING: f32 = 10.0;
/// The glyph inside the badge, and the badge itself (a rounded square concentric with the mark's
/// corners: `RADIUS - BADGE_INSET`).
const ICON: f32 = 10.0;
const BADGE: f32 = 16.0;
const BADGE_INSET: f32 = (HEIGHT - BADGE) / 2.0;
const ICON_GAP: f32 = 6.0;
/// A call to action breathes this many times when it appears, then holds still: a mark that
/// pulsed forever would keep the app repainting and turn into noise.
const PULSE_PERIOD: f64 = 1.6;
const PULSE_COUNT: f64 = 3.0;
/// Text and glyphs on the tint (the badge and the filled call to action): near-black with a warm
/// cast, which reads on orange and red where white does not.
const ON_TINT: Color32 = Color32::from_rgb(38, 22, 0);
/// Room between the indicator and what is to its left, and to the window's edge.
const GAP: f32 = 8.0;
const EDGE: f32 = 10.0;
/// The progress capsule keeps one width so the strip doesn't re-flow as the percent grows.
const PROGRESS_WIDTH: f32 = 104.0;

/// What the mark looks like and says for a phase; `None` for a phase with nothing to show.
#[derive(Clone, Debug, PartialEq)]
pub struct Look {
    pub label: String,
    pub tooltip: String,
    pub tint: Color32,
    pub icon: Option<Icon>,
    /// Filled with the tint (the restart call to action) rather than outlined by it.
    pub filled: bool,
    /// Overall progress, when installing: the capsule is drawn as a bar.
    pub progress: Option<f32>,
    /// Whether a click does anything.
    pub clickable: bool,
}

pub fn look(view: &UpdateView) -> Option<Look> {
    let (label, tooltip, tint, icon, filled, progress, clickable) = match &view.phase {
        Phase::Available {
            version,
            installable,
            note,
            ..
        } => (
            "Update".to_owned(),
            match (installable, note) {
                (true, _) => format!("DegenView {version} is available. Click to install it."),
                (false, Some(note)) => format!("DegenView {version} is available. {note}"),
                (false, None) => format!("DegenView {version} is available."),
            },
            kit::ORANGE,
            Some(Icon::ArrowUp),
            false,
            None,
            true,
        ),
        Phase::Installing(progress) => (
            format!("Updating {}%", percent(progress.fraction)),
            progress.stage.label().to_owned(),
            kit::ORANGE,
            None,
            false,
            Some(progress.fraction),
            false,
        ),
        Phase::Ready { version } => (
            "Restart".to_owned(),
            format!("DegenView {version} is ready. Restart to finish updating."),
            kit::ORANGE,
            Some(Icon::Cycle),
            true,
            None,
            true,
        ),
        Phase::InstallFailed { message, .. } => (
            "Update failed".to_owned(),
            format!("{message} Click to try again."),
            kit::RED,
            Some(Icon::Triangle),
            false,
            None,
            true,
        ),
        Phase::Unchecked | Phase::Checking | Phase::UpToDate | Phase::CheckFailed(_) => {
            return None;
        }
    };
    Some(Look {
        label,
        tooltip,
        tint,
        icon,
        filled,
        progress,
        clickable,
    })
}

impl Look {
    /// The glyph sits in a solid badge on an outlined mark; on a filled one it is drawn bare.
    fn badge(&self) -> bool {
        self.icon.is_some() && !self.filled && self.progress.is_none()
    }

    /// Whether the mark asks to be noticed (as opposed to reporting progress).
    fn attention(&self) -> bool {
        self.clickable && self.progress.is_none()
    }
}

/// The tint as text on the strip: as is on dark, deepened on light so it keeps its contrast.
fn text_tint(tint: Color32) -> Color32 {
    theme::pick(tint, darken(tint, 0.62))
}

/// `color` with its channels scaled by `factor`, staying opaque (`gamma_multiply` scales alpha too).
fn darken(color: Color32, factor: f32) -> Color32 {
    let scale = |channel: u8| (f32::from(channel) * factor).round() as u8;
    Color32::from_rgb(scale(color.r()), scale(color.g()), scale(color.b()))
}

/// 0 to 100, for a 0 to 1 fraction.
pub fn percent(fraction: f32) -> u32 {
    (fraction.clamp(0.0, 1.0) * 100.0).floor() as u32
}

/// The mark measured for this frame, so the strip can leave room for it before laying out tabs.
pub struct Layout {
    look: Look,
    label: Arc<Galley>,
    width: f32,
}

impl Layout {
    /// How much of the strip's right end it takes, gaps included.
    pub fn reserved(&self) -> f32 {
        self.width + GAP + EDGE
    }
}

pub fn layout(ui: &Ui, view: Option<&UpdateView>) -> Option<Layout> {
    let look = look(view?)?;
    let label = kit::galley(
        ui,
        &look.label,
        kit::size::CAPTION + 1.5,
        kit::MEDIUM,
        // Painted in `show` with the colour the phase needs.
        Color32::PLACEHOLDER,
        f32::INFINITY,
    );
    let width = if look.progress.is_some() {
        PROGRESS_WIDTH
    } else if look.badge() {
        BADGE_INSET + BADGE + ICON_GAP + label.size().x + PADDING
    } else if look.icon.is_some() {
        PADDING + ICON + ICON_GAP + label.size().x + PADDING
    } else {
        PADDING * 2.0 + label.size().x
    };
    Some(Layout { look, label, width })
}

/// Draws the mark with its right edge `EDGE` short of `right`, centred in `top..top + height`.
/// True when it was clicked.
pub fn show(ui: &mut Ui, layout: &Layout, right: f32, top: f32, height: f32) -> bool {
    let look = &layout.look;
    let rect = Rect::from_min_size(
        pos2(
            right - EDGE - layout.width,
            top + (height - HEIGHT) / 2.0 + 1.0,
        ),
        vec2(layout.width, HEIGHT),
    );
    let sense = if look.clickable {
        Sense::click()
    } else {
        Sense::hover()
    };
    let response = ui.interact(rect, egui::Id::new("update-indicator"), sense);
    let response = crate::ui::accessibility::label(response, &look.tooltip)
        .on_hover_text(&look.tooltip)
        .on_hover_cursor(if look.clickable {
            egui::CursorIcon::PointingHand
        } else {
            egui::CursorIcon::Default
        });
    let hovered = response.hovered() && look.clickable;
    let pressed = response.is_pointer_button_down_on();
    // 0 to 1, eased, so hovering fades in rather than snapping.
    let lift = ui
        .ctx()
        .animate_bool_with_time(response.id.with("hover"), hovered, 0.12);
    let painter = ui.painter();
    let rounding = CornerRadius::same(RADIUS as u8);

    if look.attention() {
        paint_pulse(ui, rect, &look.label, look.tint);
    }
    if lift > 0.0 {
        // A soft ring that appears around the mark under the pointer.
        let grow = 3.0 * lift;
        painter.rect_stroke(
            rect.expand(grow),
            RADIUS + grow,
            Stroke::new(2.0, theme::with_alpha(look.tint, 0.22 * lift)),
            StrokeKind::Inside,
        );
    }

    if let Some(fraction) = look.progress {
        paint_progress(painter, rect, fraction, look.tint, RADIUS);
    } else if look.filled {
        let fill = if pressed {
            darken(look.tint, 0.86)
        } else {
            look.tint.lerp_to_gamma(Color32::WHITE, 0.10 * lift)
        };
        painter.rect_filled(rect, rounding, fill);
        paint_sheen(painter, rect, 0.22);
    } else {
        let alpha = if pressed { 0.30 } else { 0.15 + 0.08 * lift };
        painter.rect_filled(rect, rounding, theme::with_alpha(look.tint, alpha));
        paint_sheen(painter, rect, 0.06);
    }
    painter.rect_stroke(
        rect,
        RADIUS,
        Stroke::new(1.0, border(look)),
        StrokeKind::Inside,
    );

    let ink = if look.filled {
        ON_TINT
    } else if look.progress.is_some() {
        theme::text_primary()
    } else {
        text_tint(look.tint)
    };
    let mut x = rect.left() + PADDING;
    if look.progress.is_some() {
        x = rect.center().x - layout.label.size().x / 2.0;
    } else if look.badge() {
        let badge = Rect::from_min_size(
            pos2(rect.left() + BADGE_INSET, rect.center().y - BADGE / 2.0),
            Vec2::splat(BADGE),
        );
        painter.rect_filled(badge, RADIUS - BADGE_INSET, look.tint);
        if let Some(icon) = look.icon {
            icons::paint(
                painter,
                Rect::from_center_size(badge.center(), Vec2::splat(ICON)),
                icon,
                ON_TINT,
            );
        }
        x = badge.right() + ICON_GAP;
    } else if let Some(icon) = look.icon {
        icons::paint(
            painter,
            Rect::from_center_size(pos2(x + ICON / 2.0, rect.center().y), Vec2::splat(ICON)),
            icon,
            ON_TINT,
        );
        x += ICON + ICON_GAP;
    }
    painter.galley(
        pos2(x, rect.center().y - layout.label.size().y / 2.0),
        layout.label.clone(),
        ink,
    );
    response.clicked()
}

/// The hairline around the mark: the tint itself, a little darker on the filled call to action so
/// it still reads against its fill.
fn border(look: &Look) -> Color32 {
    if look.filled {
        darken(look.tint, 0.75)
    } else {
        theme::with_alpha(look.tint, 0.55)
    }
}

/// A faint light wash over the top half of the mark, which gives the flat fill some depth.
fn paint_sheen(painter: &egui::Painter, rect: Rect, strength: f32) {
    let top = Rect::from_min_max(
        rect.min + vec2(1.0, 1.0),
        pos2(rect.max.x - 1.0, rect.center().y),
    );
    let radius = (RADIUS - 1.0) as u8;
    painter.rect_filled(
        top,
        CornerRadius {
            nw: radius,
            ne: radius,
            sw: 0,
            se: 0,
        },
        theme::with_alpha(Color32::WHITE, strength * 0.5),
    );
}

/// A ring that grows off the mark and fades, `PULSE_COUNT` times from when this look first
/// showed. Repaints only while it runs.
fn paint_pulse(ui: &Ui, rect: Rect, label: &str, tint: Color32) {
    let ctx = ui.ctx();
    let now = ui.input(|input| input.time);
    // Keyed on the label so a new phase (Update, then Restart) gets its own pulse.
    let id = egui::Id::new(("update-indicator-seen", label));
    let began = ctx.data_mut(|data| *data.get_temp_mut_or_insert_with(id, || now));
    let elapsed = now - began;
    if elapsed >= PULSE_PERIOD * PULSE_COUNT {
        return;
    }
    let phase = (elapsed / PULSE_PERIOD).fract() as f32;
    let grow = 7.0 * phase;
    let fade = (1.0 - phase).powf(1.6);
    ui.painter().rect_stroke(
        rect.expand(grow),
        RADIUS + grow,
        Stroke::new(1.5, theme::with_alpha(tint, 0.5 * fade)),
        StrokeKind::Outside,
    );
    ctx.request_repaint_after(std::time::Duration::from_millis(16));
}

/// A bar with corners of `radius`: a faint track in `tint` with the done part filled. Settings ▸
/// About draws its install progress with this too, as a capsule (`rect.height() / 2.0`).
pub fn paint_progress(
    painter: &egui::Painter,
    rect: Rect,
    fraction: f32,
    tint: Color32,
    radius: f32,
) {
    painter.rect_filled(rect, radius, theme::with_alpha(tint, 0.16));
    let done = rect.width() * fraction.clamp(0.0, 1.0);
    if done > 0.5 {
        // Never narrower than the corner radius allows, so a bar just begun keeps its corners.
        let fill = Rect::from_min_size(
            rect.min,
            vec2(done.max(radius * 2.0).min(rect.width()), rect.height()),
        );
        painter.rect_filled(fill, radius, theme::with_alpha(tint, 0.42));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::self_update::{Progress, Stage};
    use std::time::Duration;

    fn view(phase: Phase) -> UpdateView {
        UpdateView {
            phase,
            checking: false,
            checked_ago: Some(Duration::ZERO),
        }
    }

    fn available(installable: bool) -> Phase {
        Phase::Available {
            version: "1.2.3".to_owned(),
            url: "https://example.test".to_owned(),
            installable,
            note: (!installable).then(|| "Download it manually.".to_owned()),
        }
    }

    #[test]
    fn only_an_update_in_play_shows_a_mark() {
        for phase in [
            Phase::Unchecked,
            Phase::Checking,
            Phase::UpToDate,
            Phase::CheckFailed("offline".to_owned()),
        ] {
            assert_eq!(look(&view(phase)), None);
        }
        assert!(look(&view(available(true))).is_some());
    }

    #[test]
    fn each_phase_says_what_a_click_does() {
        let available = look(&view(available(true))).unwrap();
        assert_eq!(available.label, "Update");
        assert!(available.clickable && available.progress.is_none());
        assert!(available.tooltip.contains("1.2.3"));

        let manual = look(&view(self::available(false))).unwrap();
        assert!(manual.tooltip.contains("Download it manually."));

        let installing = look(&view(Phase::Installing(Progress {
            stage: Stage::Downloading,
            fraction: 0.426,
        })))
        .unwrap();
        assert_eq!(installing.label, "Updating 42%");
        assert_eq!(installing.progress, Some(0.426));
        assert!(!installing.clickable);
        assert_eq!(installing.tooltip, Stage::Downloading.label());

        let ready = look(&view(Phase::Ready {
            version: "1.2.3".to_owned(),
        }))
        .unwrap();
        assert_eq!(ready.label, "Restart");
        assert!(ready.filled && ready.clickable);

        let failed = look(&view(Phase::InstallFailed {
            version: "1.2.3".to_owned(),
            url: String::new(),
            message: "Checksum mismatch.".to_owned(),
        }))
        .unwrap();
        assert_eq!(failed.tint, kit::RED);
        assert!(failed.tooltip.starts_with("Checksum mismatch."));
    }

    #[test]
    fn percent_stays_within_bounds() {
        assert_eq!(percent(-1.0), 0);
        assert_eq!(percent(0.999), 99);
        assert_eq!(percent(1.0), 100);
        assert_eq!(percent(7.0), 100);
    }

    #[test]
    fn the_strip_reserves_room_for_the_mark_and_nothing_without_one() {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            assert!(layout(ui, None).is_none());
            assert!(layout(ui, Some(&view(Phase::UpToDate))).is_none());
            let update = layout(ui, Some(&view(available(true)))).unwrap();
            assert!(update.reserved() > GAP + EDGE);
            let progress = layout(
                ui,
                Some(&view(Phase::Installing(Progress::new(Stage::Downloading)))),
            )
            .unwrap();
            assert_eq!(progress.reserved(), PROGRESS_WIDTH + GAP + EDGE);
        });
        output.textures_delta.clear();
    }

    #[test]
    fn the_mark_draws_in_every_phase_and_reports_a_click_only_when_clicked() {
        let ctx = egui::Context::default();
        let phases = [
            available(true),
            Phase::Installing(Progress::new(Stage::Downloading)),
            Phase::Ready {
                version: "1.2.3".to_owned(),
            },
            Phase::InstallFailed {
                version: "1.2.3".to_owned(),
                url: String::new(),
                message: "Checksum mismatch.".to_owned(),
            },
        ];
        for phase in phases {
            let update = view(phase);
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                let layout = layout(ui, Some(&update)).unwrap();
                assert!(!show(ui, &layout, 800.0, 0.0, 40.0));
            });
            output.textures_delta.clear();
        }
    }

    /// The mark as pictures. Run by hand:
    /// `DEGENVIEW_GALLERY_DIR=<dir> cargo test --lib update_indicator_gallery -- --ignored`.
    #[test]
    #[ignore = "writes pictures; run by hand"]
    fn update_indicator_gallery() {
        use crate::ui::render_tests::Screen;
        let phases = [
            ("available", available(true)),
            (
                "installing",
                Phase::Installing(Progress {
                    stage: Stage::Downloading,
                    fraction: 0.42,
                }),
            ),
            (
                "ready",
                Phase::Ready {
                    version: "1.2.3".to_owned(),
                },
            ),
            (
                "failed",
                Phase::InstallFailed {
                    version: "1.2.3".to_owned(),
                    url: String::new(),
                    message: "Checksum mismatch.".to_owned(),
                },
            ),
        ];
        for (name, phase) in phases {
            for light in [false, true] {
                for (state, hover, frames) in
                    [("rest", false, 60), ("pulse", false, 3), ("hover", true, 6)]
                {
                    let mut screen = Screen::new(Vec2::new(220.0, 44.0));
                    screen.ctx.set_theme(if light {
                        egui::ThemePreference::Light
                    } else {
                        egui::ThemePreference::Dark
                    });
                    let update = view(phase.clone());
                    let mut last = None;
                    for _ in 0..frames {
                        let events = if hover {
                            vec![egui::Event::PointerMoved(pos2(150.0, 22.0))]
                        } else {
                            vec![]
                        };
                        last = Some(screen.run(events, |ui| {
                            theme::sync(ui.ctx());
                            let full = ui.max_rect();
                            ui.painter().rect_filled(full, 0.0, theme::chrome());
                            let layout = layout(ui, Some(&update)).unwrap();
                            show(ui, &layout, full.right(), 0.0, 44.0);
                        }));
                    }
                    let theme = if light { "light" } else { "dark" };
                    screen.save(&format!("update-{name}-{state}-{theme}"), last.unwrap());
                }
            }
        }
    }
}

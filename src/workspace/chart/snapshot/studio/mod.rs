//! The Screenshot Studio, as the workspace runs it: the session a selection (or a click on Full
//! chart or Entire layout) opens, the window drawn from it each frame, the actions the window
//! reports, the preview pipeline and the export.
//!
//! The window ([`window`]) is immediate mode and decides nothing; this module owns the session
//! (the frozen capture and the configuration) and applies what the window reports. The live charts
//! are only read: the cards paint the frozen capture (for their canvas, alert lines and paper
//! markers), and nothing is written to a chart, a layout or the undo stack. A session ends when
//! it is closed, when a chart it holds goes away, or with the workspace.

use super::*;
use crate::model::chart::snapshot::composition;
use crate::model::chart::snapshot::framing::{Base, Framing};
use crate::model::chart::snapshot::studio::{Area, StudioConfig};
use crate::ui::ChartCard;
use crate::ui::snapshot_studio::capture::Capture;
use crate::ui::snapshot_studio::compose::{self, ComposeError, Target};
use crate::ui::snapshot_studio::previews::{Composed, Key, Previews};
use crate::ui::snapshot_studio::window::{self, Action, Image, Input, Preview};
use eframe::egui::Vec2;

/// Areas on offer when the workspace has several charts to arrange, and when it has one.
const AREAS_WITH_LAYOUT: [Area; 3] = [Area::FullChart, Area::EntireLayout, Area::Portion];
const AREAS_ALONE: [Area; 2] = [Area::FullChart, Area::Portion];

/// A capture and the choices made about it, held by the workspace while the Studio is open.
pub(super) struct StudioSession {
    pub(super) capture: Capture,
    pub(super) config: StudioConfig,
    previews: Previews,
    /// The framing when the pointer went down on the preview: a drag is measured from it.
    drag: Option<Framing>,
}

impl StudioSession {
    /// A session on `capture`, in the look `config` says.
    pub(super) fn new(capture: Capture, config: StudioConfig) -> Self {
        Self {
            capture,
            config,
            previews: Previews::default(),
            drag: None,
        }
    }

    /// A session on what a selection captured, opened the way a portion opens: in `look` if the
    /// Studio went back to the charts to select, else in the Studio's own.
    pub(super) fn portion(capture: Capture, look: Option<StudioConfig>) -> Self {
        let config = look.map_or_else(
            || StudioConfig::new(Area::Portion),
            |look| StudioConfig {
                area: Area::Portion,
                framing: Framing::default(),
                ..look
            },
        );
        Self::new(capture, config)
    }

    /// The charts the capture holds.
    fn chart_ids(&self) -> Vec<u64> {
        match &self.capture {
            Capture::Single(chart) => vec![chart.chart_id()],
            Capture::Layout(layout) => layout
                .charts
                .iter()
                .map(|(chart, _)| chart.chart_id())
                .collect(),
        }
    }

    /// The capture's starting view and where its plot lies in the composition, for the framing's
    /// limits and for a drag: only one chart can be reframed.
    fn framing_basis(&self) -> Option<(Base, Vec2)> {
        let Capture::Single(chart) = &self.capture else {
            return None;
        };
        let composition = composition::layout(self.capture.size(), &self.config);
        let plot = composition::plot_in(&composition, chart.base(), chart.size());
        Some((*chart.base(), plot))
    }

    /// Puts the framing back inside the limits of the composition it has now: the limits move
    /// with the size, the style and the dials.
    fn settle_framing(&mut self) {
        if let Some((base, plot)) = self.framing_basis() {
            self.config.framing = self.config.framing.settle(&base, plot);
        }
    }

    /// A new capture of another area: the picture on screen waits for the next one.
    fn replace_capture(&mut self, capture: Capture, area: Area) {
        self.capture = capture;
        self.config.area = area;
        self.config.framing = Framing::default();
        self.drag = None;
        self.previews.restart();
    }
}

/// What applying an action means for the window.
#[derive(Debug, Eq, PartialEq)]
enum Next {
    Stay,
    Close,
    /// Back to the charts to select a portion.
    Select,
}

impl ChartWorkspace {
    /// Whether the Studio is open. While it is the workspace's keys and the charts' pointer and
    /// keyboard input wait, as they do under any sheet.
    pub(in crate::workspace) fn studio_open(&self) -> bool {
        self.snapshot.studio.is_some()
    }

    /// The card that paints a chart of a capture, for the composer.
    fn card_of(&self, chart_id: u64) -> Option<&ChartCard> {
        self.charts
            .iter()
            .find(|chart| chart.id == chart_id)
            .map(|chart| &chart.chart)
    }

    /// The Studio's export in progress, if any.
    fn studio_export(&self) -> Option<window::Export> {
        self.snapshot
            .running
            .as_ref()
            .and_then(|running| running.studio)
    }

    /// One frame of the Studio, when it is open: collects the picture a worker finished, draws
    /// the window, applies what the user did and asks for the picture the result needs.
    pub(in crate::workspace) fn show_studio(&mut self, ctx: &Context) {
        let Some(mut session) = self.snapshot.studio.take() else {
            return;
        };
        // A chart that went away takes the Studio with it: there is nothing left to draw from.
        if !session
            .chart_ids()
            .iter()
            .all(|id| self.chart_index(*id).is_some())
        {
            return;
        }
        self.tick_previews(ctx, &mut session);
        let scale = self.snapshot_prefs().options().scale;
        let composition = composition::layout(session.capture.size(), &session.config);
        let dimensions = composition::dimensions(&composition, scale);
        let summary = dimensions.as_ref().map_or_else(
            || Err(ComposeError::TooLarge.to_string()),
            |dimensions| {
                Ok(composition::summary(
                    session.config.area,
                    session.config.style,
                    dimensions,
                ))
            },
        );
        let failure = session.previews.failure().map(|error| error.to_string());
        let areas: &[Area] = if self.market_charts() > 1 {
            &AREAS_WITH_LAYOUT
        } else {
            &AREAS_ALONE
        };
        let image = session.previews.shown().map(|shown| Image {
            texture: shown.texture,
            pixels: shown.pixels,
            composition: shown.composition,
        });
        let output = window::show(
            ctx,
            Id::new(("snapshot-studio", self.id)),
            &Input {
                config: &session.config,
                areas,
                has_indicators: session.capture.has_indicators(),
                has_drawings: session.capture.has_drawings(),
                reframable: matches!(session.capture, Capture::Single(_)),
                preview: Preview {
                    image,
                    failure: failure.as_deref(),
                },
                summary: summary.as_deref().map_err(String::as_str),
                exporting: self.studio_export(),
                can_export: dimensions.is_some(),
            },
        );
        let mut next = Next::Stay;
        for action in output.actions {
            next = self.apply_studio_action(ctx, &mut session, action);
            if next != Next::Stay {
                break;
            }
        }
        match next {
            Next::Close => return,
            Next::Select => {
                self.begin_portion_selection();
                // A selection that could not begin (an export is running) leaves the Studio up.
                if self.is_selecting_portion() {
                    self.snapshot.kept_look = Some(session.config);
                    return;
                }
            }
            Next::Stay => {}
        }
        session.previews.want(Key {
            config: session.config,
            max_px: output.preview_pixels,
            coarse: output.interacting,
        });
        self.tick_previews(ctx, &mut session);
        self.snapshot.studio = Some(session);
    }

    /// Takes a finished picture and starts the one that is wanted.
    fn tick_previews(&self, ctx: &Context, session: &mut StudioSession) {
        let StudioSession {
            capture, previews, ..
        } = session;
        let cards = |id: u64| self.card_of(id);
        previews.tick(
            ctx,
            |key| {
                let (scene, _) = compose::scene(
                    capture,
                    &cards,
                    &key.config,
                    Target::Preview {
                        max_px: key.target_px(),
                    },
                )?;
                Ok(Composed {
                    scene,
                    composition: composition::layout(capture.size(), &key.config).size,
                })
            },
            |scene| work_queue::SNAPSHOTS.start(Some(ctx), move || raster::rasterize(&scene)),
        );
    }

    /// Carries out one thing the user did in the window.
    fn apply_studio_action(
        &mut self,
        ctx: &Context,
        session: &mut StudioSession,
        action: Action,
    ) -> Next {
        let basis = session.framing_basis();
        if action.is_framing() && basis.is_none() {
            // Several charts have nothing to reframe; the window offers none of it.
            return Next::Stay;
        }
        match action {
            Action::Close => return Next::Close,
            Action::SetArea(Area::Portion) => return Next::Select,
            Action::SetArea(area) => self.retake(ctx, session, area),
            Action::SetStyle(style) => {
                session.config.set_style(style);
                session.settle_framing();
            }
            Action::SetSize(size) => {
                session.config.size = size;
                session.settle_framing();
            }
            Action::SetBackground(background) => {
                session.config.background = background;
                session.settle_framing();
            }
            Action::SetInclude(include) => session.config.include = include,
            Action::DragDial(dial, value) => session.config.framing.set_dial(dial, value),
            Action::ReleaseDial => {
                session.config.framing.release_snap();
                session.settle_framing();
            }
            Action::StepDial(dial, step, sign) => {
                session.config.framing.step(dial, step, sign);
                session.settle_framing();
            }
            Action::ResetDial(dial) => {
                session.config.framing.reset_dial(dial);
                session.settle_framing();
            }
            Action::DragPreview(delta) => {
                if let Some((base, plot)) = basis {
                    let origin = *session.drag.get_or_insert(session.config.framing);
                    session.config.framing =
                        session.config.framing.dragged(&base, plot, &origin, delta);
                }
            }
            Action::ReleasePreview => {
                session.drag = None;
                session.settle_framing();
            }
            Action::NudgePreview { across, down } => {
                if let Some((base, plot)) = basis {
                    session.config.framing =
                        session.config.framing.nudged(&base, plot, across, down);
                }
            }
            Action::ResetPosition => session.config.framing.reset_position(),
            Action::ResetView => {
                session.config.framing.reset_view();
                session.drag = None;
            }
            Action::Export(export) => self.start_studio_export(ctx, session, export),
        }
        Next::Stay
    }

    /// Takes a new picture of the live workspace for `area`: the focused chart, or all of them.
    fn retake(&mut self, ctx: &Context, session: &mut StudioSession, area: Area) {
        let taken = match area {
            Area::FullChart => {
                let focused = self
                    .focused_market_chart()
                    .map(|chart| chart.id)
                    .or_else(|| session.capture.headline().map(|chart| chart.chart_id()));
                focused.map_or(Err(CaptureError::NoChart), |id| self.capture_full(ctx, id))
            }
            Area::EntireLayout => self.capture_layout(ctx),
            Area::Portion => return,
        };
        match taken {
            Ok(capture) => session.replace_capture(capture, area),
            Err(error) => self.say(error.to_string(), true),
        }
    }

    /// Makes the image at the Settings' scale and hands it to the snapshot worker, as the camera
    /// menu does: the same job, the same Save panel and the same messages once it has happened.
    fn start_studio_export(
        &mut self,
        ctx: &Context,
        session: &StudioSession,
        export: window::Export,
    ) {
        if self.snapshot_busy() {
            self.say("A snapshot is already being made.", true);
            return;
        }
        let scale = self.snapshot_prefs().options().scale;
        let composed = {
            let cards = |id: u64| self.card_of(id);
            compose::scene(
                &session.capture,
                &cards,
                &session.config,
                Target::Export(scale),
            )
        };
        let scene = match composed {
            Ok((scene, _)) => scene,
            Err(error) => {
                self.say(error.to_string(), true);
                return;
            }
        };
        let now = now_ms();
        let offset = timezone::local_utc_offset_secs(now);
        let headline = session.capture.headline();
        let timeframe =
            headline.map_or_else(String::new, |chart| chart.timeframe().label().to_owned());
        let name = match (&session.capture, headline) {
            (Capture::Single(chart), _) => {
                snapshot::file_name(chart.symbol(), &timeframe, now, offset)
            }
            _ => snapshot::layout_file_name(now, offset),
        };
        let title = headline.map_or_else(String::new, |chart| chart.title().to_owned());
        self.launch(
            ctx,
            Launch {
                action: match export {
                    window::Export::Download => SnapshotAction::Download,
                    window::Export::CopyImage => SnapshotAction::CopyImage,
                },
                scene,
                name,
                title,
                timeframe,
                destination: None,
                studio: Some(export),
            },
        );
    }
}

#[cfg(test)]
mod tests;

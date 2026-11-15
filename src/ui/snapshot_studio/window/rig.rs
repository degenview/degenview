//! What the window's tests and galleries share: the window on a screen of its own, driven one
//! frame at a time, with a preview picture composed from a captured fixture.

use super::{Action, Image, Input, Output, Preview, show};
use crate::model::chart::snapshot::composition;
use crate::model::chart::snapshot::studio::{Area, StudioConfig};
use crate::service::snapshot::raster;
use crate::ui::render_tests::Screen;
use crate::ui::snapshot_studio::capture::Capture;
use crate::ui::snapshot_studio::compose::{self, Cards, Target};
use crate::ui::snapshot_studio::fixtures::texts;
use eframe::egui::{
    self, Event, Id, Modifiers, PointerButton, Pos2, Rect, TextureHandle, TextureOptions, Vec2,
    pos2,
};
use std::collections::HashMap;

pub(super) const ID: &str = "studio-test";

/// What a frame of the window is given, apart from the picture.
pub(super) struct Scenario<'a> {
    pub config: StudioConfig,
    pub areas: &'a [Area],
    pub has_indicators: bool,
    pub has_drawings: bool,
    pub reframable: bool,
    pub preview: Shown<'a>,
    pub summary: Result<String, String>,
    pub exporting: Option<super::Export>,
    pub can_export: bool,
}

#[derive(Clone, Copy)]
pub(super) enum Shown<'a> {
    /// Still being drawn.
    Loading,
    /// A picture of the image, as the pipeline would have made it.
    Picture(Image),
    /// No picture, and why.
    Failed(&'a str),
}

impl<'a> Scenario<'a> {
    pub(super) fn new(config: StudioConfig) -> Self {
        Self {
            config,
            areas: &[Area::FullChart, Area::Portion],
            has_indicators: true,
            has_drawings: true,
            reframable: true,
            preview: Shown::Loading,
            summary: Ok("Selected portion · Styled · 1,800 × 1,200 · PNG".to_owned()),
            exporting: None,
            can_export: true,
        }
    }
}

/// The window on a screen of `size` points, in the light or the dark theme.
pub(super) struct Rig {
    pub screen: Screen,
    /// What the screen reader has been told, by name: where each named widget is.
    labels: HashMap<String, Rect>,
    /// Every word drawn in the last frame, tooltips included.
    pub words: Vec<String>,
}

impl Rig {
    pub(super) fn new(size: Vec2, light: bool) -> Self {
        let screen = Screen::new(size);
        screen.ctx.set_theme(if light {
            egui::ThemePreference::Light
        } else {
            egui::ThemePreference::Dark
        });
        screen.ctx.enable_accesskit();
        Self {
            screen,
            labels: HashMap::new(),
            words: Vec::new(),
        }
    }

    /// One frame; the window's output and what egui drew.
    pub(super) fn frame(
        &mut self,
        scenario: &Scenario<'_>,
        events: Vec<Event>,
    ) -> (Output, egui::FullOutput) {
        let mut output = None;
        let summary = scenario
            .summary
            .as_ref()
            .map(String::as_str)
            .map_err(String::as_str);
        let full = self.screen.run(events, |ui| {
            crate::ui::theme::sync(ui.ctx());
            let (image, failure) = match scenario.preview {
                Shown::Loading => (None, None),
                Shown::Picture(image) => (Some(image), None),
                Shown::Failed(text) => (None, Some(text)),
            };
            output = Some(show(
                ui.ctx(),
                Id::new(ID),
                &Input {
                    config: &scenario.config,
                    areas: scenario.areas,
                    has_indicators: scenario.has_indicators,
                    has_drawings: scenario.has_drawings,
                    reframable: scenario.reframable,
                    preview: Preview { image, failure },
                    summary,
                    exporting: scenario.exporting,
                    can_export: scenario.can_export,
                },
            ));
        });
        if let Some(update) = &full.platform_output.accesskit_update {
            for (_, node) in &update.nodes {
                if let (Some(label), Some(bounds)) = (node.label(), node.bounds()) {
                    self.labels.insert(
                        label.to_owned(),
                        Rect::from_min_max(
                            pos2(bounds.x0 as f32, bounds.y0 as f32),
                            pos2(bounds.x1 as f32, bounds.y1 as f32),
                        ),
                    );
                }
            }
        }
        self.words = texts(&full.shapes);
        (output.expect("the window was drawn"), full)
    }

    /// Where the widget a screen reader calls `label` is, as of the last frames.
    pub(super) fn find(&self, label: &str) -> Rect {
        *self
            .labels
            .get(label)
            .unwrap_or_else(|| panic!("no widget called {label:?} in {:?}", self.labels.keys()))
    }

    pub(super) fn has(&self, label: &str) -> bool {
        self.labels.contains_key(label)
    }

    /// Where the widget with `id` was last drawn.
    pub(super) fn rect_of(&self, id: Id) -> Rect {
        self.screen
            .ctx
            .read_response(id)
            .unwrap_or_else(|| panic!("no widget {id:?}"))
            .rect
    }

    /// Holds the pointer at `at` for long enough for a tooltip to show, and says its words.
    pub(super) fn hover(&mut self, scenario: &Scenario<'_>, at: Pos2) -> Vec<String> {
        let moved = self.move_to(at);
        self.frame(scenario, vec![moved]);
        let mut seen = self.words.clone();
        for _ in 0..12 {
            self.frame(scenario, Vec::new());
            seen.extend(self.words.iter().cloned());
        }
        seen
    }

    /// A drag by `by` from `from` in a few steps, then a release.
    pub(super) fn drag(&mut self, scenario: &Scenario<'_>, from: Pos2, by: Vec2) -> Vec<Action> {
        let mut actions = Vec::new();
        let first = self.move_to(from);
        actions.extend(self.frame(scenario, vec![first]).0.actions);
        actions.extend(
            self.frame(scenario, vec![self.button(from, true)])
                .0
                .actions,
        );
        for step in 1..=4 {
            let at = from + by * (step as f32 / 4.0);
            let moved = self.move_to(at);
            actions.extend(self.frame(scenario, vec![moved]).0.actions);
        }
        actions.extend(
            self.frame(scenario, vec![self.button(from + by, false)])
                .0
                .actions,
        );
        actions
    }

    /// Frames until nothing changes layout, then the last one.
    pub(super) fn settle(&mut self, scenario: &Scenario<'_>) -> (Output, egui::FullOutput) {
        let mut last = self.frame(scenario, Vec::new());
        for _ in 0..3 {
            last = self.frame(scenario, Vec::new());
        }
        last
    }

    pub(super) fn move_to(&mut self, at: Pos2) -> Event {
        Event::PointerMoved(at)
    }

    pub(super) fn button(&self, at: Pos2, pressed: bool) -> Event {
        Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        }
    }

    /// A press and release at `at`, over three frames; the actions reported on the way.
    pub(super) fn click(&mut self, scenario: &Scenario<'_>, at: Pos2) -> Vec<Action> {
        let mut actions = Vec::new();
        let move_event = self.move_to(at);
        for events in [
            vec![move_event],
            vec![self.button(at, true)],
            vec![self.button(at, false)],
        ] {
            actions.extend(self.frame(scenario, events).0.actions);
        }
        actions
    }

    /// Two quick presses and releases at `at`, as a double-click.
    pub(super) fn double_click(&mut self, scenario: &Scenario<'_>, at: Pos2) -> Vec<Action> {
        let mut actions = Vec::new();
        let moved = self.move_to(at);
        actions.extend(self.frame(scenario, vec![moved]).0.actions);
        for pressed in [true, false, true, false] {
            let press = self.button(at, pressed);
            actions.extend(self.frame(scenario, vec![press]).0.actions);
        }
        actions
    }

    pub(super) fn key(&mut self, scenario: &Scenario<'_>, key: egui::Key) -> Vec<Action> {
        self.frame(
            scenario,
            vec![Event::Key {
                key,
                physical_key: Some(key),
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            }],
        )
        .0
        .actions
    }
}

/// The picture the pipeline would make of `capture` in `config` for a room of `max_px`: uploaded
/// to the screen's context, so the window can draw it.
pub(super) fn picture(
    screen: &Screen,
    capture: &Capture,
    cards: &Cards<'_>,
    config: &StudioConfig,
    max_px: (u32, u32),
) -> (TextureHandle, Image) {
    let (scene, _) = compose::scene(capture, cards, config, Target::Preview { max_px })
        .expect("a scene to draw");
    let pixels = raster::rasterize(&scene).expect("drawn");
    let size = pixels.size;
    let handle = screen
        .ctx
        .load_texture("studio-test-picture", pixels, TextureOptions::LINEAR);
    let image = Image {
        texture: handle.id(),
        pixels: size,
        composition: composition::layout(capture.size(), config).size,
    };
    (handle, image)
}

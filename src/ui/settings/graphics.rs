//! Settings ▸ Graphics: VSync and related display options, with Advanced for power users.

use crate::graphics::prefs::{BackendChoice, DisplaySync, FrameQueue, GraphicsPrefs};
use crate::i18n::tr;
use crate::ui::icons::Icon;
use crate::ui::kit;
use crate::ui::settings::card::{self, StatusMessage};
use crate::ui::settings::window::{section, status_message};
use crate::ui::theme;
use eframe::egui::{self, Sense};

/// What the page asks the host to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphicsAction {
    Save(GraphicsPrefs),
}

pub struct GraphicsInput<'a> {
    pub prefs: GraphicsPrefs,
    /// Adapter line from startup (`NVIDIA GeForce …`), if known.
    pub adapter: &'a str,
    /// Backend actually in use this launch (may differ from the saved choice until restart).
    pub active_backend: &'a str,
}

#[derive(Default)]
pub struct GraphicsState {
    /// The draft shown on the page; loaded from the input when Settings opens the page.
    loaded: bool,
    draft: GraphicsPrefs,
    /// The saved prefs the draft was last aligned with.
    seen: GraphicsPrefs,
    /// Backend changed: needs a restart to take effect.
    pub backend_dirty: bool,
    /// Advanced section is open.
    advanced_open: bool,
    pub status: Option<StatusMessage>,
}

impl GraphicsState {
    fn sync(&mut self, saved: GraphicsPrefs) {
        if !self.loaded || self.seen != saved {
            self.loaded = true;
            self.seen = saved;
            self.draft = saved;
            self.backend_dirty = false;
        }
    }

    fn save(&mut self, actions: &mut Vec<GraphicsAction>) {
        actions.push(GraphicsAction::Save(self.draft));
    }
}

/// Draws the page. Emits a save when a control changes.
pub fn body(
    ui: &mut egui::Ui,
    state: &mut GraphicsState,
    input: &GraphicsInput<'_>,
    actions: &mut Vec<GraphicsAction>,
) {
    state.sync(input.prefs);

    section(
        ui,
        tr!("Mouse and display"),
        Some(tr!(
            "If the cursor feels sticky inside DegenView but fine outside it, leave VSync off."
        )),
        |ui| {
            let mut vsync = state.draft.display_sync.vsync_on();
            card::card(
                ui,
                Icon::GaugeHigh,
                tr!("VSync"),
                Some(tr!(
                    "On locks drawing to the screen refresh: less tearing, but the mouse can feel slow. Off is recommended."
                )),
                |ui| {
                    if card::switch(ui, &mut vsync) {
                        state.draft.display_sync =
                            DisplaySync::from_switches(vsync, state.draft.display_sync.uncapped());
                        state.save(actions);
                    }
                },
            );
        },
    );

    ui.add_space(16.0);
    if advanced_toggle(ui, &mut state.advanced_open) {
        ui.add_space(12.0);
        advanced(ui, state, input, actions);
    }

    if let Some(status) = &state.status {
        ui.add_space(12.0);
        status_message(ui, &status.text(), status.is_error);
    }
}

fn advanced_toggle(ui: &mut egui::Ui, open: &mut bool) -> bool {
    let label = if *open {
        tr!("Hide Advanced")
    } else {
        tr!("Advanced")
    };
    let response = ui.add(
        egui::Label::new(
            egui::RichText::new(label)
                .size(kit::size::SUBHEADLINE + 1.0)
                .underline()
                .color(theme::text_secondary()),
        )
        .sense(Sense::click()),
    );
    if response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
    {
        *open = !*open;
    }
    *open
}

fn advanced(
    ui: &mut egui::Ui,
    state: &mut GraphicsState,
    input: &GraphicsInput<'_>,
    actions: &mut Vec<GraphicsAction>,
) {
    section(
        ui,
        tr!("Feel"),
        Some(tr!(
            "Fine-tune how frames are queued. Most people can leave these alone."
        )),
        |ui| {
            let options: [(FrameQueue, &str); 2] =
                FrameQueue::ALL.map(|queue| (queue, queue.title()));
            let mut queue = state.draft.frame_queue;
            card::card(
                ui,
                Icon::Layers,
                tr!("Buffering"),
                Some(queue.hint()),
                |ui| {
                    kit::segmented(
                        ui,
                        egui::Id::new("graphics-buffering"),
                        &options,
                        &mut queue,
                    );
                    if queue != state.draft.frame_queue {
                        state.draft.frame_queue = queue;
                        state.save(actions);
                    }
                },
            );

            let vsync_on = state.draft.display_sync.vsync_on();
            if !vsync_on {
                ui.add_space(8.0);
                let mut uncapped = state.draft.display_sync.uncapped();
                card::card(
                    ui,
                    Icon::Bolt,
                    tr!("Ignore refresh rate"),
                    Some(tr!(
                        "Never wait for the display. Lowest mouse lag; tearing is more likely."
                    )),
                    |ui| {
                        if card::switch(ui, &mut uncapped) {
                            state.draft.display_sync = DisplaySync::from_switches(false, uncapped);
                            state.save(actions);
                        }
                    },
                );
            }
        },
    );

    ui.add_space(22.0);
    section(
        ui,
        tr!("Graphics API"),
        Some(tr!(
            "Which API starts the app. A change needs a restart. Automatic is usually right."
        )),
        |ui| {
            let choices = BackendChoice::selectable();
            let labels: Vec<String> = choices
                .iter()
                .map(|choice| choice.title().to_owned())
                .collect();
            let selected = choices
                .iter()
                .position(|choice| *choice == state.draft.backend)
                .unwrap_or(0);
            let backend_hint = if cfg!(windows) {
                tr!("Only change this if the app fails to start or still feels wrong.")
            } else if cfg!(target_os = "macos") {
                tr!("Metal on this Mac. Automatic is the only useful choice.")
            } else {
                tr!("Only change this if the app fails to start or still feels wrong.")
            };
            card::card(ui, Icon::Sliders, tr!("API"), Some(backend_hint), |ui| {
                if let Some(index) = kit::menu_picker(ui, 160.0, &labels, selected, true) {
                    let next = choices[index];
                    if next != state.draft.backend {
                        state.draft.backend = next;
                        state.backend_dirty = next != input.prefs.backend;
                        state.save(actions);
                    }
                }
            });
            if state.backend_dirty {
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new(tr!("Restart DegenView to use the new graphics API."))
                        .size(kit::size::CAPTION)
                        .color(theme::text_secondary()),
                );
            }
        },
    );

    if !input.adapter.is_empty() || !input.active_backend.is_empty() {
        ui.add_space(22.0);
        section(ui, tr!("This computer"), None, |ui| {
            if !input.adapter.is_empty() {
                card::card(
                    ui,
                    Icon::InfoCircle,
                    tr!("GPU"),
                    Some(input.adapter),
                    |_| {},
                );
            }
            if !input.active_backend.is_empty() {
                ui.add_space(8.0);
                card::card(
                    ui,
                    Icon::CheckCircleFill,
                    tr!("In use"),
                    Some(input.active_backend),
                    |_| {},
                );
            }
        });
    }
}

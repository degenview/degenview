//! Screen-reader labels for hand-painted and icon-only widgets.
//!
//! egui's accesskit integration (enabled by `eframe`'s default features) already names text
//! buttons, checkboxes, sliders, collapsing headers and plain labels from their on-screen
//! text. This module fills the gaps: painter regions (the candle canvas) and buttons whose
//! visible glyph (`×`, `🔔`, `/`) is not a useful spoken name.

use eframe::egui::{Response, WidgetInfo, WidgetType};

/// Attach a spoken name to a widget response, defaulting to a button.
///
/// The label is registered against the response's `Id` and may be read in a later frame, so
/// pass an owned `String` or a `&'static str` rather than a temporary.
pub fn label(response: Response, text: impl Into<String>) -> Response {
    label_as(response, WidgetType::Button, text)
}

/// [`label`] with an explicit [`WidgetType`], for panels, images, links and other regions.
pub fn label_as(response: Response, typ: WidgetType, text: impl Into<String>) -> Response {
    let text = text.into();
    let enabled = response.enabled();
    response.widget_info(move || WidgetInfo::labeled(typ, enabled, text.clone()));
    response
}

/// [`label_as`] for a widget that is on or off (a star, a lit tool button): the state is told to the
/// screen reader. A checkbox registered without one reads as indeterminate, so one that is drawn
/// checked or not must come through here.
pub fn selected_as(
    response: Response,
    typ: WidgetType,
    text: impl Into<String>,
    selected: bool,
) -> Response {
    let text = text.into();
    let enabled = response.enabled();
    response.widget_info(move || WidgetInfo::selected(typ, enabled, selected, text.clone()));
    response
}

/// The accessibility tree a test reads, built from every frame's update (egui sends a node again
/// only when it changed).
#[cfg(test)]
#[derive(Default)]
pub(crate) struct AccessNodes {
    nodes:
        std::collections::HashMap<eframe::egui::accesskit::NodeId, eframe::egui::accesskit::Node>,
}

#[cfg(test)]
impl AccessNodes {
    /// Starts the tree updates on `ctx`; call before the first frame.
    pub(crate) fn start(ctx: &eframe::egui::Context) -> Self {
        ctx.enable_accesskit();
        Self::default()
    }

    /// Takes the frame's tree update, and drops its textures, which a test does not draw.
    pub(crate) fn absorb(&mut self, mut output: eframe::egui::FullOutput) {
        output.textures_delta.clear();
        if let Some(update) = output.platform_output.accesskit_update {
            self.nodes.extend(update.nodes);
        }
    }

    /// The node an app named `label` (its toggled state is `node.toggled()`).
    pub(crate) fn find(&self, label: &str) -> Option<&eframe::egui::accesskit::Node> {
        self.nodes.values().find(|node| node.label() == Some(label))
    }
}

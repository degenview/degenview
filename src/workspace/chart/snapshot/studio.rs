//! What a selection leaves for the Screenshot Studio to work from.

use crate::model::chart::snapshot::studio::{Area, StudioConfig};
use crate::ui::snapshot_studio::capture::Capture;

/// A capture and the choices made about it. Held by the workspace, not drawn from yet: the
/// Studio window that opens on it comes next in this stack.
pub(super) struct StudioSession {
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the Studio window, next in this stack, draws from it"
        )
    )]
    pub(super) capture: Capture,
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the Studio window, next in this stack, draws from it"
        )
    )]
    pub(super) config: StudioConfig,
}

impl StudioSession {
    /// A session on what a selection captured, opened the way a portion opens.
    pub(super) fn portion(capture: Capture) -> Self {
        Self {
            capture,
            config: StudioConfig::new(Area::Portion),
        }
    }
}

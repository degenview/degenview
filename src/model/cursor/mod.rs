//! The chart's cursor modes: what the pointer looks like over a plot and which parts of the
//! crosshair it carries.
//!
//! A cursor is not a drawing tool. It is the resting state of the pointer, kept beside the armed
//! `Tool`: arming a drawing tool puts the cursor down, and Esc brings it back. Everything a mode
//! changes is one row of [`CursorMode::caps`], so painters and gestures read a capability and never
//! match on the mode.
//!
//! Which parts of each mode TradingView documents and which are our own choice is in
//! `docs/chart-cursors.md`.

pub mod demo;

use serde::{Deserialize, Serialize};

/// How the operating system's pointer is drawn over the plot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PointerShape {
    Crosshair,
    Arrow,
    /// The system pointer is hidden; the chart paints its own mark where the pointer is.
    Hidden,
}

/// What the pointer shows over the plot in one mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CursorCaps {
    /// The vertical and horizontal lines through the pointer (and the vertical one through each
    /// script pane under it).
    pub lines: bool,
    /// The price and time tags the lines carry on the axes.
    pub axis_tags: bool,
    /// The box naming the bar under the pointer.
    pub readout: bool,
    pub pointer: PointerShape,
    /// A drag that starts on the plot moves the chart. Off when the mode uses the drag itself.
    pub pans: bool,
    /// A click picks, moves and edits drawings.
    pub edits_drawings: bool,
    /// A press removes the drawings it passes over.
    pub erases: bool,
    /// A press or drag leaves a mark that fades (`demo::Marks`).
    pub highlights: bool,
}

/// The cursors of the cursor menu, in the order the menu lists them.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum CursorMode {
    /// The crosshair, the resting cursor.
    #[default]
    Cross,
    Dot,
    Arrow,
    Demonstration,
    Eraser,
}

impl CursorMode {
    pub const ALL: [Self; 5] = [
        Self::Cross,
        Self::Dot,
        Self::Arrow,
        Self::Demonstration,
        Self::Eraser,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Cross => "Cross",
            Self::Dot => "Dot",
            Self::Arrow => "Arrow",
            Self::Demonstration => "Demonstration",
            Self::Eraser => "Eraser",
        }
    }

    /// Whether the choice is remembered across launches. Demonstration and the Eraser are modes of
    /// the moment: a restart never opens in a mode that marks the chart or deletes.
    pub fn persists(self) -> bool {
        matches!(self, Self::Cross | Self::Dot | Self::Arrow)
    }

    /// The cursor to rest on when this one is put down: itself when it persists, else the cross.
    pub fn resting(self) -> Self {
        if self.persists() { self } else { Self::Cross }
    }

    pub fn caps(self) -> CursorCaps {
        const BASE: CursorCaps = CursorCaps {
            lines: false,
            axis_tags: false,
            readout: true,
            pointer: PointerShape::Arrow,
            pans: true,
            edits_drawings: true,
            erases: false,
            highlights: false,
        };
        match self {
            Self::Cross => CursorCaps {
                lines: true,
                axis_tags: true,
                pointer: PointerShape::Crosshair,
                ..BASE
            },
            Self::Dot => CursorCaps {
                axis_tags: true,
                pointer: PointerShape::Hidden,
                ..BASE
            },
            Self::Arrow => BASE,
            Self::Demonstration => CursorCaps {
                readout: false,
                pans: false,
                edits_drawings: false,
                highlights: true,
                ..BASE
            },
            Self::Eraser => CursorCaps {
                readout: false,
                pointer: PointerShape::Crosshair,
                pans: false,
                edits_drawings: false,
                erases: true,
                ..BASE
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cross_is_the_default_and_nothing_is_called_magic() {
        assert_eq!(CursorMode::default(), CursorMode::Cross);
        let labels: Vec<_> = CursorMode::ALL.iter().map(|mode| mode.label()).collect();
        assert_eq!(labels, ["Cross", "Dot", "Arrow", "Demonstration", "Eraser"]);
        assert!(labels.iter().all(|label| !label.contains("Magic")));
    }

    #[test]
    fn each_mode_shows_what_its_table_row_says() {
        let cross = CursorMode::Cross.caps();
        assert!(cross.lines && cross.axis_tags && cross.readout);
        assert_eq!(cross.pointer, PointerShape::Crosshair);
        let dot = CursorMode::Dot.caps();
        assert!(!dot.lines && dot.axis_tags && dot.readout);
        assert_eq!(dot.pointer, PointerShape::Hidden);
        let arrow = CursorMode::Arrow.caps();
        assert!(!arrow.lines && !arrow.axis_tags && arrow.readout);
        assert_eq!(arrow.pointer, PointerShape::Arrow);
    }

    #[test]
    fn only_the_resting_cursors_persist() {
        let kept: Vec<_> = CursorMode::ALL
            .into_iter()
            .filter(|mode| mode.persists())
            .collect();
        assert_eq!(
            kept,
            [CursorMode::Cross, CursorMode::Dot, CursorMode::Arrow]
        );
        assert_eq!(CursorMode::Eraser.resting(), CursorMode::Cross);
        assert_eq!(CursorMode::Demonstration.resting(), CursorMode::Cross);
        assert_eq!(CursorMode::Dot.resting(), CursorMode::Dot);
    }

    #[test]
    fn the_modes_that_use_the_drag_never_pan_and_only_the_eraser_deletes() {
        for mode in CursorMode::ALL {
            let caps = mode.caps();
            assert_eq!(caps.erases, mode == CursorMode::Eraser, "{mode:?}");
            assert_eq!(
                caps.highlights,
                mode == CursorMode::Demonstration,
                "{mode:?}"
            );
            let uses_the_drag = matches!(mode, CursorMode::Eraser | CursorMode::Demonstration);
            assert_eq!(caps.pans, !uses_the_drag, "{mode:?}");
            assert_eq!(caps.edits_drawings, caps.pans, "{mode:?}");
        }
        let demonstration = CursorMode::Demonstration.caps();
        assert!(!demonstration.lines && !demonstration.axis_tags && !demonstration.readout);
        assert_eq!(demonstration.pointer, PointerShape::Arrow);
        let eraser = CursorMode::Eraser.caps();
        assert!(!eraser.lines && !eraser.axis_tags && !eraser.readout);
        assert_eq!(eraser.pointer, PointerShape::Crosshair);
    }

    #[test]
    fn a_name_this_version_does_not_know_does_not_decode() {
        assert!(serde_json::from_str::<CursorMode>("\"Magic\"").is_err());
        assert_eq!(
            serde_json::from_str::<CursorMode>("\"Dot\"").unwrap(),
            CursorMode::Dot
        );
    }
}

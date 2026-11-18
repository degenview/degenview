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
}

/// The cursors of the cursor menu, in the order the menu lists them.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum CursorMode {
    /// The crosshair, the resting cursor.
    #[default]
    Cross,
    Dot,
    Arrow,
}

impl CursorMode {
    pub const ALL: [Self; 3] = [Self::Cross, Self::Dot, Self::Arrow];

    pub fn label(self) -> &'static str {
        match self {
            Self::Cross => "Cross",
            Self::Dot => "Dot",
            Self::Arrow => "Arrow",
        }
    }

    pub fn caps(self) -> CursorCaps {
        match self {
            Self::Cross => CursorCaps {
                lines: true,
                axis_tags: true,
                readout: true,
                pointer: PointerShape::Crosshair,
            },
            Self::Dot => CursorCaps {
                lines: false,
                axis_tags: true,
                readout: true,
                pointer: PointerShape::Hidden,
            },
            Self::Arrow => CursorCaps {
                lines: false,
                axis_tags: false,
                readout: true,
                pointer: PointerShape::Arrow,
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
        assert_eq!(labels, ["Cross", "Dot", "Arrow"]);
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
    fn a_name_this_version_does_not_know_does_not_decode() {
        assert!(serde_json::from_str::<CursorMode>("\"Magic\"").is_err());
        assert_eq!(
            serde_json::from_str::<CursorMode>("\"Dot\"").unwrap(),
            CursorMode::Dot
        );
    }
}

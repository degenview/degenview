//! When a running script next stops, given how it was told to go on.
//!
//! "Depth" is the number of frames on the debugger's stack: the script is depth 1 and each user
//! function being run adds one. Stepping is decided by depth and by a bar boundary, never by line
//! numbers, so a loop, a recursive call or a statement spread over several lines behaves the same
//! as a straight run of statements.

use super::commands::Resume;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StepMode {
    /// Stop only at breakpoints, errors and pause requests.
    Run,
    /// Stop at the next statement.
    Into,
    /// Stop at the next statement run at `depth` or shallower.
    Over { depth: usize },
    /// Stop at the next statement run shallower than `depth`.
    Out { depth: usize },
    /// Stop at the first statement of the next bar's execution.
    Bar,
}

impl StepMode {
    /// The mode a resume asks for, from a stop at `depth`. `None` for a stop request.
    pub fn after(resume: Resume, depth: usize) -> Option<Self> {
        Some(match resume {
            Resume::Continue => Self::Run,
            Resume::Into => Self::Into,
            Resume::Over => Self::Over { depth },
            Resume::Out => Self::Out { depth },
            Resume::Bar => Self::Bar,
            Resume::Stop => return None,
        })
    }

    /// Whether a statement about to run at `depth` is a stop. `first_of_bar` is true for the first
    /// statement hook since the current bar began executing.
    pub fn stops(self, depth: usize, first_of_bar: bool) -> bool {
        match self {
            Self::Run => false,
            Self::Into => true,
            Self::Over { depth: from } => depth <= from,
            Self::Out { depth: from } => depth < from,
            Self::Bar => first_of_bar,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn into_stops_everywhere_and_run_nowhere() {
        assert!(StepMode::Into.stops(5, false));
        assert!(!StepMode::Run.stops(1, true));
    }

    #[test]
    fn over_skips_deeper_frames_but_stops_in_the_caller() {
        let over = StepMode::Over { depth: 2 };
        assert!(!over.stops(3, false));
        assert!(over.stops(2, false));
        assert!(over.stops(1, false));
    }

    #[test]
    fn out_waits_for_a_shallower_frame() {
        let out = StepMode::Out { depth: 2 };
        assert!(!out.stops(2, false));
        assert!(!out.stops(3, false));
        assert!(out.stops(1, false));
    }

    #[test]
    fn bar_stops_only_at_the_start_of_a_bar() {
        assert!(!StepMode::Bar.stops(1, false));
        assert!(StepMode::Bar.stops(1, true));
    }

    #[test]
    fn a_stop_resume_has_no_mode() {
        assert_eq!(StepMode::after(Resume::Stop, 1), None);
        assert_eq!(
            StepMode::after(Resume::Over, 3),
            Some(StepMode::Over { depth: 3 })
        );
    }
}

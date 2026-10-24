//! The debugger's lifecycle as a pure state machine, independent of threads and UI. The session
//! thread and the toolbar both ask it what is allowed, so a button is enabled exactly when the
//! command would be accepted.

use super::commands::DebugCommand;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DebugState {
    /// No session.
    #[default]
    Idle,
    /// A session exists and has not started running the script yet.
    Preparing,
    Running,
    /// A pause was requested and the script has not reached a safepoint yet.
    Pausing,
    /// Stopped at a safepoint; the script's state is stable and can be inspected.
    Paused,
    /// A stop was requested and the script has not unwound yet.
    Stopping,
    /// The script ran to its end, or was stopped.
    Completed,
    /// The session could not start, or the script failed and the user is done with it.
    Failed,
}

/// What happened to the session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Trigger {
    Start,
    /// The script began executing.
    Began,
    PauseRequested,
    /// The script reached a safepoint and is waiting for a command.
    Stopped,
    /// A command let the script continue.
    Resumed,
    StopRequested,
    /// The script ended, or unwound after a stop.
    Finished,
    /// The script, or the start, failed.
    Errored,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidTransition {
    pub from: DebugState,
    pub trigger: Trigger,
}

impl std::fmt::Display for InvalidTransition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?} cannot take {:?}", self.from, self.trigger)
    }
}

impl std::error::Error for InvalidTransition {}

impl DebugState {
    pub fn on(self, trigger: Trigger) -> Result<Self, InvalidTransition> {
        use DebugState::*;
        use Trigger::*;
        let next = match (self, trigger) {
            (Idle | Completed | Failed, Start) => Preparing,
            (Preparing, Began) => Running,
            (Running, PauseRequested) => Pausing,
            (Running | Pausing, Stopped) => Paused,
            (Paused, Resumed) => Running,
            // A pause asked for just as the script finished changes nothing.
            (Pausing, Resumed) => Running,
            (Preparing | Running | Pausing | Paused, StopRequested) => Stopping,
            (Stopping, StopRequested) => Stopping,
            (Running | Pausing | Stopping, Finished) => Completed,
            (Preparing | Running | Pausing | Paused | Stopping, Errored) => Failed,
            (Paused, Finished) => Completed,
            (from, trigger) => return Err(InvalidTransition { from, trigger }),
        };
        Ok(next)
    }

    /// A session is alive: it owns a thread and a paused or running script.
    pub fn is_active(self) -> bool {
        matches!(
            self,
            Self::Preparing | Self::Running | Self::Pausing | Self::Paused | Self::Stopping
        )
    }

    /// Whether `command` is accepted now. `paused_on_error` is true when the stop is a runtime
    /// error: the script cannot go on, so only inspecting, stopping and restarting remain.
    pub fn allows(self, command: DebugCommand, paused_on_error: bool) -> bool {
        use DebugCommand::*;
        match command {
            Continue | StepInto | StepOver | StepOut | StepBar => {
                self == Self::Paused && !paused_on_error
            }
            Pause => self == Self::Running,
            Stop => matches!(
                self,
                Self::Preparing | Self::Running | Self::Pausing | Self::Paused
            ),
            Restart => matches!(
                self,
                Self::Running | Self::Pausing | Self::Paused | Self::Completed | Self::Failed
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use DebugState::*;
    use Trigger::*;

    fn walk(start: DebugState, triggers: &[Trigger]) -> DebugState {
        triggers
            .iter()
            .fold(start, |state, trigger| state.on(*trigger).unwrap())
    }

    #[test]
    fn a_session_runs_pauses_resumes_and_completes() {
        assert_eq!(
            walk(Idle, &[Start, Began, Stopped, Resumed, Finished]),
            Completed
        );
    }

    #[test]
    fn a_pause_request_waits_for_the_next_safepoint() {
        let state = walk(Idle, &[Start, Began, PauseRequested]);
        assert_eq!(state, Pausing);
        assert_eq!(state.on(Stopped), Ok(Paused));
        assert_eq!(Pausing.on(Finished), Ok(Completed));
    }

    #[test]
    fn stop_is_accepted_from_every_active_state_and_twice() {
        for state in [Preparing, Running, Pausing, Paused] {
            let stopping = state.on(StopRequested).unwrap();
            assert_eq!(stopping, Stopping);
            assert_eq!(stopping.on(StopRequested), Ok(Stopping));
            assert_eq!(stopping.on(Finished), Ok(Completed));
        }
    }

    #[test]
    fn a_finished_session_can_start_again_but_an_idle_one_cannot_resume() {
        assert_eq!(Completed.on(Start), Ok(Preparing));
        assert_eq!(Failed.on(Start), Ok(Preparing));
        assert_eq!(
            Idle.on(Resumed),
            Err(InvalidTransition {
                from: Idle,
                trigger: Resumed
            })
        );
        assert!(Paused.on(Start).is_err());
        assert!(Running.on(Start).is_err());
    }

    #[test]
    fn commands_are_enabled_only_where_they_would_be_accepted() {
        use DebugCommand::*;
        for command in [Continue, StepInto, StepOver, StepOut, StepBar] {
            assert!(Paused.allows(command, false));
            assert!(!Paused.allows(command, true), "{command:?} after an error");
            for state in [
                Idle, Preparing, Running, Pausing, Stopping, Completed, Failed,
            ] {
                assert!(!state.allows(command, false), "{command:?} in {state:?}");
            }
        }
        assert!(Running.allows(Pause, false));
        assert!(!Paused.allows(Pause, false));
        assert!(Paused.allows(Stop, true));
        assert!(!Idle.allows(Stop, false));
        assert!(!Stopping.allows(Stop, false));
        assert!(Paused.allows(Restart, true));
        assert!(Completed.allows(Restart, false));
        assert!(!Idle.allows(Restart, false));
    }

    #[test]
    fn every_state_pair_is_either_a_transition_or_an_error_without_panicking() {
        let states = [
            Idle, Preparing, Running, Pausing, Paused, Stopping, Completed, Failed,
        ];
        let triggers = [
            Start,
            Began,
            PauseRequested,
            Stopped,
            Resumed,
            StopRequested,
            Finished,
            Errored,
        ];
        for state in states {
            for trigger in triggers {
                let _ = state.on(trigger);
            }
        }
    }
}

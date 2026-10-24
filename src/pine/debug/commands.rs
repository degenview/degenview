//! What the person at the keyboard can ask of a session, and how a stopped script is told to go on.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DebugCommand {
    /// Run until a breakpoint, a runtime error, the end, or a pause request.
    Continue,
    /// Stop at the next safepoint.
    Pause,
    /// Run to the next statement, entering user functions.
    StepInto,
    /// Run to the next statement of the current frame, or of a caller when the frame returns.
    StepOver,
    /// Run until the current function returns to its caller.
    StepOut,
    /// Run to the first statement of the next bar's execution.
    StepBar,
    /// Throw the session away and start a new one on the current source.
    Restart,
    /// End the session.
    Stop,
}

/// The answer a [`Debugger`](super::Debugger) gives when the script has stopped.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Resume {
    Continue,
    Into,
    Over,
    Out,
    /// To the next bar's first statement.
    Bar,
    /// Unwind the script now.
    Stop,
}

impl DebugCommand {
    /// The resume a command asks for; `None` for commands that do not resume a stopped script.
    pub fn resume(self) -> Option<Resume> {
        match self {
            Self::Continue => Some(Resume::Continue),
            Self::StepInto => Some(Resume::Into),
            Self::StepOver => Some(Resume::Over),
            Self::StepOut => Some(Resume::Out),
            Self::StepBar => Some(Resume::Bar),
            Self::Stop => Some(Resume::Stop),
            Self::Pause | Self::Restart => None,
        }
    }
}

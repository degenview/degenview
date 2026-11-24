//! The debugger's and the profiler's hooks: attaching either, and what the statement loop,
//! `invoke` and `execute` call when one is attached.
//!
//! The debugger's hooks take it out of the session while they run, so it can be handed `&mut self`.
//! They are cold and never inlined: a session without hooks must not carry their code in the
//! statement loop. Every `hook_before` is answered by exactly one `hook_after` or `hook_failed`
//! with `entered`, and every `hook_enter_function` by one `hook_leave_function`; the profiler's
//! stack of open regions relies on it.

use super::{Hooks, Session};
use crate::pine::runtime::types::RuntimeContext;
use crate::pine::runtime::value::RuntimeValue;
use crate::pine::syntax::ast::Statement;
use crate::pine::{Diagnostic, SourceRange};

impl Session {
    /// Makes this session a debugging session: the script stops where the debugger says. A session
    /// is never debugged by accident; nothing but this call installs the hooks.
    pub fn attach_debugger(&mut self, debug: crate::pine::debug::DebugRuntime) {
        // Logpoints write to the Pine Logs, and the script's own `log.*()` calls are what a person
        // debugging wants to read too.
        self.logs.enable();
        self.hooks = Some(Box::new(Hooks::Debug(debug)));
    }

    /// Makes this session a profiling session: each statement, user function call and bar is
    /// timed. A session is never profiled by accident; nothing but this call installs the hooks.
    pub fn attach_profiler(&mut self, profile: crate::pine::profile::ProfileRuntime) {
        self.hooks = Some(Box::new(Hooks::Profile(profile)));
    }

    /// Takes the recorder back out, to read what it measured.
    pub fn detach_profiler(&mut self) -> Option<crate::pine::profile::ProfileRuntime> {
        match self.hooks.take().map(|hooks| *hooks) {
            Some(Hooks::Profile(profile)) => Some(profile),
            other => {
                self.hooks = other.map(Box::new);
                None
            }
        }
    }

    #[cold]
    #[inline(never)]
    pub(super) fn hook_before(
        &mut self,
        statement: &Statement,
        context: &RuntimeContext,
    ) -> Result<(), Diagnostic> {
        let Some(mut hooks) = self.hooks.take() else {
            return Ok(());
        };
        // A statement the session added is not in the script: nothing stops at it, and its time
        // is part of the statement it was taken from.
        let synthetic = statement.is_synthetic();
        let outcome = match &mut *hooks {
            Hooks::Debug(_) if synthetic => Ok(()),
            Hooks::Debug(debug) => debug.before_statement(self, statement, context),
            Hooks::Profile(profile) => {
                // Library code is not timed: its offsets belong to another text, and its time is
                // part of the statement that called it. Walking past a function's declaration runs
                // nothing; its calls are timed in `invoke`.
                let offset = statement.range().start.offset;
                if context.scope.is_some() || matches!(statement, Statement::Function { .. }) {
                    profile.enter_untimed();
                } else if synthetic {
                    profile.enter_run_ahead(offset);
                } else {
                    profile.enter_statement(offset);
                }
                Ok(())
            }
        };
        self.hooks = Some(hooks);
        outcome
    }

    /// `statement` ran to its end: whatever it declares now has a value this bar.
    #[cold]
    #[inline(never)]
    pub(super) fn hook_after(&mut self, statement: &Statement, context: &RuntimeContext) {
        match self.hooks.as_deref_mut() {
            Some(Hooks::Debug(_)) if statement.is_synthetic() => {}
            Some(Hooks::Debug(debug)) => debug.after_statement(statement, context),
            Some(Hooks::Profile(profile)) => profile.exit(),
            None => {}
        }
    }

    /// Passes a failure of `statement` through the hooks on its way up, and hands it back. `entered`
    /// is whether `hook_before` ran for the statement (it did not if the budget was spent first).
    #[cold]
    #[inline(never)]
    pub(super) fn hook_failed(
        &mut self,
        statement: &Statement,
        context: &RuntimeContext,
        error: Diagnostic,
        entered: bool,
    ) -> Diagnostic {
        if let Some(mut hooks) = self.hooks.take() {
            match &mut *hooks {
                Hooks::Debug(debug) => debug.failed(self, context, statement, &error),
                Hooks::Profile(profile) => {
                    if entered {
                        profile.exit();
                    }
                }
            }
            self.hooks = Some(hooks);
        }
        error
    }

    /// A user function's body is about to run, called from the call at `range`. `saved` is what its
    /// locals' names held as it began, `uncovered` the calling function's own values of the names
    /// that read the global again during the call (`Session::uncover_caller_globals`).
    #[cold]
    #[inline(never)]
    pub(super) fn hook_enter_function(
        &mut self,
        call_name: &str,
        range: SourceRange,
        scope: Option<&str>,
        declared_at: usize,
        saved: &[(String, Option<RuntimeValue>)],
        uncovered: &[(usize, Option<RuntimeValue>)],
    ) {
        match self.hooks.as_deref_mut() {
            Some(Hooks::Debug(debug)) => {
                // The caller's own values come first: a frame reads a name from its nearest callee
                // that set it aside.
                let caller = self.call_frames.last();
                let shadowed = uncovered
                    .iter()
                    .filter_map(|(index, own)| {
                        caller.map(|caller| (caller.saved[*index].0.clone(), own.clone()))
                    })
                    .chain(saved.iter().cloned())
                    .collect();
                debug.enter_function(call_name, range, scope, shadowed);
            }
            // A library's offsets belong to another text: its calls are not timed.
            Some(Hooks::Profile(profile)) if scope.is_none() => {
                profile.enter_function(declared_at, range.start.offset);
            }
            Some(Hooks::Profile(profile)) => profile.enter_untimed(),
            None => {}
        }
    }

    #[cold]
    #[inline(never)]
    pub(super) fn hook_leave_function(&mut self) {
        match self.hooks.as_deref_mut() {
            Some(Hooks::Debug(debug)) => debug.leave_function(),
            Some(Hooks::Profile(profile)) => profile.exit(),
            None => {}
        }
    }

    #[cold]
    #[inline(never)]
    pub(super) fn hook_begin_bar(&mut self) {
        if let Some(Hooks::Profile(profile)) = self.hooks.as_deref_mut() {
            profile.begin_bar();
        }
    }

    /// The bar ended. What measuring it added is given back to the deadline: a profiled run is
    /// slower than a normal one, and must not time out where the normal one would not.
    #[cold]
    #[inline(never)]
    pub(super) fn hook_end_bar(
        &mut self,
        outcome: crate::pine::profile::BarOutcome,
        realtime: bool,
    ) {
        if let Some(Hooks::Profile(profile)) = self.hooks.as_deref_mut() {
            let added = profile.end_bar(outcome, realtime);
            if let Some(deadline) = self.deadline_at.as_mut() {
                *deadline += added;
            }
        }
    }

    #[inline]
    pub(crate) fn debug_is_attached(&self) -> bool {
        matches!(self.hooks.as_deref(), Some(Hooks::Debug(_)))
    }

    /// A `request.security()` expression is about to run on `symbol` at `interval_seconds`.
    #[cold]
    #[inline(never)]
    pub(crate) fn debug_enter_security(&mut self, symbol: &str, interval_seconds: f64) {
        if let Some(Hooks::Debug(debug)) = self.hooks.as_deref_mut() {
            debug.enter_security(symbol, interval_seconds);
        }
    }

    #[cold]
    #[inline(never)]
    pub(crate) fn debug_leave(&mut self) {
        if let Some(Hooks::Debug(debug)) = self.hooks.as_deref_mut() {
            debug.leave_function();
        }
    }
}

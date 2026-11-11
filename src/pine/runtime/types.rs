//! Per-bar runtime scaffolding: bar-state flags, flow control, the evaluation context and a
//! user-defined function. Ported from `PineRuntimeContext.swift` + `PineRuntimeFunction.swift`.

use std::collections::HashSet;

use super::containers::Object;
use super::value::{RefKind, RuntimeValue};
use crate::model::Kline;
use crate::pine::ValueType;
use crate::pine::syntax::ast::{DeclarationMode, Expression, Parameter, Statement};
use crate::pine::syntax::traversal::nested_blocks;

/// How a block of statements ended.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Flow {
    Normal,
    Break,
    Continue,
}

/// `barstate.*` for one bar.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BarFlags {
    pub is_first: bool,
    pub is_last: bool,
    pub is_history: bool,
    pub is_realtime: bool,
    pub is_new: bool,
    pub is_confirmed: bool,
    pub is_last_confirmed_history: bool,
}

impl BarFlags {
    /// Every `barstate.*` name `value` resolves.
    pub fn value(&self, name: &str) -> Option<bool> {
        match name {
            "barstate.isfirst" => Some(self.is_first),
            "barstate.islast" => Some(self.is_last),
            "barstate.ishistory" => Some(self.is_history),
            "barstate.isrealtime" => Some(self.is_realtime),
            "barstate.isnew" => Some(self.is_new),
            "barstate.isconfirmed" => Some(self.is_confirmed),
            "barstate.islastconfirmedhistory" => Some(self.is_last_confirmed_history),
            _ => None,
        }
    }
}

/// Per-bar, per-call evaluation context.
#[derive(Clone, Debug)]
pub struct RuntimeContext {
    pub bar: Kline,
    pub flags: BarFlags,
    /// Names declared `varip` in this bar's run, which persist across realtime ticks.
    pub intrabar_names: HashSet<String>,
    /// Non-zero inside a user function: the caller's call-site key, so builtins called in the
    /// body keep separate histories per call site.
    pub site_prefix: usize,
    pub depth: usize,
    /// The library whose code is running (`user/name/version`); None for the script itself.
    pub scope: Option<String>,
    /// Inside a library function: the names that are variables there.
    pub locals: Option<std::sync::Arc<HashSet<String>>>,
    /// Inside a user function: the locals the body reads a history of (`value[1]`), which keep a
    /// series per call site.
    pub history_locals: Option<std::sync::Arc<HashSet<String>>>,
}

impl RuntimeContext {
    pub fn new(bar: Kline, flags: BarFlags) -> Self {
        Self {
            bar,
            flags,
            intrabar_names: HashSet::new(),
            site_prefix: 0,
            depth: 0,
            scope: None,
            locals: None,
            history_locals: None,
        }
    }

    pub fn record(&mut self, mode: DeclarationMode, name: &str) {
        if mode == DeclarationMode::Intrabar {
            self.intrabar_names.insert(name.to_owned());
        } else {
            self.intrabar_names.remove(name);
        }
    }
}

/// A user-defined function, with the name sets needed to give each call a clean scope.
#[derive(Clone, Debug, PartialEq)]
pub struct RuntimeFunction {
    pub parameters: std::sync::Arc<Vec<Parameter>>,
    pub body: std::sync::Arc<Vec<Statement>>,
    /// Parameters and every name declared in the body; restored after each call.
    pub locals: std::sync::Arc<HashSet<String>>,
    /// `var`/`varip` locals, which persist per call site between calls.
    pub persistent_locals: std::sync::Arc<HashSet<String>>,
    /// The locals (and parameters) whose history the body reads: only those keep a series.
    pub history_locals: std::sync::Arc<HashSet<String>>,
    /// The library that defines the function; its body runs in that library's scope.
    pub scope: Option<String>,
    /// Whether the importing script may call it (a library's helpers are private).
    pub is_exported: bool,
}

impl RuntimeFunction {
    pub fn new(parameters: Vec<Parameter>, body: Vec<Statement>) -> Self {
        let mut locals: HashSet<String> = parameters.iter().map(|p| p.name.clone()).collect();
        let mut persistent = HashSet::new();
        collect_declarations(&body, &mut locals, &mut persistent);
        // Shared, so handing a function to a call does not copy its body.
        // Only the function's own names: `close[1]` inside a function still reads the chart's series.
        let history: HashSet<String> = history_bases(&body)
            .into_iter()
            .filter(|name| locals.contains(name))
            .collect();
        Self {
            parameters: parameters.into(),
            body: body.into(),
            locals: locals.into(),
            persistent_locals: persistent.into(),
            history_locals: history.into(),
            scope: None,
            is_exported: true,
        }
    }
}

fn collect_declarations(
    statements: &[Statement],
    names: &mut HashSet<String>,
    persistent: &mut HashSet<String>,
) {
    for statement in statements {
        match statement {
            Statement::Declaration {
                name, mode, value, ..
            } => {
                names.insert(name.clone());
                if *mode != DeclarationMode::Ordinary {
                    persistent.insert(name.clone());
                }
                if let Expression::Statement(block, _) = value {
                    collect_declarations(std::slice::from_ref(block.as_ref()), names, persistent);
                }
            }
            Statement::ForRange { variable, .. } => {
                names.insert(variable.clone());
            }
            Statement::ForIn { index, value, .. } => {
                if let Some(index) = index {
                    names.insert(index.clone());
                }
                names.insert(value.clone());
            }
            Statement::TupleDeclaration {
                names: tuple_names, ..
            } => {
                names.extend(tuple_names.iter().cloned());
            }
            _ => {}
        }
        for block in nested_blocks(statement) {
            collect_declarations(block, names, persistent);
        }
    }
}

/// The plain names the statements read a history of: `value` in `value[1]`.
fn history_bases(statements: &[Statement]) -> HashSet<String> {
    fn visit_expression(expression: &Expression, found: &mut HashSet<String>) {
        if let Expression::History { base, .. } = expression
            && let Expression::Identifier(name, _) = base.as_ref()
        {
            found.insert(name.clone());
        }
        if let Expression::Statement(statement, _) = expression {
            visit_statements(std::slice::from_ref(statement.as_ref()), found);
        }
        for child in crate::pine::syntax::traversal::children(expression) {
            visit_expression(child, found);
        }
    }
    fn visit_statements(statements: &[Statement], found: &mut HashSet<String>) {
        for statement in statements {
            for expression in crate::pine::syntax::traversal::expressions(statement) {
                visit_expression(expression, found);
            }
            for block in crate::pine::syntax::traversal::nested_blocks(statement) {
                visit_statements(block, found);
            }
        }
    }
    let mut found = HashSet::new();
    visit_statements(statements, &mut found);
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bar_flags_resolve_barstate_names() {
        let flags = BarFlags {
            is_first: true,
            is_confirmed: true,
            ..BarFlags::default()
        };
        assert_eq!(flags.value("barstate.isfirst"), Some(true));
        assert_eq!(flags.value("barstate.islast"), Some(false));
        assert_eq!(flags.value("barstate.nope"), None);
    }
}

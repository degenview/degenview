//! What a script declares, known without running it: its title, whether it draws over the price,
//! its inputs and the chart points they name. Compiled once per source and kept.

use crate::pine::compiler;
use crate::pine::compiler::program::InputSchema;
use crate::pine::inputs::points::{self, PointInput};
use crate::pine::syntax::token::Limits;
use crate::store::scripts::registry::LibraryRegistry;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Default)]
pub struct ScriptDeclaration {
    /// The `indicator()` / `strategy()` title, when it has one.
    pub title: Option<String>,
    pub overlay: bool,
    pub schema: InputSchema,
    pub points: Vec<PointInput>,
    /// Whether the script compiles. One that does not has nothing to ask for.
    pub valid: bool,
}

impl ScriptDeclaration {
    pub fn of(source: &str) -> Self {
        let program = compiler::compile(source, Limits::default(), &LibraryRegistry);
        let valid = program.is_valid();
        let title = program.declaration.title.trim();
        Self {
            title: (!title.is_empty()).then(|| title.to_owned()),
            overlay: program.declaration.overlay,
            points: points::points(&program.input_schema),
            schema: program.input_schema,
            valid,
        }
    }
}

/// The declarations of the sources asked about so far.
#[derive(Default)]
pub struct Declarations(RefCell<HashMap<String, Arc<ScriptDeclaration>>>);

impl Declarations {
    pub fn of(&self, source: &str) -> Arc<ScriptDeclaration> {
        if let Some(known) = self.0.borrow().get(source) {
            return Arc::clone(known);
        }
        let declared = Arc::new(ScriptDeclaration::of(source));
        self.0
            .borrow_mut()
            .insert(source.to_owned(), Arc::clone(&declared));
        declared
    }
}

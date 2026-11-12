//! Name resolution for imported libraries, ported from `PineRuntimeSession+Libraries.swift`.
//! A library's code runs in its own scope (the import path): its functions, types, enums and
//! constants shadow nothing of the script's and are visible to it only through the alias and only
//! when exported.
//!
//! The session wires the three entry points in — `scoped_function` into `call` (so `alias.fn()` and
//! `alias.Type.new()` resolve), `type_key` into type resolution, and `library_identifier` into
//! `resolve_identifier` (so `alias.constant` and `alias.Enum.member` resolve). The compiled linkage
//! (`LibraryLinkage`) is already built by `Session::new`.

use super::session::Session;
use super::types::{RuntimeContext, RuntimeFunction};
use super::value::RuntimeValue;
use crate::pine::syntax::token::{Diagnostic, DiagnosticCategory, SourceRange};

const SEPARATOR: &str = "::";

impl Session {
    /// The user function `name` names from where the code is running: the running library's own,
    /// the script's own, or `alias.name` of an imported library.
    pub(crate) fn scoped_function(
        &self,
        name: &str,
        range: SourceRange,
        context: &RuntimeContext,
    ) -> Result<Option<RuntimeFunction>, Diagnostic> {
        Ok(self
            .scoped_function_key(name, range, context)?
            .and_then(|key| self.functions.get(&key).cloned()))
    }

    /// The registry key of the function `name` refers to from the running code, if it is one.
    pub(crate) fn scoped_function_key(
        &self,
        name: &str,
        range: SourceRange,
        context: &RuntimeContext,
    ) -> Result<Option<String>, Diagnostic> {
        if let Some(scope) = &context.scope {
            let own = format!("{scope}{SEPARATOR}{name}");
            if self.functions.contains_key(&own) {
                return Ok(Some(own));
            }
        } else if self.functions.contains_key(name) {
            return Ok(Some(name.to_owned()));
        }
        let Some((path, member)) = self
            .linkage
            .resolve(name, context.scope.as_deref().unwrap_or(""))
        else {
            return Ok(None);
        };
        let key = format!("{path}{SEPARATOR}{member}");
        if !self.functions.contains_key(&key) {
            return Ok(None);
        }
        self.require_export(&member, &path, name, range)?;
        Ok(Some(key))
    }

    /// The registry key of the type `name` refers to from the running code, if it is one.
    pub(crate) fn type_key(
        &self,
        name: &str,
        range: SourceRange,
        context: &RuntimeContext,
    ) -> Result<Option<String>, Diagnostic> {
        if let Some(scope) = &context.scope {
            let own = format!("{scope}{SEPARATOR}{name}");
            if self.types.contains_key(&own) {
                return Ok(Some(own));
            }
        } else if self.types.contains_key(name) {
            return Ok(Some(name.to_owned()));
        }
        let Some((path, member)) = self
            .linkage
            .resolve(name, context.scope.as_deref().unwrap_or(""))
        else {
            return Ok(None);
        };
        let key = format!("{path}{SEPARATOR}{member}");
        if !self.types.contains_key(&key) {
            return Ok(None);
        }
        self.require_export(&member, &path, name, range)?;
        Ok(Some(key))
    }

    /// A constant or enum member a library defines, read from inside it or through an import alias.
    /// None when `name` is neither, so the ordinary lookup carries on.
    pub(crate) fn library_identifier(
        &mut self,
        name: &str,
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<Option<RuntimeValue>, Diagnostic> {
        if let Some(scope) = context.scope.clone()
            && self.isolated_from_script(name, context)
            && let Some(value) = self.library_member(name, &scope, range, context)?
        {
            return Ok(Some(value));
        }
        let Some((path, member)) = self
            .linkage
            .resolve(name, context.scope.as_deref().unwrap_or(""))
        else {
            return Ok(None);
        };
        let Some(value) = self.library_member(&member, &path, range, context)? else {
            return Ok(None);
        };
        let root = member.split('.').next().unwrap_or(&member);
        self.require_export(root, &path, name, range)?;
        Ok(Some(value))
    }

    /// Whether `name` must not read the script's variables: library code only sees its own locals.
    fn isolated_from_script(&self, name: &str, context: &RuntimeContext) -> bool {
        if context.scope.is_none() {
            return false;
        }
        let root = name.split('.').next().unwrap_or(name);
        !context
            .locals
            .as_ref()
            .is_some_and(|locals| locals.contains(root))
    }

    /// A library's constant or enum member, by name, within `path`'s scope.
    fn library_member(
        &mut self,
        name: &str,
        path: &str,
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<Option<RuntimeValue>, Diagnostic> {
        if let Some(value) = self.library_constant(name, path, range, context)? {
            return Ok(Some(value));
        }
        let Some((enum_name, member)) = name.split_once('.') else {
            return Ok(None);
        };
        let key = format!("{path}{SEPARATOR}{enum_name}");
        let Some(members) = self.enums.get(&key) else {
            return Ok(None);
        };
        if !members.iter().any(|item| item.name == member) {
            return Err(Diagnostic::error(
                "PINE4025",
                DiagnosticCategory::Runtime,
                format!("Enum '{enum_name}' has no member '{member}'."),
                range,
            ));
        }
        Ok(Some(RuntimeValue::String(format!("{key}.{member}"))))
    }

    /// Evaluates a library constant, caching the value (a reference is never cached: it names a
    /// value in the library's own state).
    fn library_constant(
        &mut self,
        name: &str,
        path: &str,
        range: SourceRange,
        context: &mut RuntimeContext,
    ) -> Result<Option<RuntimeValue>, Diagnostic> {
        let Some(expression) = self
            .linkage
            .constants
            .get(path)
            .and_then(|constants| constants.get(name))
            .cloned()
        else {
            return Ok(None);
        };
        let key = format!("{path}{SEPARATOR}{name}");
        if let Some(cached) = self.library_constants.get(&key) {
            return Ok(Some(cached.clone()));
        }
        if !self.constants_in_progress.insert(key.clone()) {
            return Err(Diagnostic::error(
                "PINE4032",
                DiagnosticCategory::Runtime,
                format!("Library constant '{name}' is defined in terms of itself."),
                range,
            ));
        }
        let mut inner = context.clone();
        inner.scope = Some(path.to_owned());
        inner.locals = Some(Default::default());
        let value = self.eval(&expression, &mut inner);
        self.constants_in_progress.remove(&key);
        let value = value?;
        if matches!(value, RuntimeValue::Ref(_, _)) {
            return Ok(Some(value));
        }
        self.library_constants.insert(key, value.clone());
        Ok(Some(value))
    }

    /// Whether `written` is an exported name of `path`, or a runtime error.
    fn require_export(
        &self,
        member: &str,
        path: &str,
        written: &str,
        range: SourceRange,
    ) -> Result<(), Diagnostic> {
        if self
            .linkage
            .exports
            .get(path)
            .is_some_and(|exports| exports.contains(member))
        {
            Ok(())
        } else {
            Err(Diagnostic::error(
                "PINE4031",
                DiagnosticCategory::Runtime,
                format!("'{written}' is not exported by the library."),
                range,
            ))
        }
    }
}

/// The library a registry key (`"user/name/1::Type"`) belongs to; nil for the script's own.
fn library_of(key: &str) -> Option<&str> {
    key.split_once(SEPARATOR).map(|(path, _)| path)
}

#[allow(dead_code)] // kept for symmetry with the Swift helper; the session resolves by `linkage`
fn _separator() -> &'static str {
    SEPARATOR
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pine::compiler::program::SymbolInfo;
    use crate::pine::compiler::{self, NoLibraries};
    use crate::pine::syntax::token::Limits;
    use std::collections::HashMap;

    struct MapResolver(HashMap<String, String>);

    impl compiler::LibraryResolver for MapResolver {
        fn source_for_library(&self, path: &str) -> Option<String> {
            self.0.get(path).cloned()
        }
    }

    #[test]
    fn library_of_splits_on_the_separator() {
        assert_eq!(library_of("user/Math/1::Type"), Some("user/Math/1"));
        assert_eq!(library_of("plain"), None);
    }

    #[test]
    fn an_import_resolves_and_compiles_the_library() {
        let mut libraries = HashMap::new();
        libraries.insert(
            "user/Math/1".to_owned(),
            "//@version=6\nlibrary(\"Math\")\nexport double(float x) => x * 2\n".to_owned(),
        );
        let resolver = MapResolver(libraries);
        let program = compiler::compile(
            "//@version=6\nindicator(\"T\")\nimport user/Math/1 as m\ny = m.double(21)",
            Limits::default(),
            &resolver,
        );
        assert!(
            program
                .diagnostics
                .iter()
                .all(|d| d.severity != crate::pine::DiagnosticSeverity::Error)
        );
        assert_eq!(program.imports.len(), 1);
        assert_eq!(program.imports[0].alias, "m");
        assert!(program.imports[0].library.is_some());
        // The compiled library registers its exported function under the scoped key.
        let session = Session::new(
            program,
            HashMap::new(),
            Limits::default(),
            None,
            SymbolInfo::default(),
            None,
            None,
        );
        assert!(session.functions.contains_key("user/Math/1::double"));
    }

    #[test]
    fn a_missing_library_is_pine3040() {
        let program = compiler::compile(
            "//@version=6\nindicator(\"T\")\nimport user/Nope/1",
            Limits::default(),
            &NoLibraries,
        );
        assert!(program.diagnostics.iter().any(|d| d.code == "PINE3040"));
    }
}

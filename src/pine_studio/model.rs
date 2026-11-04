//! Pine Studio value types, ported from `Model/Script/*.swift`.

use crate::pine::program::ScriptType as PineScriptType;
use crate::pine::token::{Diagnostic, DiagnosticSeverity};
use serde::{Deserialize, Serialize};

/// A script's stable identity: 32 hex digits, minted once and kept in the file's extended
/// attribute (and `index.json`), so it survives renames and moves.
pub type ScriptId = String;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ScriptType {
    Indicator,
    Strategy,
    Library,
}

impl ScriptType {
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Indicator => "Indicator",
            Self::Strategy => "Strategy",
            Self::Library => "Library",
        }
    }
}

impl From<PineScriptType> for ScriptType {
    fn from(value: PineScriptType) -> Self {
        match value {
            PineScriptType::Indicator => Self::Indicator,
            PineScriptType::Strategy => Self::Strategy,
            PineScriptType::Library => Self::Library,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CompileStatus {
    #[default]
    NotCompiled,
    Valid,
    Warning,
    Error,
}

impl CompileStatus {
    pub fn of(diagnostics: &[Diagnostic]) -> Self {
        if diagnostics
            .iter()
            .any(|d| d.severity == DiagnosticSeverity::Error)
        {
            Self::Error
        } else if diagnostics.is_empty() {
            Self::Valid
        } else {
            Self::Warning
        }
    }
}

/// What the last compile of a saved script said; reused until the source changes.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptCompileRecord {
    pub source_hash: String,
    pub compiler_version: String,
    pub pine_version: Option<i64>,
    pub status: CompileStatus,
    pub diagnostics: Vec<Diagnostic>,
    pub compiled_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LocalScript {
    pub id: ScriptId,
    pub name: String,
    pub script_type: ScriptType,
    pub source: String,
    pub latest_revision_id: Option<String>,
    pub created_at_ms: i64,
    pub modified_at_ms: i64,
    pub is_favorite: bool,
    pub compile_record: Option<ScriptCompileRecord>,
}

/// Unsaved editor text, kept beside the script so a crash or quit loses nothing.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptDraft {
    pub script_id: ScriptId,
    pub source: String,
    pub modified_at_ms: i64,
    pub based_on_revision_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptVersion {
    pub id: String,
    pub script_id: ScriptId,
    pub created_at_ms: i64,
    pub source: String,
    pub compile_status: CompileStatus,
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

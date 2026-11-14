//! Bringing the Swift app's scripts and preview inputs across on first use, from the metadata
//! files it left in its data folder.

use super::{PineStudio, inputs};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

impl PineStudio {
    /// Once: the input values the Swift app saved for its preview, matched to this library's
    /// scripts by file name (the two apps number their scripts differently). Values already tried
    /// here win, and a script the user later resets is not filled again.
    pub(super) fn import_swift_inputs(&mut self) {
        let done: bool = self
            .shared
            .lock()
            .ok()
            .and_then(|s| s.setting(SWIFT_INPUTS_KEY))
            .unwrap_or(false);
        let Some(folder) = swift_data_dir().filter(|_| !done) else {
            return;
        };
        let by_swift_id = swift_script_names(&folder);
        let inputs = std::fs::read(folder.join("script_preview_inputs.json"))
            .map(|bytes| inputs::parse_swift_inputs(&bytes))
            .unwrap_or_default();
        let imported = inputs
            .into_iter()
            .filter_map(|(swift_id, values)| {
                let name = by_swift_id.get(&swift_id)?;
                let script = self.scripts.iter().find(|s| &s.name == name)?;
                Some((script.id.clone(), values))
            })
            .collect();
        self.preview.import_inputs(imported);
        if let Ok(shared) = self.shared.lock() {
            let _ = shared.set_setting(SWIFT_INPUTS_KEY, &true);
        }
    }
}

/// Where the Swift app keeps its data, to bring it across on first use.
fn swift_data_dir() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join("Library/Application Support/DegenView"))
}

pub(super) fn swift_scripts_dir() -> Option<PathBuf> {
    swift_data_dir().map(|dir| dir.join("Scripts"))
}

/// Setting that records the Swift preview inputs were brought across.
const SWIFT_INPUTS_KEY: &str = "scripts.swiftInputsImported";

/// The Swift app's script UUID → script name, from its `ScriptMetadata/index.json` (a Swift
/// dictionary with `UUID` keys is encoded as a flat `[key, value, …]` array).
fn swift_script_names(data_dir: &Path) -> HashMap<String, String> {
    let Ok(bytes) = std::fs::read(data_dir.join("ScriptMetadata/index.json")) else {
        return HashMap::new();
    };
    parse_swift_index(&bytes)
}

pub(super) fn parse_swift_index(bytes: &[u8]) -> HashMap<String, String> {
    let stem = |file: &str| file.strip_suffix(".pine").unwrap_or(file).to_owned();
    match serde_json::from_slice::<serde_json::Value>(bytes) {
        Ok(serde_json::Value::Array(flat)) => flat
            .as_chunks::<2>()
            .0
            .iter()
            .filter_map(|[id, file]| Some((id.as_str()?.to_owned(), stem(file.as_str()?))))
            .collect(),
        Ok(serde_json::Value::Object(map)) => map
            .into_iter()
            .filter_map(|(id, file)| Some((id, stem(file.as_str()?))))
            .collect(),
        _ => HashMap::new(),
    }
}

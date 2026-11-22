//! `graphics.json` in the data directory: what `ladder::plan` needs from the previous start. Every
//! call is best effort. A missing, unreadable or corrupt file is the first start, and a write that
//! fails costs only the fallback: a diagnostic must never be the reason the app fails to start.

use std::path::Path;

use super::ladder::State;

const FILE: &str = "graphics.json";

pub fn load(dir: &Path) -> State {
    std::fs::read(dir.join(FILE))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

pub fn save(dir: &Path, state: &State) {
    let _ = std::fs::create_dir_all(dir);
    if let Ok(json) = serde_json::to_vec_pretty(state) {
        let _ = std::fs::write(dir.join(FILE), json);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("dv-graphics-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_saved_state_loads_back_and_the_directory_is_made() {
        let dir = scratch("roundtrip");
        let state = State {
            build: "abc".to_owned(),
            failed: vec!["dx12".to_owned()],
            attempting: Some("gl".to_owned()),
        };
        save(&dir, &state);
        assert_eq!(load(&dir), state);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_file_is_the_first_start() {
        assert_eq!(load(&scratch("missing")), State::default());
    }

    #[test]
    fn a_corrupt_file_is_the_first_start() {
        let dir = scratch("corrupt");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(FILE), b"{ not json").unwrap();
        assert_eq!(load(&dir), State::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_missing_fields_loads_with_defaults() {
        let dir = scratch("partial");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(FILE), br#"{"build":"abc"}"#).unwrap();
        let state = load(&dir);
        assert_eq!(state.build, "abc");
        assert!(state.failed.is_empty() && state.attempting.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

//! `graphics.json` in the data directory: the `Record` kept between launches. Every call is best
//! effort. A missing, unreadable or corrupt file is a first start, and a write that fails costs only
//! the memory between launches: a diagnostic must never be the reason the app fails to start.

use std::path::Path;

use super::record::Record;

const FILE: &str = "graphics.json";

pub fn load(dir: &Path) -> Record {
    std::fs::read(dir.join(FILE))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

pub fn save(dir: &Path, record: &Record) {
    let _ = std::fs::create_dir_all(dir);
    if let Ok(json) = serde_json::to_vec_pretty(record) {
        let _ = std::fs::write(dir.join(FILE), json);
    }
}

#[cfg(test)]
mod tests {
    use super::super::record::Attempt;
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("dv-persist-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_saved_record_loads_back_and_the_directory_is_made() {
        let dir = scratch("roundtrip");
        let record = Record {
            build: "abc".to_owned(),
            failed: vec!["dx12".to_owned()],
            attempting: Some(Attempt {
                rung: "gl".to_owned(),
                launch: "1-2".to_owned(),
            }),
        };
        save(&dir, &record);
        assert_eq!(load(&dir), record);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_file_is_the_first_start() {
        assert_eq!(load(&scratch("missing")), Record::default());
    }

    #[test]
    fn a_corrupt_file_is_the_first_start() {
        let dir = scratch("corrupt");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(FILE), b"{ not json").unwrap();
        assert_eq!(load(&dir), Record::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_missing_fields_loads_with_defaults() {
        let dir = scratch("partial");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(FILE), br#"{"build":"abc"}"#).unwrap();
        let record = load(&dir);
        assert_eq!(record.build, "abc");
        assert!(record.failed.is_empty() && record.attempting.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_directory_in_the_files_place_is_a_first_start_and_saving_fails_quietly() {
        let dir = scratch("directory");
        std::fs::create_dir_all(dir.join(FILE)).unwrap();
        assert_eq!(load(&dir), Record::default());
        save(&dir, &Record::default());
        assert!(dir.join(FILE).is_dir());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

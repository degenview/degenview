//! The module layout `docs/agent-guides/core.md` asks for, checked so it cannot drift back:
//! a module with children is `foo/mod.rs` (never `foo.rs` beside `foo/`), and three files that
//! share a prefix in one directory belong in a directory of their own.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Every directory under `src/`, with the names of the `.rs` files and the subdirectories in it.
fn directories() -> Vec<(PathBuf, Vec<String>, Vec<String>)> {
    fn walk(dir: &Path, found: &mut Vec<(PathBuf, Vec<String>, Vec<String>)>) {
        let mut files = Vec::new();
        let mut dirs = Vec::new();
        for entry in fs::read_dir(dir)
            .expect("source directory readable")
            .flatten()
        {
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            if path.is_dir() {
                walk(&path, found);
                dirs.push(name);
            } else if let Some(stem) = name.strip_suffix(".rs") {
                files.push(stem.to_owned());
            }
        }
        files.sort();
        dirs.sort();
        found.push((dir.to_path_buf(), files, dirs));
    }
    let mut found = Vec::new();
    walk(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut found,
    );
    found
}

#[test]
fn no_module_sits_beside_a_directory_of_the_same_name() {
    let mut offenders = Vec::new();
    for (dir, files, dirs) in directories() {
        for file in files.iter().filter(|file| dirs.contains(file)) {
            offenders.push(format!("{0}/{1}.rs beside {0}/{1}/", dir.display(), file));
        }
    }
    assert!(
        offenders.is_empty(),
        "move each into its directory as mod.rs:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn no_directory_holds_three_files_with_one_prefix() {
    let mut offenders = Vec::new();
    for (dir, files, _) in directories() {
        let mut by_prefix: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for file in files
            .iter()
            .map(String::as_str)
            .filter(|file| *file != "mod" && !file.ends_with("_tests"))
        {
            if let Some((prefix, _)) = file.split_once('_') {
                by_prefix.entry(prefix).or_default().push(file);
            }
        }
        for (prefix, group) in by_prefix.into_iter().filter(|(_, group)| group.len() >= 3) {
            offenders.push(format!(
                "{}: {} share the prefix `{prefix}_`",
                dir.display(),
                group.join(", ")
            ));
        }
    }
    assert!(
        offenders.is_empty(),
        "group them in a directory named for the prefix, without the prefix in the file names:\n{}",
        offenders.join("\n")
    );
}

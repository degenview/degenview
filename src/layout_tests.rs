//! The module layout `docs/agent-guides/core.md` asks for, checked so it cannot drift back:
//! a module with children is `foo/mod.rs` (never `foo.rs` beside `foo/`), three files that
//! share a prefix in one directory belong in a directory of their own, and no layer imports a
//! layer above it or draws outside the view layers.

use std::collections::{BTreeMap, BTreeSet};
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

/// The layers a top-level module may not import from (`docs/architecture/modules.md`): pure logic
/// knows nothing of I/O, coordinators or drawing; I/O knows nothing of coordinators or drawing; and
/// nothing below the app reaches up into it.
const FORBIDDEN_IMPORTS: &[(&str, &[&str])] = &[
    (
        "model",
        &[
            "app",
            "host",
            "mcp",
            "native_bridge",
            "pine_studio",
            "service",
            "store",
            "sync",
            "trading",
            "ui",
            "workspace",
        ],
    ),
    (
        "pine",
        &[
            "app",
            "host",
            "mcp",
            "native_bridge",
            "pine_studio",
            "service",
            "store",
            "sync",
            "trading",
            "ui",
            "workspace",
        ],
    ),
    ("service", &["app", "host", "ui", "workspace"]),
    ("store", &["app", "host", "ui", "workspace"]),
    ("sync", &["app", "host", "ui", "workspace"]),
    ("mcp", &["app", "ui", "workspace"]),
    ("trading", &["app", "ui", "workspace"]),
    ("host", &["app"]),
    ("ui", &["app"]),
];

/// Imports that break [`FORBIDDEN_IMPORTS`] today, as `(file under src/, module imported)`. Each is
/// a known debt with its fix planned in `docs/architecture-refactor/plan.md`; the list only
/// shrinks. An entry that no longer breaks the rule fails the test too, so it is removed with the
/// fix.
const KNOWN_LAYERING_DEBT: &[(&str, &str)] = &[];

/// The top-level modules a file's non-test code names through `crate::`, including those inside
/// a `use crate::{a::.., b::..}` group.
fn crate_imports(source: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let code = production_code(source);
    let mut rest = code.as_str();
    while let Some(at) = rest.find("crate::") {
        rest = &rest[at + "crate::".len()..];
        if let Some(group) = rest.strip_prefix('{') {
            // The group's top level only: `a::{x, y}` contributes `a`, not `x` or `y`.
            let mut depth = 0usize;
            let mut top = String::new();
            for ch in group.chars() {
                match ch {
                    '{' => depth += 1,
                    '}' if depth == 0 => break,
                    '}' => depth -= 1,
                    _ if depth == 0 => top.push(ch),
                    _ => {}
                }
            }
            for item in top.split(',') {
                let head = identifier(item.trim_start());
                if !head.is_empty() {
                    found.insert(head);
                }
            }
        } else {
            let head = identifier(rest);
            if !head.is_empty() {
                found.insert(head);
            }
        }
    }
    found
}

fn identifier(text: &str) -> String {
    text.chars()
        .take_while(|ch| ch.is_alphanumeric() || *ch == '_')
        .collect()
}

/// The source before its inline test module (tests import fixtures from anywhere), with comment
/// lines left out (doc links name other layers without depending on them).
fn production_code(source: &str) -> String {
    let mut end = source.len();
    let mut from = 0;
    while let Some(at) = source[from..].find("#[cfg(test)]") {
        let attribute = from + at;
        let after = source[attribute + "#[cfg(test)]".len()..].trim_start();
        if after.starts_with("mod ") || after.starts_with("pub(crate) mod ") {
            end = attribute;
            break;
        }
        from = attribute + 1;
    }
    source[..end]
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Every non-test `.rs` file under `src/<module>/`, relative to `src/`.
fn module_sources(module: &str) -> Vec<(String, String)> {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    let mut pending = vec![src.join(module)];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir)
            .expect("source directory readable")
            .flatten()
        {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if path.is_dir() {
                if name != "tests" {
                    pending.push(path);
                }
            } else if name.ends_with(".rs") && name != "tests.rs" && !name.ends_with("_tests.rs") {
                let relative = path
                    .strip_prefix(&src)
                    .expect("under src")
                    .to_string_lossy()
                    .replace('\\', "/");
                let text = fs::read_to_string(&path).expect("source readable");
                files.push((relative, text));
            }
        }
    }
    files.sort();
    files
}

#[test]
fn no_layer_imports_a_layer_above_it() {
    let mut offenders = Vec::new();
    let mut found_debt = BTreeSet::new();
    for (module, forbidden) in FORBIDDEN_IMPORTS {
        for (file, text) in module_sources(module) {
            for import in crate_imports(&text) {
                if !forbidden.contains(&import.as_str()) {
                    continue;
                }
                if KNOWN_LAYERING_DEBT.contains(&(file.as_str(), import.as_str())) {
                    found_debt.insert((file.clone(), import));
                } else {
                    offenders.push(format!("{file} imports crate::{import}"));
                }
            }
        }
    }
    let settled: Vec<String> = KNOWN_LAYERING_DEBT
        .iter()
        .filter(|(file, import)| !found_debt.contains(&((*file).to_owned(), (*import).to_owned())))
        .map(|(file, import)| format!("(\"{file}\", \"{import}\")"))
        .collect();
    assert!(
        offenders.is_empty(),
        "move the code to the layer it belongs in (docs/architecture/modules.md):\n{}",
        offenders.join("\n")
    );
    assert!(
        settled.is_empty(),
        "these no longer break the layering; remove them from KNOWN_LAYERING_DEBT:\n{}",
        settled.join("\n")
    );
}

/// Modules that never draw: they hand views data and take actions back, so no egui surface (a
/// `Ui`, a window or a panel) appears in them. A `Context` to request a repaint or upload a texture
/// is fine.
const NON_DRAWING_MODULES: &[&str] = &[
    "host", "mcp", "model", "pine", "service", "store", "sync", "trading",
];

/// What drawing code names.
const DRAWING_SURFACES: &[&str] = &[
    "egui::Ui",
    "&mut Ui",
    "egui::Window",
    "egui::Area",
    "egui::Modal",
    "egui::CentralPanel",
    "egui::SidePanel",
    "egui::TopBottomPanel",
];

/// Files under [`NON_DRAWING_MODULES`] that draw today, each planned to move into `ui/` in
/// `docs/architecture-refactor/plan.md`. The list only shrinks.
const KNOWN_DRAWING_DEBT: &[&str] = &[];

#[test]
fn nothing_outside_the_view_layers_draws() {
    let mut offenders = Vec::new();
    let mut drawing = BTreeSet::new();
    for module in NON_DRAWING_MODULES {
        for (file, text) in module_sources(module) {
            let code = production_code(&text);
            if !DRAWING_SURFACES
                .iter()
                .any(|surface| code.contains(surface))
            {
                continue;
            }
            if KNOWN_DRAWING_DEBT.contains(&file.as_str()) {
                drawing.insert(file);
            } else {
                offenders.push(file);
            }
        }
    }
    let settled: Vec<&str> = KNOWN_DRAWING_DEBT
        .iter()
        .copied()
        .filter(|file| !drawing.contains(*file))
        .collect();
    assert!(
        offenders.is_empty(),
        "views belong in ui/; hand the view data and take its actions back:\n{}",
        offenders.join("\n")
    );
    assert!(
        settled.is_empty(),
        "these no longer draw; remove them from KNOWN_DRAWING_DEBT:\n{}",
        settled.join("\n")
    );
}

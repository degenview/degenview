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
    (
        "service",
        &["app", "host", "mcp", "sync", "ui", "workspace"],
    ),
    // The venues report what failed as a kind of failure; the words are written at the edge
    // (`service::provider_message`), so the market code can move out of the app. They ask the time
    // of their `Clock` port (`venues_reach_the_world_only_through_their_ports` holds the rest).
    ("service/market", &["clock", "i18n"]),
    (
        "store",
        &["app", "host", "pine_studio", "service", "ui", "workspace"],
    ),
    ("sync", &["app", "host", "ui", "workspace"]),
    // What a record is and which tables sync: the store builds on it, so it may not need the store.
    ("sync/record", &["store"]),
    ("mcp", &["app", "ui", "workspace"]),
    ("trading", &["app", "ui", "workspace"]),
    ("host", &["app"]),
    ("ui", &["app"]),
    // Leaves every layer may use: they import nothing of the app's own.
    ("build_info", EVERY_MODULE),
    ("clock", EVERY_MODULE),
    ("format", EVERY_MODULE),
    ("i18n", EVERY_MODULE),
    ("lock", EVERY_MODULE),
    ("paths", EVERY_MODULE),
    // Chooses the renderer before anything else starts: it needs only the data directory, the build
    // id and the startup log.
    (
        "graphics",
        &[
            "app",
            "clock",
            "format",
            "host",
            "mcp",
            "model",
            "native_bridge",
            "pine",
            "pine_studio",
            "platform",
            "service",
            "store",
            "sync",
            "trading",
            "ui",
            "workspace",
        ],
    ),
    // Runs from the panic hook, before and after the store opens: it may not need either.
    (
        "crash_report",
        &[
            "app",
            "graphics",
            "host",
            "mcp",
            "pine_studio",
            "service",
            "store",
            "sync",
            "trading",
            "ui",
            "workspace",
        ],
    ),
];

/// Every top-level module, for the leaves that may import none of them.
const EVERY_MODULE: &[&str] = &[
    "app",
    "build_info",
    "clock",
    "crash_report",
    "format",
    "graphics",
    "host",
    "i18n",
    "lock",
    "mcp",
    "model",
    "native_bridge",
    "paths",
    "pine",
    "pine_studio",
    "platform",
    "service",
    "store",
    "sync",
    "trading",
    "ui",
    "workspace",
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

#[test]
fn every_module_names_each_module_of_the_crate() {
    let lib = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs"))
        .expect("lib.rs readable");
    let declared: BTreeSet<&str> = lib
        .lines()
        .filter_map(|line| line.strip_prefix("mod ")?.strip_suffix(';'))
        .filter(|module| !["alloc_count", "layout_tests"].contains(module))
        .collect();
    let listed: BTreeSet<&str> = EVERY_MODULE.iter().copied().collect();
    assert_eq!(declared, listed, "keep EVERY_MODULE in step with lib.rs");
}

/// Every non-test `.rs` file of `module` (`src/<module>/` or the single file `src/<module>.rs`),
/// relative to `src/`.
fn module_sources(module: &str) -> Vec<(String, String)> {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let single = src.join(format!("{module}.rs"));
    if single.is_file() {
        let text = fs::read_to_string(&single).expect("source readable");
        return vec![(format!("{module}.rs"), text)];
    }
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
                // A module's own paths (`crate::format::…` inside `format.rs`) are no dependency.
                let own = module.split('/').next() == Some(import.as_str());
                if own || !forbidden.contains(&import.as_str()) {
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

/// What `service/market` may not name, as the path of a module or item and what a venue has
/// instead (`service::market::ports`). A segment ending in `*` is a prefix: `fetch_*` is every
/// `service::fetch_…`. The implementations the app runs on are in `service::wiring`, which is
/// where they are named; `service::for_source` and the `service::fetch_*` functions answer from
/// them, so a venue that calls one reaches the app's own wiring from inside it.
const PORTED_GLOBALS: &[(&[&str], &str)] = &[
    (&["crate", "service", "net", "http"], "the Http port"),
    (&["crate", "service", "net", "agent"], "the Http port"),
    (
        &["crate", "service", "net", "rate_limit"],
        "a Pace of the Http port",
    ),
    (&["ureq"], "the Http port"),
    (
        &["crate", "service", "cache", "lookup"],
        "the CacheStore port",
    ),
    (&["crate", "service", "keychain"], "the Credentials port"),
    (&["std", "time", "SystemTime"], "the Clock port"),
    (
        &["crate", "service", "wiring"],
        "the context the venue is built with",
    ),
    (
        &["crate", "service", "for_source"],
        "the venue the caller holds",
    ),
    (
        &["crate", "service", "fetch_*"],
        "the venue or the fetch the caller holds",
    ),
];

/// Uses of [`PORTED_GLOBALS`] that are not behind a port yet, as `(file under src/, path named)`.
/// None is a venue. The coin index is an application service with a global of its own, and the FX
/// rates keep a transport of their own (raw replies, their own retry schedule); both are planned
/// for `docs/architecture-refactor/plan.md` step 6. The list only shrinks, and an entry that no
/// longer matches fails the test too.
const KNOWN_UNPORTED: &[(&str, &str)] = &[
    (
        "service/market/coin_index.rs",
        "crate::service::cache::lookup",
    ),
    ("service/market/coin_index.rs", "crate::service::net::http"),
    (
        "service/market/coin_index.rs",
        "crate::service::net::rate_limit",
    ),
    ("service/market/fx_rates.rs", "crate::service::for_source"),
    ("service/market/fx_rates.rs", "crate::service::net::agent"),
    ("service/market/fx_rates.rs", "std::time::SystemTime"),
    ("service/market/fx_rates.rs", "ureq"),
];

/// A token of Rust source as far as paths go: a name, `::`, or any other single character.
#[derive(Clone, Debug, Eq, PartialEq)]
enum Token {
    Name(String),
    PathSep,
    Punct(char),
}

/// The tokens of `source`, with comments dropped and the inside of every string and character
/// literal left out (a path in a message or a doc is no dependency).
fn tokens(source: &str) -> Vec<Token> {
    let chars: Vec<char> = source.chars().collect();
    let mut found = Vec::new();
    let mut at = 0;
    while at < chars.len() {
        let ch = chars[at];
        let next = chars.get(at + 1).copied();
        match ch {
            '/' if next == Some('/') => {
                while at < chars.len() && chars[at] != '\n' {
                    at += 1;
                }
            }
            '/' if next == Some('*') => {
                let mut depth = 0;
                while at < chars.len() {
                    match (chars[at], chars.get(at + 1)) {
                        ('/', Some('*')) => {
                            depth += 1;
                            at += 2;
                        }
                        ('*', Some('/')) => {
                            depth -= 1;
                            at += 2;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => at += 1,
                    }
                }
            }
            '"' => at = end_of_string(&chars, at + 1, 0),
            'r' | 'b'
                if raw_string_hashes(&chars, at).is_some()
                    && !chars
                        .get(at.wrapping_sub(1))
                        .is_some_and(|before| before.is_alphanumeric() || *before == '_') =>
            {
                let hashes = raw_string_hashes(&chars, at).unwrap_or(0);
                let quote = chars[at..].iter().position(|c| *c == '"').unwrap_or(0);
                at = end_of_string(&chars, at + quote + 1, hashes + 1);
            }
            '\'' => {
                // A character literal is `'x'` or `'\…'`; anything else is a lifetime.
                let literal = match (next, chars.get(at + 2)) {
                    (Some('\\'), _) => chars[at + 2..]
                        .iter()
                        .position(|c| *c == '\'')
                        .map(|n| n + 3),
                    (Some(_), Some('\'')) => Some(3),
                    _ => None,
                };
                at += literal.unwrap_or(1);
            }
            ':' if next == Some(':') => {
                found.push(Token::PathSep);
                at += 2;
            }
            c if c.is_alphabetic() || c == '_' => {
                let mut name = String::new();
                while at < chars.len() && (chars[at].is_alphanumeric() || chars[at] == '_') {
                    name.push(chars[at]);
                    at += 1;
                }
                found.push(Token::Name(name));
            }
            c if c.is_whitespace() => at += 1,
            c => {
                found.push(Token::Punct(c));
                at += 1;
            }
        }
    }
    found
}

/// How many `#` open the raw string (`r"…"`, `br#"…"#`) that starts at `at`, if one does.
fn raw_string_hashes(chars: &[char], at: usize) -> Option<usize> {
    let mut cursor = at;
    if chars[cursor] == 'b' {
        cursor += 1;
    }
    if chars.get(cursor) != Some(&'r') {
        return None;
    }
    cursor += 1;
    let hashes = chars[cursor..].iter().take_while(|c| **c == '#').count();
    (chars.get(cursor + hashes) == Some(&'"')).then_some(hashes)
}

/// Where the string whose contents start at `from` ends (after its closing quote and the `closing`
/// hashes of a raw one, which has no escapes; 0 for an ordinary string).
fn end_of_string(chars: &[char], from: usize, closing: usize) -> usize {
    let mut at = from;
    while at < chars.len() {
        match chars[at] {
            '\\' if closing == 0 => at += 2,
            '"' if closing <= 1 => return at + 1,
            '"' if chars[at + 1..].iter().take_while(|c| **c == '#').count() >= closing - 1 => {
                return at + closing;
            }
            _ => at += 1,
        }
    }
    chars.len()
}

/// The paths a `use` tree names, one per leaf, with the name each leaf is known by (`None` for a
/// glob). `tokens` start after the `use` keyword and stop before its `;`.
fn use_leaves(tokens: &[Token], prefix: &[String]) -> Vec<(Vec<String>, Option<String>)> {
    let mut leaves = Vec::new();
    let mut path = prefix.to_vec();
    let mut at = 0;
    let mut alias = None;
    while at < tokens.len() {
        match &tokens[at] {
            Token::Name(name) if name == "as" => {
                if let Some(Token::Name(local)) = tokens.get(at + 1) {
                    alias = Some(local.clone());
                }
                at += 2;
            }
            Token::Name(name) if name == "self" && path.len() > prefix.len() => at += 1,
            Token::Name(name) if name == "self" => {
                // `a::{self, b}`: the module `a` itself.
                at += 1;
            }
            Token::Name(name) => {
                path.push(name.clone());
                at += 1;
            }
            Token::PathSep => at += 1,
            Token::Punct('*') => {
                path.push("*".to_owned());
                at += 1;
            }
            Token::Punct('{') => {
                let mut depth = 0;
                let mut end = at;
                for (offset, token) in tokens[at..].iter().enumerate() {
                    match token {
                        Token::Punct('{') => depth += 1,
                        Token::Punct('}') => {
                            depth -= 1;
                            if depth == 0 {
                                end = at + offset;
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                let inner = &tokens[at + 1..end];
                let mut start = 0;
                let mut level = 0;
                for (offset, token) in inner.iter().enumerate() {
                    match token {
                        Token::Punct('{') => level += 1,
                        Token::Punct('}') => level -= 1,
                        Token::Punct(',') if level == 0 => {
                            leaves.extend(use_leaves(&inner[start..offset], &path));
                            start = offset + 1;
                        }
                        _ => {}
                    }
                }
                if start < inner.len() {
                    leaves.extend(use_leaves(&inner[start..], &path));
                }
                return leaves;
            }
            Token::Punct(_) => at += 1,
        }
    }
    if path.len() > prefix.len() || tokens.iter().any(|t| *t == Token::Name("self".to_owned())) {
        let local = alias.or_else(|| path.last().filter(|last| *last != "*").cloned());
        leaves.push((path, local));
    }
    leaves
}

/// Every path `source` names, resolved: a `use` leaf as it is written out, and a path in the code
/// through the names the file's `use`s gave its first segment (`use a::b as c; c::d()` names
/// `a::b::d`). `module` is where the file sits (`["crate", "service", "market", "okx"]`), which is
/// what `self::` and `super::` start from.
fn named_paths(source: &str, module: &[String]) -> Vec<Vec<String>> {
    let tokens = tokens(source);
    let absolute = |path: Vec<String>| -> Vec<String> {
        let mut base = module.to_vec();
        let mut rest = path.as_slice();
        match rest.first().map(String::as_str) {
            Some("self") => rest = &rest[1..],
            Some("super") => {
                while rest.first().map(String::as_str) == Some("super") {
                    base.pop();
                    rest = &rest[1..];
                }
            }
            _ => return path,
        }
        base.extend(rest.iter().cloned());
        base
    };
    let mut imported = BTreeMap::new();
    let mut named = Vec::new();
    let mut at = 0;
    let mut spans = Vec::new();
    while at < tokens.len() {
        let is_use = tokens[at] == Token::Name("use".to_owned())
            && (at == 0 || tokens[at - 1] != Token::PathSep)
            && tokens.get(at + 1) != Some(&Token::Punct('('));
        if is_use {
            let end = tokens[at..]
                .iter()
                .position(|token| *token == Token::Punct(';'))
                .map_or(tokens.len(), |offset| at + offset);
            for (path, local) in use_leaves(&tokens[at + 1..end], &[]) {
                let path = absolute(path);
                if let Some(local) = local {
                    imported.insert(local, path.clone());
                }
                named.push(path);
            }
            spans.push((at, end));
            at = end;
        }
        at += 1;
    }
    let mut at = 0;
    while at < tokens.len() {
        if spans
            .iter()
            .any(|(start, end)| (*start..=*end).contains(&at))
        {
            at += 1;
            continue;
        }
        let Token::Name(first) = &tokens[at] else {
            at += 1;
            continue;
        };
        let mut path = vec![first.clone()];
        at += 1;
        while tokens.get(at) == Some(&Token::PathSep) {
            if let Some(Token::Name(next)) = tokens.get(at + 1) {
                path.push(next.clone());
                at += 2;
            } else {
                break;
            }
        }
        named.push(match imported.get(&path[0]) {
            Some(target) => {
                let mut resolved = target.clone();
                resolved.extend(path.into_iter().skip(1));
                resolved
            }
            None => absolute(path),
        });
    }
    named
}

/// The forbidden paths `source` names, as written in [`PORTED_GLOBALS`] (`a::b::c`), with what to
/// ask instead. A glob import of a module that holds a forbidden one counts.
fn ported_globals_named(source: &str, module: &[String]) -> BTreeMap<String, &'static str> {
    let mut found = BTreeMap::new();
    for path in named_paths(source, module) {
        for (forbidden, port) in PORTED_GLOBALS {
            let glob = path.last().is_some_and(|last| last == "*");
            let head: &[String] = if glob { &path[..path.len() - 1] } else { &path };
            let matches = forbidden.iter().enumerate().all(|(index, wanted)| {
                match (head.get(index), wanted.strip_suffix('*')) {
                    (Some(segment), Some(prefix)) => segment.starts_with(prefix),
                    (Some(segment), None) => segment == wanted,
                    // A glob stops short: it brings in what is below it.
                    (None, _) => glob,
                }
            });
            if matches {
                found.insert(forbidden.join("::"), *port);
            }
        }
    }
    found
}

/// The module a file under `src/` declares: `service/market/okx.rs` is `crate::service::market::okx`
/// and `service/market/ports/mod.rs` is `crate::service::market::ports`.
fn module_of(file: &str) -> Vec<String> {
    let mut module = vec!["crate".to_owned()];
    module.extend(file.trim_end_matches(".rs").split('/').map(str::to_owned));
    if module.last().is_some_and(|last| last == "mod") {
        module.pop();
    }
    module
}

#[test]
fn venues_reach_the_world_only_through_their_ports() {
    let mut offenders = Vec::new();
    let mut found_debt = BTreeSet::new();
    for (file, text) in module_sources("service/market") {
        let code = production_code(&text);
        for (global, port) in ported_globals_named(&code, &module_of(&file)) {
            if KNOWN_UNPORTED.contains(&(file.as_str(), global.as_str())) {
                found_debt.insert((file.clone(), global));
            } else {
                offenders.push(format!("{file} names `{global}`: ask {port}"));
            }
        }
    }
    let settled: Vec<String> = KNOWN_UNPORTED
        .iter()
        .filter(|(file, global)| !found_debt.contains(&((*file).to_owned(), (*global).to_owned())))
        .map(|(file, global)| format!("(\"{file}\", \"{global}\")"))
        .collect();
    assert!(
        offenders.is_empty(),
        "a venue is given its ports (docs/agent-guides/market-data.md):\n{}",
        offenders.join("\n")
    );
    assert!(
        settled.is_empty(),
        "these are behind a port now; remove them from KNOWN_UNPORTED:\n{}",
        settled.join("\n")
    );
}

#[test]
fn the_detection_follows_a_path_through_braces_aliases_and_relative_names() {
    let found = |source: &str| ported_globals_named(source, &module_of("service/market/okx.rs"));
    let names = |source: &str| found(source).into_keys().collect::<Vec<_>>();
    // A braced import, with the other names beside it.
    assert_eq!(
        names("use crate::service::net::http::{Retry, get_text};"),
        ["crate::service::net::http"]
    );
    assert_eq!(
        names("use crate::service::cache::lookup::{self, Hit};"),
        ["crate::service::cache::lookup"]
    );
    // A group of modules, a rename, and the module reached through its parent.
    assert_eq!(
        names("use crate::service::net::{http, rate_limit as limits};"),
        [
            "crate::service::net::http",
            "crate::service::net::rate_limit"
        ]
    );
    assert_eq!(
        names("use crate::service::net;\nfn f() { net::http::get_text(); }"),
        ["crate::service::net::http"]
    );
    // An alias for a module, used where the call is.
    assert_eq!(
        names("use crate::service::net::http as transport;\nfn f() { transport::get_text(); }"),
        ["crate::service::net::http"]
    );
    // The path written out in the code, relative paths and external crates.
    assert_eq!(
        names("fn f() { let _ = crate::service::keychain::api_keys::cmc_api_key(); }"),
        ["crate::service::keychain"]
    );
    assert_eq!(
        names("fn f() { let _ = super::super::net::http::get_text; }"),
        ["crate::service::net::http"],
        "this file is in `market`, so two `super`s reach `service`"
    );
    assert_eq!(
        names("use super::super::keychain::api_keys;"),
        ["crate::service::keychain"]
    );
    assert_eq!(names("fn f() { ureq::get(url); }"), ["ureq"]);
    assert_eq!(
        names("use std::time::{Duration, SystemTime};\nfn f() { SystemTime::now(); }"),
        ["std::time::SystemTime"]
    );
    // The production wiring, whether by its function or its module.
    assert_eq!(
        names("use crate::service::{self, for_source};\nfn f() { service::fetch_klines(); }"),
        ["crate::service::fetch_*", "crate::service::for_source"]
    );
    assert_eq!(
        names("use crate::service::wiring::venues;"),
        ["crate::service::wiring"]
    );
    // A glob brings in what is below it.
    assert_eq!(
        names("use crate::service::net::*;"),
        [
            "crate::service::net::agent",
            "crate::service::net::http",
            "crate::service::net::rate_limit"
        ]
    );
    // What is no dependency: the ports, a message, a comment, a lifetime and a char.
    assert!(
        found(
            "use super::ports::http::{Http, Request};\n\
             use crate::service::net::url::encode_component;\n\
             // use crate::service::net::http;\n\
             /* crate::service::keychain */\n\
             const WHY: &str = \"crate::service::net::http::get_text\";\n\
             const RAW: &str = r#\"ureq::get\"#;\n\
             fn f<'a>(x: &'a str) -> char { '\"' }"
        )
        .is_empty()
    );
}

#[test]
fn venues_are_built_only_by_the_wiring_and_tests() {
    let mut offenders = Vec::new();
    for module in EVERY_MODULE {
        for (file, text) in module_sources(module) {
            if file != "service/wiring/mod.rs"
                && file != "service/market/venues.rs"
                && production_code(&text).contains("Venues::new(")
            {
                offenders.push(file);
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "a second set of venues holds catalogs the cache clearing never reaches; ask \
         `service::wiring::venues()`:\n{}",
        offenders.join("\n")
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

/// The view layers: they hold the `work_queue::Pending` a host or service handed them and never
/// start background work of their own (`docs/agent-guides/core.md`, Background work).
const VIEW_MODULES: &[&str] = &["pine_studio", "ui"];

/// What starting background work by hand looks like.
const OWN_THREADS: &[&str] = &[
    "thread::spawn",
    "thread::Builder",
    "mpsc::channel",
    "mpsc::sync_channel",
];

/// Views that run a thread of their own, each with the reason `Pending` does not fit.
const KNOWN_OWN_THREADS: &[(&str, &str)] = &[(
    "pine_studio/ai/mod.rs",
    "the assistant's run streams many progress events and can be cancelled; Pending carries one answer",
)];

#[test]
fn views_start_no_background_work_of_their_own() {
    let mut offenders = Vec::new();
    let mut found = BTreeSet::new();
    for module in VIEW_MODULES {
        for (file, text) in module_sources(module) {
            let code = production_code(&text);
            if !OWN_THREADS.iter().any(|pattern| code.contains(pattern)) {
                continue;
            }
            if KNOWN_OWN_THREADS.iter().any(|(known, _)| *known == file) {
                found.insert(file);
            } else {
                offenders.push(file);
            }
        }
    }
    let settled: Vec<&str> = KNOWN_OWN_THREADS
        .iter()
        .map(|(file, _)| *file)
        .filter(|file| !found.contains(*file))
        .collect();
    assert!(
        offenders.is_empty(),
        "run the job through service::work_queue (start, start_for, start_search) and poll the Pending:\n{}",
        offenders.join("\n")
    );
    assert!(
        settled.is_empty(),
        "these no longer start threads; remove them from KNOWN_OWN_THREADS:\n{}",
        settled.join("\n")
    );
}

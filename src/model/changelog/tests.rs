use super::*;
use std::path::PathBuf;

fn section(version: &str, new: &[&str], fixed: &[&str]) -> Section {
    Section {
        version: version.to_owned(),
        new: new.iter().map(|&text| text.to_owned()).collect(),
        fixed: fixed.iter().map(|&text| text.to_owned()).collect(),
    }
}

#[test]
fn a_release_body_yields_its_sections_newest_first_as_written() {
    let body = "\
## 0.6.0-rc.33 (2026-11-20)

### New

- Alpha.
* Bravo.

### Fixed

- Charlie.

## 0.6.0-rc.32 (2026-11-19)

### Fixed

- Delta.
";
    assert_eq!(
        parse(body),
        [
            section("0.6.0-rc.33", &["Alpha.", "Bravo."], &["Charlie."]),
            section("0.6.0-rc.32", &[], &["Delta."]),
        ]
    );
}

#[test]
fn the_old_commit_log_bodies_yield_nothing() {
    let body = "\
## Changes since v0.6.0-rc.31

- Pine: loop counters save an outer value only when they hide one (43b9b42)
- Alpaca SIP: make no request for a start inside the free plan's delay (14ff493)

**Full diff:** https://github.com/degenview/degenview/compare/v0.6.0-rc.31...v0.6.0-rc.32

## Downloads

| Platform | Arch | Link |
| --- | --- | --- |
";
    assert_eq!(parse(body), []);
    assert_eq!(parse(""), []);
}

#[test]
fn a_release_body_with_the_commit_list_folded_after_the_sections() {
    let body = "\
## 0.6.0-rc.33 (2026-11-20)

### New

- Alpha.

<details>
<summary>All commits</summary>

- Something internal (abc1234)
## 9.9.9 (2099-01-01)

### New

- Not a release.

</details>

**Full diff:** https://example.test

## Downloads

### New

- Not a change.
";
    assert_eq!(parse(body), [section("0.6.0-rc.33", &["Alpha."], &[])]);
}

#[test]
fn only_a_version_heading_opens_a_section_and_any_other_heading_closes_it() {
    let body = "\
### New

- Before any section.

## v0.6.0 (2026-11-22)

### new

- Lowercase heading.

### Other

- Under a heading that is not New or Fixed.

### FIXED

- Upper case heading.

# Notes

### New

- After a closing heading.

## Not a version 1.2.3

### New

- Also closed.

## 0.6 (no patch)

### New

- Two numbers are not a version.
";
    assert_eq!(
        parse(body),
        [section(
            "0.6.0",
            &["Lowercase heading."],
            &["Upper case heading."]
        )]
    );
}

#[test]
fn versions_may_carry_a_prerelease_a_build_and_a_v() {
    for (heading, version) in [
        ("## 1.2.3", Some("1.2.3")),
        ("## v1.2.3-rc.4 (2026-01-01)", Some("1.2.3-rc.4")),
        ("## 1.2.3+build.5", Some("1.2.3+build.5")),
        ("## 1.2.3.4", None),
        ("## 1.2.x", None),
        ("## 1.2.3-rc_1", None),
        ("##1.2.3", None),
        ("### 1.2.3", None),
    ] {
        let found = parse(&format!("{heading}\n\n### New\n\n- A.\n"));
        let found: Vec<_> = found.iter().map(|s| s.version.as_str()).collect();
        assert_eq!(found, version.into_iter().collect::<Vec<_>>(), "{heading}");
    }
}

#[test]
fn windows_line_endings_and_stray_spacing_are_read_the_same() {
    let body = "## 1.0.0 (2026-01-01)\r\n\r\n### New\r\n\r\n-   Spaced out.  \r\n-\r\n- \r\n  - Indented.\r\n";
    assert_eq!(parse(body), [section("1.0.0", &["Spaced out."], &[])]);
}

#[test]
fn what_is_not_a_bullet_under_a_subheading_is_ignored() {
    let body = "\
## 1.0.0

### New

Prose between the lists.
1. A numbered line.
-Not a bullet.
- A bullet.
";
    assert_eq!(parse(body), [section("1.0.0", &["A bullet."], &[])]);
}

#[test]
fn a_hostile_body_is_bounded() {
    let many_entries = format!("## 1.0.0\n\n### New\n\n{}", "- Line.\n".repeat(10_000));
    let parsed = parse(&many_entries);
    assert_eq!(parsed[0].new.len(), MAX_ENTRIES);

    let long_entry = format!("## 1.0.0\n\n### New\n\n- {}\n", "é".repeat(5_000));
    let entry = &parse(&long_entry)[0].new[0];
    assert_eq!(entry.chars().count(), MAX_ENTRY_CHARS + 1);
    assert!(entry.ends_with('…'));

    let many_sections = "## 1.0.0\n\n### New\n\n- A.\n\n".repeat(1_000);
    assert_eq!(parse(&many_sections).len(), MAX_SECTIONS);

    // Text past the cap is not read at all, and a cut inside a character does not panic.
    let padding = "x".repeat(MAX_BODY_BYTES - 1);
    let late = format!("{padding}é\n## 1.0.0\n\n### New\n\n- Too late.\n");
    assert_eq!(parse(&late), []);
}

#[test]
fn merged_notes_keep_the_order_given_and_are_bounded() {
    let notes = ReleaseNotes::from_sections([
        section("2.0.0", &["B1.", "B2."], &["F1."]),
        section("1.0.0", &["A1."], &["F0."]),
    ]);
    assert_eq!(notes.new, ["B1.", "B2.", "A1."]);
    assert_eq!(notes.fixed, ["F1.", "F0."]);
    assert!(!notes.is_empty());
    assert!(ReleaseNotes::default().is_empty());
    assert!(ReleaseNotes::from_sections([section("1.0.0", &[], &[])]).is_empty());

    let flood = ReleaseNotes::from_sections((0..50).map(|n| {
        let text = n.to_string();
        section(&text, &["x"; MAX_ENTRIES], &["y"; MAX_ENTRIES])
    }));
    assert_eq!((flood.new.len(), flood.fixed.len()), (MAX_NOTES, MAX_NOTES));
}

// MARK: The repository's own files

fn repo(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(path)
}

/// One change per line, `new: <text>` or `fixed: <text>`, as `tools/changelog.sh` reads them.
fn is_change_line(line: &str) -> bool {
    let line = line.trim_end();
    ["new: ", "fixed: "].iter().any(|prefix| {
        line.strip_prefix(prefix)
            .is_some_and(|text| !text.starts_with(char::is_whitespace))
    })
}

#[test]
fn every_pending_fragment_follows_the_format() {
    let dir = repo("changes");
    let mut problems = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("changes/ exists") {
        let path = entry.expect("readable entry").path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if name == "README.md" {
            continue;
        }
        let slug = name.strip_suffix(".md");
        if !slug.is_some_and(|slug| {
            !slug.is_empty()
                && !slug.starts_with('-')
                && slug
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        }) {
            problems.push(format!(
                "changes/{name}: not a lowercase slug ending in .md"
            ));
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("readable fragment");
        for (index, line) in text.lines().enumerate() {
            if !line.trim().is_empty() && !is_change_line(line) {
                problems.push(format!(
                    "changes/{name}:{}: expected `new: <text>` or `fixed: <text>`",
                    index + 1
                ));
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn the_changelog_is_made_of_sections_the_app_can_read() {
    let text = std::fs::read_to_string(repo("CHANGELOG.md")).expect("CHANGELOG.md exists");
    assert!(text.starts_with("# Changelog\n"));
    let mut sections = 0;
    let mut subheading = None;
    for (index, line) in text.lines().enumerate().skip(1) {
        let at = || format!("CHANGELOG.md:{}: {line}", index + 1);
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("## ") {
            // `## <version> (<date>)`, the shape tools/changelog.sh writes.
            let (version, date) = rest.split_once(" (").unwrap_or_else(|| panic!("{}", at()));
            assert!(version_of(version).is_some(), "{}", at());
            let date = date.strip_suffix(')').unwrap_or_else(|| panic!("{}", at()));
            let digits = |part: &str, len: usize| {
                part.len() == len && part.bytes().all(|byte| byte.is_ascii_digit())
            };
            let mut parts = date.split('-');
            assert!(
                digits(parts.next().unwrap_or(""), 4)
                    && digits(parts.next().unwrap_or(""), 2)
                    && digits(parts.next().unwrap_or(""), 2)
                    && parts.next().is_none(),
                "{}",
                at()
            );
            sections += 1;
            subheading = None;
        } else if line == "### New" || line == "### Fixed" {
            assert!(sections > 0 && subheading != Some(line), "{}", at());
            subheading = Some(line);
        } else if let Some(entry) = line.strip_prefix("- ") {
            assert!(subheading.is_some() && !entry.trim().is_empty(), "{}", at());
        } else {
            assert!(sections == 0, "{}", at());
        }
    }
    assert_eq!(parse(&text).len(), sections, "a section is not read");
}

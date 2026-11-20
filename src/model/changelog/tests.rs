use super::*;
use std::collections::HashMap;
use std::path::PathBuf;

fn section(version: &str, new: &[&str], fixed: &[&str]) -> Section {
    Section {
        version: version.to_owned(),
        new: new.iter().map(|&text| text.to_owned()).collect(),
        fixed: fixed.iter().map(|&text| text.to_owned()).collect(),
        ..Section::default()
    }
}

fn section_with_rc_fixes(version: &str, fixed: &[&str], fixed_rc: &[&str]) -> Section {
    Section {
        fixed_rc: fixed_rc.iter().map(|&text| text.to_owned()).collect(),
        ..section(version, &[], fixed)
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
    assert!(parsed[0].omitted > 0);

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
fn entries_over_a_cap_are_counted_not_lost_silently() {
    let body = format!(
        "## 2.0.0\n\n### New\n\n{}\n### Fixed\n\n{}",
        "- Line.\n".repeat(MAX_ENTRIES + 3),
        "- Fix.\n".repeat(MAX_ENTRIES + 4),
    );
    let section = &parse(&body)[0];
    assert_eq!(
        (section.new.len(), section.fixed.len(), section.omitted),
        (MAX_ENTRIES, MAX_ENTRIES, 7)
    );
    // Within the caps nothing is left out.
    assert_eq!(parse("## 1.0.0\n\n### New\n\n- A.\n")[0].omitted, 0);
}

#[test]
fn a_section_past_the_section_cap_is_counted_in_the_last_one_kept() {
    let one = "## 1.0.0\n\n### New\n\n- A.\n\n### Fixed\n\n- B.\n\n";
    let sections = parse(&one.repeat(MAX_SECTIONS + 3));
    assert_eq!(sections.len(), MAX_SECTIONS);
    let omitted: Vec<usize> = sections.iter().map(|section| section.omitted).collect();
    assert_eq!(
        omitted[MAX_SECTIONS - 1],
        6,
        "three sections of two entries"
    );
    assert!(omitted[..MAX_SECTIONS - 1].iter().all(|&count| count == 0));

    // Text under a heading that is not a section, or after one that closes it, is not counted.
    let body = format!(
        "{}## 9.9.9\n\n### Other\n\n- Not a change.\n\n# Notes\n\n### New\n\n- Closed.\n",
        one.repeat(MAX_SECTIONS)
    );
    assert_eq!(parse(&body)[MAX_SECTIONS - 1].omitted, 0);
}

#[test]
fn merged_notes_count_what_the_sections_and_their_own_cap_leave_out() {
    let mut first = section("2.0.0", &["B."], &[]);
    first.omitted = 4;
    let mut second = section("1.0.0", &["A."], &["F."]);
    second.omitted = 2;
    assert_eq!(
        ReleaseNotes::from_sections([first, second], true).omitted,
        6
    );

    let flood = ReleaseNotes::from_sections(
        (0..3).map(|n| {
            let text = n.to_string();
            section(&text, &["x"; MAX_ENTRIES], &["y"; MAX_ENTRIES])
        }),
        true,
    );
    assert_eq!(flood.omitted, 0, "300 entries a list, at the cap");
    let flood = ReleaseNotes::from_sections(
        (0..4).map(|n| {
            let mut flooded = section(&n.to_string(), &["x"; MAX_ENTRIES], &["y"; 1]);
            flooded.omitted = 1;
            flooded
        }),
        true,
    );
    assert_eq!(
        flood.omitted,
        4 + (4 * MAX_ENTRIES - MAX_NOTES),
        "each section's own, then what the merged list cuts"
    );

    let mut huge = section("1.0.0", &[], &[]);
    huge.omitted = usize::MAX;
    let mut more = section("0.9.0", &[], &[]);
    more.omitted = 5;
    assert_eq!(
        ReleaseNotes::from_sections([huge, more], true).omitted,
        usize::MAX
    );
}

#[test]
fn merged_notes_keep_the_order_given_and_are_bounded() {
    let notes = ReleaseNotes::from_sections(
        [
            section("2.0.0", &["B1.", "B2."], &["F1."]),
            section("1.0.0", &["A1."], &["F0."]),
        ],
        true,
    );
    assert_eq!(notes.new, ["B1.", "B2.", "A1."]);
    assert_eq!(notes.fixed, ["F1.", "F0."]);
    assert!(!notes.is_empty());
    assert!(ReleaseNotes::default().is_empty());
    assert!(ReleaseNotes::from_sections([section("1.0.0", &[], &[])], true).is_empty());

    let flood = ReleaseNotes::from_sections(
        (0..50).map(|n| {
            let text = n.to_string();
            section(&text, &["x"; MAX_ENTRIES], &["y"; MAX_ENTRIES])
        }),
        true,
    );
    assert_eq!((flood.new.len(), flood.fixed.len()), (MAX_NOTES, MAX_NOTES));
}

#[test]
fn the_release_candidate_fixes_subheading_is_read_into_its_own_list() {
    let body = "\
## 0.6.0-rc.2 (2026-11-20)

### New

- Alpha.

### Fixed

- Bravo.

### Fixed in release candidates

- Charlie.

### FIXED IN RELEASE CANDIDATES

- Delta.

### Fixed in release

- Not that subheading.
";
    let mut expected = section("0.6.0-rc.2", &["Alpha."], &["Bravo."]);
    expected.fixed_rc = vec!["Charlie.".to_owned(), "Delta.".to_owned()];
    assert_eq!(parse(body), [expected]);
}

#[test]
fn release_candidate_fixes_follow_each_sections_own_fixes_when_included() {
    let sections = || {
        [
            section_with_rc_fixes("0.6.0-rc.2", &["F2."], &["R2."]),
            section_with_rc_fixes("0.6.0-rc.1", &["F1."], &["R1a.", "R1b."]),
        ]
    };
    let included = ReleaseNotes::from_sections(sections(), true);
    assert_eq!(included.fixed, ["F2.", "R2.", "F1.", "R1a.", "R1b."]);
    assert!(included.new.is_empty());

    let excluded = ReleaseNotes::from_sections(sections(), false);
    assert_eq!(excluded.fixed, ["F2.", "F1."]);
    assert_eq!(excluded.omitted, 0, "dropped on purpose, not left out");
    assert!(
        ReleaseNotes::from_sections([section_with_rc_fixes("0.6.0-rc.1", &[], &["R."])], false)
            .is_empty()
    );
}

#[test]
fn release_candidate_fixes_share_the_caps_and_omitted_counting() {
    let body = format!(
        "## 2.0.0\n\n### Fixed in release candidates\n\n{}",
        "- Fix.\n".repeat(MAX_ENTRIES + 3),
    );
    let parsed = parse(&body);
    assert_eq!(parsed[0].fixed_rc.len(), MAX_ENTRIES);
    assert_eq!((parsed[0].omitted, parsed[0].omitted_rc), (0, 3));

    // What the cap left out counts only for a reader who is shown the fixes...
    assert_eq!(ReleaseNotes::from_sections(parsed.clone(), true).omitted, 3);
    assert_eq!(ReleaseNotes::from_sections(parsed, false).omitted, 0);
    // ...and fixes merged into `fixed` count against the merged cap with the rest.
    let flood = ReleaseNotes::from_sections(
        (0..4).map(|n| section_with_rc_fixes(&n.to_string(), &[], &["y"; MAX_ENTRIES])),
        true,
    );
    assert_eq!(flood.fixed.len(), MAX_NOTES);
    assert_eq!(flood.omitted, 4 * MAX_ENTRIES - MAX_NOTES);
    let hidden = ReleaseNotes::from_sections(
        (0..4).map(|n| section_with_rc_fixes(&n.to_string(), &[], &["y"; MAX_ENTRIES])),
        false,
    );
    assert_eq!((hidden.fixed.len(), hidden.omitted), (0, 0));
}

#[test]
fn candidate_fix_overflow_is_not_a_public_omission() {
    let body = format!(
        "## 2.0.0\n\n### Fixed\n\n- Public.\n\n### Fixed in release candidates\n\n{}",
        "- Fix.\n".repeat(MAX_ENTRIES + 5),
    );
    let parsed = parse(&body);
    assert_eq!(parsed[0].omitted_rc, 5);
    assert_eq!(
        ReleaseNotes::from_sections(parsed.clone(), false).omitted,
        0
    );
    let candidate = ReleaseNotes::from_sections(parsed, true);
    assert_eq!(candidate.omitted, 5);
    assert_eq!(candidate.fixed.len(), 1 + MAX_ENTRIES);

    // The same for candidate fixes in sections past the section cap.
    let one = "## 1.0.0\n\n### Fixed in release candidates\n\n- R.\n\n### Fixed\n\n- P.\n\n";
    let sections = parse(&one.repeat(MAX_SECTIONS + 2));
    let last = &sections[MAX_SECTIONS - 1];
    assert_eq!((last.omitted, last.omitted_rc), (2, 2));
    assert_eq!(
        ReleaseNotes::from_sections(sections.clone(), false).omitted,
        2
    );
    assert_eq!(ReleaseNotes::from_sections(sections, true).omitted, 4);
}

#[test]
fn a_body_over_the_bound_is_cut_at_a_whole_line_and_the_open_section_says_so() {
    // 60 sections of four long entries, each ending in a marker a half entry would lack.
    let entry = |n: usize| format!("- Entry {n}: {} END\n", "word ".repeat(60));
    let section = |release: usize| {
        let entries: String = (0..4).map(entry).collect();
        format!("## 1.0.{release} (2026-11-20)\n\n### New\n\n{entries}\n")
    };
    let body: String = (0..60).rev().map(section).collect();
    assert!(body.len() > MAX_BODY_BYTES);
    let parsed = parse(&body);
    let (last, earlier) = parsed.split_last().expect("sections were read");
    assert!(parsed.len() < 60, "the rest was never read");
    assert!(last.truncated);
    assert!(earlier.iter().all(|section| !section.truncated));
    for section in &parsed {
        assert!(
            section.new.iter().all(|entry| entry.ends_with(" END")),
            "{}: an entry was cut mid-line",
            section.version
        );
    }
    assert!(ReleaseNotes::from_sections(parsed, false).truncated);

    // A body that fits is not truncated, and neither is a cut in a folded commit list.
    assert!(!parse(&entry(1)).iter().any(|section| section.truncated));
    let folded = format!(
        "## 1.0.0\n\n### New\n\n- A.\n\n<details>\n{}",
        "- abc1234 A commit.\n".repeat(5_000)
    );
    assert!(folded.len() > MAX_BODY_BYTES);
    assert!(!parse(&folded)[0].truncated);
}

// MARK: The repository's own files

fn repo(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(path)
}

/// The text of a change line, `new: <text>`, `fixed: <text>` or `fixed-rc: <text>`, as
/// `tools/changelog.sh` reads them; none for any other line.
fn change_text(line: &str) -> Option<&str> {
    let line = line.trim_end();
    ["new: ", "fixed: ", "fixed-rc: "]
        .iter()
        .find_map(|prefix| line.strip_prefix(prefix))
        .filter(|text| !text.starts_with(char::is_whitespace))
}

/// What is wrong with `fragments` (file name, text), as `file:line: problem`: a line that is not a
/// change, and a change whose text an earlier line already gave, whichever prefix it has.
fn fragment_problems<'a>(fragments: impl IntoIterator<Item = (&'a str, &'a str)>) -> Vec<String> {
    let mut problems = Vec::new();
    let mut first_seen: HashMap<&str, String> = HashMap::new();
    for (name, text) in fragments {
        for (index, line) in text.lines().enumerate() {
            let at = format!("changes/{name}:{}", index + 1);
            if line.trim().is_empty() {
                continue;
            }
            if line.contains("<!--") || line.contains("-->") {
                problems.push(format!("{at}: a change cannot contain `<!--` or `-->`"));
                continue;
            }
            let Some(change) = change_text(line) else {
                problems.push(format!(
                    "{at}: expected `new: <text>`, `fixed: <text>` or `fixed-rc: <text>`"
                ));
                continue;
            };
            if let Some(earlier) = first_seen.get(change) {
                problems.push(format!("{at}: the same change as {earlier}"));
            } else {
                first_seen.insert(change, at);
            }
        }
    }
    problems
}

#[test]
fn every_pending_fragment_follows_the_format() {
    let dir = repo("changes");
    let mut problems = Vec::new();
    let mut fragments = Vec::new();
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
        fragments.push((name, text));
    }
    // Name order, as the script reads them, so a duplicate names the same two lines.
    fragments.sort();
    problems.extend(fragment_problems(
        fragments
            .iter()
            .map(|(name, text)| (name.as_str(), text.as_str())),
    ));
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn a_fragment_line_is_a_new_fixed_or_fixed_in_candidates_change() {
    for line in ["new: A.", "fixed: B.", "fixed-rc: C.", "fixed-rc: C.  \r"] {
        assert!(change_text(line).is_some(), "{line}");
    }
    for line in [
        "new:A.",
        "new:  A.",
        "fixed-rc:C.",
        "fixed-rc A.",
        "rc: A.",
        "- new: A.",
        "fixed-rc:  A.",
        "New: A.",
    ] {
        assert!(change_text(line).is_none(), "{line}");
    }
}

#[test]
fn a_change_cannot_carry_the_marker_that_hides_candidate_fixes() {
    let problems = fragment_problems([
        ("one", "fixed-rc: Arrows --> no longer break.\n"),
        ("two", "new: An <!-- opener.\n"),
    ]);
    assert_eq!(
        problems,
        [
            "changes/one:1: a change cannot contain `<!--` or `-->`",
            "changes/two:1: a change cannot contain `<!--` or `-->`",
        ]
    );
}

#[test]
fn the_same_change_twice_is_refused_naming_both_lines() {
    let clean = fragment_problems([
        ("one", "new: A.\n\nfixed: B.\n"),
        ("two", "fixed-rc: C.\nfixed: D.\n"),
    ]);
    assert!(clean.is_empty(), "{clean:?}");

    let repeated = fragment_problems([
        ("one", "new: Same words.\nfixed: B.\n"),
        ("two", "fixed: Other.\nfixed-rc: Same words.\n"),
        ("three", "new: B.\n"),
    ]);
    assert_eq!(
        repeated,
        [
            "changes/two:2: the same change as changes/one:1",
            "changes/three:1: the same change as changes/one:2",
        ]
    );

    let within = fragment_problems([("one", "fixed: A.\nfixed: A.\n")]);
    assert_eq!(within, ["changes/one:2: the same change as changes/one:1"]);
}

#[test]
fn a_line_that_is_not_a_change_is_refused_with_its_place() {
    let problems = fragment_problems([("one", "new: A.\n- bullet\n\nfixed:B.\n")]);
    assert_eq!(problems.len(), 2);
    assert!(problems[0].starts_with("changes/one:2: expected"));
    assert!(problems[1].starts_with("changes/one:4: expected"));
}

/// Panics, naming the line, unless `text` is a changelog the app can read: the heading, then
/// sections in the shape `tools/changelog.sh` writes, each of which `parse` reads on its own.
///
/// Section by section, not the whole file at once: `parse` caps a release body (`MAX_SECTIONS`,
/// `MAX_BODY_BYTES`), a limit that suits what one release carries and that the archive, which grows
/// with every release, must not be held to.
fn assert_changelog_readable(text: &str) {
    assert!(text.starts_with("# Changelog\n"));
    let mut sections: Vec<(String, String)> = Vec::new();
    let mut subheading = None;
    for (index, line) in text.lines().enumerate().skip(1) {
        let at = || format!("CHANGELOG.md:{}: {line}", index + 1);
        if let Some(rest) = line.strip_prefix("## ") {
            // `## <version> (<date>)`, the shape tools/changelog.sh writes.
            let (version, date) = rest.split_once(" (").unwrap_or_else(|| panic!("{}", at()));
            let version = version_of(version).unwrap_or_else(|| panic!("{}", at()));
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
            sections.push((version, String::new()));
            subheading = None;
        } else if matches!(
            line,
            "### New" | "### Fixed" | "### Fixed in release candidates"
        ) {
            assert!(!sections.is_empty() && subheading != Some(line), "{}", at());
            subheading = Some(line);
        } else if let Some(entry) = line.strip_prefix("- ") {
            assert!(subheading.is_some() && !entry.trim().is_empty(), "{}", at());
        } else if !line.is_empty() {
            assert!(sections.is_empty(), "{}", at());
        }
        if let Some((_, chunk)) = sections.last_mut() {
            chunk.push_str(line);
            chunk.push('\n');
        }
    }
    for (version, chunk) in &sections {
        let read = parse(chunk);
        assert!(
            matches!(read.as_slice(), [one] if one.version == *version),
            "the section for {version} is not read as one section: {read:?}"
        );
    }
}

#[test]
fn the_changelog_is_made_of_sections_the_app_can_read() {
    let text = std::fs::read_to_string(repo("CHANGELOG.md")).expect("CHANGELOG.md exists");
    assert_changelog_readable(&text);
}

#[test]
fn an_archive_longer_than_a_release_body_may_be_is_still_valid() {
    let section = |release: usize| {
        let entries: String = (0..14)
            .map(|n| format!("- Change {n} of release {release}, worded at some length.\n"))
            .collect();
        format!("## 0.6.{release} (2026-11-20)\n\n### New\n\n{entries}\n### Fixed\n\n{entries}\n")
    };
    let archive: String = std::iter::once("# Changelog\n\nIntro.\n\n".to_owned())
        .chain((0..MAX_SECTIONS + 6).rev().map(section))
        .collect();
    assert!(archive.len() > MAX_BODY_BYTES);
    assert!(
        parse(&archive).len() <= MAX_SECTIONS,
        "the display cap holds"
    );

    assert_changelog_readable(&archive);
}

/// What `tools/changelog.sh notes <tag> <previous>` prints for `changelog`, run on a copy of the
/// script in a scratch tree so the repository's own files are never touched.
#[cfg(unix)]
fn notes_of(changelog: &str, tag: &str, previous: &str) -> String {
    // Tests run in parallel and may ask for the same tag, so each call gets its own tree.
    static CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let root = std::env::temp_dir().join(format!(
        "degenview-changelog-notes-{}-{}-{}",
        std::process::id(),
        CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        tag
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("tools")).expect("scratch tree");
    std::fs::copy(repo("tools/changelog.sh"), root.join("tools/changelog.sh"))
        .expect("script copied");
    std::fs::write(root.join("CHANGELOG.md"), changelog).expect("changelog written");
    let output = std::process::Command::new("sh")
        .arg(root.join("tools/changelog.sh"))
        .args(["notes", tag, previous])
        .output()
        .expect("sh runs");
    let _ = std::fs::remove_dir_all(&root);
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout).expect("text")
}

/// A changelog with a stable release made only of candidate fixes, candidates that fixed only
/// candidate bugs, and one that also changed something a stable reader sees.
#[cfg(unix)]
const CANDIDATE_CHANGELOG: &str = "\
# Changelog

## 0.6.0 (2026-11-20)

### Fixed in release candidates

- Fixed only in this one.

## 0.6.0-rc.3 (2026-11-19)

### Fixed in release candidates

- A candidate-only fix.

## 0.6.0-rc.2 (2026-11-18)

### New

- A new thing.

### Fixed

- A public fix.

### Fixed in release candidates

- Another candidate fix.

## 0.6.0-rc.1 (2026-11-17)

### Fixed in release candidates

- A candidate-only fix, first.

## 0.5.0 (2026-11-01)

### New

- Old.
";

#[cfg(unix)]
#[test]
fn a_stable_releases_notes_hide_the_candidate_fixes_in_a_comment_instead_of_dropping_them() {
    assert_eq!(
        notes_of(CANDIDATE_CHANGELOG, "v0.6.0", "v0.5.0"),
        "\
<!-- release-candidate fixes
## 0.6.0 (2026-11-20)

### Fixed in release candidates

- Fixed only in this one.
-->

<!-- release-candidate fixes
## 0.6.0-rc.3 (2026-11-19)

### Fixed in release candidates

- A candidate-only fix.
-->

## 0.6.0-rc.2 (2026-11-18)

### New

- A new thing.

### Fixed

- A public fix.

<!-- release-candidate fixes
### Fixed in release candidates

- Another candidate fix.
-->

<!-- release-candidate fixes
## 0.6.0-rc.1 (2026-11-17)

### Fixed in release candidates

- A candidate-only fix, first.
-->
",
        "a section of candidate fixes only is one comment, heading included"
    );
    assert_eq!(
        notes_of(CANDIDATE_CHANGELOG, "v0.6.0-rc.3", "v0.6.0-rc.2"),
        "## 0.6.0-rc.3 (2026-11-19)\n\n### Fixed in release candidates\n\n- A candidate-only fix.\n",
        "a candidate's own notes show them"
    );
    assert_eq!(
        notes_of(CANDIDATE_CHANGELOG, "v0.6.0-rc.2", "v0.6.0-rc.1"),
        "## 0.6.0-rc.2 (2026-11-18)\n\n### New\n\n- A new thing.\n\n### Fixed\n\n- A public fix.\n\n### Fixed in release candidates\n\n- Another candidate fix.\n"
    );

    // A release that had no fragments keeps its heading: nothing was hidden in it.
    let bare =
        "# Changelog\n\n## 0.6.0 (2026-11-20)\n\n## 0.5.0 (2026-11-01)\n\n### New\n\n- Old.\n";
    assert_eq!(
        notes_of(bare, "v0.6.0", "v0.5.0"),
        "## 0.6.0 (2026-11-20)\n"
    );
}

#[cfg(unix)]
#[test]
fn the_app_reads_the_hidden_candidate_fixes_of_a_stable_body_for_candidates_only() {
    let body = notes_of(CANDIDATE_CHANGELOG, "v0.6.0", "v0.5.0");
    let parsed = parse(&body);
    let versions: Vec<_> = parsed.iter().map(|s| s.version.as_str()).collect();
    assert_eq!(
        versions,
        ["0.6.0", "0.6.0-rc.3", "0.6.0-rc.2", "0.6.0-rc.1"]
    );
    assert_eq!(parsed[0].fixed_rc, ["Fixed only in this one."]);
    assert_eq!(parsed[2].fixed_rc, ["Another candidate fix."]);

    let stable = ReleaseNotes::from_sections(parsed.clone(), false);
    assert_eq!(stable.new, ["A new thing."]);
    assert_eq!(stable.fixed, ["A public fix."]);

    let candidate = ReleaseNotes::from_sections(parsed, true);
    assert_eq!(
        candidate.fixed,
        [
            "Fixed only in this one.",
            "A candidate-only fix.",
            "A public fix.",
            "Another candidate fix.",
            "A candidate-only fix, first.",
        ]
    );
}

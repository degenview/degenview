use super::*;
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

/// One run of `tools/changelog.sh`.
#[cfg(unix)]
#[derive(Debug)]
struct Run {
    success: bool,
    stdout: String,
    stderr: String,
}

#[cfg(unix)]
fn run_script(script: &std::path::Path, args: &[&str]) -> Run {
    let output = std::process::Command::new("sh")
        .arg(script)
        .args(args)
        .output()
        .expect("sh runs");
    Run {
        success: output.status.success(),
        stdout: String::from_utf8(output.stdout).expect("text"),
        stderr: String::from_utf8(output.stderr).expect("text"),
    }
}

/// A scratch tree holding a copy of `tools/changelog.sh` and the given files, so the script, which
/// works on the tree around it, never touches the repository's own `CHANGELOG.md` or `changes/`.
/// The script is the only validator of the fragment format: these tests run it rather than
/// restating its rules.
#[cfg(unix)]
struct Tree {
    root: PathBuf,
}

#[cfg(unix)]
impl Tree {
    fn new(files: &[(&str, &str)]) -> Self {
        // Tests run in parallel, so each tree is its own.
        static TREES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "degenview-changelog-{}-{}",
            std::process::id(),
            TREES.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("tools")).expect("scratch tree");
        std::fs::copy(repo("tools/changelog.sh"), root.join("tools/changelog.sh"))
            .expect("script copied");
        let tree = Self { root };
        for (path, text) in files {
            let path = tree.root.join(path);
            std::fs::create_dir_all(path.parent().expect("a file in the tree")).expect("dir");
            std::fs::write(path, text).expect("file written");
        }
        tree
    }

    fn run(&self, args: &[&str]) -> Run {
        run_script(&self.root.join("tools/changelog.sh"), args)
    }

    /// Runs the script and panics, with everything it said, unless it succeeded.
    fn output_of(&self, args: &[&str]) -> String {
        let run = self.run(args);
        assert!(run.success, "{run:?}");
        run.stdout
    }

    fn read(&self, path: &str) -> String {
        std::fs::read_to_string(self.root.join(path)).expect("file exists")
    }
}

#[cfg(unix)]
impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// What the script says to a fragment tree, as (success, stderr).
#[cfg(unix)]
fn checked(files: &[(&str, &str)]) -> (bool, String) {
    let run = Tree::new(files).run(&["check"]);
    assert!(run.stdout.is_empty(), "`check` prints nothing: {run:?}");
    (run.success, run.stderr)
}

#[cfg(unix)]
#[test]
fn every_pending_fragment_passes_the_script() {
    // `check` only reads, so this runs on the repository itself.
    let run = run_script(&repo("tools/changelog.sh"), &["check"]);
    assert!(run.success, "{}", run.stderr);
    assert!(run.stdout.is_empty(), "{run:?}");
}

#[cfg(unix)]
#[test]
fn a_clean_fragment_passes_whatever_its_spacing() {
    let (success, stderr) = checked(&[
        ("changes/one.md", "new: A.\n\n  \t\nfixed: B.  \r\n"),
        ("changes/two.md", "fixed-rc: C.\nfixed: D.\n"),
        ("changes/README.md", "Not a fragment.\n"),
    ]);
    assert!(success, "{stderr}");
}

#[cfg(unix)]
#[test]
fn the_same_change_twice_is_refused_naming_both_lines() {
    let (success, stderr) = checked(&[
        ("changes/one.md", "new: Same words.\nfixed: B.\n"),
        ("changes/two.md", "fixed: Other.\nfixed-rc: Same words.\n"),
        ("changes/three.md", "new: B.\n"),
    ]);
    assert!(!success);
    // Name order, whatever the prefix.
    assert_eq!(
        stderr,
        "error: changes/three.md:1: the same change as changes/one.md:2\n\
         error: changes/two.md:2: the same change as changes/one.md:1\n"
    );

    let (success, stderr) = checked(&[("changes/one.md", "fixed: A.\nfixed: A.\n")]);
    assert!(!success);
    assert_eq!(
        stderr,
        "error: changes/one.md:2: the same change as changes/one.md:1\n"
    );
}

#[cfg(unix)]
#[test]
fn a_change_cannot_carry_the_marker_that_hides_candidate_fixes() {
    let (success, stderr) = checked(&[
        ("changes/one.md", "fixed-rc: Arrows --> no longer break.\n"),
        ("changes/two.md", "new: An <!-- opener.\n"),
    ]);
    assert!(!success);
    assert_eq!(
        stderr,
        "error: changes/one.md:1: a change cannot contain `<!--` or `-->`\n\
         error: changes/two.md:1: a change cannot contain `<!--` or `-->`\n"
    );
}

#[cfg(unix)]
#[test]
fn a_line_that_is_not_a_change_is_refused_with_its_place() {
    let (success, stderr) = checked(&[(
        "changes/one.md",
        "new: A.\n- bullet\n\nfixed:B.\nNew: C.\nfixed:  D.\n",
    )]);
    assert!(!success);
    let expected = "expected `new: <text>`, `fixed: <text>` or `fixed-rc: <text>`";
    let lines: Vec<_> = stderr.lines().collect();
    assert_eq!(
        lines,
        [2, 4, 5, 6].map(|line| format!("error: changes/one.md:{line}: {expected}"))
    );
}

#[cfg(unix)]
#[test]
fn a_line_of_only_a_non_breaking_space_is_not_blank() {
    // Unicode treats U+00A0 as white space, the script (byte-wise, in the C locale) does not, and
    // the script decides what a release accepts.
    let (success, stderr) = checked(&[("changes/one.md", "new: A.\n\u{a0}\n")]);
    assert!(!success);
    assert!(
        stderr.starts_with("error: changes/one.md:2: expected"),
        "{stderr}"
    );
}

#[cfg(unix)]
#[test]
fn a_fragment_not_named_like_a_slug_is_refused() {
    for name in ["Bad_Name.md", "-leading.md", "two words.md"] {
        let path = format!("changes/{name}");
        let (success, stderr) = checked(&[(path.as_str(), "new: A.\n")]);
        assert!(!success, "{name}");
        assert!(
            stderr.starts_with(&format!("error: {path}: a fragment is named like its slug")),
            "{name}: {stderr}"
        );
    }
}

/// What a release does with fragments, end to end through the script and the parser: the section
/// `render` writes is the text of the fragments, and what `insert` and `notes` hand the app is read
/// back as the same changes.
#[cfg(unix)]
#[test]
fn fragments_survive_render_insert_and_notes_into_what_the_app_reads() {
    let tree = Tree::new(&[
        (
            "changes/one.md",
            "new: Charts can be compared.  \r\nfixed: Loops show the right values.\t\r\n",
        ),
        (
            "changes/two.md",
            "fixed-rc: The Update button opens its sheet.   \nnew: Sheets list what changed.\n",
        ),
        (
            "CHANGELOG.md",
            "# Changelog\n\n## 0.6.0 (2026-11-01)\n\n### New\n\n- Old.\n",
        ),
    ]);
    let new = ["Charts can be compared.", "Sheets list what changed."];
    let fixed = ["Loops show the right values."];
    let fixed_rc = ["The Update button opens its sheet."];

    let rendered = tree.output_of(&["render", "0.7.0", "2026-11-21"]);
    let sections = parse(&rendered);
    assert_eq!(sections.len(), 1, "{rendered}");
    assert_eq!(sections[0].version, "0.7.0");
    assert_eq!(sections[0].new, new);
    assert_eq!(sections[0].fixed, fixed);
    assert_eq!(sections[0].fixed_rc, fixed_rc);

    tree.output_of(&["insert", "0.7.0-rc.1", "2026-11-20"]);
    tree.output_of(&["insert", "0.7.0", "2026-11-21"]);
    assert_changelog_readable(&tree.read("CHANGELOG.md"));

    // The candidate's notes show the fixes only candidates had; the app reads them as such.
    let candidate_body = tree.output_of(&["notes", "v0.7.0-rc.1", "v0.6.0"]);
    let candidate = parse(&candidate_body);
    assert_eq!(candidate.len(), 1, "{candidate_body}");
    assert_eq!(candidate[0].version, "0.7.0-rc.1");
    assert_eq!(candidate[0].fixed_rc, fixed_rc);

    // The stable release carries both sections, the candidate fixes hidden in comments that the
    // app still reads, so a stable reader is not told about them and a candidate's reader is.
    let stable_body = tree.output_of(&["notes", "v0.7.0", "v0.6.0"]);
    let stable = parse(&stable_body);
    let versions: Vec<_> = stable
        .iter()
        .map(|section| section.version.as_str())
        .collect();
    assert_eq!(versions, ["0.7.0", "0.7.0-rc.1"]);
    assert!(stable_body.contains("<!-- release-candidate fixes"));
    let for_stable = ReleaseNotes::from_sections(stable.clone(), false);
    assert_eq!(for_stable.new, [new, new].concat());
    assert_eq!(for_stable.fixed, [fixed, fixed].concat());
    let for_candidate = ReleaseNotes::from_sections(stable, true);
    assert_eq!(
        for_candidate.fixed,
        [fixed[0], fixed_rc[0], fixed[0], fixed_rc[0]]
    );
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

/// What `tools/changelog.sh notes <tag> <previous>` prints for `changelog`.
#[cfg(unix)]
fn notes_of(changelog: &str, tag: &str, previous: &str) -> String {
    Tree::new(&[("CHANGELOG.md", changelog)]).output_of(&["notes", tag, previous])
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

fn version(text: &str) -> Version {
    Version::parse(text).unwrap()
}

#[test]
fn versions_order_numerically_not_textually() {
    let v = version;
    assert!(v("0.10.0") > v("0.9.9"));
    assert!(v("v1.0.0") > v("0.99.99"));
    assert!(v("0.2.0-rc.1") < v("0.2.0"));
    assert!(v("0.2.0-rc1") < v("0.2.0"));
    assert_eq!(v("v0.1.0"), v("0.1.0+build5"));
    assert!(Version::parse("0.1").is_none());
    assert!(Version::parse("nightly").is_none());
    assert!(Version::parse("1.2.3.4").is_none());
}

/// The client reads remote repository state, so it shares the contract of `tools/release.sh`.
#[test]
fn versions_must_be_strict_semver() {
    for tag in [
        "vv1.0.0",
        "01.0.0",
        "1.00.0",
        "1.0.0-01",
        "+1.0.0",
        "1.0.0-",
        "1.0.0-rc..1",
        "1.0.0-rc.1+",
        "1.0.0+a..b",
        "1.0.0-rc_1",
        "1.-1.0",
        "99999999999999999999.0.0",
    ] {
        assert!(Version::parse(tag).is_none(), "{tag}");
    }
    for tag in [
        "v1.0.0",
        "V1.0.0",
        "0.0.0",
        "1.0.0-rc.1",
        "1.0.0-0.3.7",
        "1.0.0+20130313",
    ] {
        assert!(Version::parse(tag).is_some(), "{tag}");
    }
}

#[test]
fn prereleases_follow_semver_precedence() {
    let v = version;
    // Numeric parts compare as numbers: rc.10 is after rc.2.
    assert!(v("0.6.0-rc.2") < v("0.6.0-rc.10"));
    assert!(v("0.6.0-rc.10") < v("0.6.0"));
    assert!(v("0.6.0-alpha.1") < v("0.6.0-beta.1"));
    assert!(v("0.6.0-beta.9") < v("0.6.0-rc.1"));
    // A longer list wins when the shorter is its prefix, and a candidate for the next
    // version is after the current release.
    assert!(v("0.6.0-rc") < v("0.6.0-rc.1"));
    assert!(v("0.5.1") < v("0.6.0-rc.1"));
}

/// The changelog the repository ships is read newest first by the app and by `tools/changelog.sh
/// notes`, so its sections must descend.
#[test]
fn the_changelog_lists_versions_newest_first() {
    let text = include_str!("../../../CHANGELOG.md");
    // Section by section: `parse` caps one release body, and the archive outgrows that.
    let versions: Vec<Version> = text
        .split("\n## ")
        .skip(1)
        .flat_map(|section| parse(&format!("## {section}")))
        .map(|section| Version::parse(&section.version).expect("a readable version"))
        .collect();
    assert_eq!(versions.len(), text.matches("\n## ").count());
    assert!(
        versions.windows(2).all(|pair| pair[0] > pair[1]),
        "CHANGELOG.md sections are not strictly newest first"
    );
}

/// A release body as the workflow writes it: one version's section.
fn changes(version: &str, new: &[&str], fixed: &[&str]) -> String {
    let mut text = format!("## {version} (2026-11-20)\n");
    for (title, lines) in [("New", new), ("Fixed", fixed)] {
        if !lines.is_empty() {
            text.push_str(&format!("\n### {title}\n\n"));
            for line in lines {
                text.push_str(&format!("- {line}\n"));
            }
        }
    }
    text
}

/// `body` followed by a "Fixed in release candidates" list.
fn with_rc_fixes(body: String, lines: &[&str]) -> String {
    let mut text = format!("{body}\n### Fixed in release candidates\n\n");
    for line in lines {
        text.push_str(&format!("- {line}\n"));
    }
    text
}

/// A published release of the workflow.
fn listed<'a>(tag: &'a str, body: &'a str) -> ListedRelease<'a> {
    ListedRelease {
        tag,
        draft: false,
        trusted: true,
        body,
    }
}

/// The notes and coverage of a range, with the list of releases read to its end.
fn range(releases: &[ListedRelease<'_>], running: &str, offered: &str) -> (ReleaseNotes, Coverage) {
    notes_for_range(
        releases.iter().copied(),
        &version(running),
        &version(offered),
        false,
    )
}

#[test]
fn a_range_carries_the_changes_after_the_running_version_up_to_the_offered_one() {
    let three = changes("0.6.0-rc.3", &["Three."], &["Fix three."]);
    let two = changes("0.6.0-rc.2", &["Two."], &[]);
    let one = changes("0.6.0-rc.1", &["One."], &[]);
    let installed = changes("0.5.1", &["Already installed."], &[]);
    let releases = [
        listed("v0.6.0-rc.3", &three),
        listed("v0.6.0-rc.2", &two),
        listed("v0.6.0-rc.1", &one),
        listed("v0.5.1", &installed),
    ];
    let (notes, _) = range(&releases, "0.5.1", "0.6.0-rc.3");
    assert_eq!(notes.new, ["Three.", "Two.", "One."]);
    assert_eq!(notes.fixed, ["Fix three."]);
    // Only what the running build lacks.
    let (notes, _) = range(&releases, "0.6.0-rc.1", "0.6.0-rc.3");
    assert_eq!(notes.new, ["Three.", "Two."]);
}

#[test]
fn a_stable_range_shows_the_candidates_changes_once_each() {
    // The stable release repeats the candidates' sections, as the workflow writes it; the copy in
    // the release of that version is the one kept.
    let stable = [
        changes("0.6.0", &["Final."], &[]),
        changes("0.6.0-rc.2", &["Two, repeated."], &[]),
        changes("0.6.0-rc.1", &["One, repeated."], &[]),
    ]
    .join("\n");
    let two = changes("0.6.0-rc.2", &["Two."], &[]);
    let one = changes("0.6.0-rc.1", &["One."], &[]);
    let releases = [
        listed("v0.6.0", &stable),
        listed("v0.6.0-rc.2", &two),
        listed("v0.6.0-rc.1", &one),
        listed("v0.5.1", ""),
    ];
    let (notes, _) = range(&releases, "0.5.1", "0.6.0");
    assert_eq!(notes.new, ["Final.", "Two.", "One."]);
    // The same when the aggregate is read before the candidates' own releases.
    let reversed = [releases[3], releases[2], releases[1], releases[0]];
    let (notes, _) = range(&reversed, "0.5.1", "0.6.0");
    assert_eq!(notes.new, ["Final.", "Two.", "One."]);
}

#[test]
fn fixes_for_bugs_only_candidates_had_are_hidden_from_a_stable_version() {
    let candidate_two = with_rc_fixes(
        changes("0.6.0-rc.2", &["Two."], &["Fix two."]),
        &["Candidate fix two."],
    );
    let candidate_one = with_rc_fixes(
        changes("0.6.0-rc.1", &["One."], &[]),
        &["Candidate fix one."],
    );
    // The stable aggregate may repeat the candidates' sections with their lists; the notes drop
    // them either way.
    let stable = [
        with_rc_fixes(changes("0.6.0", &["Final."], &["Fix final."]), &["Late."]),
        candidate_two.clone(),
        candidate_one.clone(),
    ]
    .join("\n");
    let releases = [
        listed("v0.6.0", &stable),
        listed("v0.6.0-rc.2", &candidate_two),
        listed("v0.6.0-rc.1", &candidate_one),
        listed("v0.5.1", ""),
    ];
    let (notes, _) = range(&releases, "0.5.1", "0.6.0");
    assert_eq!(notes.new, ["Final.", "Two.", "One."]);
    assert_eq!(notes.fixed, ["Fix final.", "Fix two."]);
    assert_eq!(notes.omitted, 0, "hidden fixes are not 'more'");
    // Someone on a candidate is shown them: they may have hit them.
    let (notes, _) = range(&releases, "0.6.0-rc.1", "0.6.0");
    assert_eq!(
        notes.fixed,
        ["Fix final.", "Late.", "Fix two.", "Candidate fix two."]
    );
    // Offered a candidate only, a stable version still does not see them.
    let (notes, _) = range(&releases[1..], "0.5.1", "0.6.0-rc.2");
    assert_eq!(notes.fixed, ["Fix two."]);
    let (notes, _) = range(&releases[1..], "0.6.0-rc.1", "0.6.0-rc.2");
    assert_eq!(notes.fixed, ["Fix two.", "Candidate fix two."]);
}

#[test]
fn a_candidate_fix_first_shipped_in_the_stable_release_is_hidden_in_its_body_and_still_read() {
    // As `tools/changelog.sh notes` writes a stable body: the fix is in an HTML comment, which
    // GitHub does not draw, and no candidate's own release lists it.
    let stable = "\
## 0.6.0 (2026-11-20)

### Fixed

- Public fix.

<!-- release-candidate fixes
### Fixed in release candidates

- A bug the last candidate had.
-->
";
    let one = changes("0.6.0-rc.1", &["One."], &[]);
    let releases = [
        listed("v0.6.0", stable),
        listed("v0.6.0-rc.1", &one),
        listed("v0.5.1", ""),
    ];
    // Someone on the first candidate may have hit that bug, so they are told.
    let (notes, _) = range(&releases, "0.6.0-rc.1", "0.6.0");
    assert_eq!(
        notes.fixed,
        ["Public fix.", "A bug the last candidate had."]
    );
    // Someone on a stable version never had it.
    let (notes, _) = range(&releases, "0.5.1", "0.6.0");
    assert_eq!(notes.fixed, ["Public fix."]);
    assert_eq!(notes.omitted, 0);
}

#[test]
fn notes_come_only_from_published_releases_of_the_workflow() {
    let draft_body = changes("0.6.0-rc.2", &["From a draft."], &[]);
    let hand_made_body = changes("0.6.0-rc.3", &["From a person."], &[]);
    let mut forged_body = changes("0.6.0-rc.4", &["Forged by a stranger."], &[]);
    forged_body.push_str(&changes("0.6.0-rc.1", &["Forged too."], &[]));
    let four = changes("0.6.0-rc.4", &["Four."], &[]);
    let one = changes("0.6.0-rc.1", &["One."], &[]);
    let stranger = |tag, body| ListedRelease {
        trusted: false,
        ..listed(tag, body)
    };
    let releases = [
        stranger("v0.6.0-rc.5", &forged_body),
        listed("v0.6.0-rc.4", &four),
        stranger("v0.6.0-rc.3", &hand_made_body),
        ListedRelease {
            draft: true,
            ..listed("v0.6.0-rc.2", &draft_body)
        },
        listed("v0.6.0-rc.1", &one),
    ];
    let (notes, _) = range(&releases, "0.5.1", "0.6.0-rc.5");
    assert_eq!(notes.new, ["Four.", "One."]);
}

#[test]
fn changes_the_notes_cannot_hold_are_counted_once_not_per_copy() {
    let lines: Vec<String> = (0..MAX_ENTRIES + 5)
        .map(|n| format!("Change {n}."))
        .collect();
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    // The stable release repeats the candidate's section with more entries than the candidate's
    // own release lists; only the own copy is read, so only its 5 are left out.
    let repeated: Vec<&str> = lines.iter().copied().chain(["Extra.", "More."]).collect();
    let stable = [
        changes("0.6.0", &["Final."], &[]),
        changes("0.6.0-rc.1", &repeated, &[]),
    ]
    .join("\n");
    let candidate = changes("0.6.0-rc.1", &lines, &[]);
    let releases = [listed("v0.6.0", &stable), listed("v0.6.0-rc.1", &candidate)];
    let (notes, _) = range(&releases, "0.5.1", "0.6.0");
    assert_eq!(notes.new.len(), 1 + MAX_ENTRIES);
    assert_eq!(notes.omitted, 5);
}

#[test]
fn sections_outside_the_range_or_without_a_readable_version_are_left_out() {
    let body = [
        changes("0.6.0-rc.9", &["Newer than the offer."], &[]),
        changes("0.6.0-rc.2", &["In range."], &[]),
        changes("0.6.0-rc.1", &["The running one."], &[]),
        changes("0.5.0", &["Older."], &[]),
        changes("0.6.0-rc1", &["Not a version."], &[]),
    ]
    .join("\n");
    let releases = [listed("v0.6.0-rc.2", &body)];
    let (notes, _) = range(&releases, "0.6.0-rc.1", "0.6.0-rc.2");
    assert_eq!(notes.new, ["In range."]);
}

#[test]
fn releases_without_notes_or_with_a_commit_log_carry_none() {
    let old = "## Changes since v0.5.0\n\n- Something (abc1234)\n\n## Downloads\n";
    for body in [old, ""] {
        let (notes, coverage) = range(&[listed("v0.6.0", body)], "0.5.1", "0.6.0");
        assert!(notes.is_empty());
        assert_eq!(coverage.unsummarized, 1);
    }
}

#[test]
fn releases_in_the_range_with_no_trusted_section_are_counted_once() {
    let hand_made = changes("0.6.0-rc.2", &["From a person."], &[]);
    let four = changes("0.6.0-rc.4", &["Four."], &[]);
    let one = changes("0.6.0-rc.1", &["One."], &[]);
    let releases = [
        ListedRelease {
            draft: true,
            ..listed("v0.6.0-rc.5", "")
        },
        listed("v0.6.0-rc.4", &four),
        listed("v0.6.0-rc.3", ""),
        ListedRelease {
            trusted: false,
            ..listed("v0.6.0-rc.2", &hand_made)
        },
        listed("v0.6.0-rc.1", &one),
        listed("v0.6.0-rc.1", ""),
        listed("v0.5.1", ""),
        listed("v0.5.0", ""),
        listed("nightly", ""),
    ];
    // rc.3 has no notes and rc.2's are not the workflow's; a draft, the duplicate rc.1, a tag that
    // is no version and the releases up to the running one are not gaps.
    let (notes, coverage) = range(&releases, "0.5.1", "0.6.0-rc.5");
    assert_eq!(notes.new, ["Four.", "One."]);
    assert_eq!(coverage.unsummarized, 2);
}

#[test]
fn a_release_another_release_summarizes_is_not_a_gap() {
    // The stable release's notes repeat the candidate's section; the candidate has none.
    let mut body = changes("0.6.0", &["Final."], &[]);
    body.push_str(&changes("0.6.0-rc.1", &["Candidate."], &[]));
    let releases = [listed("v0.6.0", &body), listed("v0.6.0-rc.1", "")];
    let (notes, coverage) = range(&releases, "0.5.1", "0.6.0");
    assert_eq!(notes.new, ["Final.", "Candidate."]);
    assert_eq!(coverage.unsummarized, 0);
}

#[test]
fn an_incomplete_listing_is_passed_through_not_counted() {
    let releases = [listed("v0.6.0", "")];
    for history_incomplete in [false, true] {
        let (_, coverage) = notes_for_range(
            releases.iter().copied(),
            &version("0.5.1"),
            &version("0.6.0"),
            history_incomplete,
        );
        assert_eq!(coverage.history_incomplete, history_incomplete);
        assert_eq!(coverage.unsummarized, 1);
    }
}

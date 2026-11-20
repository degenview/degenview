//! The changelog as the app reads it: the sections of a release's notes, and the notes an update
//! carries. `tools/changelog.sh` writes the text; this is the one reader of its shape.
//!
//! ```text
//! ## 0.6.0-rc.33 (2026-11-20)
//!
//! ### New
//!
//! - A line the user notices.
//!
//! ### Fixed
//!
//! - Another one.
//!
//! ### Fixed in release candidates
//!
//! - A bug only the candidates had.
//! ```
//!
//! A stable release's body wraps each `Fixed in release candidates` subsection in an HTML comment
//! (`<!-- release-candidate fixes` before it, `-->` after it; a section with nothing else is wrapped
//! whole, heading included) so GitHub does not draw bugs a stable reader never had. The two marker
//! lines are neither headings nor bullets, so they are ignored here and the heading and bullets
//! between them are read like any other: a person running a candidate still sees those fixes.
//!
//! The text comes from a GitHub release body, so it is bounded and never trusted to be well formed:
//! anything that is not a version heading, a `New`/`Fixed`/`Fixed in release candidates` subheading
//! or a bullet under one is ignored, which is why the commit-list bodies of older releases yield
//! nothing. A body longer than the bound is cut at the end of a whole line, and the section being
//! read there is marked `truncated`.

mod version;

#[cfg(test)]
mod tests;

pub use version::{Stability, Version};

use std::collections::{BTreeMap, BTreeSet};

/// The longest release body read, in bytes.
const MAX_BODY_BYTES: usize = 64 * 1024;
/// The most sections one body yields.
const MAX_SECTIONS: usize = 64;
/// The most entries one section keeps in each of its three lists.
pub const MAX_ENTRIES: usize = 100;
/// The longest entry kept, in characters; a longer one is cut with an ellipsis.
const MAX_ENTRY_CHARS: usize = 400;
/// The most entries merged notes keep in each list. The release page has the rest.
pub const MAX_NOTES: usize = 300;
/// Ends a list that a cut-short body left incomplete.
const TRUNCATED_LINE: &str = "And more changes in the full release notes.";
/// Ends a list whose releases the update check read only the newest of.
const HISTORY_INCOMPLETE_LINE: &str =
    "Older releases are not listed here, so some changes may be missing.";

/// What one release added to the changelog.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Section {
    /// Without a leading `v`: "0.6.0-rc.33".
    pub version: String,
    pub new: Vec<String>,
    pub fixed: Vec<String>,
    /// Fixes for bugs that only release candidates had, kept apart so a person updating from a
    /// stable release (who never had them) is not shown them.
    pub fixed_rc: Vec<String>,
    /// Entries the reader's caps left out. A section past `MAX_SECTIONS` has no place of its own, so
    /// its entries are counted here, in the last section kept.
    pub omitted: usize,
    /// The same for the release-candidate fixes, counted apart: a reader who is not shown those
    /// fixes (`ReleaseNotes::from_sections`) was not told about them, so they are not "more".
    pub omitted_rc: usize,
    /// The body ended before the text did while this section was being read (`MAX_BODY_BYTES`), so
    /// it may hold more than it lists, and later sections may be missing. `omitted` cannot say how
    /// many.
    pub truncated: bool,
}

/// The changes of several releases together, newest release first, as an update sheet lists them.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ReleaseNotes {
    pub new: Vec<String>,
    pub fixed: Vec<String>,
    /// How many more changes there were than the lists hold, whichever cap left them out. Not
    /// counting what `truncated` cut off.
    pub omitted: usize,
    /// A body was cut short: there are more changes than `omitted` counts.
    pub truncated: bool,
}

/// How far the notes of a range reach: what the lists cannot say about the releases they were read
/// from. Kept beside `ReleaseNotes` rather than in it, because it describes the range and the list
/// of releases, not the text of any section.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Coverage {
    /// How many releases in the range had no section of their own anywhere, so the lists may not
    /// hold everything they changed: releases from before the changelog, or with notes the app does
    /// not trust.
    pub unsummarized: usize,
    /// The list of releases the notes were read from ended before reaching the reader's own
    /// version, so releases in the range may be missing from it altogether (and are not in
    /// `unsummarized`).
    pub history_incomplete: bool,
}

/// One entry of a list of published releases, as far as the notes of a range need it.
#[derive(Clone, Copy, Debug)]
pub struct ListedRelease<'a> {
    /// The release's tag, "v0.6.0-rc.33"; a tag that is not a version still counts as a release
    /// but has no place in any range.
    pub tag: &'a str,
    /// A draft is not published: it supplies nothing and is not a gap.
    pub draft: bool,
    /// Its body was written by the release workflow, the only author whose notes are read. Any
    /// published release counts toward the range, trusted or not.
    pub trusted: bool,
    /// The notes, as the release body holds them.
    pub body: &'a str,
}

impl ReleaseNotes {
    /// `sections` in the order they are to read: the lists are concatenated, `MAX_NOTES` entries at
    /// most each, and what the sections and this cap leave out is counted.
    ///
    /// With `include_rc_fixes` a section's release-candidate fixes follow its own fixes in `fixed`
    /// (a person running a candidate may have had those bugs). Without it they are dropped and not
    /// counted as omitted, those a section's cap left out included: they are not part of what this
    /// reader is told about.
    pub fn from_sections(
        sections: impl IntoIterator<Item = Section>,
        include_rc_fixes: bool,
    ) -> Self {
        let mut notes = Self::default();
        for section in sections {
            notes.new.extend(section.new);
            notes.fixed.extend(section.fixed);
            if include_rc_fixes {
                notes.fixed.extend(section.fixed_rc);
                notes.omitted = notes.omitted.saturating_add(section.omitted_rc);
            }
            notes.omitted = notes.omitted.saturating_add(section.omitted);
            notes.truncated |= section.truncated;
        }
        for list in [&mut notes.new, &mut notes.fixed] {
            notes.omitted = notes
                .omitted
                .saturating_add(list.len().saturating_sub(MAX_NOTES));
            list.truncate(MAX_NOTES);
        }
        notes
    }

    pub fn is_empty(&self) -> bool {
        self.new.is_empty() && self.fixed.is_empty()
    }

    /// The lines that end the lists, in order: what the caps left out (not as a count when a body
    /// was cut short, which makes any count too low), then the releases with no summary, then the
    /// releases that were never listed. A reader draws them after a list; with nothing to list it
    /// says so and nothing more.
    pub fn footnotes(&self, coverage: &Coverage) -> Vec<String> {
        let left_out = if self.truncated {
            Some(TRUNCATED_LINE.to_owned())
        } else {
            (self.omitted > 0).then(|| omitted_line(self.omitted))
        };
        [
            left_out,
            (coverage.unsummarized > 0).then(|| unsummarized_line(coverage.unsummarized)),
            coverage
                .history_incomplete
                .then(|| HISTORY_INCOMPLETE_LINE.to_owned()),
        ]
        .into_iter()
        .flatten()
        .collect()
    }
}

/// What ends a list that left `count` changes out.
fn omitted_line(count: usize) -> String {
    let changes = if count == 1 { "change" } else { "changes" };
    format!("And {count} more {changes} in the full release notes.")
}

/// What ends a list that is missing the changes of `count` releases with no summary.
fn unsummarized_line(count: usize) -> String {
    if count == 1 {
        "1 release since your version has no summary, so its changes are not listed.".to_owned()
    } else {
        format!(
            "{count} releases since your version have no summary, so their changes are not listed."
        )
    }
}

/// The changes after `running` up to and including `offered`, newest version first, read from the
/// bodies of `releases`, and how far that reach. A release candidate's section is repeated in the
/// stable release that follows it; the copy from the release carrying that version is the one kept,
/// else the first one listed. A channel does not limit this: a stable user going from 0.5.1 to
/// 0.6.0 sees the candidates' changes too, except the fixes for bugs only candidates had, which are
/// listed only to someone running a candidate.
///
/// Only published (not draft) releases the workflow wrote supply sections, and only the ones in the
/// range with a readable version. `history_incomplete` says whoever listed the releases stopped
/// before the running version's place in the list, so releases in the range may not be among
/// `releases` at all (and are not counted as unsummarized).
pub fn notes_for_range<'a>(
    releases: impl IntoIterator<Item = ListedRelease<'a>>,
    running: &Version,
    offered: &Version,
    history_incomplete: bool,
) -> (ReleaseNotes, Coverage) {
    let in_range = |version: &Version| version > running && version <= offered;
    // Version, then whether the section came from the release of that very version.
    let mut found: BTreeMap<Version, (bool, Section)> = BTreeMap::new();
    // Every published release in the range counts, whoever published it: one whose version no
    // trusted body has a section for is a gap the reader is told about.
    let mut published = BTreeSet::new();
    for release in releases.into_iter().filter(|release| !release.draft) {
        let own = Version::parse(release.tag);
        if let Some(version) = own.clone().filter(in_range) {
            published.insert(version);
        }
        if !release.trusted {
            continue;
        }
        for section in parse(release.body) {
            let Some(version) = Version::parse(&section.version).filter(in_range) else {
                continue;
            };
            let is_own = own.as_ref() == Some(&version);
            if found
                .get(&version)
                .is_none_or(|(had_own, _)| is_own && !had_own)
            {
                found.insert(version, (is_own, section));
            }
        }
    }
    let unsummarized = published
        .iter()
        .filter(|version| !found.contains_key(version))
        .count();
    let notes = ReleaseNotes::from_sections(
        found.into_values().rev().map(|(_, section)| section),
        running.is_prerelease(),
    );
    (
        notes,
        Coverage {
            unsummarized,
            history_incomplete,
        },
    )
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Category {
    New,
    Fixed,
    FixedInCandidates,
}

/// The sections of `text`, in the order they appear. A `## ` heading whose first word is a version
/// opens one; any other `#` or `##` heading closes it, as does the end of the text.
pub fn parse(text: &str) -> Vec<Section> {
    let mut sections: Vec<Section> = Vec::new();
    let mut open = false;
    // Inside a section past `MAX_SECTIONS`: its entries are counted, not kept.
    let mut dropped = false;
    let mut category = None;
    // How many `<details>` blocks the line is inside: the commit list lives in one.
    let mut folded = 0_usize;
    let (text, cut) = bounded(text);
    for line in text.lines() {
        let trimmed = line.trim();
        let lower = trimmed.to_ascii_lowercase();
        if lower.starts_with("<details") {
            folded += 1;
            continue;
        }
        if lower.starts_with("</details") {
            folded = folded.saturating_sub(1);
            continue;
        }
        if folded > 0 {
            continue;
        }
        if let Some((level, title)) = heading(trimmed) {
            match level {
                1 | 2 => {
                    category = None;
                    open = false;
                    dropped = false;
                    if level == 2
                        && let Some(version) = version_of(title)
                    {
                        if sections.len() < MAX_SECTIONS {
                            sections.push(Section {
                                version,
                                ..Section::default()
                            });
                            open = true;
                        } else {
                            dropped = true;
                        }
                    }
                }
                _ if open || dropped => category = category_of(title),
                _ => {}
            }
            continue;
        }
        let (Some(category), Some(entry)) = (category, bullet(line)) else {
            continue;
        };
        let Some(section) = sections.last_mut().filter(|_| open || dropped) else {
            continue;
        };
        let list = match category {
            Category::New => &mut section.new,
            Category::Fixed => &mut section.fixed,
            Category::FixedInCandidates => &mut section.fixed_rc,
        };
        if open && list.len() < MAX_ENTRIES {
            list.push(entry);
        } else {
            let left_out = if category == Category::FixedInCandidates {
                &mut section.omitted_rc
            } else {
                &mut section.omitted
            };
            *left_out = left_out.saturating_add(1);
        }
    }
    // The cut fell in a section's own text (not in a folded commit list or between sections), so
    // what follows may have been changes.
    if cut
        && (open || dropped)
        && folded == 0
        && let Some(section) = sections.last_mut()
    {
        section.truncated = true;
    }
    sections
}

/// `text` up to `MAX_BODY_BYTES`, cut back to the end of a whole line, and whether it was cut. A
/// line cut in half could be a shorter entry that reads as complete.
fn bounded(text: &str) -> (&str, bool) {
    if text.len() <= MAX_BODY_BYTES {
        return (text, false);
    }
    let mut end = MAX_BODY_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let whole_lines = text[..end].rfind('\n').map_or(0, |at| at + 1);
    (&text[..whole_lines], true)
}

/// A markdown ATX heading: how many `#`, and its text.
fn heading(line: &str) -> Option<(usize, &str)> {
    let level = line.bytes().take_while(|&byte| byte == b'#').count();
    let title = line[level..].strip_prefix(' ')?;
    (1..=6).contains(&level).then_some((level, title.trim()))
}

/// The version a section heading opens, without a leading `v`: its first word, when that is
/// `X.Y.Z` with an optional `-prerelease`/`+build` tail.
fn version_of(title: &str) -> Option<String> {
    let word = title.split_whitespace().next()?;
    let word = word.strip_prefix('v').unwrap_or(word);
    let (core, tail) = match word.find(['-', '+']) {
        Some(at) => word.split_at(at),
        None => (word, ""),
    };
    let mut numbers = core.split('.');
    let numeric = |part: Option<&str>| {
        part.is_some_and(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
    };
    let core_ok = numeric(numbers.next())
        && numeric(numbers.next())
        && numeric(numbers.next())
        && numbers.next().is_none();
    let tail_ok = tail
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'+'));
    (core_ok && tail_ok).then(|| word.to_owned())
}

fn category_of(title: &str) -> Option<Category> {
    if title.eq_ignore_ascii_case("new") {
        Some(Category::New)
    } else if title.eq_ignore_ascii_case("fixed") {
        Some(Category::Fixed)
    } else if title.eq_ignore_ascii_case("fixed in release candidates") {
        Some(Category::FixedInCandidates)
    } else {
        None
    }
}

/// The text of a top-level `- ` or `* ` bullet, trimmed and cut to `MAX_ENTRY_CHARS`; none for a
/// line that is not one, or an empty one.
fn bullet(line: &str) -> Option<String> {
    let text = line
        .strip_prefix("- ")
        .or_else(|| line.strip_prefix("* "))?;
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let mut chars = text.chars();
    let kept: String = chars.by_ref().take(MAX_ENTRY_CHARS).collect();
    Some(if chars.next().is_some() {
        format!("{}…", kept.trim_end())
    } else {
        kept
    })
}

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
//! ```
//!
//! The text comes from a GitHub release body, so it is bounded and never trusted to be well formed:
//! anything that is not a version heading, a `New`/`Fixed` subheading or a bullet under one is
//! ignored, which is why the commit-list bodies of older releases yield nothing.

#[cfg(test)]
mod tests;

/// The longest release body read, in bytes.
const MAX_BODY_BYTES: usize = 64 * 1024;
/// The most sections one body yields.
const MAX_SECTIONS: usize = 64;
/// The most entries one section keeps in each of its two lists.
const MAX_ENTRIES: usize = 100;
/// The longest entry kept, in characters; a longer one is cut with an ellipsis.
const MAX_ENTRY_CHARS: usize = 400;
/// The most entries merged notes keep in each list. The release page has the rest.
pub const MAX_NOTES: usize = 300;

/// What one release added to the changelog.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Section {
    /// Without a leading `v`: "0.6.0-rc.33".
    pub version: String,
    pub new: Vec<String>,
    pub fixed: Vec<String>,
}

/// The changes of several releases together, newest release first, as an update sheet lists them.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ReleaseNotes {
    pub new: Vec<String>,
    pub fixed: Vec<String>,
}

impl ReleaseNotes {
    /// `sections` in the order they are to read: the lists are concatenated, `MAX_NOTES` entries at
    /// most each.
    pub fn from_sections(sections: impl IntoIterator<Item = Section>) -> Self {
        let mut notes = Self::default();
        for section in sections {
            notes.new.extend(section.new);
            notes.fixed.extend(section.fixed);
        }
        notes.new.truncate(MAX_NOTES);
        notes.fixed.truncate(MAX_NOTES);
        notes
    }

    pub fn is_empty(&self) -> bool {
        self.new.is_empty() && self.fixed.is_empty()
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Category {
    New,
    Fixed,
}

/// The sections of `text`, in the order they appear. A `## ` heading whose first word is a version
/// opens one; any other `#` or `##` heading closes it, as does the end of the text.
pub fn parse(text: &str) -> Vec<Section> {
    let mut sections: Vec<Section> = Vec::new();
    let mut open = false;
    let mut category = None;
    // How many `<details>` blocks the line is inside: the commit list lives in one.
    let mut folded = 0_usize;
    for line in bounded(text).lines() {
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
                    if level == 2
                        && let Some(version) = version_of(title)
                        && sections.len() < MAX_SECTIONS
                    {
                        sections.push(Section {
                            version,
                            ..Section::default()
                        });
                        open = true;
                    }
                }
                _ if open => category = category_of(title),
                _ => {}
            }
            continue;
        }
        let (Some(category), Some(entry)) = (category, bullet(line)) else {
            continue;
        };
        let Some(section) = sections.last_mut().filter(|_| open) else {
            continue;
        };
        let list = match category {
            Category::New => &mut section.new,
            Category::Fixed => &mut section.fixed,
        };
        if list.len() < MAX_ENTRIES {
            list.push(entry);
        }
    }
    sections
}

/// `text` up to `MAX_BODY_BYTES`, cut on a character boundary.
fn bounded(text: &str) -> &str {
    let mut end = text.len().min(MAX_BODY_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
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

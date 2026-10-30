//! The theme file: the JSON a theme is exported to and imported from.
//!
//! ```json
//! {
//!   "format": "degenview-editor-theme",
//!   "version": 1,
//!   "name": "My Dracula",
//!   "base": "builtin.dracula",
//!   "syntax": { "default": "#F8F8F2", "keyword": "#FF79C6", … },
//!   "editor": { "background": "#282A36", "selection": "#44475A", … },
//!   "typography": { "fontFamily": "JetBrains Mono", "fontSize": 14.0, "lineHeight": "relaxed" }
//! }
//! ```
//!
//! An export is complete: every colour of the theme, resolved, so the file looks the same whatever
//! a later version does to the built-in it started from. `typography` holds only what the theme
//! sets. Colours are `#RRGGBB` or `#RRGGBBAA`. On import, the colours that match the base are
//! dropped, so the imported theme overrides only what differs.
//!
//! Import never trusts the file: it is read only up to [`MAX_FILE_BYTES`], must be an object of this
//! format and a version this build reads, and every value is checked. A colour it cannot read is
//! skipped and reported, not guessed; a name it cannot use or an unknown base fails the import.

use super::{
    BuiltInTheme, ColorKey, EditorTheme, LineHeight, NewTheme, SurfaceColor, SyntaxRole,
    TypographyOverride, clamp_size, clean_family, clean_name,
};
use serde::Serialize;
use serde::ser::{SerializeMap, Serializer};
use serde_json::Value;
use std::collections::BTreeMap;

pub const FORMAT: &str = "degenview-editor-theme";
pub const VERSION: u32 = 1;
/// Larger files are refused unread. A theme is a few kilobytes.
pub const MAX_FILE_BYTES: usize = 256 * 1024;

/// Pairs written as a JSON object in the order given, so a file reads top to bottom the way the
/// editor's settings do.
struct Ordered(Vec<(&'static str, String)>);

impl Serialize for Ordered {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (key, value) in &self.0 {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}

#[derive(Serialize)]
struct File<'a> {
    format: &'static str,
    version: u32,
    name: &'a str,
    base: &'a str,
    syntax: Ordered,
    editor: Ordered,
    #[serde(skip_serializing_if = "TypographyOverride::is_empty")]
    typography: TypographyOverride,
}

/// The theme as a file: pretty-printed JSON with every colour.
pub fn export(theme: &EditorTheme) -> String {
    let palette = theme.palette();
    let file = File {
        format: FORMAT,
        version: VERSION,
        name: &theme.name,
        base: theme.base_theme().id(),
        syntax: Ordered(
            SyntaxRole::ALL
                .iter()
                .map(|role| (role.key(), palette.syntax[role.index()].to_hex()))
                .collect(),
        ),
        editor: Ordered(
            SurfaceColor::ALL
                .iter()
                .map(|surface| (surface.key(), palette.surfaces[surface.index()].to_hex()))
                .collect(),
        ),
        typography: theme.typography.clone().sanitized(),
    };
    let mut text = serde_json::to_string_pretty(&file).expect("a theme file serialises");
    text.push('\n');
    text
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ImportError {
    TooLarge,
    NotJson,
    NotAThemeFile,
    UnsupportedVersion(u64),
    MissingName,
    BadName(String),
    UnknownBase(String),
}

impl ImportError {
    pub fn message(&self) -> String {
        match self {
            Self::TooLarge => format!(
                "That file is larger than {} KB, which no theme is.",
                MAX_FILE_BYTES / 1024
            ),
            Self::NotJson => "That file is not valid JSON.".to_owned(),
            Self::NotAThemeFile => "That file is not a DegenView editor theme.".to_owned(),
            Self::UnsupportedVersion(version) => format!(
                "That theme is version {version}; this version of DegenView reads version {VERSION}. Update DegenView to import it."
            ),
            Self::MissingName => "That theme has no name.".to_owned(),
            Self::BadName(why) => format!("That theme's name cannot be used: {why}"),
            Self::UnknownBase(base) => {
                format!("That theme starts from \"{base}\", which this version does not have.")
            }
        }
    }
}

/// What an import read: the theme, and what was left out and why.
#[derive(Clone, Debug, PartialEq)]
pub struct Imported {
    pub theme: NewTheme,
    /// One line for each value that was skipped.
    pub skipped: Vec<String>,
}

/// Reads a theme file. See the module docs for what is checked.
pub fn import(text: &str) -> Result<Imported, ImportError> {
    if text.len() > MAX_FILE_BYTES {
        return Err(ImportError::TooLarge);
    }
    let value: Value = serde_json::from_str(text).map_err(|_| ImportError::NotJson)?;
    let file = value.as_object().ok_or(ImportError::NotAThemeFile)?;
    if file.get("format").and_then(Value::as_str) != Some(FORMAT) {
        return Err(ImportError::NotAThemeFile);
    }
    match file.get("version").and_then(Value::as_u64) {
        Some(version) if version == u64::from(VERSION) => {}
        Some(version) => return Err(ImportError::UnsupportedVersion(version)),
        None => return Err(ImportError::NotAThemeFile),
    }
    let name = file
        .get("name")
        .and_then(Value::as_str)
        .ok_or(ImportError::MissingName)?;
    let name = clean_name(name).map_err(|error| ImportError::BadName(error.message()))?;
    let base_text = file
        .get("base")
        .and_then(Value::as_str)
        .ok_or_else(|| ImportError::UnknownBase(String::new()))?;
    let base = BuiltInTheme::from_id(base_text)
        .ok_or_else(|| ImportError::UnknownBase(base_text.chars().take(60).collect()))?;

    let mut skipped = Vec::new();
    let base_palette = base.palette();
    let mut overrides = BTreeMap::new();
    for (section, known) in [("syntax", true), ("editor", false)] {
        let Some(entries) = file.get(section) else {
            continue;
        };
        let Some(entries) = entries.as_object() else {
            skipped.push(format!(
                "\"{section}\" is not an object, so none of it was read."
            ));
            continue;
        };
        for (key, value) in entries {
            // A colour belongs to the section its kind names: a role under "syntax", a surface
            // under "editor". Anything else is a colour this version does not have.
            let colour_key = ColorKey::from_key(key)
                .filter(|colour| matches!(colour, ColorKey::Syntax(_)) == known);
            let Some(colour_key) = colour_key else {
                skipped.push(format!(
                    "\"{key}\" is not a {section} colour this version has."
                ));
                continue;
            };
            let Some(colour) = value.as_str().and_then(super::Rgba::parse) else {
                skipped.push(format!("\"{key}\" is not a colour (#RRGGBB or #RRGGBBAA)."));
                continue;
            };
            if colour != base_palette.get(colour_key) {
                overrides.insert(key.clone(), colour.to_hex());
            }
        }
    }

    let typography = match file.get("typography") {
        None => TypographyOverride::default(),
        Some(Value::Object(entries)) => read_typography(entries, &mut skipped),
        Some(_) => {
            skipped.push("\"typography\" is not an object, so none of it was read.".to_owned());
            TypographyOverride::default()
        }
    };
    Ok(Imported {
        theme: NewTheme {
            name,
            base,
            overrides,
            typography,
        },
        skipped,
    })
}

fn read_typography(
    entries: &serde_json::Map<String, Value>,
    skipped: &mut Vec<String>,
) -> TypographyOverride {
    let mut typography = TypographyOverride::default();
    if let Some(value) = entries.get("fontFamily") {
        match value.as_str().and_then(clean_family) {
            Some(family) => typography.font_family = Some(family),
            None => skipped.push("\"fontFamily\" is not a usable font name.".to_owned()),
        }
    }
    if let Some(value) = entries.get("fontSize") {
        match value.as_f64().filter(|size| size.is_finite()) {
            Some(size) => typography.font_size = Some(clamp_size(size as f32)),
            None => skipped.push("\"fontSize\" is not a number.".to_owned()),
        }
    }
    if let Some(value) = entries.get("lineHeight") {
        match value.as_str().and_then(LineHeight::from_key) {
            Some(height) => typography.line_height = Some(height),
            None => skipped
                .push("\"lineHeight\" is not one of compact, default, relaxed, loose.".to_owned()),
        }
    }
    typography
}

#[cfg(test)]
mod tests;

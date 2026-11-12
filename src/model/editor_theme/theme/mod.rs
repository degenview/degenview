//! A custom theme, and the rules for the collection of them.
//!
//! **Inheritance is one level deep.** A custom theme is a built-in theme (its `base`) plus the
//! colours it overrides. Making a theme from another custom theme, or duplicating one, copies the
//! base and the overrides, so nothing ever points at another custom theme: there is no cycle to
//! prevent, no parent to delete from under a child, and a built-in can only improve in an update
//! for every colour a theme does not override. Resetting a colour removes its override, which gives
//! back the base's colour, not a fixed default.
//!
//! Overrides are kept as the text they were written as (`#RRGGBB[AA]`) under the colour's key, so a
//! key this version does not know (a newer version's colour) or a value it cannot read is carried
//! along untouched instead of being dropped, and is simply ignored when the palette is built.

use super::typography::tolerant;
use super::{BuiltInTheme, ColorKey, Palette, Rgba, TypographyOverride};
use crate::model::watchlist;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub type ThemeId = u64;

pub const MAX_NAME_CHARS: usize = 48;
/// A reasonable ceiling on custom themes: a collection is read whole on every change.
pub const MAX_THEMES: usize = 200;
/// Ids custom themes get start above this, so none is ever small enough to be mistaken for anything
/// else. (They are wall-clock ids, as every document's is.)
pub const FIRST_USER_ID: ThemeId = 1_000;
/// The schema of a stored theme.
pub const FORMAT_VERSION: u32 = 1;
/// Taken by the tile that follows the app's appearance.
pub const AUTOMATIC_NAME: &str = "Automatic";

fn default_base() -> String {
    BuiltInTheme::Dark.id().to_owned()
}

fn current_version() -> u32 {
    FORMAT_VERSION
}

/// A theme a person made.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditorTheme {
    pub id: ThemeId,
    pub name: String,
    /// The id of the built-in theme it starts from. One this version does not know reads as Dark.
    #[serde(default = "default_base")]
    pub base: String,
    /// Colour key to colour text, for the colours that differ from the base's.
    #[serde(default, deserialize_with = "tolerant")]
    pub overrides: BTreeMap<String, String>,
    #[serde(default, deserialize_with = "tolerant")]
    pub typography: TypographyOverride,
    #[serde(default = "current_version")]
    pub version: u32,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
    /// Fields a newer version wrote, written back as they were.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

impl EditorTheme {
    pub fn base_theme(&self) -> BuiltInTheme {
        BuiltInTheme::from_id(&self.base).unwrap_or(BuiltInTheme::Dark)
    }

    /// The theme's colours: the base's, with each readable override laid over it.
    pub fn palette(&self) -> Palette {
        let mut palette = self.base_theme().palette();
        for (key, text) in &self.overrides {
            if let (Some(key), Some(color)) = (ColorKey::from_key(key), Rgba::parse(text)) {
                palette.set(key, color);
            }
        }
        palette
    }

    /// Whether `key` differs from the base's colour.
    pub fn is_overridden(&self, key: ColorKey) -> bool {
        self.overrides
            .get(key.key())
            .and_then(|text| Rgba::parse(text))
            .is_some_and(|color| color != self.base_theme().palette().get(key))
    }

    /// How many colours differ from the base's.
    pub fn override_count(&self) -> usize {
        let base = self.base_theme().palette();
        self.overrides
            .iter()
            .filter(|(key, text)| {
                matches!(
                    (ColorKey::from_key(key), Rgba::parse(text)),
                    (Some(key), Some(color)) if color != base.get(key)
                )
            })
            .count()
    }
}

/// Where a new theme starts from.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ThemeSource {
    BuiltIn(BuiltInTheme),
    Custom(ThemeId),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ThemeError {
    EmptyName,
    NameTooLong,
    ControlCharacters,
    DuplicateName,
    Missing,
    TooMany,
}

impl ThemeError {
    pub fn message(self) -> String {
        match self {
            Self::EmptyName => "Give the theme a name.".to_owned(),
            Self::NameTooLong => format!("Names are at most {MAX_NAME_CHARS} characters."),
            Self::ControlCharacters => "Names cannot contain control characters.".to_owned(),
            Self::DuplicateName => "A theme with that name already exists.".to_owned(),
            Self::Missing => "That theme no longer exists.".to_owned(),
            Self::TooMany => format!("You can keep at most {MAX_THEMES} custom themes."),
        }
    }
}

/// `raw` trimmed and checked as a theme name. Any Unicode text is a name.
pub fn clean_name(raw: &str) -> Result<String, ThemeError> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(ThemeError::EmptyName);
    }
    if name.chars().any(char::is_control) {
        return Err(ThemeError::ControlCharacters);
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(ThemeError::NameTooLong);
    }
    Ok(name.to_owned())
}

/// What a theme file holds, ready to become a theme.
#[derive(Clone, Debug, PartialEq)]
pub struct NewTheme {
    pub name: String,
    pub base: BuiltInTheme,
    pub overrides: BTreeMap<String, String>,
    pub typography: TypographyOverride,
}

/// The custom themes, in the order they were made, and every rule about changing them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ThemeBook {
    themes: Vec<EditorTheme>,
}

impl ThemeBook {
    pub fn new(themes: Vec<EditorTheme>) -> Self {
        Self { themes }
    }

    pub fn saved(&self) -> &[EditorTheme] {
        &self.themes
    }

    pub fn get(&self, id: ThemeId) -> Option<&EditorTheme> {
        self.themes.iter().find(|theme| theme.id == id)
    }

    fn get_mut(&mut self, id: ThemeId) -> Result<&mut EditorTheme, ThemeError> {
        self.themes
            .iter_mut()
            .find(|theme| theme.id == id)
            .ok_or(ThemeError::Missing)
    }

    /// Whether `name` is a built-in's, the automatic tile's or another theme's (ignoring case).
    fn name_taken(&self, name: &str, except: Option<ThemeId>) -> bool {
        let lowered = name.to_lowercase();
        lowered == AUTOMATIC_NAME.to_lowercase()
            || BuiltInTheme::ALL
                .iter()
                .any(|theme| theme.name().to_lowercase() == lowered)
            || self
                .themes
                .iter()
                .any(|theme| Some(theme.id) != except && theme.name.to_lowercase() == lowered)
    }

    /// `raw` as a name for a theme (`except` is the theme being renamed, whose own name is free):
    /// the cleaned name, or why it cannot be used.
    pub fn check_name(&self, raw: &str, except: Option<ThemeId>) -> Result<String, ThemeError> {
        let name = clean_name(raw)?;
        if self.name_taken(&name, except) {
            return Err(ThemeError::DuplicateName);
        }
        Ok(name)
    }

    /// `base`, or `base 2`, `base 3`… the first that is free. A name already at the length limit
    /// loses its end to make room for the number.
    pub fn free_name(&self, base: &str) -> String {
        let base = base.trim();
        if !self.name_taken(base, None) {
            return base.to_owned();
        }
        (2..)
            .map(|number| {
                let suffix = format!(" {number}");
                let room = MAX_NAME_CHARS.saturating_sub(suffix.chars().count());
                let head: String = base.chars().take(room).collect();
                format!("{}{suffix}", head.trim_end())
            })
            .find(|name| !self.name_taken(name, None))
            .expect("some number is free")
    }

    fn push(&mut self, theme: EditorTheme) -> Result<ThemeId, ThemeError> {
        if self.themes.len() >= MAX_THEMES {
            return Err(ThemeError::TooMany);
        }
        let id = theme.id;
        self.themes.push(theme);
        Ok(id)
    }

    /// A theme named `name` that starts as `source` looks. A custom source is flattened: the new
    /// theme takes its base and its overrides.
    pub fn create(
        &mut self,
        id: ThemeId,
        name: &str,
        source: ThemeSource,
        now_ms: i64,
    ) -> Result<ThemeId, ThemeError> {
        let name = clean_name(name)?;
        if self.name_taken(&name, None) {
            return Err(ThemeError::DuplicateName);
        }
        let (base, overrides, typography) = self.starting_point(source)?;
        self.push(EditorTheme {
            id,
            name,
            base: base.id().to_owned(),
            overrides,
            typography,
            version: FORMAT_VERSION,
            created_at: now_ms,
            updated_at: now_ms,
            extra: BTreeMap::new(),
        })
    }

    fn starting_point(
        &self,
        source: ThemeSource,
    ) -> Result<(BuiltInTheme, BTreeMap<String, String>, TypographyOverride), ThemeError> {
        match source {
            ThemeSource::BuiltIn(theme) => {
                Ok((theme, BTreeMap::new(), TypographyOverride::default()))
            }
            ThemeSource::Custom(id) => {
                let theme = self.get(id).ok_or(ThemeError::Missing)?;
                Ok((
                    theme.base_theme(),
                    theme.overrides.clone(),
                    theme.typography.clone(),
                ))
            }
        }
    }

    /// A copy of a built-in or custom theme named `<name> copy` (numbered when taken).
    pub fn duplicate(
        &mut self,
        id: ThemeId,
        source: ThemeSource,
        now_ms: i64,
    ) -> Result<ThemeId, ThemeError> {
        let name = match source {
            ThemeSource::BuiltIn(theme) => theme.name().to_owned(),
            ThemeSource::Custom(from) => self.get(from).ok_or(ThemeError::Missing)?.name.clone(),
        };
        let name = self.free_name(&format!("{name} copy"));
        self.create(id, &name, source, now_ms)
    }

    /// A theme from a file's contents, under `new.name` or the first free name like it.
    pub fn import(
        &mut self,
        id: ThemeId,
        new: NewTheme,
        now_ms: i64,
    ) -> Result<ThemeId, ThemeError> {
        let name = self.free_name(&clean_name(&new.name)?);
        self.push(EditorTheme {
            id,
            name,
            base: new.base.id().to_owned(),
            overrides: new.overrides,
            typography: new.typography.sanitized(),
            version: FORMAT_VERSION,
            created_at: now_ms,
            updated_at: now_ms,
            extra: BTreeMap::new(),
        })
    }

    pub fn rename(&mut self, id: ThemeId, name: &str, now_ms: i64) -> Result<(), ThemeError> {
        let name = clean_name(name)?;
        self.get_mut(id)?;
        if self.name_taken(&name, Some(id)) {
            return Err(ThemeError::DuplicateName);
        }
        let theme = self.get_mut(id)?;
        theme.name = name;
        theme.updated_at = now_ms;
        Ok(())
    }

    pub fn delete(&mut self, id: ThemeId) -> Result<(), ThemeError> {
        self.get_mut(id)?;
        self.themes.retain(|theme| theme.id != id);
        Ok(())
    }

    /// Sets a colour. A colour equal to the base's is no override, so the theme stays as small as
    /// what a person actually changed.
    pub fn set_color(
        &mut self,
        id: ThemeId,
        key: ColorKey,
        color: Rgba,
        now_ms: i64,
    ) -> Result<(), ThemeError> {
        let theme = self.get_mut(id)?;
        if color == theme.base_theme().palette().get(key) {
            theme.overrides.remove(key.key());
        } else {
            theme.overrides.insert(key.key().to_owned(), color.to_hex());
        }
        theme.updated_at = now_ms;
        Ok(())
    }

    /// Gives `keys` back the base's colours.
    pub fn reset_colors(
        &mut self,
        id: ThemeId,
        keys: &[ColorKey],
        now_ms: i64,
    ) -> Result<(), ThemeError> {
        let theme = self.get_mut(id)?;
        for key in keys {
            theme.overrides.remove(key.key());
        }
        theme.updated_at = now_ms;
        Ok(())
    }

    /// Every colour and the typography back to the base's.
    pub fn reset_all(&mut self, id: ThemeId, now_ms: i64) -> Result<(), ThemeError> {
        let theme = self.get_mut(id)?;
        theme.overrides.clear();
        theme.typography = TypographyOverride::default();
        theme.updated_at = now_ms;
        Ok(())
    }

    pub fn set_typography(
        &mut self,
        id: ThemeId,
        typography: TypographyOverride,
        now_ms: i64,
    ) -> Result<(), ThemeError> {
        let theme = self.get_mut(id)?;
        theme.typography = typography.sanitized();
        theme.updated_at = now_ms;
        Ok(())
    }

    /// Puts a theme back as it was (an undo of a session of edits), keeping its place and name
    /// unless the name is now taken.
    pub fn restore(&mut self, previous: EditorTheme) -> Result<(), ThemeError> {
        let theme = self.get_mut(previous.id)?;
        *theme = previous;
        Ok(())
    }
}

/// An id for a new theme: wall-clock based like every document id, and always above
/// [`FIRST_USER_ID`].
pub fn new_user_id() -> ThemeId {
    watchlist::new_id().max(FIRST_USER_ID)
}

#[cfg(test)]
mod tests;

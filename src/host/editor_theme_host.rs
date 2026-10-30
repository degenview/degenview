//! The editor's themes and typography, shared by every window and by Settings.
//!
//! The host owns the custom themes ([`ThemeBook`]) and the choices ([`EditorPrefs`]) in memory.
//! Every change takes effect at once, in this process: the appearance and font the editors read
//! ([`editor_appearance::publish`]) are replaced and every window is asked to repaint. Saving is
//! separate and unhurried: a change marks the host dirty and [`EditorThemeHost::tick`] writes it a
//! moment after the last one, so dragging a colour writes the database once, not every frame. The
//! app flushes on exit. Nothing here touches the store while a window is drawing, so Settings can
//! call it while it holds the store.
//!
//! Fonts: the font a person picked is found among the installed fonts ([`FontCatalog`]), read on a
//! thread and handed to egui when it arrives. Until then, and when the font is not installed on
//! this Mac, the editor keeps the system's monospace font and the saved name is kept, so the font
//! comes back when it does.

use crate::model::editor_theme::codec;
use crate::model::editor_theme::{
    BuiltInTheme, ColorKey, EditorPrefs, EditorTheme, Rgba, ThemeBook, ThemeError, ThemeId,
    ThemeSelection, ThemeSource, Typography, TypographyOverride, new_user_id,
};
use crate::model::watchlist::now_ms;
use crate::service::font_catalog::{FontCatalog, read_font};
use crate::store::Store;
use crate::store::editor_theme::Loaded;
use crate::ui::editor_appearance::{self, Appearance, EditorFont};
use crate::ui::theme;
use eframe::egui;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long after the last change the database is written.
pub const SAVE_DELAY: Duration = Duration::from_millis(400);

/// What the editor's font is doing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FontStatus {
    /// The system's monospace font, because none is chosen.
    System,
    /// Waiting for the list of installed fonts, or for the file to be read.
    Loading(String),
    Using(String),
    /// The chosen font is not installed on this Mac. The name is kept.
    Missing(String),
    Failed(String, String),
}

type FontReply = (String, Result<Arc<egui::FontData>, String>);

struct Inner {
    ctx: Option<egui::Context>,
    book: ThemeBook,
    prefs: EditorPrefs,
    /// Why changes to themes are not saved, when a stored row could not be read.
    themes_unwritable: Option<String>,
    themes_dirty: bool,
    prefs_dirty: bool,
    changed_at: Option<Instant>,
    catalog: FontCatalog,
    font: FontStatus,
    /// The family the app's fonts were last built with (`None`: the system font).
    installed: Option<String>,
    /// The family being read from disk.
    pending: Option<String>,
    replies: Receiver<FontReply>,
    reply_to: Sender<FontReply>,
    families: Option<Arc<Vec<String>>>,
    /// What the last save, or the last refused change, said.
    notice: Option<String>,
}

/// What Settings draws.
#[derive(Clone, Debug, PartialEq)]
pub struct View {
    pub selected: ThemeSelection,
    pub customs: Vec<CustomTheme>,
    /// What a person set for the editor.
    pub typography: Typography,
    /// What the editor uses: the selected theme's own typography over [`Self::typography`].
    pub effective: Typography,
    pub font: FontStatus,
    pub fonts_ready: bool,
    /// The installed monospaced families, sorted.
    pub fonts: Arc<Vec<String>>,
    pub unwritable: Option<String>,
    pub notice: Option<String>,
}

/// A custom theme as a tile and an editor need it.
#[derive(Clone, Debug, PartialEq)]
pub struct CustomTheme {
    pub theme: EditorTheme,
    pub base: BuiltInTheme,
}

/// The result of importing a file.
#[derive(Clone, Debug, PartialEq)]
pub struct Imported {
    pub id: ThemeId,
    pub name: String,
    pub skipped: Vec<String>,
}

/// A name safe to give a file: letters, digits, spaces, dashes and underscores of any script, the
/// rest as dashes.
pub fn file_stem(name: &str) -> String {
    let stem: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let stem = stem.trim().trim_matches('-').trim();
    if stem.is_empty() {
        "theme".to_owned()
    } else {
        stem.to_owned()
    }
}

fn effective(book: &ThemeBook, prefs: &EditorPrefs) -> Typography {
    match prefs.selected {
        ThemeSelection::Custom(id) => book.get(id).map_or_else(
            || prefs.typography.clone(),
            |theme| prefs.typography.with(&theme.typography),
        ),
        _ => prefs.typography.clone(),
    }
}

impl Inner {
    /// A selection that names a theme that is gone is automatic: the editor never wears nothing.
    fn normalize(&mut self) {
        if let ThemeSelection::Custom(id) = self.prefs.selected
            && self.book.get(id).is_none()
        {
            self.prefs.selected = ThemeSelection::Auto;
            self.prefs_dirty = true;
        }
    }

    fn appearance(&self) -> Option<Arc<Appearance>> {
        match self.prefs.selected {
            ThemeSelection::Auto => None,
            ThemeSelection::BuiltIn(theme) => Some(editor_appearance::of_built_in(theme)),
            ThemeSelection::Custom(id) => self
                .book
                .get(id)
                .map(|theme| Arc::new(Appearance::from_palette(&theme.palette()))),
        }
    }

    /// Hands the editors what they should draw with, and asks every window to repaint.
    fn publish(&mut self) {
        self.normalize();
        let typography = effective(&self.book, &self.prefs);
        editor_appearance::publish(
            self.appearance(),
            EditorFont {
                custom: self.installed.is_some(),
                size: typography.font_size,
                line_height: typography.line_height,
            },
        );
        if let Some(ctx) = &self.ctx {
            ctx.request_repaint();
        }
    }

    fn changed(&mut self, prefs: bool, themes: bool) {
        self.prefs_dirty |= prefs;
        self.themes_dirty |= themes;
        self.changed_at = Some(Instant::now());
        self.sync_font();
        self.publish();
    }

    fn uninstall_font(&mut self) {
        if self.installed.take().is_some()
            && let Some(ctx) = &self.ctx
        {
            theme::install_editor_font(ctx, None);
        }
    }

    /// Makes the app's fonts follow the family the editor should use, once the installed fonts
    /// are known, unless that family is already loaded, known to be missing, or being read.
    fn sync_font(&mut self) {
        let Some(family) = effective(&self.book, &self.prefs).font_family else {
            self.uninstall_font();
            self.pending = None;
            self.font = FontStatus::System;
            return;
        };
        let is = |name: &String| name.eq_ignore_ascii_case(&family);
        if matches!(&self.font,
            FontStatus::Using(name) | FontStatus::Missing(name) | FontStatus::Failed(name, _)
            if is(name))
            || self.pending.as_ref().is_some_and(is)
        {
            return;
        }
        if !self.catalog.is_ready() {
            self.font = FontStatus::Loading(family);
            return;
        }
        let Some(entry) = self.catalog.find(&family) else {
            self.uninstall_font();
            self.font = FontStatus::Missing(family);
            return;
        };
        self.font = FontStatus::Loading(family.clone());
        self.pending = Some(family.clone());
        let reply_to = self.reply_to.clone();
        let ctx = self.ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("font-read".into())
            .spawn(move || {
                let result = read_font(&entry)
                    .map(|font| {
                        let mut data = egui::FontData::from_owned(font.bytes);
                        data.index = font.index;
                        Arc::new(data)
                    })
                    .map_err(|error| error.to_string());
                let _ = reply_to.send((family, result));
                if let Some(ctx) = ctx {
                    ctx.request_repaint();
                }
            });
        if let Err(error) = spawned {
            self.pending = None;
            self.font = FontStatus::Failed(
                effective(&self.book, &self.prefs)
                    .font_family
                    .unwrap_or_default(),
                error.to_string(),
            );
        }
    }

    /// Takes the fonts that have been read since the last look. Returns whether anything changed.
    fn poll_fonts(&mut self) -> bool {
        let mut changed = false;
        while let Ok((family, result)) = self.replies.try_recv() {
            if !self
                .pending
                .as_ref()
                .is_some_and(|p| p.eq_ignore_ascii_case(&family))
            {
                continue; // a read for a font nobody wants any more
            }
            self.pending = None;
            let wanted = effective(&self.book, &self.prefs).font_family;
            if !wanted
                .as_ref()
                .is_some_and(|w| w.eq_ignore_ascii_case(&family))
            {
                self.sync_font();
                changed = true;
                continue;
            }
            match result {
                Ok(data) => {
                    if let Some(ctx) = &self.ctx {
                        theme::install_editor_font(ctx, Some(data));
                    }
                    self.installed = Some(family.clone());
                    self.font = FontStatus::Using(family);
                }
                Err(error) => {
                    self.uninstall_font();
                    self.font = FontStatus::Failed(family, error);
                }
            }
            changed = true;
        }
        // The list of fonts arrived while a saved font was waiting for it.
        if matches!(self.font, FontStatus::Loading(_))
            && self.pending.is_none()
            && self.catalog.is_ready()
        {
            self.sync_font();
            changed = true;
        }
        changed
    }

    fn writable(&self) -> Result<(), String> {
        match &self.themes_unwritable {
            Some(why) => Err(why.clone()),
            None => Ok(()),
        }
    }

    fn flush(&mut self, store: &Store) {
        if self.themes_dirty && self.themes_unwritable.is_none() {
            match store.replace_editor_themes(self.book.saved()) {
                Ok(()) => self.themes_dirty = false,
                Err(error) => self.notice = Some(format!("Could not save your themes: {error}")),
            }
        }
        if self.prefs_dirty {
            match store.set_editor_prefs(&self.prefs) {
                Ok(()) => self.prefs_dirty = false,
                Err(error) => {
                    self.notice = Some(format!("Could not save the editor settings: {error}"))
                }
            }
        }
        if !self.themes_dirty && !self.prefs_dirty {
            self.changed_at = None;
            self.notice = None;
        }
    }
}

#[derive(Clone)]
pub struct EditorThemeHost {
    inner: Arc<Mutex<Inner>>,
}

impl Default for EditorThemeHost {
    /// A host with no custom themes and no installed fonts: what a test or an empty database has.
    fn default() -> Self {
        Self::new(
            Loaded {
                book: ThemeBook::default(),
                prefs: EditorPrefs::default(),
                themes_unwritable: None,
            },
            FontCatalog::default(),
        )
    }
}

impl EditorThemeHost {
    pub fn new(loaded: Loaded, catalog: FontCatalog) -> Self {
        let (reply_to, replies) = channel();
        let mut inner = Inner {
            ctx: None,
            book: loaded.book,
            prefs: loaded.prefs,
            themes_unwritable: loaded.themes_unwritable,
            themes_dirty: false,
            prefs_dirty: false,
            changed_at: None,
            catalog,
            font: FontStatus::System,
            installed: None,
            pending: None,
            replies,
            reply_to,
            families: None,
            notice: None,
        };
        inner.normalize();
        inner.sync_font();
        inner.publish();
        Self {
            inner: Arc::new(Mutex::new(inner)),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().expect("editor theme lock poisoned")
    }

    /// Tells the host which context to repaint and to install fonts into, and picks up a font the
    /// saved choices name.
    pub fn attach(&self, ctx: &egui::Context) {
        let mut inner = self.lock();
        inner.ctx = Some(ctx.clone());
        inner.sync_font();
        inner.publish();
    }

    /// Once a frame: picks up fonts that have been read, and saves what changed a moment ago.
    pub fn tick(&self, store: &Store, now: Instant) {
        let mut inner = self.lock();
        if inner.poll_fonts() {
            inner.publish();
        }
        if inner
            .changed_at
            .is_some_and(|at| now.duration_since(at) >= SAVE_DELAY)
        {
            inner.flush(store);
        }
    }

    /// Saves anything not saved yet. The app calls it as it quits.
    pub fn flush(&self, store: &Store) {
        self.lock().flush(store);
    }

    pub fn view(&self) -> View {
        let mut inner = self.lock();
        let ready = inner.catalog.is_ready();
        if ready && inner.families.is_none() {
            let names = inner
                .catalog
                .entries()
                .into_iter()
                .map(|e| e.family)
                .collect();
            inner.families = Some(Arc::new(names));
        }
        View {
            selected: inner.prefs.selected,
            customs: inner
                .book
                .saved()
                .iter()
                .map(|theme| CustomTheme {
                    base: theme.base_theme(),
                    theme: theme.clone(),
                })
                .collect(),
            typography: inner.prefs.typography.clone(),
            effective: effective(&inner.book, &inner.prefs),
            font: inner.font.clone(),
            fonts_ready: ready,
            fonts: inner.families.clone().unwrap_or_default(),
            unwritable: inner.themes_unwritable.clone(),
            notice: inner.notice.clone(),
        }
    }

    /// Why `name` cannot be used for a theme (a new one, or `renaming`'s), if it cannot.
    pub fn name_problem(&self, name: &str, renaming: Option<ThemeId>) -> Option<String> {
        self.lock()
            .book
            .check_name(name, renaming)
            .err()
            .map(ThemeError::message)
    }

    /// The custom theme `id`, as it is now.
    pub fn theme(&self, id: ThemeId) -> Option<EditorTheme> {
        self.lock().book.get(id).cloned()
    }

    pub fn select(&self, selection: ThemeSelection) {
        let mut inner = self.lock();
        inner.prefs.selected = selection;
        inner.changed(true, false);
    }

    pub fn set_typography(&self, typography: Typography) {
        let mut inner = self.lock();
        inner.prefs.typography = typography.sanitized();
        inner.changed(true, false);
    }

    fn edit<R>(
        &self,
        change: impl FnOnce(&mut Inner) -> Result<R, ThemeError>,
    ) -> Result<R, String> {
        let mut inner = self.lock();
        inner.writable()?;
        let result = change(&mut inner).map_err(ThemeError::message)?;
        inner.changed(false, true);
        Ok(result)
    }

    /// Makes a theme from `source` and wears it.
    pub fn create(&self, name: &str, source: ThemeSource) -> Result<ThemeId, String> {
        self.edit(|inner| {
            let id = inner.book.create(new_user_id(), name, source, now_ms())?;
            inner.prefs.selected = ThemeSelection::Custom(id);
            inner.prefs_dirty = true;
            Ok(id)
        })
    }

    /// A copy of `source`, to customise. It is worn.
    pub fn duplicate(&self, source: ThemeSource) -> Result<ThemeId, String> {
        self.edit(|inner| {
            let id = inner.book.duplicate(new_user_id(), source, now_ms())?;
            inner.prefs.selected = ThemeSelection::Custom(id);
            inner.prefs_dirty = true;
            Ok(id)
        })
    }

    pub fn rename(&self, id: ThemeId, name: &str) -> Result<(), String> {
        self.edit(|inner| inner.book.rename(id, name, now_ms()))
    }

    /// Deletes a theme. If it was worn, the editor goes back to following the app.
    pub fn delete(&self, id: ThemeId) -> Result<(), String> {
        self.edit(|inner| {
            inner.book.delete(id)?;
            if inner.prefs.selected == ThemeSelection::Custom(id) {
                inner.prefs.selected = ThemeSelection::Auto;
                inner.prefs_dirty = true;
            }
            Ok(())
        })
    }

    pub fn set_color(&self, id: ThemeId, key: ColorKey, color: Rgba) -> Result<(), String> {
        self.edit(|inner| inner.book.set_color(id, key, color, now_ms()))
    }

    pub fn reset_colors(&self, id: ThemeId, keys: &[ColorKey]) -> Result<(), String> {
        self.edit(|inner| inner.book.reset_colors(id, keys, now_ms()))
    }

    pub fn reset_all(&self, id: ThemeId) -> Result<(), String> {
        self.edit(|inner| inner.book.reset_all(id, now_ms()))
    }

    pub fn set_theme_typography(
        &self,
        id: ThemeId,
        typography: TypographyOverride,
    ) -> Result<(), String> {
        self.edit(|inner| inner.book.set_typography(id, typography, now_ms()))
    }

    /// Puts a theme back as it was, for "Reset Changes".
    pub fn restore(&self, previous: EditorTheme) -> Result<(), String> {
        self.edit(|inner| inner.book.restore(previous))
    }

    /// The text of a theme file for a built-in or custom theme, and a name for the file.
    pub fn export(&self, source: ThemeSource) -> Result<(String, String), String> {
        let inner = self.lock();
        let theme = match source {
            ThemeSource::BuiltIn(built_in) => EditorTheme {
                id: 0,
                name: built_in.name().to_owned(),
                base: built_in.id().to_owned(),
                overrides: Default::default(),
                typography: TypographyOverride::default(),
                version: 1,
                created_at: 0,
                updated_at: 0,
                extra: Default::default(),
            },
            ThemeSource::Custom(id) => inner
                .book
                .get(id)
                .cloned()
                .ok_or_else(|| ThemeError::Missing.message())?,
        };
        Ok((file_stem(&theme.name), codec::export(&theme)))
    }

    /// Reads a theme file and wears the theme it makes. A file never replaces a theme: its name
    /// is made free and it gets an id of its own.
    pub fn import(&self, text: &str) -> Result<Imported, String> {
        let read = codec::import(text).map_err(|error| error.message())?;
        self.edit(|inner| {
            let id = inner.book.import(new_user_id(), read.theme, now_ms())?;
            inner.prefs.selected = ThemeSelection::Custom(id);
            inner.prefs_dirty = true;
            let name = inner
                .book
                .get(id)
                .map(|t| t.name.clone())
                .unwrap_or_default();
            Ok(Imported {
                id,
                name,
                skipped: read.skipped,
            })
        })
    }
}

#[cfg(test)]
mod tests;

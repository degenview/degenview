//! The persistence authority for user-authored scripts, ported from `ScriptStore.swift`.
//!
//! Each script is a plain `<name>.pine` file in `scripts_dir`, so it can be opened, edited or
//! backed up with any tool. Everything a `.pine` file can't carry — the stable id charts
//! reference, favorite flag, revisions, draft, compile cache — lives beside it in
//! `meta_root/<id>/`. A file finds its id through an extended attribute (which survives Finder
//! renames and moves) and falls back to `index.json` (id → file name) for editors whose atomic
//! saves drop extended attributes.

use super::model::{
    CompileStatus, LocalScript, ScriptCompileRecord, ScriptDraft, ScriptId, ScriptType,
    ScriptVersion, now_ms,
};
use super::name::{self, FILE_EXTENSION, ScriptNameError};
use super::registry::LibraryRegistry;
use crate::pine::compiler;
use crate::pine::token::Limits;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub const COMPILER_VERSION: &str = "pine-local-3";
pub const ID_ATTRIBUTE: &str = "es.nico.degenview.script-id";
const MAX_REVISIONS: usize = 100;

#[derive(Debug)]
pub enum ScriptStoreError {
    MissingScript,
    NameConflict,
    UnreadableFile,
    /// The file was edited outside the app since the app last read it, so saving would overwrite
    /// that edit.
    ChangedOnDisk,
    Name(ScriptNameError),
    Io(String),
}

impl fmt::Display for ScriptStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingScript => f.write_str("The script no longer exists."),
            Self::NameConflict => f.write_str("A script with that name already exists."),
            Self::UnreadableFile => f.write_str("The file isn't a readable text file."),
            Self::ChangedOnDisk => f.write_str("The file changed outside the app."),
            Self::Name(error) => error.fmt(f),
            Self::Io(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for ScriptStoreError {}

impl From<ScriptNameError> for ScriptStoreError {
    fn from(error: ScriptNameError) -> Self {
        Self::Name(error)
    }
}

impl From<std::io::Error> for ScriptStoreError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

type Result<T> = std::result::Result<T, ScriptStoreError>;

/// Everything about a script except its name and source, which the `.pine` file owns.
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Record {
    id: ScriptId,
    #[serde(rename = "type")]
    script_type: ScriptType,
    latest_revision_id: Option<String>,
    created_at_ms: i64,
    modified_at_ms: i64,
    is_favorite: bool,
    compile_record: Option<ScriptCompileRecord>,
}

impl Record {
    fn of(script: &LocalScript) -> Self {
        Self {
            id: script.id.clone(),
            script_type: script.script_type,
            latest_revision_id: script.latest_revision_id.clone(),
            created_at_ms: script.created_at_ms,
            modified_at_ms: script.modified_at_ms,
            is_favorite: script.is_favorite,
            compile_record: script.compile_record.clone(),
        }
    }

    fn script(self, name: String, source: String) -> LocalScript {
        LocalScript {
            id: self.id,
            name,
            script_type: self.script_type,
            source,
            latest_revision_id: self.latest_revision_id,
            created_at_ms: self.created_at_ms,
            modified_at_ms: self.modified_at_ms,
            is_favorite: self.is_favorite,
            compile_record: self.compile_record,
        }
    }
}

/// What tells that a file was rewritten without reading it: when, and how long.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileStamp {
    modified_ms: i64,
    len: u64,
}

#[cfg(test)]
impl FileStamp {
    pub fn for_test(modified_ms: i64, len: u64) -> Self {
        Self { modified_ms, len }
    }
}

pub struct ScriptStore {
    scripts_dir: PathBuf,
    meta_root: PathBuf,
    metadata: HashMap<ScriptId, LocalScript>,
    /// Script id → file name inside `scripts_dir`; persisted as `index.json`.
    file_names: HashMap<ScriptId, String>,
    loaded: bool,
}

impl ScriptStore {
    pub fn new(scripts_dir: PathBuf, meta_root: PathBuf) -> Self {
        Self {
            scripts_dir,
            meta_root,
            metadata: HashMap::new(),
            file_names: HashMap::new(),
            loaded: false,
        }
    }

    /// The store under the app's data directory.
    pub fn default_location() -> Option<Self> {
        let base = crate::store::data_dir()?;
        Some(Self::new(base.join("Scripts"), base.join("ScriptMetadata")))
    }

    pub fn scripts_dir(&self) -> &Path {
        &self.scripts_dir
    }

    /// Always rescans the folder, so files added, edited or renamed outside the app show up.
    /// Newest first.
    pub fn all_scripts(&mut self) -> Result<Vec<LocalScript>> {
        self.rescan()?;
        let mut scripts: Vec<LocalScript> = self.metadata.values().cloned().collect();
        scripts.sort_by(|a, b| {
            b.modified_at_ms
                .cmp(&a.modified_at_ms)
                .then_with(|| a.name.cmp(&b.name))
        });
        Ok(scripts)
    }

    pub fn script(&mut self, id: &str) -> Result<Option<LocalScript>> {
        self.rescan()?;
        Ok(self.metadata.get(id).cloned())
    }

    /// Creates a script. With `disambiguating` a taken name becomes "name 2"; without it a taken
    /// name is a `NameConflict`.
    pub fn create(
        &mut self,
        requested: &str,
        script_type: ScriptType,
        source: Option<&str>,
        disambiguating: bool,
    ) -> Result<LocalScript> {
        self.load_if_needed()?;
        let base = name::validate(requested)?;
        if !disambiguating && self.is_name_taken(&base, None) {
            return Err(ScriptStoreError::NameConflict);
        }
        let name = if disambiguating {
            disambiguated(&base, &self.taken_names(None))
        } else {
            base
        };
        let now = now_ms();
        let id = new_id();
        let script = LocalScript {
            id: id.clone(),
            source: source
                .map(str::to_owned)
                .unwrap_or_else(|| template(script_type, &name)),
            name,
            script_type,
            latest_revision_id: None,
            created_at_ms: now,
            modified_at_ms: now,
            is_favorite: false,
            compile_record: None,
        };
        self.write_file(&id, &script.name, &script.source)?;
        self.write_record(&Record::of(&script))?;
        self.metadata.insert(id, script.clone());
        self.save_index()?;
        self.publish_libraries();
        Ok(script)
    }

    /// Adds a script that already has an identity because it came from another Mac. A taken
    /// name becomes "name 2". The source is saved through the normal path, so it is compiled and
    /// gets a revision like any other save.
    pub fn adopt(
        &mut self,
        id: &str,
        requested: &str,
        source: &str,
        favorite: bool,
    ) -> Result<LocalScript> {
        self.load_if_needed()?;
        if self.metadata.contains_key(id) {
            return Err(ScriptStoreError::NameConflict);
        }
        let base = name::validate(requested)?;
        let name = disambiguated(&base, &self.taken_names(None));
        let now = now_ms();
        let script = LocalScript {
            id: id.to_owned(),
            name,
            script_type: declared_type(source),
            source: with_trailing_newline(source),
            latest_revision_id: None,
            created_at_ms: now,
            modified_at_ms: now,
            is_favorite: favorite,
            compile_record: None,
        };
        self.write_file(id, &script.name, &script.source)?;
        self.write_record(&Record::of(&script))?;
        self.metadata.insert(id.to_owned(), script);
        self.save_index()?;
        self.save(id, source)
    }

    /// Brings an existing script in line with another Mac's version: name, source, favorite.
    /// A name that is taken here by a different script becomes "name 2" rather than failing.
    pub fn update_from_remote(
        &mut self,
        id: &str,
        requested: &str,
        source: &str,
        favorite: bool,
    ) -> Result<()> {
        self.load_if_needed()?;
        let current = self
            .metadata
            .get(id)
            .cloned()
            .ok_or(ScriptStoreError::MissingScript)?;
        let clean = name::validate(requested)?;
        let target = if self.is_name_taken(&clean, Some(id)) {
            disambiguated(&clean, &self.taken_names(Some(id)))
        } else {
            clean
        };
        if target != current.name {
            self.rename(id, &target)?;
        }
        if with_trailing_newline(source) != current.source {
            self.save(id, source)?;
        }
        if favorite != current.is_favorite {
            self.set_favorite(id, favorite)?;
        }
        Ok(())
    }

    /// A fresh script id, for a conflict copy made while applying another Mac's version.
    pub fn fresh_id() -> ScriptId {
        new_id()
    }

    /// Saves a copy as "<name> copy" (disambiguated on conflict): a plain script, not a
    /// favorite, no history.
    pub fn duplicate(&mut self, id: &str) -> Result<LocalScript> {
        let original = self.script(id)?.ok_or(ScriptStoreError::MissingScript)?;
        self.create(
            &format!("{} copy", original.name),
            original.script_type,
            Some(&original.source),
            true,
        )
    }

    /// Copies an outside `.pine` file into the library. The copy is named after the file
    /// (disambiguated on conflict) and typed by the script's own declaration.
    pub fn import_file(&mut self, path: &Path) -> Result<LocalScript> {
        let source = read_source(path).ok_or(ScriptStoreError::UnreadableFile)?;
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let script_type = declared_type(&source);
        self.create(&stem, script_type, Some(&source), true)
    }

    /// Copies every `.pine` file in `folder` into the library (names disambiguated), returning
    /// how many arrived. Used once to bring the Swift app's scripts across; the originals stay.
    pub fn import_folder(&mut self, folder: &Path) -> Result<usize> {
        let mut files: Vec<PathBuf> = fs::read_dir(folder)?
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|x| x.eq_ignore_ascii_case(FILE_EXTENSION))
                    && !file_name_of(path).is_some_and(|n| n.starts_with('.'))
            })
            .collect();
        files.sort();
        let mut imported = 0;
        for file in files {
            if self.import_file(&file).is_ok() {
                imported += 1;
            }
        }
        Ok(imported)
    }

    /// The id of the library script stored at `path`, or `None` when it is outside the library.
    pub fn script_id_for_file(&mut self, path: &Path) -> Result<Option<ScriptId>> {
        let canonical = |p: &Path| fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
        let Some(folder) = path.parent() else {
            return Ok(None);
        };
        if canonical(folder) != canonical(&self.scripts_dir) {
            return Ok(None);
        }
        self.rescan()?;
        let file = path.file_name().map(|n| n.to_string_lossy().into_owned());
        Ok(self
            .file_names
            .iter()
            .find(|(_, name)| Some(*name) == file.as_ref())
            .map(|(id, _)| id.clone()))
    }

    /// Where the script's `.pine` file lives inside the library.
    pub fn file_path(&mut self, id: &str) -> Result<Option<PathBuf>> {
        self.load_if_needed()?;
        Ok(self.file_names.get(id).map(|n| self.scripts_dir.join(n)))
    }

    /// When the script's file was last written and how long it is, from its metadata alone: cheap
    /// enough to ask several times a second. `None` if the file is not there.
    pub fn file_stamp(&mut self, id: &str) -> Option<FileStamp> {
        let path = self.file_path(id).ok()??;
        let metadata = fs::metadata(path).ok()?;
        Some(FileStamp {
            modified_ms: system_time_ms(metadata.modified().ok()?),
            len: metadata.len(),
        })
    }

    /// `save`, unless the file is no longer what the caller last read (`expected_disk`): then
    /// nothing is written and the answer is `ChangedOnDisk`. An edit made outside the app is never
    /// silently overwritten by an editor that has not seen it. A file that is gone is saved anew.
    pub fn save_if_unchanged(
        &mut self,
        id: &str,
        source: &str,
        expected_disk: &str,
    ) -> Result<LocalScript> {
        self.load_if_needed()?;
        if let Some(path) = self.file_path(id)?
            && let Some(on_disk) = read_source(&path)
            && with_trailing_newline(&on_disk) != with_trailing_newline(expected_disk)
        {
            return Err(ScriptStoreError::ChangedOnDisk);
        }
        self.save(id, source)
    }

    /// Saves new source under the script's current name. The type follows the script's own
    /// declaration; while the source has no single declaration (mid-edit) it keeps the old one.
    pub fn save(&mut self, id: &str, source: &str) -> Result<LocalScript> {
        self.load_if_needed()?;
        let source = with_trailing_newline(source);
        let source = source.as_str();
        let mut script = self
            .metadata
            .get(id)
            .cloned()
            .ok_or(ScriptStoreError::MissingScript)?;
        // Invalid source saves too; its errors are recorded in the compile record.
        let compiled = compiler::compile(source, Limits::default(), &LibraryRegistry);
        let status = CompileStatus::of(&compiled.diagnostics);
        let now = now_ms();
        if source != script.source || script.latest_revision_id.is_none() {
            let revision = ScriptVersion {
                id: new_id(),
                script_id: id.to_owned(),
                created_at_ms: now,
                source: source.to_owned(),
                compile_status: status,
            };
            self.write_revision(&revision)?;
            script.latest_revision_id = Some(revision.id);
        }
        // PINE3001: a missing or duplicated declaration, so `script_type` is only a default.
        if !compiled.diagnostics.iter().any(|d| d.code == "PINE3001") {
            script.script_type = compiled.declaration.script_type.into();
        }
        script.source = source.to_owned();
        script.modified_at_ms = now;
        script.compile_record = Some(ScriptCompileRecord {
            source_hash: hash_source(source),
            compiler_version: COMPILER_VERSION.to_owned(),
            pine_version: compiled.declaration.pine_version,
            status,
            diagnostics: compiled.diagnostics,
            compiled_at_ms: now,
        });
        self.write_file(id, &script.name, source)?;
        self.write_record(&Record::of(&script))?;
        let _ = fs::remove_file(self.dir(id).join("draft.json"));
        self.metadata.insert(id.to_owned(), script.clone());
        self.save_index()?;
        self.prune_revisions(id, MAX_REVISIONS);
        self.publish_libraries();
        Ok(script)
    }

    /// Renames the script's file and nothing else: source, revisions and draft are untouched.
    pub fn rename(&mut self, id: &str, new_name: &str) -> Result<LocalScript> {
        self.load_if_needed()?;
        let mut script = self
            .metadata
            .get(id)
            .cloned()
            .ok_or(ScriptStoreError::MissingScript)?;
        let clean = name::validate(new_name)?;
        if clean == script.name {
            return Ok(script);
        }
        if self.is_name_taken(&clean, Some(id)) {
            return Err(ScriptStoreError::NameConflict);
        }
        self.rename_file(id, &clean)?;
        script.name = clean;
        script.modified_at_ms = now_ms();
        self.write_record(&Record::of(&script))?;
        self.metadata.insert(id.to_owned(), script.clone());
        self.save_index()?;
        self.publish_libraries();
        Ok(script)
    }

    pub fn set_favorite(&mut self, id: &str, favorite: bool) -> Result<()> {
        self.load_if_needed()?;
        let script = self
            .metadata
            .get_mut(id)
            .ok_or(ScriptStoreError::MissingScript)?;
        script.is_favorite = favorite;
        let record = Record::of(script);
        self.write_record(&record)
    }

    pub fn save_draft(&mut self, draft: &ScriptDraft) -> Result<()> {
        self.load_if_needed()?;
        if !self.metadata.contains_key(&draft.script_id) {
            return Err(ScriptStoreError::MissingScript);
        }
        let dir = self.dir(&draft.script_id);
        fs::create_dir_all(&dir)?;
        atomic_write_json(draft, &dir.join("draft.json"))
    }

    pub fn draft(&mut self, id: &str) -> Result<Option<ScriptDraft>> {
        self.load_if_needed()?;
        Ok(read_json(&self.dir(id).join("draft.json")))
    }

    /// Forgets the unsaved text kept beside a script, so it reopens as saved.
    pub fn discard_draft(&mut self, id: &str) -> Result<()> {
        self.load_if_needed()?;
        match fs::remove_file(self.dir(id).join("draft.json")) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.into()),
            _ => Ok(()),
        }
    }

    /// Newest first.
    pub fn revisions(&mut self, id: &str) -> Result<Vec<ScriptVersion>> {
        self.load_if_needed()?;
        let mut versions: Vec<ScriptVersion> = fs::read_dir(self.dir(id).join("Revisions"))
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .filter_map(|e| read_json(&e.path()))
            .collect();
        versions.sort_by_key(|v| std::cmp::Reverse(v.created_at_ms));
        Ok(versions)
    }

    /// Moves the script's file to the Trash (so it can be recovered) and drops its metadata.
    pub fn delete(&mut self, id: &str) -> Result<()> {
        self.load_if_needed()?;
        if !self.metadata.contains_key(id) {
            return Ok(());
        }
        if let Some(file_name) = self.file_names.get(id) {
            let file = self.scripts_dir.join(file_name);
            if file.exists() && trash(&file).is_err() {
                fs::remove_file(&file)?;
            }
        }
        let _ = fs::remove_dir_all(self.dir(id));
        self.metadata.remove(id);
        self.file_names.remove(id);
        self.save_index()?;
        self.publish_libraries();
        Ok(())
    }

    /// Writes a copy of the script to `destination`. The id attribute does not travel with it.
    pub fn export(&mut self, id: &str, destination: &Path) -> Result<()> {
        let script = self.script(id)?.ok_or(ScriptStoreError::MissingScript)?;
        atomic_write(destination, script.source.as_bytes())?;
        Ok(())
    }

    // MARK: Scanning

    fn load_if_needed(&mut self) -> Result<()> {
        if self.loaded { Ok(()) } else { self.rescan() }
    }

    /// Rebuilds the in-memory catalog from the `.pine` files on disk.
    fn rescan(&mut self) -> Result<()> {
        fs::create_dir_all(&self.scripts_dir)?;
        fs::create_dir_all(&self.meta_root)?;
        if !self.loaded {
            self.file_names = read_json(&self.index_path()).unwrap_or_default();
        }

        let mut files: Vec<(PathBuf, Option<ScriptId>)> = fs::read_dir(&self.scripts_dir)?
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.extension()
                    .is_some_and(|x| x.eq_ignore_ascii_case(FILE_EXTENSION))
                    && !p
                        .file_name()
                        .is_some_and(|n| n.to_string_lossy().starts_with('.'))
            })
            .map(|p| {
                let id = read_id(&p);
                (p, id)
            })
            .collect();
        // A Finder duplicate copies the extended attribute too. The file the index already
        // knows under that id keeps it; any other claimant gets a new id.
        let known = |entry: &(PathBuf, Option<ScriptId>)| {
            entry.1.as_ref().is_some_and(|id| {
                self.file_names.get(id).map(String::as_str) == file_name_of(&entry.0).as_deref()
            })
        };
        files.sort_by(|a, b| {
            known(b)
                .cmp(&known(a))
                .then_with(|| file_name_of(&a.0).cmp(&file_name_of(&b.0)))
        });
        let ids_by_name: HashMap<String, ScriptId> = self
            .file_names
            .iter()
            .map(|(id, name)| (name.clone(), id.clone()))
            .collect();

        let mut scripts: HashMap<ScriptId, LocalScript> = HashMap::new();
        let mut names: HashMap<ScriptId, String> = HashMap::new();
        for (path, attribute_id) in files {
            let Some(source) = read_source(&path) else {
                continue;
            };
            let Some(file_name) = file_name_of(&path) else {
                continue;
            };
            let mut id = attribute_id.clone();
            if id.as_ref().is_none_or(|i| scripts.contains_key(i)) {
                id = ids_by_name.get(&file_name).cloned();
            }
            if id.as_ref().is_none_or(|i| scripts.contains_key(i)) {
                id = Some(new_id());
            }
            let script_id = id.unwrap_or_else(new_id);
            if attribute_id.as_deref() != Some(script_id.as_str()) {
                write_id(&script_id, &path);
            }

            let file_modified = fs::metadata(&path)
                .and_then(|m| m.modified())
                .map(system_time_ms)
                .unwrap_or_else(|_| now_ms());
            let mut record: Record = match read_json(&self.record_path(&script_id)) {
                Some(existing) => existing,
                None => {
                    let created = fs::metadata(&path)
                        .and_then(|m| m.created())
                        .map(system_time_ms)
                        .unwrap_or_else(|_| now_ms());
                    let fresh = Record {
                        id: script_id.clone(),
                        script_type: declared_type(&source),
                        latest_revision_id: None,
                        created_at_ms: created,
                        modified_at_ms: file_modified,
                        is_favorite: false,
                        compile_record: None,
                    };
                    let _ = self.write_record(&fresh);
                    fresh
                }
            };
            // Edited outside the app: the cached diagnostics describe another source.
            if record
                .compile_record
                .as_ref()
                .is_some_and(|c| c.source_hash != hash_source(&source))
            {
                record.compile_record = None;
            }
            record.modified_at_ms = record.modified_at_ms.max(file_modified);
            let display = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            scripts.insert(script_id.clone(), record.script(display, source));
            names.insert(script_id, file_name);
        }

        self.metadata = scripts;
        let index_changed = names != self.file_names;
        self.file_names = names;
        self.loaded = true;
        if index_changed {
            self.save_index()?;
        }
        self.publish_libraries();
        Ok(())
    }

    fn publish_libraries(&self) {
        let scripts: Vec<LocalScript> = self.metadata.values().cloned().collect();
        LibraryRegistry::publish(&scripts);
    }

    // MARK: Files

    fn index_path(&self) -> PathBuf {
        self.meta_root.join("index.json")
    }

    fn dir(&self, id: &str) -> PathBuf {
        self.meta_root.join(id)
    }

    fn record_path(&self, id: &str) -> PathBuf {
        self.dir(id).join("script.json")
    }

    fn file_for_name(&self, name: &str) -> PathBuf {
        self.scripts_dir.join(format!("{name}.{FILE_EXTENSION}"))
    }

    /// Defense in depth behind the name validator: a file must land directly in the library.
    fn assert_inside_library(&self, path: &Path) -> Result<()> {
        if path.parent() == Some(self.scripts_dir.as_path()) {
            Ok(())
        } else {
            Err(ScriptNameError::ForbiddenCharacter('/').into())
        }
    }

    fn write_file(&mut self, id: &str, name: &str, source: &str) -> Result<()> {
        let path = self.file_for_name(name);
        self.assert_inside_library(&path)?;
        atomic_write(&path, source.as_bytes())?;
        // An atomic write replaces the file, dropping the previous extended attributes.
        write_id(id, &path);
        if let Some(file_name) = file_name_of(&path) {
            self.file_names.insert(id.to_owned(), file_name);
        }
        Ok(())
    }

    fn rename_file(&mut self, id: &str, name: &str) -> Result<()> {
        let Some(old_name) = self.file_names.get(id).cloned() else {
            return Ok(());
        };
        let source = self.scripts_dir.join(&old_name);
        let destination = self.file_for_name(name);
        self.assert_inside_library(&destination)?;
        if !source.exists() {
            return Ok(());
        }
        let new_file_name = file_name_of(&destination).unwrap_or_default();
        if old_name.to_lowercase() == new_file_name.to_lowercase() {
            // A case-only rename on a case-insensitive volume: the destination "exists".
            let temporary = self.scripts_dir.join(format!(".renaming-{id}"));
            fs::rename(&source, &temporary)?;
            fs::rename(&temporary, &destination)?;
        } else {
            if destination.exists() {
                return Err(ScriptStoreError::NameConflict);
            }
            fs::rename(&source, &destination)?;
        }
        self.file_names.insert(id.to_owned(), new_file_name);
        Ok(())
    }

    fn save_index(&self) -> Result<()> {
        atomic_write_json(&self.file_names, &self.index_path())
    }

    fn write_record(&self, record: &Record) -> Result<()> {
        fs::create_dir_all(self.dir(&record.id))?;
        atomic_write_json(record, &self.record_path(&record.id))
    }

    fn write_revision(&self, revision: &ScriptVersion) -> Result<()> {
        let dir = self.dir(&revision.script_id).join("Revisions");
        fs::create_dir_all(&dir)?;
        atomic_write_json(revision, &dir.join(format!("{}.json", revision.id)))
    }

    fn prune_revisions(&mut self, id: &str, keeping: usize) {
        let Ok(revisions) = self.revisions(id) else {
            return;
        };
        for revision in revisions.into_iter().skip(keeping) {
            let _ = fs::remove_file(self.dir(id).join(format!("Revisions/{}.json", revision.id)));
        }
    }

    // MARK: Names

    /// Lowercased names in use: catalog scripts plus every file on disk (files not yet in the
    /// catalog still occupy their names).
    fn taken_names(&self, excluding: Option<&str>) -> HashSet<String> {
        let mut taken: HashSet<String> = self
            .metadata
            .values()
            .filter(|s| Some(s.id.as_str()) != excluding)
            .map(|s| s.name.to_lowercase())
            .collect();
        let own = excluding.and_then(|id| self.file_names.get(id));
        for entry in fs::read_dir(&self.scripts_dir)
            .into_iter()
            .flatten()
            .flatten()
        {
            let file = entry.file_name().to_string_lossy().into_owned();
            if Some(&file) != own {
                taken.insert(stem(&file).to_lowercase());
            }
        }
        taken
    }

    fn is_name_taken(&self, name: &str, excluding: Option<&str>) -> bool {
        self.taken_names(excluding).contains(&name.to_lowercase())
    }
}

/// `source` ending in a newline: saved scripts always do, so files stay POSIX text and diffs
/// stay clean. Empty source stays empty; extra trailing newlines are the author's.
pub fn with_trailing_newline(source: &str) -> String {
    let mut text = source.to_owned();
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text
}

/// The starter source for a new script of `script_type`.
pub fn template(script_type: ScriptType, title: &str) -> String {
    let safe = title.replace('"', "'");
    match script_type {
        ScriptType::Indicator => {
            format!("//@version=6\nindicator(\"{safe}\", overlay=true)\nplot(close)\n")
        }
        ScriptType::Strategy => format!("//@version=6\nstrategy(\"{safe}\", overlay=true)\n"),
        ScriptType::Library => format!("//@version=6\nlibrary(\"{safe}\")\n"),
    }
}

/// The type a script's own declaration gives (indicator when it has none).
pub fn declared_type(source: &str) -> ScriptType {
    compiler::compile(source, Limits::default(), &LibraryRegistry)
        .declaration
        .script_type
        .into()
}

fn disambiguated(base: &str, taken: &HashSet<String>) -> String {
    if !taken.contains(&base.to_lowercase()) {
        return base.to_owned();
    }
    let mut n = 2;
    while taken.contains(&format!("{base} {n}").to_lowercase()) {
        n += 1;
    }
    format!("{base} {n}")
}

/// A stable SHA-256 hash of a script's source, shared by compile-cache and alert identity.
pub fn hash_source(source: &str) -> String {
    use sha2::{Digest, Sha256};

    format!("{:x}", Sha256::digest(source.as_bytes()))
}

fn stem(file_name: &str) -> String {
    Path::new(file_name)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| file_name.to_owned())
}

fn file_name_of(path: &Path) -> Option<String> {
    path.file_name().map(|n| n.to_string_lossy().into_owned())
}

fn system_time_ms(time: std::time::SystemTime) -> i64 {
    time.duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// UTF-8, falling back to ISO Latin-1.
fn read_source(path: &Path) -> Option<String> {
    let data = fs::read(path).ok()?;
    Some(match String::from_utf8(data) {
        Ok(text) => text,
        Err(error) => error.into_bytes().iter().map(|&b| b as char).collect(),
    })
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Option<T> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    fs::write(&temporary, bytes)?;
    fs::rename(&temporary, path)
}

fn atomic_write_json<T: Serialize>(value: &T, path: &Path) -> Result<()> {
    let bytes =
        serde_json::to_vec_pretty(value).map_err(|e| ScriptStoreError::Io(e.to_string()))?;
    atomic_write(path, &bytes)?;
    Ok(())
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// 32 random-looking hex digits. `RandomState` is seeded from the OS, so no RNG crate is needed.
fn new_id() -> ScriptId {
    use std::hash::{BuildHasher, Hasher};
    let state = std::collections::hash_map::RandomState::new();
    let mut parts = [0u64; 2];
    for part in &mut parts {
        let mut hasher = state.build_hasher();
        hasher.write_u64(now_ms() as u64);
        hasher.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
        *part = hasher.finish();
    }
    format!("{:016x}{:016x}", parts[0], parts[1])
}

/// Moves `file` into `~/.Trash` (the app's own `Trash` folder off macOS), keeping a recoverable
/// copy under a free name.
fn trash(file: &Path) -> std::io::Result<()> {
    let trash_dir = if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".Trash"))
    } else {
        crate::store::data_dir().map(|dir| dir.join("Trash"))
    }
    .ok_or_else(|| std::io::Error::other("no home directory"))?;
    fs::create_dir_all(&trash_dir)?;
    let base = stem(&file_name_of(file).unwrap_or_default());
    let mut target = trash_dir.join(format!("{base}.{FILE_EXTENSION}"));
    let mut n = 2;
    while target.exists() {
        target = trash_dir.join(format!("{base} {n}.{FILE_EXTENSION}"));
        n += 1;
    }
    fs::rename(file, target)
}

#[cfg(target_os = "macos")]
fn read_id(path: &Path) -> Option<ScriptId> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let path = CString::new(path.as_os_str().as_bytes()).ok()?;
    let attribute = CString::new(ID_ATTRIBUTE).ok()?;
    let mut buffer = [0u8; 64];
    // SAFETY: both strings are NUL-terminated and `buffer` is valid for `buffer.len()` bytes.
    let length = unsafe {
        libc::getxattr(
            path.as_ptr(),
            attribute.as_ptr(),
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            0,
            0,
        )
    };
    if length <= 0 {
        return None;
    }
    let text = std::str::from_utf8(&buffer[..length as usize]).ok()?;
    (text.len() == 32 && text.bytes().all(|b| b.is_ascii_hexdigit())).then(|| text.to_owned())
}

#[cfg(target_os = "macos")]
fn write_id(id: &str, path: &Path) {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let (Ok(path), Ok(attribute)) = (
        CString::new(path.as_os_str().as_bytes()),
        CString::new(ID_ATTRIBUTE),
    ) else {
        return;
    };
    // SAFETY: both strings are NUL-terminated and `id` is valid for `id.len()` bytes.
    unsafe {
        libc::setxattr(
            path.as_ptr(),
            attribute.as_ptr(),
            id.as_ptr().cast(),
            id.len(),
            0,
            0,
        );
    }
}

#[cfg(not(target_os = "macos"))]
fn read_id(_path: &Path) -> Option<ScriptId> {
    None
}

#[cfg(not(target_os = "macos"))]
fn write_id(_id: &str, _path: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_hash_is_sha256() {
        assert_eq!(
            hash_source("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    fn temp_store(tag: &str) -> (ScriptStore, PathBuf) {
        let root = std::env::temp_dir().join(format!("degenview-store-{tag}-{}", new_id()));
        let store = ScriptStore::new(root.join("Scripts"), root.join("ScriptMetadata"));
        (store, root)
    }

    #[test]
    fn create_uses_template_and_disambiguates() {
        let (mut store, root) = temp_store("create");
        let first = store
            .create("Alpha", ScriptType::Indicator, None, true)
            .unwrap();
        assert_eq!(
            first.source,
            "//@version=6\nindicator(\"Alpha\", overlay=true)\nplot(close)\n"
        );
        let second = store
            .create("alpha", ScriptType::Strategy, None, true)
            .unwrap();
        assert_eq!(second.name, "alpha 2");
        assert!(matches!(
            store.create("ALPHA", ScriptType::Indicator, None, false),
            Err(ScriptStoreError::NameConflict)
        ));
        assert_eq!(store.all_scripts().unwrap().len(), 2);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn save_updates_type_from_declaration_and_records_revisions() {
        let (mut store, root) = temp_store("save");
        let script = store
            .create("S", ScriptType::Indicator, None, true)
            .unwrap();
        let saved = store
            .save(&script.id, "//@version=6\nstrategy(\"S\")\n")
            .unwrap();
        assert_eq!(saved.script_type, ScriptType::Strategy);
        assert_eq!(
            saved.compile_record.as_ref().unwrap().compiler_version,
            COMPILER_VERSION
        );
        // Mid-edit source with no declaration keeps the old type.
        let kept = store.save(&script.id, "plot(close)\n").unwrap();
        assert_eq!(kept.script_type, ScriptType::Strategy);
        assert_eq!(store.revisions(&script.id).unwrap().len(), 2);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn save_ends_the_file_in_a_newline() {
        let (mut store, root) = temp_store("newline");
        let script = store
            .create("N", ScriptType::Indicator, None, true)
            .unwrap();
        let saved = store
            .save(&script.id, "//@version=6\nindicator(\"N\")\nplot(close)")
            .unwrap();
        assert!(saved.source.ends_with("plot(close)\n"));
        let path = store.file_path(&script.id).unwrap().unwrap();
        assert!(fs::read_to_string(path).unwrap().ends_with("plot(close)\n"));
        // Already terminated, extra blank lines and empty source are left alone.
        assert_eq!(with_trailing_newline("a\n"), "a\n");
        assert_eq!(with_trailing_newline("a\n\n"), "a\n\n");
        assert_eq!(with_trailing_newline(""), "");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rename_duplicate_favorite_and_draft_round_trip() {
        let (mut store, root) = temp_store("crud");
        let script = store
            .create("One", ScriptType::Indicator, None, true)
            .unwrap();
        let renamed = store.rename(&script.id, "Two").unwrap();
        assert_eq!(renamed.name, "Two");
        assert_eq!(renamed.id, script.id);
        assert!(root.join("Scripts/Two.pine").exists());
        assert!(!root.join("Scripts/One.pine").exists());
        // Case-only rename.
        assert_eq!(store.rename(&script.id, "TWO").unwrap().name, "TWO");

        let copy = store.duplicate(&script.id).unwrap();
        assert_eq!(copy.name, "TWO copy");
        assert!(!copy.is_favorite);
        assert_ne!(copy.id, script.id);

        store.set_favorite(&script.id, true).unwrap();
        let draft = ScriptDraft {
            script_id: script.id.clone(),
            source: "draft".into(),
            modified_at_ms: 5,
            based_on_revision_id: None,
        };
        store.save_draft(&draft).unwrap();
        assert_eq!(store.draft(&script.id).unwrap(), Some(draft));

        // A fresh store over the same folders sees the same ids and the favorite.
        let mut again = ScriptStore::new(root.join("Scripts"), root.join("ScriptMetadata"));
        let all = again.all_scripts().unwrap();
        let loaded = all.iter().find(|s| s.id == script.id).expect("same id");
        assert!(loaded.is_favorite);
        assert_eq!(loaded.name, "TWO");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn external_files_appear_and_stale_compile_records_drop() {
        let (mut store, root) = temp_store("external");
        let script = store
            .create("Ext", ScriptType::Indicator, None, true)
            .unwrap();
        store
            .save(
                &script.id,
                "//@version=6\nindicator(\"Ext\")\nplot(close)\n",
            )
            .unwrap();
        fs::write(
            root.join("Scripts/Ext.pine"),
            "//@version=6\nindicator(\"Ext\")\nplot(open)\n",
        )
        .unwrap();
        fs::write(
            root.join("Scripts/Outside.pine"),
            "//@version=6\nlibrary(\"Outside\")\n",
        )
        .unwrap();
        let all = store.all_scripts().unwrap();
        assert_eq!(all.len(), 2);
        assert!(
            all.iter()
                .find(|s| s.id == script.id)
                .unwrap()
                .compile_record
                .is_none()
        );
        let outside = all.iter().find(|s| s.name == "Outside").unwrap();
        assert_eq!(outside.script_type, ScriptType::Library);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_folder_imports_whole_and_leaves_the_originals() {
        let (mut store, root) = temp_store("folder");
        let source = root.join("swift");
        fs::create_dir_all(&source).unwrap();
        fs::write(
            source.join("One.pine"),
            "//@version=6\nindicator(\"One\")\n",
        )
        .unwrap();
        fs::write(source.join("Two.pine"), "//@version=6\nlibrary(\"Two\")\n").unwrap();
        fs::write(source.join("notes.txt"), "ignored").unwrap();
        assert_eq!(store.import_folder(&source).unwrap(), 2);
        let all = store.all_scripts().unwrap();
        assert_eq!(all.len(), 2);
        assert!(source.join("One.pine").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn import_and_delete() {
        let (mut store, root) = temp_store("import");
        let outside = root.join("incoming.pine");
        fs::create_dir_all(&root).unwrap();
        fs::write(&outside, "//@version=6\nstrategy(\"x\")\n").unwrap();
        let imported = store.import_file(&outside).unwrap();
        assert_eq!(imported.name, "incoming");
        assert_eq!(imported.script_type, ScriptType::Strategy);
        let binary = root.join("bad.pine");
        fs::write(&binary, [0xff, 0xfe, 0x41]).unwrap();
        // Latin-1 fallback still reads it.
        assert!(store.import_file(&binary).is_ok());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_save_is_refused_when_the_file_changed_outside_since_it_was_read() {
        let (mut store, dir) = temp_store("checked-save");
        let script = store
            .create("Checked", ScriptType::Indicator, None, false)
            .unwrap();
        let read = store.script(&script.id).unwrap().unwrap().source;
        let path = store.file_path(&script.id).unwrap().unwrap();
        // Unchanged: saves.
        let saved = store
            .save_if_unchanged(&script.id, "//@version=6\nindicator(\"A\")\n", &read)
            .unwrap();
        // Another tool rewrites the file.
        fs::write(&path, "//@version=6\nindicator(\"Outside\")\n").unwrap();
        let refused = store.save_if_unchanged(&script.id, "mine\n", &saved.source);
        assert!(matches!(refused, Err(ScriptStoreError::ChangedOnDisk)));
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "//@version=6\nindicator(\"Outside\")\n",
            "the outside edit is untouched"
        );
        // A plain save still overwrites, as when the person chose to keep their version.
        store.save(&script.id, "mine\n").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "mine\n");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_rewrite_changes_the_files_stamp_and_a_missing_file_has_none() {
        let (mut store, dir) = temp_store("stamp");
        let script = store
            .create("Stamped", ScriptType::Indicator, None, false)
            .unwrap();
        let before = store.file_stamp(&script.id).unwrap();
        assert_eq!(before, store.file_stamp(&script.id).unwrap());
        let path = store.file_path(&script.id).unwrap().unwrap();
        fs::write(
            &path,
            "a much longer text than the template had before\n".repeat(20),
        )
        .unwrap();
        assert_ne!(before, store.file_stamp(&script.id).unwrap());
        assert!(store.file_stamp("no-such-script").is_none());
        let _ = fs::remove_dir_all(dir);
    }
}

//! The monospaced fonts installed on this machine, for the editor's font picker.
//!
//! Fonts are found by walking the system's font folders and reading, from each file, only the
//! handful of bytes that say what it is: the table directory, then the `name`, `OS/2` and `post`
//! tables. A scan therefore reads kilobytes from each of hundreds of files (a CJK font is tens of
//! megabytes) and runs on its own thread; the picker shows its answer when it arrives. A font is
//! listed once per family, by its regular face, and only when it declares itself monospaced.
//!
//! Nothing here loads a font into the app: that is [`read_font`], for the one a person picked.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use ttf_parser::PlatformId;
use ttf_parser::name::Table as NameTable;
use ttf_parser::name_id;

/// The largest font file that is loaded. Larger ones are not a code font.
pub const MAX_FONT_BYTES: u64 = 64 * 1024 * 1024;
/// A table this big is not one of the three that are read.
const MAX_TABLE_BYTES: u32 = 4 * 1024 * 1024;
const MAX_DEPTH: usize = 4;

/// One installed family, by the file and face that is its regular style.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FontEntry {
    pub family: String,
    pub path: PathBuf,
    /// The face inside a collection (`.ttc`), 0 for a plain font.
    pub index: u32,
}

/// What a face says about itself.
#[derive(Debug, Eq, PartialEq)]
struct Face {
    family: String,
    monospaced: bool,
    regular: bool,
    /// Whether it has scalable outlines (`glyf` or `CFF`). A bitmap-only font (`bdat`, `sbix`) maps
    /// its characters but draws nothing at a size egui asks for: an editor in it would be blank.
    outlines: bool,
}

struct TableRecord {
    tag: [u8; 4],
    offset: u32,
    length: u32,
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn read_at(file: &mut File, offset: u64, length: usize) -> Option<Vec<u8>> {
    file.seek(SeekFrom::Start(offset)).ok()?;
    let mut bytes = vec![0; length];
    file.read_exact(&mut bytes).ok()?;
    Some(bytes)
}

/// The offsets of every face in the file: one for a plain font, each member of a collection.
fn face_offsets(file: &mut File) -> Vec<u64> {
    let Some(head) = read_at(file, 0, 12) else {
        return Vec::new();
    };
    match &head[..4] {
        b"ttcf" => {
            let count = u32_at(&head, 8).unwrap_or(0).min(64) as usize;
            let Some(offsets) = read_at(file, 12, count * 4) else {
                return Vec::new();
            };
            (0..count)
                .filter_map(|member| u32_at(&offsets, member * 4))
                .map(u64::from)
                .collect()
        }
        // TrueType, OpenType with CFF, and the two older TrueType tags.
        [0, 1, 0, 0] | b"OTTO" | b"true" | b"typ1" => vec![0],
        _ => Vec::new(),
    }
}

fn table_records(file: &mut File, face: u64) -> Option<Vec<TableRecord>> {
    let head = read_at(file, face, 12)?;
    let count = usize::from(u16_at(&head, 4)?).min(64);
    let directory = read_at(file, face + 12, count * 16)?;
    Some(
        (0..count)
            .filter_map(|at| {
                let record = directory.get(at * 16..at * 16 + 16)?;
                Some(TableRecord {
                    tag: record[..4].try_into().ok()?,
                    offset: u32_at(record, 8)?,
                    length: u32_at(record, 12)?,
                })
            })
            .collect(),
    )
}

fn table(file: &mut File, records: &[TableRecord], tag: &[u8; 4]) -> Option<Vec<u8>> {
    let record = records.iter().find(|record| &record.tag == tag)?;
    if record.length == 0 || record.length > MAX_TABLE_BYTES {
        return None;
    }
    read_at(file, u64::from(record.offset), record.length as usize)
}

/// What the face at `offset` says it is, or `None` for something unreadable or nameless.
fn read_face(file: &mut File, offset: u64) -> Option<Face> {
    let records = table_records(file, offset)?;
    let names = table(file, &records, b"name")?;
    let names = NameTable::parse(&names)?;
    // Unicode names first; failing those, a Macintosh name made only of ASCII (which is all of
    // Roman that a family name needs, and the only kind Menlo and Monaco carry).
    let text = |id: u16| {
        let unicode = names
            .names
            .into_iter()
            .filter(|name| name.name_id == id && name.is_unicode())
            .find_map(|name| name.to_string());
        unicode.or_else(|| {
            names
                .names
                .into_iter()
                .filter(|name| name.name_id == id && name.platform_id == PlatformId::Macintosh)
                .find_map(|name| {
                    name.name
                        .is_ascii()
                        .then(|| String::from_utf8_lossy(name.name).into_owned())
                })
        })
    };
    let family = text(name_id::TYPOGRAPHIC_FAMILY)
        .or_else(|| text(name_id::FAMILY))?
        .trim()
        .to_owned();
    let subfamily = text(name_id::TYPOGRAPHIC_SUBFAMILY)
        .or_else(|| text(name_id::SUBFAMILY))
        .unwrap_or_default()
        .to_lowercase();

    let post = table(file, &records, b"post");
    let os2 = table(file, &records, b"OS/2");
    let fixed_pitch = post
        .as_deref()
        .and_then(|post| u32_at(post, 12))
        .is_some_and(|fixed| fixed != 0);
    // Panose's proportion digit 9 is "monospaced", for a font whose post table does not say.
    let panose_mono = os2.as_deref().and_then(|os2| os2.get(35)).copied() == Some(9);

    // Regular: neither italic nor bold, at weight 400 (a variable font's default instance is).
    let regular = match os2.as_deref() {
        Some(os2) => {
            let weight = u16_at(os2, 4).unwrap_or(400);
            let selection = u16_at(os2, 62).unwrap_or(0);
            let styled = selection & 0b10_0001 != 0 && selection & 0b100_0000 == 0;
            !styled && (350..=450).contains(&weight)
        }
        None => matches!(
            subfamily.as_str(),
            "regular" | "book" | "roman" | "normal" | ""
        ),
    };
    let outlines = records
        .iter()
        .any(|record| matches!(&record.tag, b"glyf" | b"CFF " | b"CFF2"));
    Some(Face {
        family,
        monospaced: fixed_pitch || panose_mono,
        regular,
        outlines,
    })
}

fn is_font_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "ttf" | "otf" | "ttc" | "otc"
            )
        })
}

fn collect_files(dir: &Path, depth: usize, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            if depth < MAX_DEPTH {
                collect_files(&path, depth + 1, files);
            }
        } else if is_font_file(&path) {
            files.push(path);
        }
    }
}

/// The monospaced families in `dirs`, each once by its regular face, sorted by name (ignoring
/// case). A file that cannot be read, a face with no name and a hidden system font (one whose
/// name starts with a dot) are skipped.
pub fn scan(dirs: &[PathBuf]) -> Vec<FontEntry> {
    let mut files = Vec::new();
    for dir in dirs {
        collect_files(dir, 0, &mut files);
    }
    let mut found: Vec<FontEntry> = Vec::new();
    for path in files {
        let Ok(mut file) = File::open(&path) else {
            continue;
        };
        for (index, offset) in face_offsets(&mut file).into_iter().enumerate() {
            let Some(face) = read_face(&mut file, offset) else {
                continue;
            };
            if !face.monospaced
                || !face.regular
                || !face.outlines
                || face.family.is_empty()
                || face.family.starts_with('.')
            {
                continue;
            }
            let entry = FontEntry {
                family: face.family,
                path: path.clone(),
                index: index as u32,
            };
            // The first file wins a family; `files` is in a fixed order, so so does the same one
            // on every scan.
            if !found
                .iter()
                .any(|known| known.family.eq_ignore_ascii_case(&entry.family))
            {
                found.push(entry);
            }
        }
    }
    found.sort_by_key(|entry| entry.family.to_lowercase());
    found
}

/// The folders fonts are installed in on this system.
pub fn system_font_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut dirs: Vec<PathBuf> = Vec::new();
    if cfg!(target_os = "macos") {
        dirs.extend(
            ["/System/Library/Fonts", "/Library/Fonts"]
                .iter()
                .map(PathBuf::from),
        );
        dirs.extend(home.iter().map(|home| home.join("Library/Fonts")));
    } else if cfg!(target_os = "windows") {
        dirs.extend(std::env::var_os("WINDIR").map(|windir| PathBuf::from(windir).join("Fonts")));
        dirs.extend(std::env::var_os("LOCALAPPDATA").map(|local| {
            PathBuf::from(local)
                .join("Microsoft")
                .join("Windows")
                .join("Fonts")
        }));
    } else {
        dirs.extend(
            ["/usr/share/fonts", "/usr/local/share/fonts"]
                .iter()
                .map(PathBuf::from),
        );
        dirs.extend(
            home.iter()
                .flat_map(|home| [home.join(".fonts"), home.join(".local/share/fonts")]),
        );
    }
    dirs
}

/// A font file read for use.
pub struct LoadedFont {
    pub bytes: Vec<u8>,
    pub index: u32,
}

/// Reads the font `entry` names. Refuses a file over [`MAX_FONT_BYTES`].
pub fn read_font(entry: &FontEntry) -> io::Result<LoadedFont> {
    let file = File::open(&entry.path)?;
    if file.metadata()?.len() > MAX_FONT_BYTES {
        return Err(io::Error::other("that font file is too large to use"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_FONT_BYTES).read_to_end(&mut bytes)?;
    Ok(LoadedFont {
        bytes,
        index: entry.index,
    })
}

/// Whether `font` can draw letters: it parses, and `A`, `e` and `0` have outlines. Catches what the
/// scan's table check cannot (a font whose outline tables are empty or broken), before the editor is
/// handed a font that would draw nothing.
pub fn check_outlines(font: &LoadedFont) -> Result<(), String> {
    struct Sink;
    impl ttf_parser::OutlineBuilder for Sink {
        fn move_to(&mut self, _: f32, _: f32) {}
        fn line_to(&mut self, _: f32, _: f32) {}
        fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {}
        fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {}
        fn close(&mut self) {}
    }
    let face = ttf_parser::Face::parse(&font.bytes, font.index)
        .map_err(|_| "it is not a font this app can read".to_owned())?;
    for letter in ['A', 'e', '0'] {
        let drawn = face
            .glyph_index(letter)
            .and_then(|glyph| face.outline_glyph(glyph, &mut Sink));
        if drawn.is_none() {
            return Err("it has no scalable letters to draw with (a bitmap font)".to_owned());
        }
    }
    Ok(())
}

#[derive(Default)]
struct State {
    entries: Vec<FontEntry>,
    scanned: bool,
}

/// The installed monospaced fonts, found once on a background thread. Clones share the answer.
#[derive(Clone, Default)]
pub struct FontCatalog {
    state: Arc<Mutex<State>>,
}

impl FontCatalog {
    /// Starts the scan. `done` runs on the scanning thread when the list is ready (to ask for a
    /// repaint); a catalog started twice scans twice.
    pub fn start(dirs: Vec<PathBuf>, done: impl FnOnce() + Send + 'static) -> Self {
        let catalog = Self::default();
        let shared = Arc::clone(&catalog.state);
        let spawned = std::thread::Builder::new()
            .name("font-scan".into())
            .spawn(move || {
                let entries = scan(&dirs);
                if let Ok(mut state) = shared.lock() {
                    *state = State {
                        entries,
                        scanned: true,
                    };
                }
                done();
            });
        if spawned.is_err()
            && let Ok(mut state) = catalog.state.lock()
        {
            // No thread, no fonts: the picker offers the default alone rather than waiting.
            state.scanned = true;
        }
        catalog
    }

    /// A catalog that has already finished, with `entries`, for a test.
    #[cfg(test)]
    pub fn with_entries(entries: Vec<FontEntry>) -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                entries,
                scanned: true,
            })),
        }
    }

    pub fn is_ready(&self) -> bool {
        self.state.lock().map(|state| state.scanned).unwrap_or(true)
    }

    pub fn entries(&self) -> Vec<FontEntry> {
        self.state
            .lock()
            .map(|state| state.entries.clone())
            .unwrap_or_default()
    }

    /// The entry for `family`, ignoring case.
    pub fn find(&self, family: &str) -> Option<FontEntry> {
        self.state.lock().ok().and_then(|state| {
            state
                .entries
                .iter()
                .find(|entry| entry.family.eq_ignore_ascii_case(family))
                .cloned()
        })
    }
}

#[cfg(test)]
mod tests;

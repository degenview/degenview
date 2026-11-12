//! The on-disk cache for regenerable network data: icon bytes, persisted lookups and FX rates.
//!
//! Everything here can be deleted at any time and rebuilt from the network, so a failure to read
//! or write is never an error — it is a miss. User data lives in SQLite (`store`), never here.
//!
//! Layout under the cache root (`store::cache_dir`):
//!
//! ```text
//! images/<aa>/<sha256>     one file per icon URL
//! lookups/<namespace>.json persisted keyed lookups (`cache::lookup`)
//! fx/<name>.json           FX and BTC history (`fx_rates`)
//! ```
//!
//! File names are always generated (a hash or a constant), never taken from a URL or a ticker, so
//! nothing a provider returns can name a path. Writes are atomic (temporary file + rename).
//! Clearing removes only the three directories above, so pointing `DEGENVIEW_CACHE_DIR` at a
//! shared folder can never delete anything else.

use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{OnceLock, RwLock};
use std::time::{Duration, SystemTime};

/// Longest an image is kept without being fetched again.
pub const IMAGE_MAX_AGE: Duration = Duration::from_secs(90 * 86_400);
/// Soft cap on `images/`; pruning evicts the oldest files down to `PRUNE_TARGET_PERCENT` of it.
pub const IMAGE_MAX_BYTES: u64 = 128 * 1024 * 1024;
const PRUNE_TARGET_PERCENT: u64 = 75;
/// A temporary file older than this belongs to a write that died.
const STALE_TEMPORARY: Duration = Duration::from_secs(3_600);
const TEMPORARY_SUFFIX: &str = ".tmp";

/// A top-level directory of the cache.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Kind {
    Images,
    Lookups,
    Fx,
}

impl Kind {
    pub const ALL: [Self; 3] = [Self::Images, Self::Lookups, Self::Fx];

    fn directory(self) -> &'static str {
        match self {
            Self::Images => "images",
            Self::Lookups => "lookups",
            Self::Fx => "fx",
        }
    }
}

/// A file read back from the cache, with when it was written.
#[derive(Debug)]
pub struct Stored {
    pub bytes: Vec<u8>,
    pub modified: SystemTime,
}

impl Stored {
    pub fn age(&self, now: SystemTime) -> Duration {
        now.duration_since(self.modified).unwrap_or_default()
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Usage {
    pub files: u64,
    pub bytes: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CacheStats {
    pub images: Usage,
    /// Persisted lookups and FX rates.
    pub lookups: Usage,
    pub location: Option<PathBuf>,
}

impl CacheStats {
    pub fn total_bytes(&self) -> u64 {
        self.images.bytes + self.lookups.bytes
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ClearOutcome {
    pub freed_bytes: u64,
    /// Entries that could not be removed (in use, or no permission).
    pub failures: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PruneReport {
    pub removed_files: u64,
    pub freed_bytes: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_age: Duration,
    pub max_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_age: IMAGE_MAX_AGE,
            max_bytes: IMAGE_MAX_BYTES,
        }
    }
}

pub struct DiskCache {
    root: Option<PathBuf>,
    /// Bumped by every `clear`: a write that began before it is dropped, so an in-flight fetch
    /// cannot bring back what the user just cleared.
    generation: AtomicU64,
    /// Writers hold the read side across check-and-write, `clear` the write side.
    gate: RwLock<()>,
}

/// The app's cache. Disabled (every read misses, every write is dropped) under `cfg!(test)`, as
/// Keychain is, so tests never read or fill a developer's real cache.
pub fn global() -> &'static DiskCache {
    static CACHE: OnceLock<DiskCache> = OnceLock::new();
    CACHE.get_or_init(|| {
        if cfg!(test) {
            DiskCache::disabled()
        } else {
            DiskCache::new(crate::store::cache_dir())
        }
    })
}

/// A stable, filesystem-safe name for any string: the SHA-256 hex digest.
pub fn digest_name(key: &str) -> String {
    Sha256::digest(key.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

impl DiskCache {
    pub fn new(root: Option<PathBuf>) -> Self {
        Self {
            root,
            generation: AtomicU64::new(0),
            gate: RwLock::new(()),
        }
    }

    #[cfg(test)]
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self::new(Some(root.into()))
    }

    pub fn disabled() -> Self {
        Self::new(None)
    }

    /// The folder a kind lives in, for code that manages its own files there (`fx_rates`).
    pub fn directory(&self, kind: Kind) -> Option<PathBuf> {
        Some(self.root.as_ref()?.join(kind.directory()))
    }

    pub fn is_enabled(&self) -> bool {
        self.root.is_some()
    }

    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    /// The path for `name` within `kind`, or `None` for a disabled cache or a name that is not a
    /// plain file name.
    fn path(&self, kind: Kind, name: &str) -> Option<PathBuf> {
        if !is_plain_name(name) {
            return None;
        }
        let mut path = self.root.as_ref()?.join(kind.directory());
        if kind == Kind::Images {
            // Shard by the first two characters so no directory grows to thousands of files.
            path.push(name.get(..2)?);
        }
        Some(path.join(name))
    }

    pub fn read(&self, kind: Kind, name: &str) -> Option<Stored> {
        let path = self.path(kind, name)?;
        let modified = fs::metadata(&path).ok()?.modified().ok()?;
        let bytes = fs::read(&path).ok()?;
        Some(Stored { bytes, modified })
    }

    /// Writes `bytes` atomically, unless the cache was cleared since `generation` was read.
    /// Returns whether the file is now on disk.
    pub fn write(&self, kind: Kind, name: &str, bytes: &[u8], generation: u64) -> bool {
        let Some(path) = self.path(kind, name) else {
            return false;
        };
        let Ok(_gate) = self.gate.read() else {
            return false;
        };
        if self.generation() != generation {
            return false;
        }
        write_atomic(&path, bytes)
    }

    /// Sizes of what is cached. Walks the tree: call it off the UI thread.
    pub fn stats(&self) -> CacheStats {
        let Some(root) = &self.root else {
            return CacheStats::default();
        };
        let usage = |kind: Kind| {
            let mut usage = Usage::default();
            for file in walk(&root.join(kind.directory())) {
                usage.files += 1;
                usage.bytes += file.len;
            }
            usage
        };
        let (lookups, fx) = (usage(Kind::Lookups), usage(Kind::Fx));
        CacheStats {
            images: usage(Kind::Images),
            lookups: Usage {
                files: lookups.files + fx.files,
                bytes: lookups.bytes + fx.bytes,
            },
            location: Some(root.clone()),
        }
    }

    /// Evicts images older than `limits.max_age`, then the oldest until the folder fits under
    /// the size cap with headroom, and sweeps temporary files left by interrupted writes.
    pub fn prune(&self, now: SystemTime, limits: Limits) -> PruneReport {
        let Some(root) = &self.root else {
            return PruneReport::default();
        };
        let Ok(_gate) = self.gate.read() else {
            return PruneReport::default();
        };
        let mut report = PruneReport::default();
        let remove = |file: &File, report: &mut PruneReport| {
            if fs::remove_file(&file.path).is_ok() {
                report.removed_files += 1;
                report.freed_bytes += file.len;
                true
            } else {
                false
            }
        };

        for kind in Kind::ALL {
            for file in walk(&root.join(kind.directory())) {
                if is_temporary(&file.path) && age(now, file.modified) > STALE_TEMPORARY {
                    remove(&file, &mut report);
                }
            }
        }

        let mut images: Vec<File> = walk(&root.join(Kind::Images.directory()))
            .into_iter()
            .filter(|file| !is_temporary(&file.path))
            .collect();
        images
            .retain(|file| age(now, file.modified) <= limits.max_age || !remove(file, &mut report));
        let mut total: u64 = images.iter().map(|file| file.len).sum();
        if total > limits.max_bytes {
            let target = limits.max_bytes / 100 * PRUNE_TARGET_PERCENT;
            images.sort_by_key(|file| file.modified);
            for file in &images {
                if total <= target {
                    break;
                }
                if remove(file, &mut report) {
                    total -= file.len;
                }
            }
        }
        report
    }

    /// Removes every cached file and invalidates in-flight writes. Only the cache's own
    /// directories are touched.
    pub fn clear(&self) -> ClearOutcome {
        let Some(root) = &self.root else {
            return ClearOutcome::default();
        };
        let Ok(_gate) = self.gate.write() else {
            return ClearOutcome::default();
        };
        let mut outcome = ClearOutcome::default();
        for kind in Kind::ALL {
            let directory = root.join(kind.directory());
            // `symlink_metadata`: a link planted at one of our paths is removed, never followed.
            let Ok(metadata) = fs::symlink_metadata(&directory) else {
                continue;
            };
            let bytes: u64 = walk(&directory).iter().map(|file| file.len).sum();
            let removed = if metadata.is_dir() {
                fs::remove_dir_all(&directory)
            } else {
                fs::remove_file(&directory)
            };
            match removed {
                Ok(()) => outcome.freed_bytes += bytes,
                Err(_) => outcome.failures += 1,
            }
        }
        self.generation.fetch_add(1, Ordering::AcqRel);
        outcome
    }
}

fn age(now: SystemTime, modified: SystemTime) -> Duration {
    now.duration_since(modified).unwrap_or_default()
}

/// A single path component made of characters that mean the same on every filesystem.
fn is_plain_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

fn is_temporary(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(TEMPORARY_SUFFIX))
}

/// A temporary file in the destination's folder, then a rename: readers see the old file or the
/// new one, never half of it. `rename` replaces an existing file on every supported platform.
fn write_atomic(path: &Path, bytes: &[u8]) -> bool {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let (Some(parent), Some(name)) = (path.parent(), path.file_name().and_then(|n| n.to_str()))
    else {
        return false;
    };
    if fs::create_dir_all(parent).is_err() {
        return false;
    }
    let temporary = parent.join(format!(
        "{name}.{}.{}{TEMPORARY_SUFFIX}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let written = fs::write(&temporary, bytes).is_ok() && fs::rename(&temporary, path).is_ok();
    if !written {
        let _ = fs::remove_file(&temporary);
    }
    written
}

struct File {
    path: PathBuf,
    len: u64,
    modified: SystemTime,
}

/// Every regular file under `directory`, recursively. Links are skipped, not followed.
fn walk(directory: &Path) -> Vec<File> {
    let mut files = Vec::new();
    let mut pending = vec![directory.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                pending.push(entry.path());
            } else if kind.is_file()
                && let Ok(metadata) = entry.metadata()
            {
                files.push(File {
                    path: entry.path(),
                    len: metadata.len(),
                    modified: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                });
            }
        }
    }
    files
}

#[cfg(test)]
pub(crate) mod testing {
    use super::DiskCache;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    /// A cache in a fresh temporary directory, removed on drop.
    pub struct TempCache {
        pub cache: DiskCache,
        directory: PathBuf,
    }

    impl TempCache {
        pub fn new() -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let directory = std::env::temp_dir().join(format!(
                "degenview-cache-test-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            Self {
                cache: DiskCache::at(&directory),
                directory,
            }
        }

        pub fn path(&self) -> &Path {
            &self.directory
        }
    }

    impl Drop for TempCache {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::TempCache;
    use super::*;

    fn set_age(cache: &DiskCache, kind: Kind, name: &str, age: Duration) {
        let path = cache.path(kind, name).unwrap();
        let file = fs::File::options().write(true).open(path).unwrap();
        file.set_modified(SystemTime::now() - age).unwrap();
    }

    #[test]
    fn a_written_file_reads_back_and_a_rewrite_replaces_it() {
        let temp = TempCache::new();
        let cache = &temp.cache;
        let generation = cache.generation();
        assert!(cache.read(Kind::Images, "abcd").is_none());
        assert!(cache.write(Kind::Images, "abcd", b"one", generation));
        assert_eq!(cache.read(Kind::Images, "abcd").unwrap().bytes, b"one");
        assert!(cache.write(Kind::Images, "abcd", b"two", generation));
        let stored = cache.read(Kind::Images, "abcd").unwrap();
        assert_eq!(stored.bytes, b"two");
        assert!(stored.age(SystemTime::now()) < Duration::from_secs(60));
        // Sharded by the first two characters, and no temporary file is left behind.
        assert!(temp.path().join("images/ab/abcd").is_file());
        assert_eq!(
            fs::read_dir(temp.path().join("images/ab")).unwrap().count(),
            1
        );
    }

    #[test]
    fn only_plain_file_names_are_accepted() {
        let temp = TempCache::new();
        let generation = temp.cache.generation();
        for name in [
            "",
            ".hidden",
            "../escape",
            "a/b",
            "a\\b",
            "with space",
            "ünï",
        ] {
            assert!(
                !temp.cache.write(Kind::Lookups, name, b"x", generation),
                "{name:?}"
            );
            assert!(temp.cache.read(Kind::Lookups, name).is_none());
        }
        assert!(
            temp.cache
                .write(Kind::Lookups, "icon_url.json", b"x", generation)
        );
        assert!(!temp.path().join("escape").exists());
    }

    #[test]
    fn a_disabled_cache_misses_and_drops_writes() {
        let cache = DiskCache::disabled();
        assert!(!cache.is_enabled());
        assert!(!cache.write(Kind::Images, "abcd", b"x", cache.generation()));
        assert!(cache.read(Kind::Images, "abcd").is_none());
        assert_eq!(cache.stats(), CacheStats::default());
        assert_eq!(cache.clear(), ClearOutcome::default());
        // Tests never see the real cache.
        assert!(!global().is_enabled());
    }

    #[test]
    fn digest_names_are_stable_and_safe() {
        assert_eq!(
            digest_name("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert!(is_plain_name(&digest_name("https://x/y.png?1")));
    }

    #[test]
    fn stats_count_each_kind_and_fold_fx_into_lookups() {
        let temp = TempCache::new();
        let cache = &temp.cache;
        let generation = cache.generation();
        cache.write(Kind::Images, "aa11", &[0; 100], generation);
        cache.write(Kind::Images, "bb22", &[0; 50], generation);
        cache.write(Kind::Lookups, "icon_url.json", &[0; 10], generation);
        cache.write(Kind::Fx, "rates.json", &[0; 5], generation);
        let stats = cache.stats();
        assert_eq!(
            stats.images,
            Usage {
                files: 2,
                bytes: 150
            }
        );
        assert_eq!(
            stats.lookups,
            Usage {
                files: 2,
                bytes: 15
            }
        );
        assert_eq!(stats.total_bytes(), 165);
        assert_eq!(stats.location.as_deref(), Some(temp.path()));
    }

    #[test]
    fn prune_drops_expired_images_then_the_oldest_over_the_cap() {
        let temp = TempCache::new();
        let cache = &temp.cache;
        let generation = cache.generation();
        let day = Duration::from_secs(86_400);
        for (name, days) in [("aa01", 100), ("aa02", 3), ("aa03", 2), ("aa04", 1)] {
            cache.write(Kind::Images, name, &[0; 100], generation);
            set_age(cache, Kind::Images, name, day * days);
        }
        // "aa01" is past the 90-day limit. That leaves 300 bytes against a cap of 250, so the
        // oldest go until 75% of the cap (150) holds.
        let limits = Limits {
            max_age: day * 90,
            max_bytes: 250,
        };
        let report = cache.prune(SystemTime::now(), limits);
        for name in ["aa01", "aa02", "aa03"] {
            assert!(cache.read(Kind::Images, name).is_none(), "{name} evicted");
        }
        assert!(cache.read(Kind::Images, "aa04").is_some());
        assert_eq!(
            report,
            PruneReport {
                removed_files: 3,
                freed_bytes: 300
            }
        );
    }

    #[test]
    fn prune_sweeps_dead_temporary_files_but_not_live_ones() {
        let temp = TempCache::new();
        let cache = &temp.cache;
        cache.write(Kind::Lookups, "keep.json", b"x", cache.generation());
        let directory = temp.path().join("lookups");
        let dead = directory.join("a.1.0.tmp");
        let live = directory.join("b.1.1.tmp");
        fs::write(&dead, b"x").unwrap();
        fs::write(&live, b"x").unwrap();
        fs::File::options()
            .write(true)
            .open(&dead)
            .unwrap()
            .set_modified(SystemTime::now() - Duration::from_secs(7_200))
            .unwrap();
        cache.prune(SystemTime::now(), Limits::default());
        assert!(!dead.exists());
        assert!(live.exists());
        assert!(directory.join("keep.json").exists());
    }

    #[test]
    fn clear_removes_only_the_cache_directories_and_invalidates_pending_writes() {
        let temp = TempCache::new();
        let cache = &temp.cache;
        let before = cache.generation();
        cache.write(Kind::Images, "aa11", &[0; 40], before);
        cache.write(Kind::Lookups, "icon_url.json", &[0; 10], before);
        cache.write(Kind::Fx, "rates.json", &[0; 5], before);
        // Something that is not ours must survive.
        fs::write(temp.path().join("notes.txt"), b"keep").unwrap();

        let outcome = cache.clear();
        assert_eq!(
            outcome,
            ClearOutcome {
                freed_bytes: 55,
                failures: 0
            }
        );
        assert!(cache.read(Kind::Images, "aa11").is_none());
        assert_eq!(cache.stats().total_bytes(), 0);
        assert!(temp.path().join("notes.txt").exists());

        // A fetch that started before the clear cannot bring its file back…
        assert!(!cache.write(Kind::Images, "aa11", b"late", before));
        assert!(cache.read(Kind::Images, "aa11").is_none());
        // …but a fresh one can.
        assert!(cache.write(Kind::Images, "aa11", b"new", cache.generation()));
        // Clearing an empty or never-created cache is fine.
        cache.clear();
        assert_eq!(cache.clear(), ClearOutcome::default());
    }
}

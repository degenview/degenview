//! Persisted lookups that rarely change — which icon belongs to a market, the coin index, the
//! exchanges' symbol catalogs — kept in memory and mirrored to one JSON file per namespace.
//!
//! A namespace is read from disk once (`warm` does it at launch, off the UI thread) and answered
//! from memory afterwards, so a per-frame `get` is a lock and a hash lookup with no I/O. Writes
//! are batched: the first `put` after a flush schedules one, which writes the whole namespace
//! atomically a moment later.
//!
//! This is a cache, not a document table: a file that cannot be decoded, or was written by a
//! different `VERSION`, is dropped and rebuilt from the network instead of being treated as a
//! fault. Entries stay readable past their TTL (`Hit::fresh` says so) so a caller can keep using
//! a stale answer while it refreshes, or when the network is down; they are discarded for good
//! after `RETENTION`.

use super::disk::{self, DiskCache, Kind};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// Bump when a namespace's stored shape changes: old files are then ignored.
const VERSION: u32 = 1;
/// Entries older than this are dropped on load, so a table cannot grow without bound.
const RETENTION: Duration = Duration::from_secs(90 * 86_400);
/// How long a burst of writes is allowed to gather before it is flushed.
const FLUSH_DELAY: Duration = Duration::from_secs(2);

#[derive(Serialize, Deserialize)]
struct StoredFile {
    version: u32,
    entries: BTreeMap<String, Record>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Record {
    at_ms: i64,
    /// `null` is a remembered "nothing here", with its own TTL.
    value: serde_json::Value,
}

#[derive(Default)]
struct Table {
    entries: HashMap<String, Record>,
    flush_scheduled: bool,
}

/// A stored answer, how old it is, and whether it is still inside the TTL the caller asked about.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit<T> {
    pub value: T,
    pub age: Duration,
    pub fresh: bool,
}

pub struct LookupCache {
    disk: &'static DiskCache,
    now_ms: Box<dyn Fn() -> i64 + Send + Sync>,
    /// Only `shared()` sets this, and the scheduled flush finds it again through `shared()`;
    /// `None` flushes only on an explicit `flush`, as tests want.
    flush_delay: Option<Duration>,
    tables: Mutex<HashMap<String, Table>>,
}

/// The app's lookup cache, on the app's disk cache.
pub fn shared() -> &'static LookupCache {
    static SHARED: OnceLock<LookupCache> = OnceLock::new();
    SHARED.get_or_init(|| {
        LookupCache::build(disk::global(), Box::new(system_now_ms), Some(FLUSH_DELAY))
    })
}

fn system_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as i64)
}

impl LookupCache {
    fn build(
        disk: &'static DiskCache,
        now_ms: Box<dyn Fn() -> i64 + Send + Sync>,
        flush_delay: Option<Duration>,
    ) -> Self {
        Self {
            disk,
            now_ms,
            flush_delay,
            tables: Mutex::new(HashMap::new()),
        }
    }

    #[cfg(test)]
    pub(crate) fn for_tests(
        disk: &'static DiskCache,
        now_ms: Box<dyn Fn() -> i64 + Send + Sync>,
    ) -> Self {
        Self::build(disk, now_ms, None)
    }

    /// Reads these namespaces from disk now, so the first `get` on the UI thread does no I/O.
    pub fn warm(&self, namespaces: &[&str]) {
        if let Ok(mut tables) = self.tables.lock() {
            for namespace in namespaces {
                self.table(&mut tables, namespace);
            }
        }
    }

    /// The stored answer for `key`, if any, and whether it is younger than `ttl`. A stored
    /// "nothing here" comes back as `Hit { value: None, .. }` when `T` is an `Option`.
    pub fn get<T: DeserializeOwned>(
        &self,
        namespace: &str,
        key: &str,
        ttl: Duration,
    ) -> Option<Hit<T>> {
        let now = (self.now_ms)();
        let record = {
            let mut tables = self.tables.lock().ok()?;
            self.table(&mut tables, namespace).entries.get(key)?.clone()
        };
        let value = serde_json::from_value(record.value).ok()?;
        let age = Duration::from_millis(now.saturating_sub(record.at_ms).max(0).unsigned_abs());
        Some(Hit {
            value,
            age,
            fresh: age < ttl,
        })
    }

    pub fn put<T: Serialize>(&self, namespace: &str, key: &str, value: &T) {
        let Ok(value) = serde_json::to_value(value) else {
            return;
        };
        let record = Record {
            at_ms: (self.now_ms)(),
            value,
        };
        let schedule = {
            let Ok(mut tables) = self.tables.lock() else {
                return;
            };
            let table = self.table(&mut tables, namespace);
            table.entries.insert(key.to_owned(), record);
            self.disk.is_enabled() && !std::mem::replace(&mut table.flush_scheduled, true)
        };
        if schedule {
            self.schedule_flush(namespace);
        }
    }

    /// Writes a namespace to disk now.
    pub fn flush(&self, namespace: &str) {
        // The generation is read before the snapshot: a `clear` in between makes the write drop.
        let generation = self.disk.generation();
        let snapshot = {
            let Ok(mut tables) = self.tables.lock() else {
                return;
            };
            let Some(table) = tables.get_mut(namespace) else {
                return;
            };
            table.flush_scheduled = false;
            StoredFile {
                version: VERSION,
                entries: table
                    .entries
                    .iter()
                    .map(|(key, record)| (key.clone(), record.clone()))
                    .collect(),
            }
        };
        if let Ok(bytes) = serde_json::to_vec(&snapshot) {
            self.disk
                .write(Kind::Lookups, &file_name(namespace), &bytes, generation);
        }
    }

    /// Forgets everything held in memory (after the disk cache was cleared).
    pub fn clear_memory(&self) {
        if let Ok(mut tables) = self.tables.lock() {
            tables.clear();
        }
    }

    fn schedule_flush(&self, namespace: &str) {
        let Some(delay) = self.flush_delay else {
            return;
        };
        let namespace = namespace.to_owned();
        // A thread of its own: sleeping on the shared work queue would hold one of its workers.
        std::thread::spawn(move || {
            std::thread::sleep(delay);
            shared().flush(&namespace);
        });
    }

    /// The namespace's table, loading it from disk the first time it is asked for.
    fn table<'a>(&self, tables: &'a mut HashMap<String, Table>, namespace: &str) -> &'a mut Table {
        tables
            .entry(namespace.to_owned())
            .or_insert_with(|| self.load(namespace))
    }

    fn load(&self, namespace: &str) -> Table {
        let now = (self.now_ms)();
        let retention_ms = i64::try_from(RETENTION.as_millis()).unwrap_or(i64::MAX);
        let entries = self
            .disk
            .read(Kind::Lookups, &file_name(namespace))
            .and_then(|stored| serde_json::from_slice::<StoredFile>(&stored.bytes).ok())
            .filter(|file| file.version == VERSION)
            .map(|file| {
                file.entries
                    .into_iter()
                    .filter(|(_, record)| now.saturating_sub(record.at_ms) <= retention_ms)
                    .collect()
            })
            .unwrap_or_default();
        Table {
            entries,
            flush_scheduled: false,
        }
    }
}

fn file_name(namespace: &str) -> String {
    format!("{namespace}.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::cache::disk::testing::TempCache;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicI64, Ordering};

    const DAY_MS: i64 = 86_400_000;

    struct Fixture {
        temp: TempCache,
        clock: Arc<AtomicI64>,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                temp: TempCache::new(),
                clock: Arc::new(AtomicI64::new(1_000 * DAY_MS)),
            }
        }

        /// A cache over the same directory, as a relaunch would build one.
        fn open(&self) -> LookupCache {
            let disk: &'static DiskCache = Box::leak(Box::new(DiskCache::at(self.temp.path())));
            let clock = Arc::clone(&self.clock);
            LookupCache::build(disk, Box::new(move || clock.load(Ordering::Relaxed)), None)
        }

        fn advance_days(&self, days: i64) {
            self.clock.fetch_add(days * DAY_MS, Ordering::Relaxed);
        }
    }

    fn ttl(days: u64) -> Duration {
        Duration::from_secs(days * 86_400)
    }

    #[test]
    fn an_answer_is_fresh_until_its_ttl_then_stale() {
        let fixture = Fixture::new();
        let cache = fixture.open();
        assert_eq!(cache.get::<String>("ns", "btc", ttl(1)), None);
        cache.put("ns", "btc", &"https://x/btc.png".to_owned());
        let hit = cache.get::<String>("ns", "btc", ttl(1)).unwrap();
        assert_eq!(hit.value, "https://x/btc.png");
        assert!(hit.fresh);
        fixture.advance_days(2);
        let stale = cache.get::<String>("ns", "btc", ttl(1)).unwrap();
        assert_eq!(stale.value, "https://x/btc.png");
        assert!(!stale.fresh);
        assert!(cache.get::<String>("ns", "btc", ttl(30)).unwrap().fresh);
    }

    #[test]
    fn a_remembered_miss_is_distinct_from_never_asked() {
        let fixture = Fixture::new();
        let cache = fixture.open();
        cache.put("ns", "zzz", &None::<String>);
        let hit = cache.get::<Option<String>>("ns", "zzz", ttl(1)).unwrap();
        assert_eq!(hit.value, None);
        assert!(hit.fresh);
        assert_eq!(cache.get::<Option<String>>("ns", "other", ttl(1)), None);
    }

    #[test]
    fn answers_survive_a_relaunch() {
        let fixture = Fixture::new();
        let first = fixture.open();
        first.put("ns", "a", &1_u32);
        first.put("ns", "b", &2_u32);
        first.flush("ns");

        let second = fixture.open();
        second.warm(&["ns"]);
        assert_eq!(second.get::<u32>("ns", "a", ttl(1)).unwrap().value, 1);
        assert_eq!(second.get::<u32>("ns", "b", ttl(1)).unwrap().value, 2);
        // Another namespace is its own file.
        assert_eq!(second.get::<u32>("other", "a", ttl(1)), None);
    }

    #[test]
    fn an_unwritten_put_is_not_on_disk() {
        let fixture = Fixture::new();
        fixture.open().put("ns", "a", &1_u32);
        assert_eq!(fixture.open().get::<u32>("ns", "a", ttl(1)), None);
    }

    #[test]
    fn a_corrupt_or_foreign_file_is_discarded_not_fatal() {
        let fixture = Fixture::new();
        let disk = DiskCache::at(fixture.temp.path());
        let generation = disk.generation();
        disk.write(Kind::Lookups, "broken.json", b"{not json", generation);
        disk.write(
            Kind::Lookups,
            "future.json",
            br#"{"version":99,"entries":{"a":{"at_ms":1,"value":1}}}"#,
            generation,
        );
        let cache = fixture.open();
        assert_eq!(cache.get::<u32>("broken", "a", ttl(1)), None);
        assert_eq!(cache.get::<u32>("future", "a", ttl(1)), None);
        // And the namespace is usable again afterwards.
        cache.put("broken", "a", &7_u32);
        cache.flush("broken");
        assert_eq!(
            fixture
                .open()
                .get::<u32>("broken", "a", ttl(1))
                .unwrap()
                .value,
            7
        );
    }

    #[test]
    fn entries_past_retention_are_dropped_on_load() {
        let fixture = Fixture::new();
        let first = fixture.open();
        first.put("ns", "old", &1_u32);
        fixture.advance_days(60);
        first.put("ns", "recent", &2_u32);
        first.flush("ns");
        fixture.advance_days(40); // "old" is now 100 days old, "recent" 40.
        let second = fixture.open();
        assert_eq!(second.get::<u32>("ns", "old", ttl(365)), None);
        assert_eq!(
            second.get::<u32>("ns", "recent", ttl(365)).unwrap().value,
            2
        );
    }

    #[test]
    fn a_value_of_the_wrong_shape_is_a_miss() {
        let fixture = Fixture::new();
        let cache = fixture.open();
        cache.put("ns", "a", &"text");
        assert_eq!(cache.get::<u32>("ns", "a", ttl(1)), None);
    }

    #[test]
    fn clearing_memory_forgets_and_a_pending_flush_does_not_resurrect() {
        let fixture = Fixture::new();
        let cache = fixture.open();
        cache.put("ns", "a", &1_u32);
        cache.flush("ns");
        cache.put("ns", "b", &2_u32);
        cache.disk.clear();
        cache.clear_memory();
        assert_eq!(cache.get::<u32>("ns", "a", ttl(1)), None);
        // A flush that raced the clear finds no table, and the disk is empty either way.
        cache.flush("ns");
        assert_eq!(fixture.open().get::<u32>("ns", "a", ttl(1)), None);
    }
}

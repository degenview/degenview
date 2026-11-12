//! Remote images through the disk cache: egui's `http(s)://` bytes loader, replaced.
//!
//! Every `egui::Image::new("https://…")` in the app goes through here, so logos come from memory,
//! then from disk, and only then from the network:
//!
//! 1. **Memory**: bytes already loaded this session.
//! 2. **Disk** (`cache::disk`): read synchronously — an icon is a few kilobytes — so a relaunch
//!    paints its logos on the first frame instead of after a round trip. A copy older than
//!    `SOFT_TTL` is shown as it is and refreshed in the background (stale-while-revalidate), and
//!    stays in use if that refresh fails.
//! 3. **Network**: on a dedicated worker pool, so a screenful of new logos never queues in front
//!    of a chart's candles. The body must sniff as an image, which keeps an HTML error page from
//!    ever being cached or decoded.
//!
//! A failed download is retried after `FAILURE_RETRY`; egui's own loader remembers a failure for
//! the whole session. Anything that is not `http(s)` falls through to egui's other loaders.

use crate::service::cache::disk::{self, DiskCache, Kind, digest_name};
use crate::service::work_queue;
use eframe::egui::{
    self,
    load::{Bytes, BytesLoadResult, BytesLoader, BytesPoll, LoadError},
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

/// How long a cached image is trusted before it is refreshed in the background.
const SOFT_TTL: Duration = Duration::from_secs(30 * 86_400);
/// How soon a failed download is tried again.
const FAILURE_RETRY: Duration = Duration::from_secs(180);
/// The largest image worth caching; a logo is a few kilobytes.
const MAX_IMAGE_BYTES: u64 = 1024 * 1024;

type Job = Box<dyn FnOnce() + Send + 'static>;
/// Downloads an image's bytes.
type Fetch = Box<dyn Fn(&str) -> Result<Vec<u8>, String> + Send + Sync>;

enum Entry {
    Ready {
        bytes: Arc<[u8]>,
        mime: &'static str,
    },
    Pending,
    Failed {
        retry_at: Instant,
        message: String,
    },
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Mode {
    /// Nothing usable yet: the answer goes to memory too.
    Load,
    /// A stale copy is already showing: only the disk copy is renewed.
    Refresh,
}

struct Inner {
    disk: &'static DiskCache,
    fetch: Fetch,
    spawn: Box<dyn Fn(Job) + Send + Sync>,
    failure_retry: Duration,
    entries: Mutex<HashMap<String, Entry>>,
}

pub struct CachedImageLoader {
    inner: Arc<Inner>,
}

static LIVE: OnceLock<Arc<Inner>> = OnceLock::new();

/// Puts the cached loader in front of egui's own. Call after `install_image_loaders`: egui asks
/// the most recently added loader first.
pub fn install(ctx: &egui::Context) {
    let loader = CachedImageLoader::new(
        disk::global(),
        Box::new(|url| {
            crate::service::net::http::get_bytes(url, MAX_IMAGE_BYTES)
                .map_err(|error| error.to_string())
        }),
        Box::new(|job| work_queue::IMAGES.submit(job)),
    );
    let _ = LIVE.set(Arc::clone(&loader.inner));
    ctx.add_bytes_loader(Arc::new(loader));
}

/// Forgets what the live loader holds in memory, after the disk cache was cleared.
pub fn clear_memory() {
    if let Some(inner) = LIVE.get() {
        inner.forget_all();
    }
}

impl CachedImageLoader {
    pub const ID: &'static str = egui::generate_loader_id!(CachedImageLoader);

    fn new(disk: &'static DiskCache, fetch: Fetch, spawn: Box<dyn Fn(Job) + Send + Sync>) -> Self {
        Self {
            inner: Arc::new(Inner {
                disk,
                fetch,
                spawn,
                failure_retry: FAILURE_RETRY,
                entries: Mutex::new(HashMap::new()),
            }),
        }
    }
}

impl BytesLoader for CachedImageLoader {
    fn id(&self) -> &str {
        Self::ID
    }

    fn load(&self, ctx: &egui::Context, uri: &str) -> BytesLoadResult {
        if !is_remote(uri) {
            return Err(LoadError::NotSupported);
        }
        self.inner.load(ctx, uri)
    }

    fn forget(&self, uri: &str) {
        if let Ok(mut entries) = self.inner.entries.lock() {
            entries.remove(uri);
        }
    }

    fn forget_all(&self) {
        self.inner.forget_all();
    }

    fn has_pending(&self) -> bool {
        self.inner.entries.lock().is_ok_and(|entries| {
            entries
                .values()
                .any(|entry| matches!(entry, Entry::Pending))
        })
    }

    fn byte_size(&self) -> usize {
        self.inner.entries.lock().map_or(0, |entries| {
            entries
                .values()
                .map(|entry| match entry {
                    Entry::Ready { bytes, .. } => bytes.len(),
                    _ => 0,
                })
                .sum()
        })
    }
}

fn is_remote(uri: &str) -> bool {
    uri.starts_with("https://") || uri.starts_with("http://")
}

fn ready(bytes: &Arc<[u8]>, mime: &'static str) -> BytesLoadResult {
    Ok(BytesPoll::Ready {
        size: None,
        bytes: Bytes::Shared(Arc::clone(bytes)),
        mime: Some(mime.to_owned()),
    })
}

impl Inner {
    fn forget_all(&self) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.clear();
        }
    }

    fn load(self: &Arc<Self>, ctx: &egui::Context, uri: &str) -> BytesLoadResult {
        {
            let Ok(mut entries) = self.entries.lock() else {
                return Err(LoadError::Loading("image cache unavailable".to_owned()));
            };
            match entries.get(uri) {
                Some(Entry::Ready { bytes, mime }) => return ready(bytes, mime),
                Some(Entry::Pending) => return Ok(BytesPoll::Pending { size: None }),
                Some(Entry::Failed { retry_at, message }) if Instant::now() < *retry_at => {
                    return Err(LoadError::Loading(message.clone()));
                }
                _ => {}
            }
            entries.insert(uri.to_owned(), Entry::Pending);
        }

        if let Some(stored) = self.disk.read(Kind::Images, &digest_name(uri))
            && let Some(mime) = sniff_mime(&stored.bytes)
        {
            let stale = stored.age(SystemTime::now()) > SOFT_TTL;
            let bytes: Arc<[u8]> = stored.bytes.into();
            let result = ready(&bytes, mime);
            if let Ok(mut entries) = self.entries.lock() {
                entries.insert(uri.to_owned(), Entry::Ready { bytes, mime });
            }
            if stale {
                self.spawn_fetch(ctx, uri, Mode::Refresh);
            }
            return result;
        }
        self.spawn_fetch(ctx, uri, Mode::Load);
        Ok(BytesPoll::Pending { size: None })
    }

    fn spawn_fetch(self: &Arc<Self>, ctx: &egui::Context, uri: &str, mode: Mode) {
        // Read before the download: a `clear` while it runs makes the write below drop.
        let generation = self.disk.generation();
        let inner = Arc::clone(self);
        let ctx = ctx.clone();
        let uri = uri.to_owned();
        (self.spawn)(Box::new(move || {
            let fetched = (inner.fetch)(&uri).and_then(|bytes| {
                sniff_mime(&bytes)
                    .map(|mime| (bytes, mime))
                    .ok_or_else(|| "not an image".to_owned())
            });
            match fetched {
                Ok((bytes, mime)) => {
                    inner
                        .disk
                        .write(Kind::Images, &digest_name(&uri), &bytes, generation);
                    if mode == Mode::Load {
                        inner.settle(&uri, |entry| {
                            *entry = Entry::Ready {
                                bytes: bytes.into(),
                                mime,
                            };
                        });
                    }
                }
                // A failed refresh leaves the stale copy showing.
                Err(message) if mode == Mode::Load => {
                    let retry_at = Instant::now() + inner.failure_retry;
                    inner.settle(&uri, |entry| {
                        *entry = Entry::Failed { retry_at, message };
                    });
                }
                Err(_) => {}
            }
            ctx.request_repaint();
        }));
    }

    /// Applies `update` to a request still waiting for its answer. If the entry is gone (the
    /// cache was cleared, or the image forgotten) the answer is no longer wanted.
    fn settle(&self, uri: &str, update: impl FnOnce(&mut Entry)) {
        if let Ok(mut entries) = self.entries.lock()
            && let Some(entry) = entries.get_mut(uri)
            && matches!(entry, Entry::Pending)
        {
            update(entry);
        }
    }
}

/// The MIME type of an image by its leading bytes, or `None` for anything else — an HTML error
/// page, an empty body, a truncated download.
pub fn sniff_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else if is_svg(bytes) {
        Some("image/svg+xml")
    } else {
        None
    }
}

fn is_svg(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(512)];
    let text = String::from_utf8_lossy(head);
    let text = text.trim_start_matches('\u{feff}').trim_start();
    text.starts_with("<svg") || (text.starts_with("<?xml") && text.contains("<svg"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::cache::disk::testing::TempCache;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nrest-of-file";
    const URL: &str = "https://cdn.example/btc.png?1";

    struct Fixture {
        temp: TempCache,
        fetches: Arc<AtomicUsize>,
        /// What the next download answers with.
        reply: Arc<Mutex<Result<Vec<u8>, String>>>,
        /// Jobs held back, when a test wants to run them later.
        deferred: Arc<Mutex<Vec<Job>>>,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                temp: TempCache::new(),
                fetches: Arc::new(AtomicUsize::new(0)),
                reply: Arc::new(Mutex::new(Ok(PNG.to_vec()))),
                deferred: Arc::new(Mutex::new(Vec::new())),
            }
        }

        /// A loader over the same directory, as a relaunch would build one. Downloads run
        /// inline, unless `defer` holds them for `run_deferred`.
        fn loader(&self, defer: bool) -> CachedImageLoader {
            let disk: &'static DiskCache = Box::leak(Box::new(DiskCache::at(self.temp.path())));
            let (fetches, reply) = (Arc::clone(&self.fetches), Arc::clone(&self.reply));
            let deferred = Arc::clone(&self.deferred);
            let mut loader = CachedImageLoader::new(
                disk,
                Box::new(move |_| {
                    fetches.fetch_add(1, Ordering::SeqCst);
                    reply.lock().unwrap().clone()
                }),
                Box::new(move |job| {
                    if defer {
                        deferred.lock().unwrap().push(job);
                    } else {
                        job();
                    }
                }),
            );
            Arc::get_mut(&mut loader.inner).unwrap().failure_retry = Duration::ZERO;
            loader
        }

        fn run_deferred(&self) {
            let jobs: Vec<Job> = self.deferred.lock().unwrap().drain(..).collect();
            for job in jobs {
                job();
            }
        }

        fn fetches(&self) -> usize {
            self.fetches.load(Ordering::SeqCst)
        }
    }

    fn is_ready(result: &BytesLoadResult) -> bool {
        matches!(result, Ok(BytesPoll::Ready { .. }))
    }

    fn is_pending(result: &BytesLoadResult) -> bool {
        matches!(result, Ok(BytesPoll::Pending { .. }))
    }

    fn age_on_disk(fixture: &Fixture, url: &str, age: Duration) {
        let path = fixture
            .temp
            .path()
            .join("images")
            .join(&digest_name(url)[..2])
            .join(digest_name(url));
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(SystemTime::now() - age)
            .unwrap();
    }

    #[test]
    fn a_miss_downloads_once_then_serves_from_memory_with_its_mime() {
        let fixture = Fixture::new();
        let loader = fixture.loader(false);
        let ctx = egui::Context::default();
        assert!(is_pending(&loader.load(&ctx, URL)));
        let Ok(BytesPoll::Ready { bytes, mime, .. }) = loader.load(&ctx, URL) else {
            panic!("ready after the download");
        };
        assert_eq!(bytes.as_ref(), PNG);
        assert_eq!(mime.as_deref(), Some("image/png"));
        assert!(is_ready(&loader.load(&ctx, URL)));
        assert_eq!(fixture.fetches(), 1);
    }

    #[test]
    fn a_relaunch_paints_from_disk_on_the_first_call_without_the_network() {
        let fixture = Fixture::new();
        let ctx = egui::Context::default();
        fixture.loader(false).load(&ctx, URL).ok();

        let relaunched = fixture.loader(false);
        assert!(is_ready(&relaunched.load(&ctx, URL)));
        assert_eq!(fixture.fetches(), 1, "no second download");
    }

    #[test]
    fn a_stale_copy_shows_at_once_and_is_renewed_in_the_background() {
        let fixture = Fixture::new();
        let ctx = egui::Context::default();
        fixture.loader(false).load(&ctx, URL).ok();
        age_on_disk(&fixture, URL, Duration::from_secs(40 * 86_400));
        *fixture.reply.lock().unwrap() = Ok(b"GIF89anew".to_vec());

        let relaunched = fixture.loader(false);
        assert!(is_ready(&relaunched.load(&ctx, URL)));
        assert_eq!(fixture.fetches(), 2, "one refresh");
        // The renewed copy is what the next launch sees.
        let stored = relaunched
            .inner
            .disk
            .read(Kind::Images, &digest_name(URL))
            .unwrap();
        assert_eq!(stored.bytes, b"GIF89anew");
        assert!(stored.age(SystemTime::now()) < Duration::from_secs(60));
    }

    #[test]
    fn a_failed_refresh_keeps_the_stale_copy() {
        let fixture = Fixture::new();
        let ctx = egui::Context::default();
        fixture.loader(false).load(&ctx, URL).ok();
        age_on_disk(&fixture, URL, Duration::from_secs(40 * 86_400));
        *fixture.reply.lock().unwrap() = Err("offline".to_owned());

        let relaunched = fixture.loader(false);
        assert!(is_ready(&relaunched.load(&ctx, URL)));
        assert!(is_ready(&relaunched.load(&ctx, URL)));
        let stored = relaunched
            .inner
            .disk
            .read(Kind::Images, &digest_name(URL))
            .unwrap();
        assert_eq!(stored.bytes, PNG);
    }

    #[test]
    fn a_failed_download_is_an_error_now_and_retried_later_not_pinned() {
        let fixture = Fixture::new();
        let loader = fixture.loader(false);
        let ctx = egui::Context::default();
        *fixture.reply.lock().unwrap() = Err("offline".to_owned());
        assert!(is_pending(&loader.load(&ctx, URL)));
        // The retry delay has passed (zero in this fixture): the network is asked again.
        *fixture.reply.lock().unwrap() = Ok(PNG.to_vec());
        assert!(is_pending(&loader.load(&ctx, URL)));
        assert!(is_ready(&loader.load(&ctx, URL)));
        assert_eq!(fixture.fetches(), 2);
    }

    #[test]
    fn a_failure_waits_out_its_retry_delay() {
        let fixture = Fixture::new();
        let mut loader = fixture.loader(false);
        Arc::get_mut(&mut loader.inner).unwrap().failure_retry = Duration::from_secs(3_600);
        let ctx = egui::Context::default();
        *fixture.reply.lock().unwrap() = Err("offline".to_owned());
        loader.load(&ctx, URL).ok();
        for _ in 0..3 {
            assert!(matches!(
                loader.load(&ctx, URL),
                Err(LoadError::Loading(message)) if message == "offline"
            ));
        }
        assert_eq!(fixture.fetches(), 1);
    }

    #[test]
    fn a_body_that_is_not_an_image_is_never_cached() {
        let fixture = Fixture::new();
        let mut loader = fixture.loader(false);
        Arc::get_mut(&mut loader.inner).unwrap().failure_retry = Duration::from_secs(3_600);
        let ctx = egui::Context::default();
        *fixture.reply.lock().unwrap() = Ok(b"<!doctype html><title>429</title>".to_vec());
        loader.load(&ctx, URL).ok();
        assert!(loader.load(&ctx, URL).is_err());
        assert!(
            loader
                .inner
                .disk
                .read(Kind::Images, &digest_name(URL))
                .is_none()
        );
    }

    #[test]
    fn other_schemes_are_left_to_the_other_loaders() {
        let fixture = Fixture::new();
        let loader = fixture.loader(false);
        let ctx = egui::Context::default();
        for uri in [
            "file:///tmp/a.png",
            "bytes://logo",
            "ftp://x/a.png",
            "a.png",
        ] {
            assert!(matches!(
                loader.load(&ctx, uri),
                Err(LoadError::NotSupported)
            ));
        }
        assert_eq!(fixture.fetches(), 0);
    }

    #[test]
    fn forgetting_memory_reloads_from_disk_not_the_network() {
        let fixture = Fixture::new();
        let loader = fixture.loader(false);
        let ctx = egui::Context::default();
        loader.load(&ctx, URL).ok();
        assert!(loader.byte_size() > 0);
        loader.forget_all();
        assert_eq!(loader.byte_size(), 0);
        assert!(is_ready(&loader.load(&ctx, URL)));
        loader.forget(URL);
        assert!(is_ready(&loader.load(&ctx, URL)));
        assert_eq!(fixture.fetches(), 1);
    }

    #[test]
    fn a_download_that_finishes_after_a_clear_leaves_no_trace() {
        let fixture = Fixture::new();
        let loader = fixture.loader(true);
        let ctx = egui::Context::default();
        assert!(is_pending(&loader.load(&ctx, URL)));
        // The user clears the cache while the download is in flight.
        loader.inner.disk.clear();
        loader.inner.forget_all();
        fixture.run_deferred();

        assert_eq!(fixture.fetches(), 1);
        assert!(
            loader
                .inner
                .disk
                .read(Kind::Images, &digest_name(URL))
                .is_none()
        );
        assert_eq!(loader.byte_size(), 0);
        // Asking again starts over.
        assert!(is_pending(&loader.load(&ctx, URL)));
    }

    #[test]
    fn mime_types_are_sniffed_from_the_leading_bytes() {
        assert_eq!(sniff_mime(PNG), Some("image/png"));
        assert_eq!(sniff_mime(&[0xFF, 0xD8, 0xFF, 0xE0, 0]), Some("image/jpeg"));
        assert_eq!(sniff_mime(b"GIF89a.."), Some("image/gif"));
        assert_eq!(sniff_mime(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(sniff_mime(b"  <svg xmlns=\"x\"/>"), Some("image/svg+xml"));
        assert_eq!(
            sniff_mime(b"<?xml version=\"1.0\"?>\n<svg/>"),
            Some("image/svg+xml")
        );
        for rejected in [
            &b""[..],
            b"<!doctype html><html><svg></svg></html>",
            b"{\"error\":\"rate limited\"}",
            b"RIFF\0\0\0\0WAVE",
            b"\x89PNG",
        ] {
            assert_eq!(sniff_mime(rejected), None, "{rejected:?}");
        }
    }
}

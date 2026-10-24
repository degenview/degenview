//! Allocation counters for the Pine benchmarks (`src/pine/bench_tests.rs`).
//!
//! The counting allocator is installed only for `cargo test --features alloc-count`, because a
//! global allocator wraps every allocation in the test binary. Without the feature
//! [`snapshot`] returns zeros and the benchmarks print no allocation columns.

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Stats {
    /// Allocations made since the process started.
    pub allocations: u64,
    /// Bytes requested since the process started.
    pub bytes: u64,
    /// The most bytes live at once since the last [`reset_peak`].
    pub peak: u64,
}

#[cfg(feature = "alloc-count")]
mod counting {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

    static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);
    static BYTES: AtomicU64 = AtomicU64::new(0);
    static LIVE: AtomicU64 = AtomicU64::new(0);
    static PEAK: AtomicU64 = AtomicU64::new(0);

    struct Counting;

    // SAFETY: every method forwards to `System` unchanged and only adds relaxed counter updates.
    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            note_alloc(layout.size());
            unsafe { System.alloc(layout) }
        }

        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            note_alloc(layout.size());
            unsafe { System.alloc_zeroed(layout) }
        }

        unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
            LIVE.fetch_sub(layout.size() as u64, Relaxed);
            unsafe { System.dealloc(pointer, layout) }
        }

        unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            LIVE.fetch_sub(layout.size() as u64, Relaxed);
            note_alloc(new_size);
            unsafe { System.realloc(pointer, layout, new_size) }
        }
    }

    fn note_alloc(size: usize) {
        ALLOCATIONS.fetch_add(1, Relaxed);
        BYTES.fetch_add(size as u64, Relaxed);
        let live = LIVE.fetch_add(size as u64, Relaxed) + size as u64;
        PEAK.fetch_max(live, Relaxed);
    }

    #[global_allocator]
    static ALLOCATOR: Counting = Counting;

    pub fn snapshot() -> super::Stats {
        super::Stats {
            allocations: ALLOCATIONS.load(Relaxed),
            bytes: BYTES.load(Relaxed),
            peak: PEAK.load(Relaxed),
        }
    }

    pub fn reset_peak() {
        PEAK.store(LIVE.load(Relaxed), Relaxed);
    }
}

#[cfg(feature = "alloc-count")]
pub use counting::{reset_peak, snapshot};

#[cfg(not(feature = "alloc-count"))]
pub fn snapshot() -> Stats {
    Stats::default()
}

#[cfg(not(feature = "alloc-count"))]
pub fn reset_peak() {}

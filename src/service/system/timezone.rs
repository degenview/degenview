//! The viewer's UTC offset, for showing and typing dates in local time. macOS only (through
//! `localtime_r`); elsewhere, and under test, it is UTC so results are deterministic.
//!
//! Tables ask once per cell every frame, so on macOS answers are kept per minute of the instant
//! asked about (zones change offset on whole minutes) and all dropped once a minute, so a change
//! of the system zone shows within a minute.

/// Seconds east of UTC at `epoch_ms`, in the system time zone (DST included).
pub fn local_utc_offset_secs(epoch_ms: i64) -> i64 {
    if cfg!(test) {
        return 0;
    }
    let epoch_secs = epoch_ms.div_euclid(1_000);
    #[cfg(target_os = "macos")]
    {
        static OFFSETS: OffsetCache = OffsetCache::new();
        OFFSETS.offset(epoch_secs, std::time::Instant::now(), platform::offset)
    }
    #[cfg(not(target_os = "macos"))]
    {
        platform::offset(epoch_secs)
    }
}

/// Offsets already asked for, by the minute of the instant, since `started`.
#[cfg(any(target_os = "macos", test))]
struct OffsetCache {
    state: std::sync::Mutex<Option<OffsetCacheState>>,
}

#[cfg(any(target_os = "macos", test))]
struct OffsetCacheState {
    started: std::time::Instant,
    by_minute: std::collections::HashMap<i64, i64>,
}

#[cfg(any(target_os = "macos", test))]
impl OffsetCache {
    /// How long answers are kept: a change of the system zone shows within this.
    const REFRESH: std::time::Duration = std::time::Duration::from_secs(60);
    /// Minutes kept at most; a long scroll through history starts the map again.
    const CAPACITY: usize = 4_096;

    const fn new() -> Self {
        Self {
            state: std::sync::Mutex::new(None),
        }
    }

    /// The offset at the start of `epoch_secs`'s minute, from `source` unless it was asked for
    /// within `REFRESH` of `now`.
    fn offset(
        &self,
        epoch_secs: i64,
        now: std::time::Instant,
        source: impl FnOnce(i64) -> i64,
    ) -> i64 {
        let minute = epoch_secs.div_euclid(60);
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let state = state.get_or_insert_with(|| OffsetCacheState {
            started: now,
            by_minute: std::collections::HashMap::new(),
        });
        if now.saturating_duration_since(state.started) >= Self::REFRESH
            || state.by_minute.len() >= Self::CAPACITY
        {
            state.started = now;
            state.by_minute.clear();
        }
        *state
            .by_minute
            .entry(minute)
            .or_insert_with(|| source(minute * 60))
    }
}

#[cfg(any(target_os = "macos", test))]
/// A sane offset: real zones sit within −12h…+14h.
pub fn clamp_offset(secs: i64) -> i64 {
    secs.clamp(-12 * 3_600, 14 * 3_600)
}

#[cfg(target_os = "macos")]
mod platform {
    use std::ffi::{c_char, c_int, c_long};

    /// `struct tm` as macOS lays it out.
    #[repr(C)]
    struct Tm {
        tm_sec: c_int,
        tm_min: c_int,
        tm_hour: c_int,
        tm_mday: c_int,
        tm_mon: c_int,
        tm_year: c_int,
        tm_wday: c_int,
        tm_yday: c_int,
        tm_isdst: c_int,
        tm_gmtoff: c_long,
        tm_zone: *const c_char,
    }

    unsafe extern "C" {
        fn localtime_r(time: *const i64, result: *mut Tm) -> *mut Tm;
    }

    pub fn offset(epoch_secs: i64) -> i64 {
        let mut tm = Tm {
            tm_sec: 0,
            tm_min: 0,
            tm_hour: 0,
            tm_mday: 0,
            tm_mon: 0,
            tm_year: 0,
            tm_wday: 0,
            tm_yday: 0,
            tm_isdst: 0,
            tm_gmtoff: 0,
            tm_zone: std::ptr::null(),
        };
        // SAFETY: `epoch_secs` and `tm` are valid for the call; `localtime_r` writes only into
        // `tm` and returns null on failure, in which case `tm_gmtoff` stays zero.
        let result = unsafe { localtime_r(&raw const epoch_secs, &raw mut tm) };
        if result.is_null() {
            return 0;
        }
        super::clamp_offset(tm.tm_gmtoff)
    }

    #[cfg(test)]
    pub fn offset_for_test(epoch_secs: i64) -> i64 {
        offset(epoch_secs)
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    pub fn offset(_: i64) -> i64 {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn under_test_the_zone_is_utc() {
        assert_eq!(local_utc_offset_secs(1_709_649_000_000), 0);
    }

    #[test]
    fn offsets_are_asked_for_once_a_minute_of_the_instant() {
        use std::cell::Cell;
        use std::time::{Duration, Instant};
        let cache = OffsetCache::new();
        let start = Instant::now();
        let asked = Cell::new(Vec::new());
        let source = |secs: i64| {
            let mut seen = asked.take();
            seen.push(secs);
            asked.set(seen);
            // A zone that moves an hour forward at 120 s.
            if secs < 120 { 3_600 } else { 7_200 }
        };
        assert_eq!(cache.offset(61, start, source), 3_600);
        // The same minute again, even a second later: no new question.
        assert_eq!(
            cache.offset(119, start + Duration::from_secs(1), source),
            3_600
        );
        // The next minute is the zone's new offset, asked at the minute's start.
        assert_eq!(cache.offset(150, start, source), 7_200);
        assert_eq!(asked.take(), vec![60, 120]);
        // A minute later every answer is asked for again, so a new system zone shows.
        assert_eq!(
            cache.offset(61, start + Duration::from_secs(60), source),
            3_600
        );
        assert_eq!(asked.take(), vec![60]);
    }

    #[test]
    fn offsets_before_1970_use_the_minute_they_fall_in() {
        let cache = OffsetCache::new();
        let now = std::time::Instant::now();
        assert_eq!(cache.offset(-1, now, |secs| secs), -60);
    }

    #[test]
    fn offsets_are_kept_within_real_zones() {
        assert_eq!(clamp_offset(3_600), 3_600);
        assert_eq!(clamp_offset(-20 * 3_600), -12 * 3_600);
        assert_eq!(clamp_offset(20 * 3_600), 14 * 3_600);
    }

    /// The real system call returns a plausible offset for any instant.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_system_zone_gives_a_plausible_offset() {
        for epoch in [0, 1_709_649_000, 1_720_000_000, 4_102_444_800] {
            let offset = platform::offset_for_test(epoch);
            assert!((-12 * 3_600..=14 * 3_600).contains(&offset), "{offset}");
            assert_eq!(offset % 900, 0, "zones sit on quarter hours: {offset}");
        }
    }
}

//! The viewer's UTC offset, for showing and typing dates in local time. macOS only (through
//! `localtime_r`); elsewhere, and under test, it is UTC so results are deterministic.

// Transient: the portfolio host reads this; other screens adopt it as they need local time.
#![allow(dead_code)]

/// Seconds east of UTC at `epoch_ms`, in the system time zone (DST included).
pub fn local_utc_offset_secs(epoch_ms: i64) -> i64 {
    if cfg!(test) {
        return 0;
    }
    platform::offset(epoch_ms.div_euclid(1_000))
}

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

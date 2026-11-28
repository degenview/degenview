//! The time zone used on chart time axes, crosshair tags, bar markers, and bar tables.
//!
//! Kept in `model` so every layer (`model`, `ui`, `host`, `app`) can read the active choice
//! without importing `service`. The system offset provider is registered once at startup from
//! `lib.rs`, matching the pattern used by `model::pine::bundled`.

use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

/// Time zone shown on chart scales, tooltips, and bar tables.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChartTimezone {
    #[default]
    System,
    Utc,
    LosAngeles,
    Chicago,
    NewYork,
    SaoPaulo,
    London,
    Frankfurt,
    Athens,
    Moscow,
    Dubai,
    Mumbai,
    Bangkok,
    Singapore,
    Tokyo,
    Sydney,
}

impl ChartTimezone {
    /// Every selectable time zone in display order.
    pub const ALL: [Self; 16] = [
        Self::System,
        Self::Utc,
        Self::LosAngeles,
        Self::Chicago,
        Self::NewYork,
        Self::SaoPaulo,
        Self::London,
        Self::Frankfurt,
        Self::Athens,
        Self::Moscow,
        Self::Dubai,
        Self::Mumbai,
        Self::Bangkok,
        Self::Singapore,
        Self::Tokyo,
        Self::Sydney,
    ];

    /// Human-readable label shown in Settings.
    pub fn label(self) -> &'static str {
        match self {
            Self::System => crate::i18n::tr!("Same as system"),
            Self::Utc => crate::i18n::tr!("(UTC+0) UTC"),
            Self::LosAngeles => crate::i18n::tr!("(UTC-8) Los Angeles"),
            Self::Chicago => crate::i18n::tr!("(UTC-6) Chicago"),
            Self::NewYork => crate::i18n::tr!("(UTC-5) New York"),
            Self::SaoPaulo => crate::i18n::tr!("(UTC-3) São Paulo"),
            Self::London => crate::i18n::tr!("(UTC+0) London"),
            Self::Frankfurt => crate::i18n::tr!("(UTC+1) Frankfurt"),
            Self::Athens => crate::i18n::tr!("(UTC+2) Athens"),
            Self::Moscow => crate::i18n::tr!("(UTC+3) Moscow"),
            Self::Dubai => crate::i18n::tr!("(UTC+4) Dubai"),
            Self::Mumbai => crate::i18n::tr!("(UTC+5:30) Mumbai"),
            Self::Bangkok => crate::i18n::tr!("(UTC+7) Bangkok"),
            Self::Singapore => crate::i18n::tr!("(UTC+8) Singapore"),
            Self::Tokyo => crate::i18n::tr!("(UTC+9) Tokyo"),
            Self::Sydney => crate::i18n::tr!("(UTC+10) Sydney"),
        }
    }

    /// Fixed seconds east of UTC, or `None` when following the system time zone.
    pub const fn fixed_offset_secs(self) -> Option<i64> {
        match self {
            Self::System => None,
            Self::Utc | Self::London => Some(0),
            Self::LosAngeles => Some(-8 * 3_600),
            Self::Chicago => Some(-6 * 3_600),
            Self::NewYork => Some(-5 * 3_600),
            Self::SaoPaulo => Some(-3 * 3_600),
            Self::Frankfurt => Some(3_600),
            Self::Athens => Some(2 * 3_600),
            Self::Moscow => Some(3 * 3_600),
            Self::Dubai => Some(4 * 3_600),
            Self::Mumbai => Some(5 * 3_600 + 1_800),
            Self::Bangkok => Some(7 * 3_600),
            Self::Singapore => Some(8 * 3_600),
            Self::Tokyo => Some(9 * 3_600),
            Self::Sydney => Some(10 * 3_600),
        }
    }

    pub const fn index(self) -> u8 {
        match self {
            Self::System => 0,
            Self::Utc => 1,
            Self::LosAngeles => 2,
            Self::Chicago => 3,
            Self::NewYork => 4,
            Self::SaoPaulo => 5,
            Self::London => 6,
            Self::Frankfurt => 7,
            Self::Athens => 8,
            Self::Moscow => 9,
            Self::Dubai => 10,
            Self::Mumbai => 11,
            Self::Bangkok => 12,
            Self::Singapore => 13,
            Self::Tokyo => 14,
            Self::Sydney => 15,
        }
    }

    pub const fn from_index(index: u8) -> Self {
        let index = index as usize;
        if index < Self::ALL.len() {
            Self::ALL[index]
        } else {
            Self::System
        }
    }
}

/// Active chart time zone and its change revision packed into one atomic word:
/// bits 0-7 hold the `ChartTimezone` index, bits 16-63 hold the revision counter.
struct Active(AtomicU64);

const TZ_MASK: u64 = 0xff;
const REVISION_SHIFT: u32 = 16;

impl Active {
    const fn new() -> Self {
        Self(AtomicU64::new(0))
    }

    fn timezone(&self) -> ChartTimezone {
        ChartTimezone::from_index((self.0.load(Ordering::Acquire) & TZ_MASK) as u8)
    }

    fn revision(&self) -> u64 {
        self.0.load(Ordering::Acquire) >> REVISION_SHIFT
    }

    fn set(&self, tz: ChartTimezone) {
        let state = u64::from(tz.index());
        // A repeat of what is already set is not a change, and repaints nothing.
        let _ = self
            .0
            .try_update(Ordering::AcqRel, Ordering::Acquire, |old| {
                let unchanged = old & TZ_MASK;
                (unchanged != state)
                    .then(|| state | ((old >> REVISION_SHIFT) + 1) << REVISION_SHIFT)
            });
    }
}

static ACTIVE: Active = Active::new();
static SYSTEM_OFFSET_PROVIDER: OnceLock<fn(i64) -> i64> = OnceLock::new();

/// Registers the function that resolves the system UTC offset at a given epoch millisecond.
/// Called once at startup from `lib.rs`.
pub fn set_system_offset_provider(provider: fn(i64) -> i64) {
    let _ = SYSTEM_OFFSET_PROVIDER.set(provider);
}

/// The chart time zone currently in use.
pub fn active_timezone() -> ChartTimezone {
    ACTIVE.timezone()
}

/// Switches the active chart time zone, bumping [`revision`] when it actually changed.
pub fn set_active_timezone(tz: ChartTimezone) {
    ACTIVE.set(tz);
}

/// Changes whenever the active chart time zone changes, so open windows know to repaint.
pub fn revision() -> u64 {
    ACTIVE.revision()
}

/// Seconds east of UTC at `epoch_ms` for the active chart time zone, falling back to
/// `system_offset_fn(epoch_ms)` when `ChartTimezone::System` is selected.
pub fn active_offset_secs(epoch_ms: i64, system_offset_fn: impl FnOnce(i64) -> i64) -> i64 {
    active_timezone()
        .fixed_offset_secs()
        .unwrap_or_else(|| system_offset_fn(epoch_ms))
}

/// Seconds east of UTC at `epoch_ms` using the registered system offset provider when
/// `ChartTimezone::System` is active (or `0` if none has been registered, as in unit tests).
pub fn effective_offset_secs(epoch_ms: i64) -> i64 {
    active_offset_secs(epoch_ms, |ms| {
        SYSTEM_OFFSET_PROVIDER
            .get()
            .map_or(0, |provider| provider(ms))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_timezones_round_trip_index_and_serde() {
        assert_eq!(ChartTimezone::ALL.len(), 16);
        for (expected_idx, tz) in ChartTimezone::ALL.iter().copied().enumerate() {
            assert_eq!(tz.index() as usize, expected_idx);
            assert_eq!(ChartTimezone::from_index(tz.index()), tz);
            let json = serde_json::to_string(&tz).expect("serialize");
            let decoded: ChartTimezone = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(decoded, tz);
        }
        assert_eq!(ChartTimezone::from_index(255), ChartTimezone::System);
    }

    #[test]
    fn active_bumps_revision_only_on_change() {
        let active = Active::new();
        assert_eq!(active.timezone(), ChartTimezone::System);
        assert_eq!(active.revision(), 0);

        active.set(ChartTimezone::Tokyo);
        assert_eq!(active.timezone(), ChartTimezone::Tokyo);
        assert_eq!(active.revision(), 1);

        active.set(ChartTimezone::Tokyo);
        assert_eq!(active.revision(), 1);

        active.set(ChartTimezone::Mumbai);
        assert_eq!(active.timezone(), ChartTimezone::Mumbai);
        assert_eq!(active.revision(), 2);
    }

    #[test]
    fn fixed_offsets_match_expected_values() {
        assert_eq!(ChartTimezone::System.fixed_offset_secs(), None);
        assert_eq!(ChartTimezone::Utc.fixed_offset_secs(), Some(0));
        assert_eq!(ChartTimezone::NewYork.fixed_offset_secs(), Some(-18_000));
        assert_eq!(ChartTimezone::Moscow.fixed_offset_secs(), Some(10_800));
        assert_eq!(ChartTimezone::Mumbai.fixed_offset_secs(), Some(19_800));
        assert_eq!(ChartTimezone::Tokyo.fixed_offset_secs(), Some(32_400));
        assert_eq!(ChartTimezone::Sydney.fixed_offset_secs(), Some(36_000));
    }
}

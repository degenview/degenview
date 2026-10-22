//! Pure retry policy for the upload side: when to try again, and what each failure means. No
//! tight retry loops, and the server's own retry timing is always respected.

use super::transport::ErrorKind;

pub const BACKOFF_BASE_MS: i64 = 1_000;
pub const BACKOFF_MAX_MS: i64 = 15 * 60 * 1_000;
/// Quota and permission problems do not fix themselves in seconds; check back hourly.
pub const PERSISTENT_RETRY_MS: i64 = 60 * 60 * 1_000;

/// Exponential backoff with deterministic jitter (up to 25%) derived from the entry's `seq`, so
/// many entries failing together do not retry in lockstep. A server `retry_after` is a floor.
pub fn backoff_ms(attempts: i64, seq: i64, retry_after_s: Option<u64>) -> i64 {
    let exponent = attempts.clamp(0, 20) as u32;
    let base = BACKOFF_BASE_MS
        .saturating_mul(1_i64 << exponent)
        .min(BACKOFF_MAX_MS);
    let jitter_permille = (seq.unsigned_abs().wrapping_mul(2_654_435_761) % 250) as i64;
    let delay = base + base * jitter_permille / 1_000;
    let server = retry_after_s.map_or(0, |seconds| (seconds as i64).saturating_mul(1_000));
    delay.max(server)
}

/// What to do with an outbox entry after a failed upload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    /// Try again at the given delay.
    RetryAfter(i64),
    /// Create the zone, then try again without counting an attempt.
    EnsureZone,
    /// Reconcile with the cloud's version.
    Conflict,
    /// The cloud no longer has the record this edit was based on; save it as new.
    Recreate,
    /// Start over from a fresh fetch.
    StaleToken,
    /// Cannot be sent as is; needs the user.
    Block,
    /// No usable account right now; keep everything and wait for one.
    WaitForAccount,
    /// A condition that needs the user or time, rechecked slowly.
    Persistent(i64),
}

pub fn classify(
    kind: ErrorKind,
    attempts: i64,
    seq: i64,
    retry_after_s: Option<u64>,
) -> Disposition {
    match kind {
        ErrorKind::Network | ErrorKind::Unknown | ErrorKind::RateLimited => {
            Disposition::RetryAfter(backoff_ms(attempts, seq, retry_after_s))
        }
        ErrorKind::ZoneNotFound => Disposition::EnsureZone,
        ErrorKind::ServerRecordChanged => Disposition::Conflict,
        ErrorKind::UnknownItem => Disposition::Recreate,
        ErrorKind::ChangeTokenExpired => Disposition::StaleToken,
        ErrorKind::TooLarge => Disposition::Block,
        ErrorKind::AccountUnavailable => Disposition::WaitForAccount,
        ErrorKind::Quota | ErrorKind::Permission => Disposition::Persistent(
            PERSISTENT_RETRY_MS.max(retry_after_s.map_or(0, |s| s as i64 * 1_000)),
        ),
    }
}

/// Whether the user should be told about a failure at all. Transient trouble stays quiet; the
/// status line shows it only if it persists.
pub fn is_transient(kind: ErrorKind) -> bool {
    matches!(
        kind,
        ErrorKind::Network
            | ErrorKind::RateLimited
            | ErrorKind::ZoneNotFound
            | ErrorKind::ServerRecordChanged
            | ErrorKind::UnknownItem
            | ErrorKind::ChangeTokenExpired
            | ErrorKind::Unknown
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_then_caps() {
        let delays: Vec<_> = (0..14)
            .map(|attempt| backoff_ms(attempt, 7, None))
            .collect();
        assert!(
            delays.windows(2).all(|pair| pair[0] <= pair[1]),
            "{delays:?}"
        );
        assert!(delays[0] >= BACKOFF_BASE_MS);
        assert!(delays[0] < BACKOFF_BASE_MS * 2);
        assert!(*delays.last().unwrap() <= BACKOFF_MAX_MS + BACKOFF_MAX_MS / 4);
        assert!(*delays.last().unwrap() >= BACKOFF_MAX_MS);
    }

    #[test]
    fn jitter_is_bounded_and_deterministic() {
        for seq in 0..500 {
            let delay = backoff_ms(3, seq, None);
            assert!((8_000..=10_000).contains(&delay), "{delay}");
            assert_eq!(delay, backoff_ms(3, seq, None));
        }
        let distinct: std::collections::HashSet<_> =
            (0..500).map(|seq| backoff_ms(3, seq, None)).collect();
        assert!(distinct.len() > 20);
    }

    #[test]
    fn the_servers_retry_after_is_a_floor() {
        assert_eq!(backoff_ms(0, 1, Some(120)), 120_000);
        assert!(backoff_ms(10, 1, Some(1)) > 1_000);
    }

    #[test]
    fn absurd_attempt_counts_do_not_overflow() {
        assert!(backoff_ms(i64::MAX, i64::MIN, None) <= BACKOFF_MAX_MS * 2);
        assert!(backoff_ms(-5, -5, None) >= BACKOFF_BASE_MS);
    }

    #[test]
    fn failures_are_classified() {
        assert!(matches!(
            classify(ErrorKind::Network, 0, 1, None),
            Disposition::RetryAfter(_)
        ));
        assert_eq!(
            classify(ErrorKind::ZoneNotFound, 0, 1, None),
            Disposition::EnsureZone
        );
        assert_eq!(
            classify(ErrorKind::ServerRecordChanged, 0, 1, None),
            Disposition::Conflict
        );
        assert_eq!(
            classify(ErrorKind::UnknownItem, 0, 1, None),
            Disposition::Recreate
        );
        assert_eq!(
            classify(ErrorKind::ChangeTokenExpired, 0, 1, None),
            Disposition::StaleToken
        );
        assert_eq!(
            classify(ErrorKind::TooLarge, 0, 1, None),
            Disposition::Block
        );
        assert_eq!(
            classify(ErrorKind::AccountUnavailable, 0, 1, None),
            Disposition::WaitForAccount
        );
        assert_eq!(
            classify(ErrorKind::Quota, 0, 1, None),
            Disposition::Persistent(PERSISTENT_RETRY_MS)
        );
        assert!(
            matches!(classify(ErrorKind::RateLimited, 0, 1, Some(300)), Disposition::RetryAfter(ms) if ms >= 300_000)
        );
    }

    #[test]
    fn only_transient_errors_stay_quiet() {
        assert!(is_transient(ErrorKind::Network));
        assert!(is_transient(ErrorKind::RateLimited));
        assert!(!is_transient(ErrorKind::Quota));
        assert!(!is_transient(ErrorKind::Permission));
        assert!(!is_transient(ErrorKind::TooLarge));
    }
}

//! The version of a release: `major.minor.patch` as `tools/release.sh` writes it, ordered as SemVer
//! says. Shared by the updater (which release is newer, which channel offers it) and the changelog
//! (which sections lie between two versions), so both read a tag the same way.

/// `major.minor.patch` with an optional `v` prefix and an optional `-prerelease`/`+build` tail,
/// ordered by SemVer 2.0: a prerelease sorts before its release (`0.2.0-rc.1 < 0.2.0`) and its
/// dot-separated parts compare numerically when numeric (`rc.2 < rc.10`), else as text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Version {
    core: [u64; 3],
    prerelease: Option<String>,
}

/// What a version's prerelease part says about how settled it is.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Stability {
    Stable,
    ReleaseCandidate,
    /// `alpha`, `beta`, `nightly`, anything unrecognised: the updater offers none of these.
    Other,
}

impl Version {
    /// SemVer 2.0 with one optional `v`: three numbers without leading zeros, then an optional
    /// `-prerelease` and `+build`. Anything looser is not a tag the release tooling creates.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let text = text.strip_prefix(['v', 'V']).unwrap_or(text);
        let (text, build) = match text.split_once('+') {
            Some((text, build)) => (text, Some(build)),
            None => (text, None),
        };
        if build.is_some_and(|build| !build.split('.').all(is_identifier)) {
            return None;
        }
        let (core, prerelease) = match text.split_once('-') {
            Some((core, pre)) => (core, Some(pre)),
            None => (text, None),
        };
        let mut parts = core.split('.');
        let mut numbers = [0_u64; 3];
        for slot in &mut numbers {
            *slot = parse_number(parts.next()?)?;
        }
        if parts.next().is_some() {
            return None;
        }
        if let Some(pre) = prerelease {
            let valid = pre.split('.').all(|part| {
                is_identifier(part) && (!is_digits(part) || parse_number(part).is_some())
            });
            if !valid {
                return None;
            }
        }
        Some(Self {
            core: numbers,
            prerelease: prerelease.map(str::to_owned),
        })
    }

    /// A release candidate is exactly `rc.N` (the form `tools/release.sh` writes); `rc1`, `RC.1`
    /// and everything else after the hyphen are "other", which no channel offers.
    pub fn stability(&self) -> Stability {
        let Some(pre) = &self.prerelease else {
            return Stability::Stable;
        };
        match pre.strip_prefix("rc.") {
            Some(number) if is_digits(number) => Stability::ReleaseCandidate,
            _ => Stability::Other,
        }
    }

    /// Whether the version has a `-prerelease` part: a release candidate or anything else.
    pub fn is_prerelease(&self) -> bool {
        self.prerelease.is_some()
    }
}

fn is_digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit())
}

/// A SemVer identifier: non-empty ASCII letters, digits and hyphens.
fn is_identifier(text: &str) -> bool {
    !text.is_empty()
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

/// Digits only, no leading zero (`0` itself is fine), and small enough for a `u64`.
fn parse_number(text: &str) -> Option<u64> {
    (is_digits(text) && (text == "0" || !text.starts_with('0')))
        .then(|| text.parse().ok())
        .flatten()
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering;
        self.core
            .cmp(&other.core)
            .then_with(|| match (&self.prerelease, &other.prerelease) {
                (None, None) => Ordering::Equal,
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (Some(a), Some(b)) => cmp_prerelease(a, b),
            })
    }
}

/// SemVer 2.0 precedence of two prerelease strings: part by part, numeric parts numerically and
/// below text parts, the shorter list first when one is a prefix of the other.
fn cmp_prerelease(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let (mut a, mut b) = (a.split('.'), b.split('.'));
    loop {
        let order = match (a.next(), b.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) => match (x.parse::<u64>(), y.parse::<u64>()) {
                (Ok(x), Ok(y)) => x.cmp(&y),
                (Ok(_), Err(_)) => Ordering::Less,
                (Err(_), Ok(_)) => Ordering::Greater,
                (Err(_), Err(_)) => x.cmp(y),
            },
        };
        if order != Ordering::Equal {
            return order;
        }
    }
}

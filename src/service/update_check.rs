//! "Is there a newer DegenView?" — asks GitHub for the recent releases of `degenview/degenview`,
//! picks the newest one the chosen `Channel` accepts and compares its tag with the running version.
//! Read-only: it never downloads or installs anything, it only reports where the release page is.

use crate::service::http::{self, Retry};
use serde::Deserialize;

/// The recent releases, prereleases included (`/releases/latest` never returns one, and is "the
/// most recently created stable release", not the highest version).
const RELEASE_LIST_URL: &str = "https://api.github.com/repos/degenview/degenview/releases";
/// Releases per page of the list, and how many pages one check reads at most. The list is ordered
/// by creation, so a run of release candidates can push the newest stable release past page one.
const PAGE_SIZE: usize = 30;
const MAX_PAGES: usize = 3;
/// Where "view release" falls back to when the API reply carries no page URL.
pub const RELEASES_URL: &str = "https://github.com/degenview/degenview/releases";

/// Which releases the updater offers. A channel also takes everything more stable than itself, so
/// a Release Candidate user moves on to `0.6.0` rather than staying on `0.6.0-rc.2`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Channel {
    /// Tags without a prerelease part: `0.5.1`.
    #[default]
    Stable,
    /// Stable plus `-rc.N` tags: `0.6.0-rc.1`.
    ReleaseCandidate,
}

impl Channel {
    pub const ALL: [Self; 2] = [Self::Stable, Self::ReleaseCandidate];

    pub fn label(self) -> &'static str {
        match self {
            Self::Stable => "Stable",
            Self::ReleaseCandidate => "Release Candidate",
        }
    }

    /// The value saved in the `update.channel` setting.
    pub fn key(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::ReleaseCandidate => "rc",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|channel| channel.key() == key)
    }

    /// The channel a build of `version` came from, for a user who never chose: a release candidate
    /// keeps getting release candidates.
    pub fn of_version(version: &str) -> Self {
        match Version::parse(version).map(|version| version.kind()) {
            Some(Kind::ReleaseCandidate) => Self::ReleaseCandidate,
            _ => Self::Stable,
        }
    }

    fn accepts(self, version: &Version) -> bool {
        matches!(
            (self, version.kind()),
            (_, Kind::Stable) | (Self::ReleaseCandidate, Kind::ReleaseCandidate)
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UpdateOutcome {
    UpToDate,
    Available(Release),
    Failed(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Release {
    /// The tag as published: "v0.2.0".
    pub tag: String,
    /// The tag without a leading `v`: "0.2.0".
    pub version: String,
    pub url: String,
    /// This platform's download, when the release has one `service::self_update` can install.
    pub asset: Option<ReleaseAsset>,
    /// Why the release is only offered for manual download, when it is not installable.
    pub manual_only: Option<&'static str>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReleaseAsset {
    pub name: String,
    pub url: String,
    pub size: u64,
    /// Lowercase hex SHA-256, from the `digest` GitHub computes for every upload.
    pub sha256: String,
}

/// The largest download the updater will accept.
const MAX_ASSET_BYTES: u64 = 256 * 1024 * 1024;
/// Accounts GitHub may record as a release's author: the release workflow's bot, and nobody else.
/// The maintainer's own account is deliberately absent: a token that can act as the maintainer
/// (the one `gh` keeps on a developer machine) could otherwise publish an immutable release with
/// its own assets for a signed tag, with no workflow run and so no provenance attestation. A
/// release made by hand is a manual download.
const RELEASE_AUTHORS: &[&str] = &["github-actions[bot]"];
const DOWNLOAD_PREFIX: &str = "https://github.com/degenview/degenview/releases/download/";

#[derive(Deserialize)]
struct ReleaseDto {
    tag_name: String,
    html_url: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    /// Set by GitHub once a release is published as immutable: its assets and tag can no longer
    /// change.
    #[serde(default)]
    immutable: bool,
    author: Option<AuthorDto>,
    #[serde(default)]
    assets: Vec<AssetDto>,
}

#[derive(Deserialize)]
struct AuthorDto {
    login: String,
}

#[derive(Deserialize)]
struct AssetDto {
    name: String,
    browser_download_url: String,
    size: u64,
    digest: Option<String>,
}

/// The release file name ends with this on the platform the updater can replace in place.
fn platform_suffix() -> Option<&'static str> {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some("-macos-arm64.zip")
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        Some("-macos-x64.zip")
    } else if cfg!(all(windows, target_arch = "x86_64")) {
        // The zip `release.yml` builds for Windows: `degenview.exe` at its root.
        Some("-windows-x64.zip")
    } else if cfg!(all(windows, target_arch = "aarch64")) {
        // Native ARM64. An x64 build running emulated on Windows on ARM is still x86_64 here, so it
        // keeps taking the x64 zip.
        Some("-windows-arm64.zip")
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        // The tarball `release.yml` builds for Linux: `./degenview` at its root. The Flatpak next to
        // it ends `.flatpak` and is never picked; a Flatpak install is refused by `install_target`.
        Some("-linux-x64.tar.gz")
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        Some("-linux-arm64.tar.gz")
    } else {
        None
    }
}

/// The platform's download for the release tagged `tag`: exactly `DegenView-<tag><suffix>`, as
/// `release.yml` names it. A stale or misnamed file that merely ends in the suffix is not offered.
fn select_asset(assets: Vec<AssetDto>, tag: &str, suffix: Option<&str>) -> Option<ReleaseAsset> {
    let expected = format!("DegenView-{tag}{}", suffix?);
    let asset = assets.into_iter().find(|asset| asset.name == expected)?;
    let sha256 = asset.digest?.strip_prefix("sha256:")?.to_ascii_lowercase();
    let valid = sha256.len() == 64 && sha256.bytes().all(|byte| byte.is_ascii_hexdigit());
    (valid
        && asset.browser_download_url.starts_with(DOWNLOAD_PREFIX)
        && (1..=MAX_ASSET_BYTES).contains(&asset.size))
    .then_some(ReleaseAsset {
        name: asset.name,
        url: asset.browser_download_url,
        size: asset.size,
        sha256,
    })
}

/// Headers for every GitHub API request the app makes.
pub fn github_headers() -> [(&'static str, &'static str); 3] {
    [
        ("Accept", "application/vnd.github+json"),
        (
            "User-Agent",
            concat!("DegenView/", env!("CARGO_PKG_VERSION")),
        ),
        ("X-GitHub-Api-Version", "2022-11-28"),
    ]
}

/// Blocking: up to `MAX_PAGES` GETs. Call it from a worker thread.
pub fn check(current: &str, channel: Channel) -> UpdateOutcome {
    check_pages(current, channel, |page| {
        let url = format!("{RELEASE_LIST_URL}?per_page={PAGE_SIZE}&page={page}");
        http::get_text(&url, &github_headers(), Retry::Transient)
    })
}

/// `check` over a page source, so the paging is tested without a network.
fn check_pages(
    current: &str,
    channel: Channel,
    fetch: impl Fn(usize) -> Result<String, http::Error>,
) -> UpdateOutcome {
    let Some(running) = Version::parse(current) else {
        return UpdateOutcome::Failed(format!("Could not compare version {current}."));
    };
    let mut releases = Vec::new();
    for page in 1..=MAX_PAGES {
        let body = match fetch(page) {
            Ok(body) => body,
            // No published release yet (or repository not public): nothing newer exists. Past the
            // first page it only means the list ended.
            Err(http::Error::Status(404)) => break,
            Err(http::Error::Status(403 | 429)) => {
                return UpdateOutcome::Failed(
                    "GitHub is rate limiting requests. Try again later.".to_owned(),
                );
            }
            Err(error) => return UpdateOutcome::Failed(error.to_string()),
        };
        let Some(listed) = parse_page(&body) else {
            return UpdateOutcome::Failed("GitHub sent an unreadable reply.".to_owned());
        };
        let last = listed.len() < PAGE_SIZE;
        // The list is newest-created first: once a page holds a release this build already has (or
        // an older one) that the channel accepts, later pages only hold older releases.
        let reached_history = listed
            .iter()
            .any(|release| offered_version(release, channel).is_some_and(|v| v <= running));
        releases.extend(listed);
        if last || reached_history {
            break;
        }
    }
    pick(releases, &running, channel)
}

fn parse_page(body: &str) -> Option<Vec<ReleaseDto>> {
    serde_json::from_str(body).ok()
}

#[cfg(test)]
fn evaluate(body: &str, current: &str, channel: Channel) -> UpdateOutcome {
    check_pages(current, channel, |_| Ok(body.to_owned()))
}

/// The release's version, when it is a published release `channel` offers.
fn offered_version(release: &ReleaseDto, channel: Channel) -> Option<Version> {
    if release.draft {
        return None;
    }
    let version = Version::parse(&release.tag_name)?;
    // GitHub's flag and the tag agree for releases the workflow publishes; a stable tag marked as
    // a prerelease is not trusted as stable.
    let consistent = !release.prerelease || version.prerelease.is_some();
    (consistent && channel.accepts(&version)).then_some(version)
}

fn pick(releases: Vec<ReleaseDto>, running: &Version, channel: Channel) -> UpdateOutcome {
    // A tag this version cannot read, or that no channel offers, is skipped rather than failing
    // the whole check: one stray tag must not stop updates.
    let newest = releases
        .into_iter()
        .filter_map(|release| Some((offered_version(&release, channel)?, release)))
        .max_by(|(a, _), (b, _)| a.cmp(b));
    match newest {
        Some((latest, release)) if latest > *running => UpdateOutcome::Available(offer(release)),
        _ => UpdateOutcome::UpToDate,
    }
}

fn offer(release: ReleaseDto) -> Release {
    // Only install what cannot be swapped after publishing and that the release workflow published;
    // anything else is left to a manual download.
    let manual_only = if !release.immutable {
        Some("Not published as an immutable release, so it needs a manual download.")
    } else if !release.author.as_ref().is_some_and(|author| {
        RELEASE_AUTHORS
            .iter()
            .any(|login| login.eq_ignore_ascii_case(&author.login))
    }) {
        Some("Not published by the release workflow, so it needs a manual download.")
    } else {
        None
    };
    let assets = if manual_only.is_none() {
        release.assets
    } else {
        Vec::new()
    };
    Release {
        version: release
            .tag_name
            .trim()
            .trim_start_matches(['v', 'V'])
            .to_owned(),
        url: release
            .html_url
            .filter(|url| url.starts_with("https://github.com/"))
            .unwrap_or_else(|| RELEASES_URL.to_owned()),
        asset: select_asset(assets, &release.tag_name, platform_suffix()),
        manual_only,
        tag: release.tag_name,
    }
}

/// `major.minor.patch` with an optional `v` prefix and an optional `-prerelease`/`+build` tail,
/// ordered by SemVer 2.0: a prerelease sorts before its release (`0.2.0-rc.1 < 0.2.0`) and its
/// dot-separated parts compare numerically when numeric (`rc.2 < rc.10`), else as text.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Version {
    core: [u64; 3],
    prerelease: Option<String>,
}

/// What a version's prerelease part says about how settled it is.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Kind {
    Stable,
    ReleaseCandidate,
    /// `alpha`, `beta`, `nightly`, anything unrecognised: no channel offers these.
    Other,
}

impl Version {
    /// SemVer 2.0 with one optional `v`: three numbers without leading zeros, then an optional
    /// `-prerelease` and `+build`. Anything looser is not a tag the release tooling creates.
    fn parse(text: &str) -> Option<Self> {
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
    fn kind(&self) -> Kind {
        let Some(pre) = &self.prerelease else {
            return Kind::Stable;
        };
        match pre.strip_prefix("rc.") {
            Some(number) if is_digits(number) => Kind::ReleaseCandidate,
            _ => Kind::Other,
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A release as the release workflow publishes it: immutable, authored by the bot.
    fn reply(tag: &str) -> String {
        reply_by(tag, true, "github-actions[bot]")
    }

    fn reply_by(tag: &str, immutable: bool, author: &str) -> String {
        let digest = format!("sha256:{}", "ab".repeat(32));
        let assets: Vec<String> = [
            "macos-arm64.zip",
            "macos-x64.zip",
            "windows-x64.zip",
            "windows-arm64.zip",
            "linux-x64.tar.gz",
            "linux-arm64.tar.gz",
        ]
        .iter()
        .map(|platform| {
            format!(
                r#"{{"name":"DegenView-{tag}-{platform}","size":10,"digest":"{digest}","browser_download_url":"{DOWNLOAD_PREFIX}{tag}/DegenView-{tag}-{platform}"}}"#
            )
        })
        .collect();
        let prerelease = tag.contains('-');
        format!(
            r#"{{"tag_name":"{tag}","prerelease":{prerelease},"immutable":{immutable},"author":{{"login":"{author}"}},"html_url":"https://github.com/degenview/degenview/releases/tag/{tag}","assets":[{}]}}"#,
            assets.join(",")
        )
    }

    /// The reply for a list of releases, newest first as GitHub sends them.
    fn list(replies: &[String]) -> String {
        format!("[{}]", replies.join(","))
    }

    fn eval(replies: &[String], current: &str, channel: Channel) -> UpdateOutcome {
        evaluate(&list(replies), current, channel)
    }

    /// Stable channel, a single release.
    fn one(body: &str, current: &str) -> UpdateOutcome {
        evaluate(&format!("[{body}]"), current, Channel::Stable)
    }

    fn available(outcome: UpdateOutcome) -> Release {
        match outcome {
            UpdateOutcome::Available(release) => release,
            other => panic!("expected an update, got {other:?}"),
        }
    }

    #[test]
    fn versions_order_numerically_not_textually() {
        let v = |text| Version::parse(text).unwrap();
        assert!(v("0.10.0") > v("0.9.9"));
        assert!(v("v1.0.0") > v("0.99.99"));
        assert!(v("0.2.0-rc.1") < v("0.2.0"));
        assert!(v("0.2.0-rc1") < v("0.2.0"));
        assert_eq!(v("v0.1.0"), v("0.1.0+build5"));
        assert!(Version::parse("0.1").is_none());
        assert!(Version::parse("nightly").is_none());
        assert!(Version::parse("1.2.3.4").is_none());
    }

    /// The client reads remote repository state, so it shares the contract of `tools/release.sh`.
    #[test]
    fn versions_must_be_strict_semver() {
        for tag in [
            "vv1.0.0",
            "01.0.0",
            "1.00.0",
            "1.0.0-01",
            "+1.0.0",
            "1.0.0-",
            "1.0.0-rc..1",
            "1.0.0-rc.1+",
            "1.0.0+a..b",
            "1.0.0-rc_1",
            "1.-1.0",
            "99999999999999999999.0.0",
        ] {
            assert!(Version::parse(tag).is_none(), "{tag}");
        }
        for tag in [
            "v1.0.0",
            "V1.0.0",
            "0.0.0",
            "1.0.0-rc.1",
            "1.0.0-0.3.7",
            "1.0.0+20130313",
        ] {
            assert!(Version::parse(tag).is_some(), "{tag}");
        }
    }

    #[test]
    fn prereleases_follow_semver_precedence() {
        let v = |text| Version::parse(text).unwrap();
        // Numeric parts compare as numbers: rc.10 is after rc.2.
        assert!(v("0.6.0-rc.2") < v("0.6.0-rc.10"));
        assert!(v("0.6.0-rc.10") < v("0.6.0"));
        assert!(v("0.6.0-alpha.1") < v("0.6.0-beta.1"));
        assert!(v("0.6.0-beta.9") < v("0.6.0-rc.1"));
        // A longer list wins when the shorter is its prefix, and a candidate for the next
        // version is after the current release.
        assert!(v("0.6.0-rc") < v("0.6.0-rc.1"));
        assert!(v("0.5.1") < v("0.6.0-rc.1"));
    }

    #[test]
    fn a_prereleases_channel_comes_from_its_name() {
        assert_eq!(Channel::of_version("0.5.1"), Channel::Stable);
        for tag in ["0.6.0-rc.1", "v0.6.0-rc.12"] {
            assert_eq!(Channel::of_version(tag), Channel::ReleaseCandidate, "{tag}");
        }
        // Only the dotted `rc.N` that `tools/release.sh` writes is a release candidate.
        for tag in [
            "0.6.0-beta.1",
            "0.6.0-nightly",
            "0.6.0-1",
            "garbage",
            "v0.6.0-rc1",
            "0.6.0-RC.2",
            "0.6.0-rc.",
            "0.6.0-rc.1.2",
        ] {
            assert_eq!(Channel::of_version(tag), Channel::Stable, "{tag}");
        }
        for channel in Channel::ALL {
            assert_eq!(Channel::from_key(channel.key()), Some(channel));
        }
        assert_eq!(Channel::from_key("beta"), None);
    }

    #[test]
    fn a_newer_tag_is_an_update() {
        let release = available(one(&reply("v0.2.0"), "0.1.0"));
        assert_eq!(release.tag, "v0.2.0");
        assert_eq!(release.version, "0.2.0");
        assert_eq!(
            release.url,
            "https://github.com/degenview/degenview/releases/tag/v0.2.0"
        );
        assert_eq!(release.manual_only, None);
        // An installable asset exists only on platforms the updater can replace in place.
        assert_eq!(release.asset.is_some(), platform_suffix().is_some());
    }

    #[test]
    fn only_immutable_releases_from_the_workflow_are_installable() {
        let hand_made = available(one(&reply_by("v0.2.0", true, "someone"), "0.1.0"));
        assert!(hand_made.asset.is_none());
        assert!(hand_made.manual_only.unwrap().contains("release workflow"));
        // Not even the maintainer's own account: only the workflow (which records provenance).
        for login in ["nicoSWD", "nicoswd"] {
            let by_maintainer = available(one(&reply_by("v0.2.0", true, login), "0.1.0"));
            assert!(by_maintainer.asset.is_none(), "{login}");
            assert!(by_maintainer.manual_only.is_some(), "{login}");
        }
        // The bot's login is matched case-insensitively, like GitHub logins.
        let by_bot = available(one(&reply_by("v0.2.0", true, "GitHub-Actions[bot]"), "0.1.0"));
        assert_eq!(by_bot.manual_only, None);
        let mutable = available(one(
            &reply_by("v0.2.0", false, "github-actions[bot]"),
            "0.1.0",
        ));
        assert!(mutable.asset.is_none());
        assert!(mutable.manual_only.unwrap().contains("immutable"));
        let anonymous = r#"{"tag_name":"v0.2.0","immutable":true}"#;
        let anonymous = available(one(anonymous, "0.1.0"));
        assert!(anonymous.asset.is_none() && anonymous.manual_only.is_some());
    }

    #[test]
    fn same_or_older_tag_is_up_to_date() {
        assert_eq!(one(&reply("v0.1.0"), "0.1.0"), UpdateOutcome::UpToDate);
        assert_eq!(one(&reply("v0.1.0"), "0.3.0"), UpdateOutcome::UpToDate);
        assert_eq!(
            evaluate("[]", "0.1.0", Channel::Stable),
            UpdateOutcome::UpToDate
        );
    }

    #[test]
    fn drafts_are_ignored_and_stable_never_sees_a_prerelease() {
        let draft = r#"{"tag_name":"v9.0.0","draft":true}"#;
        assert_eq!(one(draft, "0.1.0"), UpdateOutcome::UpToDate);
        let candidate = reply("v9.0.0-rc.1");
        assert_eq!(one(&candidate, "0.1.0"), UpdateOutcome::UpToDate);
        // A stable-looking tag that GitHub marks as a prerelease is not offered either.
        let marked = r#"{"tag_name":"v9.0.0","prerelease":true}"#;
        assert_eq!(one(marked, "0.1.0"), UpdateOutcome::UpToDate);
    }

    #[test]
    fn each_channel_picks_the_newest_tag_it_accepts() {
        let releases = [
            reply("v0.6.0-rc.10"),
            reply("v0.6.0-rc.2"),
            reply("v0.5.2"),
            reply("v0.5.1"),
            reply("v0.7.0-beta.1"),
            reply("nightly"),
        ];
        let stable = available(eval(&releases, "0.5.0", Channel::Stable));
        assert_eq!(stable.version, "0.5.2");
        let candidate = available(eval(&releases, "0.5.2", Channel::ReleaseCandidate));
        assert_eq!(candidate.version, "0.6.0-rc.10");
        assert_eq!(candidate.tag, "v0.6.0-rc.10");
        // Nothing newer than the newest stable for a stable user already on it.
        assert_eq!(
            eval(&releases, "0.5.2", Channel::Stable),
            UpdateOutcome::UpToDate
        );
    }

    #[test]
    fn a_release_candidate_user_moves_on_to_the_release() {
        let releases = [reply("v0.6.0"), reply("v0.6.0-rc.2"), reply("v0.5.1")];
        let release = available(eval(&releases, "0.6.0-rc.1", Channel::ReleaseCandidate));
        assert_eq!(release.version, "0.6.0");
        // Order in the reply does not matter.
        let shuffled = [reply("v0.5.1"), reply("v0.6.0-rc.2"), reply("v0.6.0")];
        let release = available(eval(&shuffled, "0.6.0-rc.1", Channel::ReleaseCandidate));
        assert_eq!(release.version, "0.6.0");
    }

    #[test]
    fn switching_to_stable_never_downgrades() {
        let releases = [reply("v0.6.0-rc.1"), reply("v0.5.1")];
        assert_eq!(
            eval(&releases, "0.6.0-rc.1", Channel::Stable),
            UpdateOutcome::UpToDate
        );
        assert_eq!(
            eval(&releases, "0.6.0-rc.1", Channel::ReleaseCandidate),
            UpdateOutcome::UpToDate
        );
    }

    #[test]
    fn foreign_release_urls_fall_back_to_the_releases_page() {
        let body = r#"{"tag_name":"v0.2.0","html_url":"https://evil.example/x"}"#;
        assert_eq!(available(one(body, "0.1.0")).url, RELEASES_URL);
    }

    /// A page source serving `all` in pages of `PAGE_SIZE`, counting how many pages were read.
    fn paged(
        all: Vec<String>,
        reads: &std::cell::Cell<usize>,
    ) -> impl Fn(usize) -> Result<String, http::Error> + '_ {
        move |page| {
            reads.set(reads.get() + 1);
            let slice = all.chunks(PAGE_SIZE).nth(page - 1).unwrap_or_default();
            Ok(list(slice))
        }
    }

    fn candidates(count: usize) -> Vec<String> {
        (1..=count)
            .rev()
            .map(|number| reply(&format!("v0.9.0-rc.{number}")))
            .collect()
    }

    #[test]
    fn a_stable_release_behind_a_run_of_candidates_is_found_on_a_later_page() {
        let mut all = candidates(31);
        all.push(reply("v0.5.2"));
        let reads = std::cell::Cell::new(0);
        let outcome = check_pages("0.5.1", Channel::Stable, paged(all, &reads));
        assert_eq!(available(outcome).version, "0.5.2");
        assert_eq!(reads.get(), 2);
    }

    #[test]
    fn paging_stops_once_the_running_version_is_reached() {
        let mut all = vec![reply("v0.5.2")];
        all.extend(candidates(40));
        let reads = std::cell::Cell::new(0);
        let outcome = check_pages("0.5.2", Channel::Stable, paged(all, &reads));
        assert_eq!(outcome, UpdateOutcome::UpToDate);
        assert_eq!(reads.get(), 1);
    }

    #[test]
    fn paging_is_bounded() {
        let reads = std::cell::Cell::new(0);
        let outcome = check_pages("0.5.1", Channel::Stable, paged(candidates(200), &reads));
        assert_eq!(outcome, UpdateOutcome::UpToDate);
        assert_eq!(reads.get(), MAX_PAGES);
    }

    #[test]
    fn a_failing_later_page_fails_the_check_but_a_missing_one_ends_the_list() {
        let first = list(&candidates(PAGE_SIZE));
        let serve = |second: http::Error| {
            let first = first.clone();
            move |page: usize| match page {
                1 => Ok(first.clone()),
                _ => Err(match &second {
                    http::Error::Status(code) => http::Error::Status(*code),
                    other => http::Error::Network(other.to_string()),
                }),
            }
        };
        let rate_limited = check_pages("0.5.1", Channel::Stable, serve(http::Error::Status(429)));
        assert!(matches!(rate_limited, UpdateOutcome::Failed(_)));
        let ended = check_pages("0.5.1", Channel::Stable, serve(http::Error::Status(404)));
        assert_eq!(ended, UpdateOutcome::UpToDate);
    }

    /// `release.yml` publishes exactly the names the updater looks for.
    #[test]
    fn the_workflow_publishes_every_asset_the_updater_picks() {
        let workflow = include_str!("../../.github/workflows/release.yml");
        for suffix in [
            "macos-arm64.zip",
            "macos-x64.zip",
            "windows-x64.zip",
            "windows-arm64.zip",
            "linux-x64.tar.gz",
            "linux-arm64.tar.gz",
        ] {
            assert!(
                workflow.contains(suffix),
                "release.yml does not list {suffix}"
            );
        }
    }

    #[test]
    fn only_the_exactly_named_asset_is_offered() {
        let digest = format!("sha256:{}", "ab".repeat(32));
        let suffix = Some("-macos-arm64.zip");
        for name in [
            "old-macos-arm64.zip",
            "DegenView-v0.9.9-macos-arm64.zip",
            "DegenView-v1.0.0-extra-macos-arm64.zip",
            "DegenView-1.0.0-macos-arm64.zip",
        ] {
            let assets = vec![asset(name, "v1.0.0/a", 5, Some(&digest))];
            assert!(select_asset(assets, "v1.0.0", suffix).is_none(), "{name}");
        }
        let exact = vec![asset(
            "DegenView-v1.0.0-macos-arm64.zip",
            "v1.0.0/a",
            5,
            Some(&digest),
        )];
        assert!(select_asset(exact, "v1.0.0", suffix).is_some());
    }

    fn asset(name: &str, url_tail: &str, size: u64, digest: Option<&str>) -> AssetDto {
        AssetDto {
            name: name.to_owned(),
            browser_download_url: format!("{DOWNLOAD_PREFIX}{url_tail}"),
            size,
            digest: digest.map(str::to_owned),
        }
    }

    #[test]
    fn the_platforms_zip_is_picked_with_its_digest() {
        let digest = format!("sha256:{}", "AB".repeat(32));
        let assets = vec![
            asset(
                "DegenView-v1.0.0-macos-x64.zip",
                "v1.0.0/x64",
                10,
                Some(&digest),
            ),
            asset(
                "DegenView-v1.0.0-macos-arm64.zip",
                "v1.0.0/arm",
                20,
                Some(&digest),
            ),
        ];
        let picked = select_asset(assets, "v1.0.0", Some("-macos-arm64.zip")).unwrap();
        assert_eq!(picked.size, 20);
        assert_eq!(picked.sha256, "ab".repeat(32));
        assert!(picked.url.ends_with("v1.0.0/arm"));
    }

    /// The Windows release has an installer next to the zip the updater unpacks; only the zip may
    /// ever be picked, whatever order GitHub lists them in.
    #[test]
    fn the_windows_installer_is_never_picked_as_the_update() {
        let digest = format!("sha256:{}", "ab".repeat(32));
        for order in [[0, 1], [1, 0]] {
            let files = [
                ("DegenView-v1.0.0-windows-x64-setup.exe", "v1.0.0/setup", 30),
                ("DegenView-v1.0.0-windows-x64.zip", "v1.0.0/zip", 20),
            ];
            let assets = order
                .map(|i| asset(files[i].0, files[i].1, files[i].2, Some(&digest)))
                .into_iter()
                .collect();
            let picked = select_asset(assets, "v1.0.0", Some("-windows-x64.zip")).unwrap();
            assert_eq!(picked.name, "DegenView-v1.0.0-windows-x64.zip");
        }
        let only_setup = vec![asset(
            "DegenView-v1.0.0-windows-x64-setup.exe",
            "v1.0.0/setup",
            30,
            Some(&digest),
        )];
        assert!(select_asset(only_setup, "v1.0.0", Some("-windows-x64.zip")).is_none());
    }

    /// Windows ships an x64 and an ARM64 zip, each next to its installer: a build takes the zip for its
    /// own CPU and never the other zip or an installer, whatever order GitHub lists them in.
    #[test]
    fn each_windows_cpu_takes_only_its_own_zip() {
        let digest = format!("sha256:{}", "ab".repeat(32));
        let files = [
            "DegenView-v1.0.0-windows-x64-setup.exe",
            "DegenView-v1.0.0-windows-arm64-setup.exe",
            "DegenView-v1.0.0-windows-x64.zip",
            "DegenView-v1.0.0-windows-arm64.zip",
        ];
        for (suffix, expected) in [
            ("-windows-x64.zip", files[2]),
            ("-windows-arm64.zip", files[3]),
        ] {
            for reversed in [false, true] {
                let mut names = files.to_vec();
                if reversed {
                    names.reverse();
                }
                let assets = names
                    .iter()
                    .map(|name| asset(name, &format!("v1.0.0/{name}"), 40, Some(&digest)))
                    .collect();
                let picked = select_asset(assets, "v1.0.0", Some(suffix)).unwrap();
                assert_eq!(picked.name, expected);
            }
        }
    }

    /// Linux ships a tarball and, per CPU, a Flatpak bundle; only the tarball may be picked.
    #[test]
    fn the_linux_tarball_is_picked_and_the_flatpak_never_is() {
        let digest = format!("sha256:{}", "ab".repeat(32));
        for suffix in ["-linux-x64.tar.gz", "-linux-arm64.tar.gz"] {
            let cpu = suffix.trim_end_matches(".tar.gz");
            for order in [[0, 1], [1, 0]] {
                let files = [
                    (format!("DegenView-v1.0.0{cpu}.flatpak"), "v1.0.0/flatpak"),
                    (format!("DegenView-v1.0.0{cpu}.tar.gz"), "v1.0.0/tarball"),
                ];
                let assets = order
                    .map(|i| asset(&files[i].0, files[i].1, 40, Some(&digest)))
                    .into_iter()
                    .collect();
                let picked = select_asset(assets, "v1.0.0", Some(suffix)).unwrap();
                assert!(picked.name.ends_with(".tar.gz"), "{}", picked.name);
            }
            let only_flatpak = vec![asset(
                &format!("DegenView-v1.0.0{cpu}.flatpak"),
                "v1.0.0/flatpak",
                40,
                Some(&digest),
            )];
            assert!(select_asset(only_flatpak, "v1.0.0", Some(suffix)).is_none());
        }
    }

    /// The Mac release has a disk image next to the zip the updater unpacks; only the zip may ever be
    /// picked, whatever order GitHub lists them in.
    #[test]
    fn the_macos_disk_image_is_never_picked_as_the_update() {
        let digest = format!("sha256:{}", "ab".repeat(32));
        for order in [[0, 1], [1, 0]] {
            let files = [
                ("DegenView-v1.0.0-macos-arm64.dmg", "v1.0.0/dmg", 30),
                ("DegenView-v1.0.0-macos-arm64.zip", "v1.0.0/zip", 20),
            ];
            let assets = order
                .map(|i| asset(files[i].0, files[i].1, files[i].2, Some(&digest)))
                .into_iter()
                .collect();
            let picked = select_asset(assets, "v1.0.0", Some("-macos-arm64.zip")).unwrap();
            assert_eq!(picked.name, "DegenView-v1.0.0-macos-arm64.zip");
        }
        let only_image = vec![asset(
            "DegenView-v1.0.0-macos-arm64.dmg",
            "v1.0.0/dmg",
            30,
            Some(&digest),
        )];
        assert!(select_asset(only_image, "v1.0.0", Some("-macos-arm64.zip")).is_none());
    }

    #[test]
    fn assets_without_a_trustworthy_digest_or_url_are_not_installable() {
        let digest = format!("sha256:{}", "ab".repeat(32));
        let pick = |asset| select_asset(vec![asset], "v1.0.0", Some("-macos-arm64.zip"));
        let name = "DegenView-v1.0.0-macos-arm64.zip";
        assert!(pick(asset(name, "v1.0.0/a", 5, None)).is_none());
        assert!(pick(asset(name, "v1.0.0/a", 5, Some("sha256:abc"))).is_none());
        assert!(pick(asset(name, "v1.0.0/a", 5, Some("md5:00"))).is_none());
        assert!(pick(asset(name, "v1.0.0/a", 0, Some(&digest))).is_none());
        assert!(pick(asset(name, "v1.0.0/a", MAX_ASSET_BYTES + 1, Some(&digest))).is_none());
        let mut foreign = asset(name, "v1.0.0/a", 5, Some(&digest));
        foreign.browser_download_url = "https://evil.example/a.zip".to_owned();
        assert!(pick(foreign).is_none());
        assert!(select_asset(vec![asset(name, "x", 5, Some(&digest))], "v1.0.0", None).is_none());
    }

    #[test]
    fn garbage_is_a_failure_and_a_stray_tag_is_skipped() {
        assert!(matches!(
            evaluate("<html>", "0.1.0", Channel::Stable),
            UpdateOutcome::Failed(_)
        ));
        // A single object is not a release list.
        assert!(matches!(
            evaluate(r#"{"tag_name":"v0.2.0"}"#, "0.1.0", Channel::Stable),
            UpdateOutcome::Failed(_)
        ));
        assert!(matches!(
            evaluate("[]", "garbage", Channel::Stable),
            UpdateOutcome::Failed(_)
        ));
        // An unreadable tag does not stop the others from being offered.
        let releases = [reply("nightly"), reply("v0.2.0")];
        assert_eq!(
            available(eval(&releases, "0.1.0", Channel::Stable)).version,
            "0.2.0"
        );
        assert_eq!(
            one(r#"{"tag_name":"nightly"}"#, "0.1.0"),
            UpdateOutcome::UpToDate
        );
    }
}

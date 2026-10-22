//! "Is there a newer DegenView?" — asks GitHub for the recent releases of `degenview/degenview`,
//! picks the newest one the chosen `Channel` accepts and compares its tag with the running version.
//! Read-only: it never downloads or installs anything, it only reports where the release page is.

use crate::service::http::{self, Retry};
use serde::Deserialize;

/// The recent releases, prereleases included (`/releases/latest` never returns one, and is "the
/// most recently created stable release", not the highest version).
const RELEASE_LIST_URL: &str =
    "https://api.github.com/repos/degenview/degenview/releases?per_page=30";
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
/// Accounts GitHub may record as a release's author: the release workflow's bot, and the
/// maintainer for a release they published by hand.
const RELEASE_AUTHORS: &[&str] = &["github-actions[bot]", "nicoSWD"];
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
    } else {
        None
    }
}

fn select_asset(assets: Vec<AssetDto>, suffix: Option<&str>) -> Option<ReleaseAsset> {
    let suffix = suffix?;
    let asset = assets
        .into_iter()
        .find(|asset| asset.name.ends_with(suffix))?;
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

/// Blocking: one GET. Call it from a worker thread.
pub fn check(current: &str, channel: Channel) -> UpdateOutcome {
    let body = http::get_text(RELEASE_LIST_URL, &github_headers(), Retry::Transient);
    match body {
        Ok(body) => evaluate(&body, current, channel),
        // No published release yet (or repository not public): nothing newer exists.
        Err(http::Error::Status(404)) => UpdateOutcome::UpToDate,
        Err(http::Error::Status(403 | 429)) => {
            UpdateOutcome::Failed("GitHub is rate limiting requests. Try again later.".to_owned())
        }
        Err(error) => UpdateOutcome::Failed(error.to_string()),
    }
}

fn evaluate(body: &str, current: &str, channel: Channel) -> UpdateOutcome {
    let releases: Vec<ReleaseDto> = match serde_json::from_str(body) {
        Ok(releases) => releases,
        Err(_) => return UpdateOutcome::Failed("GitHub sent an unreadable reply.".to_owned()),
    };
    let Some(running) = Version::parse(current) else {
        return UpdateOutcome::Failed(format!("Could not compare version {current}."));
    };
    // A tag this version cannot read, or that no channel offers, is skipped rather than failing
    // the whole check: one stray tag must not stop updates.
    let newest = releases
        .into_iter()
        .filter(|release| !release.draft)
        .filter_map(|release| {
            let version = Version::parse(&release.tag_name)?;
            // GitHub's flag and the tag agree for releases the workflow publishes; a stable tag
            // marked as a prerelease is not trusted as stable.
            let consistent = !release.prerelease || version.prerelease.is_some();
            (consistent && channel.accepts(&version)).then_some((version, release))
        })
        .max_by(|(a, _), (b, _)| a.cmp(b));
    match newest {
        Some((latest, release)) if latest > running => UpdateOutcome::Available(offer(release)),
        _ => UpdateOutcome::UpToDate,
    }
}

fn offer(release: ReleaseDto) -> Release {
    // Only install what cannot be swapped after publishing and that the maintainer or the release
    // workflow published; anything else is left to a manual download.
    let manual_only = if !release.immutable {
        Some("Not published as an immutable release, so it needs a manual download.")
    } else if !release.author.as_ref().is_some_and(|author| {
        RELEASE_AUTHORS
            .iter()
            .any(|login| login.eq_ignore_ascii_case(&author.login))
    }) {
        Some(
            "Not published by the maintainer or the release workflow, so it needs a manual download.",
        )
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
        asset: select_asset(assets, platform_suffix()),
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
    fn parse(text: &str) -> Option<Self> {
        let text = text.trim().trim_start_matches(['v', 'V']);
        let text = text.split('+').next()?;
        let (core, prerelease) = match text.split_once('-') {
            Some((core, pre)) => (core, Some(pre.to_owned())),
            None => (text, None),
        };
        let mut parts = core.split('.');
        let mut numbers = [0_u64; 3];
        for slot in &mut numbers {
            *slot = parts.next()?.parse().ok()?;
        }
        parts.next().is_none().then_some(Self {
            core: numbers,
            prerelease,
        })
    }

    /// Named by the letters that start the prerelease, so `rc.1`, `rc1` and `RC.1` are all
    /// release candidates.
    fn kind(&self) -> Kind {
        let Some(pre) = &self.prerelease else {
            return Kind::Stable;
        };
        let name: String = pre
            .chars()
            .take_while(char::is_ascii_alphabetic)
            .collect::<String>()
            .to_ascii_lowercase();
        match name.as_str() {
            "rc" => Kind::ReleaseCandidate,
            _ => Kind::Other,
        }
    }
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
        let assets: Vec<String> = ["macos-arm64", "macos-x64"]
            .iter()
            .map(|platform| {
                format!(
                    r#"{{"name":"DegenView-{tag}-{platform}.zip","size":10,"digest":"{digest}","browser_download_url":"{DOWNLOAD_PREFIX}{tag}/DegenView-{tag}-{platform}.zip"}}"#
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
        for tag in ["0.6.0-rc.1", "v0.6.0-rc1", "0.6.0-RC.2"] {
            assert_eq!(Channel::of_version(tag), Channel::ReleaseCandidate, "{tag}");
        }
        for tag in ["0.6.0-beta.1", "0.6.0-nightly", "0.6.0-1", "garbage"] {
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
        assert!(hand_made.manual_only.unwrap().contains("maintainer"));
        // The maintainer's own account is accepted, case-insensitively like GitHub logins.
        for login in ["nicoSWD", "nicoswd"] {
            let by_maintainer = available(one(&reply_by("v0.2.0", true, login), "0.1.0"));
            assert_eq!(by_maintainer.manual_only, None, "{login}");
        }
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
        let picked = select_asset(assets, Some("-macos-arm64.zip")).unwrap();
        assert_eq!(picked.size, 20);
        assert_eq!(picked.sha256, "ab".repeat(32));
        assert!(picked.url.ends_with("v1.0.0/arm"));
    }

    #[test]
    fn assets_without_a_trustworthy_digest_or_url_are_not_installable() {
        let digest = format!("sha256:{}", "ab".repeat(32));
        let pick = |asset| select_asset(vec![asset], Some("-macos-arm64.zip"));
        let name = "DegenView-v1.0.0-macos-arm64.zip";
        assert!(pick(asset(name, "v1.0.0/a", 5, None)).is_none());
        assert!(pick(asset(name, "v1.0.0/a", 5, Some("sha256:abc"))).is_none());
        assert!(pick(asset(name, "v1.0.0/a", 5, Some("md5:00"))).is_none());
        assert!(pick(asset(name, "v1.0.0/a", 0, Some(&digest))).is_none());
        assert!(pick(asset(name, "v1.0.0/a", MAX_ASSET_BYTES + 1, Some(&digest))).is_none());
        let mut foreign = asset(name, "v1.0.0/a", 5, Some(&digest));
        foreign.browser_download_url = "https://evil.example/a.zip".to_owned();
        assert!(pick(foreign).is_none());
        assert!(select_asset(vec![asset(name, "x", 5, Some(&digest))], None).is_none());
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

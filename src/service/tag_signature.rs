//! Checks that a release's git tag was signed by the DegenView maintainer before the updater
//! downloads anything.
//!
//! Tags are SSH-signed (`git tag -s`) and GitHub verifies the signature when the tag is pushed,
//! reporting `verification { verified, reason }` on the tag object. That alone only says "signed by
//! a key registered on *some* GitHub account", so the tag's tagger must also be the maintainer's
//! identity: GitHub only reports `valid` when the signing key belongs to an account that owns the
//! tagger's email, which makes the email trustworthy on a verified tag. A repository write token
//! cannot add a signing key to the maintainer's account, so it cannot forge a signed *tag*. (A
//! user-level `gh` OAuth token with the `admin:ssh_signing_key` scope can: keep that scope off any
//! token a developer machine or an agent session holds, and sign tags with a hardware-backed key.)
//!
//! The trust anchor is therefore GitHub's account and key registry, not a key baked into the app,
//! and rotating the signing key means registering the new one on the account. The signature
//! vouches for "this tag names this commit", not for the files attached to the release. Immutable
//! releases lock a published release's assets and its tag name, but a token that can create
//! releases could still publish its own assets for a signed tag that has no release yet (while the
//! workflow builds, or after a failed run). macOS has a second check, the Developer ID signature;
//! Windows and Linux have none beyond this and the GitHub digest (see
//! `docs/agent-guides/releases-updates.md`).

use crate::service::http::{self, Retry};
use crate::service::update_check::github_headers;
use serde::Deserialize;

/// Tagger emails accepted on a verified tag. `tools/release.sh` and the release workflow check the
/// same identity.
const MAINTAINER_EMAILS: &[&str] = &["nicoswd@gmail.com"];
const API: &str = "https://api.github.com/repos/degenview/degenview/git";

#[derive(Debug, Eq, PartialEq)]
pub enum TagError {
    /// No tag object, a lightweight tag, or no signature on it.
    Unsigned,
    /// Signed, but GitHub does not accept the signature or the tagger is not the maintainer.
    NotTrusted,
    /// Signed, but for a different tag or a non-commit target.
    WrongTag,
    Network(String),
}

impl std::fmt::Display for TagError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsigned => f.write_str("This release's tag is not signed."),
            Self::NotTrusted => {
                f.write_str("This release's tag is not signed by the DegenView maintainer.")
            }
            Self::WrongTag => f.write_str("This release's tag signature belongs to another tag."),
            Self::Network(message) => {
                write!(f, "Could not check the release signature: {message}")
            }
        }
    }
}

#[derive(Deserialize)]
struct RefDto {
    object: RefObjectDto,
}

#[derive(Deserialize)]
struct RefObjectDto {
    sha: String,
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Deserialize)]
struct TagDto {
    tag: String,
    object: TargetDto,
    tagger: Option<TaggerDto>,
    verification: Option<VerificationDto>,
}

#[derive(Deserialize)]
struct TargetDto {
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Deserialize)]
struct TaggerDto {
    email: String,
}

#[derive(Deserialize)]
struct VerificationDto {
    verified: bool,
    reason: String,
}

/// Blocking: two GETs. `Ok` only when `tag` is an annotated tag GitHub verified, tagged by the
/// maintainer.
pub fn verify_release_tag(tag: &str) -> Result<(), TagError> {
    verify_release_tag_at(API, tag)
}

fn verify_release_tag_at(api: &str, tag: &str) -> Result<(), TagError> {
    if tag.is_empty()
        || !tag
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        return Err(TagError::WrongTag);
    }
    let fetch = |url: String| {
        http::get_text(&url, &github_headers(), Retry::Transient).map_err(network_error)
    };
    let unreadable = |_| TagError::Network("unreadable reply from GitHub".to_owned());
    let reference: RefDto =
        serde_json::from_str(&fetch(format!("{api}/ref/tags/{tag}"))?).map_err(unreadable)?;
    if reference.object.kind != "tag" {
        // A lightweight tag points straight at a commit and cannot carry a signature.
        return Err(TagError::Unsigned);
    }
    let object: TagDto =
        serde_json::from_str(&fetch(format!("{api}/tags/{}", reference.object.sha))?)
            .map_err(unreadable)?;
    evaluate(tag, &object)
}

/// GitHub answers 403 (or 429) to an unauthenticated client that used its hourly quota, which is
/// shared by everything on the same address. Say so, as the update check does, instead of showing
/// "HTTP 403".
fn network_error(error: http::Error) -> TagError {
    match error {
        http::Error::Status(403 | 429) => {
            TagError::Network("GitHub is rate limiting requests. Try again later.".to_owned())
        }
        other => TagError::Network(other.to_string()),
    }
}

fn evaluate(tag: &str, object: &TagDto) -> Result<(), TagError> {
    let Some(verification) = &object.verification else {
        return Err(TagError::Unsigned);
    };
    if !(verification.verified && verification.reason == "valid") {
        return Err(if verification.reason == "unsigned" {
            TagError::Unsigned
        } else {
            TagError::NotTrusted
        });
    }
    // Verified, but make sure it is for this tag and not a replayed older one.
    if object.tag != tag || object.object.kind != "commit" {
        return Err(TagError::WrongTag);
    }
    let maintainer = object.tagger.as_ref().is_some_and(|tagger| {
        MAINTAINER_EMAILS
            .iter()
            .any(|email| email.eq_ignore_ascii_case(&tagger.email))
    });
    if maintainer {
        Ok(())
    } else {
        Err(TagError::NotTrusted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// GitHub's reply for the published v0.1.2 tag object (signature and payload elided).
    const REAL: &str = include_str!("testdata/tag_v0.1.2.json");

    fn tag(edit: impl FnOnce(&mut serde_json::Value)) -> TagDto {
        let mut json: serde_json::Value = serde_json::from_str(REAL).unwrap();
        edit(&mut json);
        serde_json::from_value(json).unwrap()
    }

    /// The release workflow refuses to build a tag by anyone else, so its identity and the one the
    /// app trusts must be the same.
    #[test]
    fn the_workflow_and_the_app_trust_the_same_maintainer() {
        let workflow = include_str!("../../.github/workflows/release.yml");
        let emails: Vec<&str> = workflow
            .lines()
            .filter_map(|line| line.trim().strip_prefix("MAINTAINER_EMAIL:"))
            .map(str::trim)
            .collect();
        assert!(!emails.is_empty(), "release.yml names no MAINTAINER_EMAIL");
        for email in emails {
            assert!(
                MAINTAINER_EMAILS
                    .iter()
                    .any(|trusted| trusted.eq_ignore_ascii_case(email)),
                "{email} is trusted by release.yml but not by the app"
            );
        }
    }

    #[test]
    fn the_real_v0_1_2_tag_is_trusted() {
        assert_eq!(evaluate("v0.1.2", &tag(|_| {})), Ok(()));
    }

    #[test]
    fn the_maintainer_email_matches_case_insensitively() {
        let upper = tag(|json| json["tagger"]["email"] = "NicoSWD@Gmail.com".into());
        assert_eq!(evaluate("v0.1.2", &upper), Ok(()));
    }

    #[test]
    fn a_valid_signature_by_someone_else_is_not_trusted() {
        let other = tag(|json| json["tagger"]["email"] = "attacker@example.com".into());
        assert_eq!(evaluate("v0.1.2", &other), Err(TagError::NotTrusted));
        let none = tag(|json| json["tagger"] = serde_json::Value::Null);
        assert_eq!(evaluate("v0.1.2", &none), Err(TagError::NotTrusted));
    }

    #[test]
    fn unverified_tags_are_refused_by_reason() {
        for (reason, expected) in [
            ("unsigned", TagError::Unsigned),
            ("unknown_key", TagError::NotTrusted),
            ("bad_email", TagError::NotTrusted),
            ("expired_key", TagError::NotTrusted),
            ("invalid", TagError::NotTrusted),
        ] {
            let unverified = tag(|json| {
                json["verification"]["verified"] = false.into();
                json["verification"]["reason"] = reason.into();
            });
            assert_eq!(evaluate("v0.1.2", &unverified), Err(expected), "{reason}");
        }
    }

    #[test]
    fn verified_must_be_true_and_reason_valid() {
        let odd = tag(|json| json["verification"]["verified"] = false.into());
        assert_eq!(evaluate("v0.1.2", &odd), Err(TagError::NotTrusted));
        let odd = tag(|json| json["verification"]["reason"] = "gpgverify_error".into());
        assert_eq!(evaluate("v0.1.2", &odd), Err(TagError::NotTrusted));
    }

    #[test]
    fn a_missing_verification_is_unsigned() {
        let none = tag(|json| json["verification"] = serde_json::Value::Null);
        assert_eq!(evaluate("v0.1.2", &none), Err(TagError::Unsigned));
    }

    #[test]
    fn a_signature_replayed_for_another_tag_or_target_is_rejected() {
        assert_eq!(evaluate("v0.1.3", &tag(|_| {})), Err(TagError::WrongTag));
        let tree = tag(|json| json["object"]["type"] = "tree".into());
        assert_eq!(evaluate("v0.1.2", &tree), Err(TagError::WrongTag));
    }

    #[test]
    fn a_rate_limited_check_says_so_instead_of_showing_the_status_code() {
        for status in [403, 429] {
            assert_eq!(
                network_error(http::Error::Status(status)).to_string(),
                "Could not check the release signature: GitHub is rate limiting requests. Try again later."
            );
        }
        assert_eq!(
            network_error(http::Error::Status(500)).to_string(),
            "Could not check the release signature: HTTP 500"
        );
    }

    #[test]
    fn odd_tag_names_never_reach_the_network() {
        for name in ["", "../x", "v1/../../x", "v1 2"] {
            assert_eq!(
                verify_release_tag_at("http://127.0.0.1:1", name),
                Err(TagError::WrongTag)
            );
        }
    }

    /// Hits GitHub: `cargo test -- --ignored the_published_tag`.
    #[test]
    #[ignore = "queries GitHub"]
    fn the_published_tag_verifies_live() {
        assert_eq!(verify_release_tag("v0.1.2"), Ok(()));
    }
}

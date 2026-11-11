//! A host of the user's own, speaking DegenView's protocol: one `POST` of the PNG as `image/png`,
//! an optional bearer token, and `{"url": "https://…"}` back. `docs/snapshots.md` writes the
//! protocol down, and <https://github.com/degenview/snapshots> is a server that speaks it.

use super::{
    FieldKind, FieldSpec, Glyph, Link, Logo, ProviderDescriptor, Purpose, Readiness, Request,
    SecretPresence, Secrets, SnapshotProvider,
};
use crate::model::snapshot::{
    FieldValues, SELF_HOSTED, SharedSnapshot, SnapshotHost, UploadError, parse_upload_response,
};
use crate::service::snapshot_upload::token_is_valid;
use std::borrow::Cow;

const HOST: &str = "host";
const TOKEN: &str = "token";

/// Where the token has always been kept, from before providers.
const TOKEN_ACCOUNT: &str = "host-token-v1";

/// Where a developer or a machine with no secure storage gives the token.
pub const TOKEN_ENV: &str = "DEGENVIEW_SNAPSHOT_TOKEN";

fn host_problem(text: &str) -> Option<&'static str> {
    SnapshotHost::parse(text).err().map(|error| error.message())
}

fn token_problem(text: &str) -> Option<&'static str> {
    (!token_is_valid(text.trim()))
        .then_some("A token has no spaces and only plain letters, digits and symbols.")
}

static DESCRIPTOR: ProviderDescriptor = ProviderDescriptor {
    id: SELF_HOSTED,
    name: "Host your own",
    summary: "Upload to a server you run",
    logo: Logo::Glyph(Glyph::Server),
    setup_steps: &[
        "Run DegenView Snapshots on a server you control, or any host that implements the same protocol.",
        "Enter its upload address below. Add the access token if the host asks for one.",
    ],
    links: &[Link {
        label: "DegenView Snapshots on GitHub",
        url: "https://github.com/degenview/snapshots",
    }],
    notice: "Images go to the host you enter, which you run or trust. Whoever has a link can open the image, and the host decides how long it keeps it.",
    fields: &[
        FieldSpec {
            key: HOST,
            title: "Upload address",
            hint: "An https:// address that takes the image and answers with a link.",
            placeholder: "https://snapshots.example.com/v1/snapshots",
            kind: FieldKind::Text,
            required: true,
            default: "",
            env: None,
            account: None,
            needed: "Add the address of your snapshot host in Settings ▸ Snapshots to share.",
            remove_label: "",
            validate: host_problem,
        },
        FieldSpec {
            key: TOKEN,
            title: "Access token (optional)",
            hint: "Sent as a bearer token, if the host asks for one.",
            placeholder: "",
            kind: FieldKind::Secret,
            required: false,
            default: "",
            env: Some(TOKEN_ENV),
            account: Some(TOKEN_ACCOUNT),
            needed: "",
            remove_label: "Remove token",
            validate: token_problem,
        },
    ],
    testable: false,
};

pub struct SelfHosted;

pub static SELF_HOSTED_PROVIDER: SelfHosted = SelfHosted;

fn host_of(settings: &FieldValues) -> Option<SnapshotHost> {
    SnapshotHost::parse(settings.get(HOST)?).ok()
}

impl SnapshotProvider for SelfHosted {
    fn descriptor(&self) -> &'static ProviderDescriptor {
        &DESCRIPTOR
    }

    fn readiness(&self, settings: &FieldValues, _: &SecretPresence) -> Readiness {
        if host_of(settings).is_some() {
            Readiness::Ready
        } else {
            Readiness::Needs(
                "Add the address of your snapshot host in Settings ▸ Snapshots to share."
                    .to_owned(),
            )
        }
    }

    fn label(&self, settings: &FieldValues) -> String {
        host_of(settings).map_or_else(|| DESCRIPTOR.name.to_owned(), |host| host.authority())
    }

    fn consent_key(&self, settings: &FieldValues) -> String {
        host_of(settings).map_or_else(String::new, |host| host.authority())
    }

    fn consent_note(&self, _: &FieldValues, present: &SecretPresence) -> Option<String> {
        // The token is for the host the user set it up with: a changed address would send it
        // somewhere new, so the question says it goes along.
        present
            .has(TOKEN)
            .then(|| "Your saved access token is sent with it.".to_owned())
    }

    fn status_line(&self, settings: &FieldValues) -> String {
        host_of(settings).map_or_else(
            || DESCRIPTOR.summary.to_owned(),
            |host| format!("Uploads to {}", host.authority()),
        )
    }

    fn request<'a>(
        &self,
        settings: &FieldValues,
        secrets: &Secrets,
        png: &'a [u8],
        _: Purpose,
    ) -> Result<Request<'a>, UploadError> {
        let host = host_of(settings).ok_or(UploadError::Offline)?;
        let mut headers = Vec::new();
        if let Some(token) = secrets.get(TOKEN) {
            if !token_is_valid(token) {
                return Err(UploadError::Unauthorized);
            }
            headers.push(("Authorization", format!("Bearer {token}")));
        }
        Ok(Request {
            url: host.endpoint().to_owned(),
            headers,
            content_type: "image/png".to_owned(),
            body: Cow::Borrowed(png),
        })
    }

    fn parse(&self, settings: &FieldValues, body: &[u8]) -> Result<SharedSnapshot, UploadError> {
        let host = host_of(settings).ok_or(UploadError::BadResponse)?;
        parse_upload_response(body, &host)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(host: &str) -> FieldValues {
        FieldValues::from([(HOST.to_owned(), host.to_owned())])
    }

    #[test]
    fn a_host_is_ready_when_its_address_is_valid_and_the_token_is_optional() {
        let none = SecretPresence::default();
        assert_eq!(
            SELF_HOSTED_PROVIDER.readiness(&settings("https://s.example/up"), &none),
            Readiness::Ready
        );
        for bad in ["", "s.example", "http://s.example/up"] {
            assert!(
                matches!(
                    SELF_HOSTED_PROVIDER.readiness(&settings(bad), &none),
                    Readiness::Needs(_)
                ),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn the_address_is_what_the_user_agrees_to_and_reads() {
        let on_a_path = settings("https://snap.example.com/v1/snapshots");
        assert_eq!(
            SELF_HOSTED_PROVIDER.consent_key(&on_a_path),
            "snap.example.com"
        );
        assert_eq!(SELF_HOSTED_PROVIDER.label(&on_a_path), "snap.example.com");
        // The same host on another path is the same consent; another port is not.
        assert_eq!(
            SELF_HOSTED_PROVIDER.consent_key(&settings("https://snap.example.com/other")),
            "snap.example.com"
        );
        assert_eq!(
            SELF_HOSTED_PROVIDER.consent_key(&settings("https://snap.example.com:8443/up")),
            "snap.example.com:8443"
        );
    }

    #[test]
    fn the_request_is_the_png_as_it_is_with_an_optional_bearer_token() {
        let png = b"\x89PNG\r\n\x1a\nDATA";
        let plain = SELF_HOSTED_PROVIDER
            .request(
                &settings("https://s.example/up"),
                &Secrets::default(),
                png,
                Purpose::Share,
            )
            .unwrap();
        assert_eq!(plain.url, "https://s.example/up");
        assert_eq!(plain.content_type, "image/png");
        assert_eq!(&*plain.body, png);
        assert!(plain.headers.is_empty(), "no token, no Authorization");

        let mut secrets = Secrets::default();
        secrets.insert(TOKEN, "tok123".to_owned());
        let authorised = SELF_HOSTED_PROVIDER
            .request(
                &settings("https://s.example/up"),
                &secrets,
                png,
                Purpose::Share,
            )
            .unwrap();
        assert_eq!(
            authorised.headers,
            [("Authorization", "Bearer tok123".to_owned())]
        );
    }

    #[test]
    fn a_token_that_cannot_ride_in_a_header_is_not_sent() {
        let mut secrets = Secrets::default();
        secrets.insert(TOKEN, "has space".to_owned());
        assert!(matches!(
            SELF_HOSTED_PROVIDER.request(
                &settings("https://s.example/up"),
                &secrets,
                b"x",
                Purpose::Share
            ),
            Err(UploadError::Unauthorized)
        ));
    }

    #[test]
    fn the_fields_are_checked_as_they_are_typed() {
        assert_eq!(host_problem("https://s.example/up"), None);
        assert!(host_problem("s.example").is_some());
        assert!(
            host_problem("http://s.example/up")
                .unwrap()
                .contains("https://")
        );
        assert!(host_problem("https://u:p@s.example/up").is_some());
        assert_eq!(token_problem("abc123"), None);
        assert!(token_problem("has space").is_some());
        assert!(token_problem("é").is_some());
    }

    #[test]
    fn the_answer_is_read_as_before() {
        let shared = SELF_HOSTED_PROVIDER
            .parse(
                &settings("https://s.example/up"),
                br#"{"url":"https://s.example/i/abc","delete_url":"https://s.example/d/abc"}"#,
            )
            .unwrap();
        assert_eq!(shared.url, "https://s.example/i/abc");
        assert_eq!(
            shared.delete_url.as_deref(),
            Some("https://s.example/d/abc")
        );
        assert_eq!(
            SELF_HOSTED_PROVIDER.parse(&settings("https://s.example/up"), b"nope"),
            Err(UploadError::BadResponse)
        );
    }

    #[test]
    fn the_token_keeps_the_account_it_was_always_saved_under() {
        assert_eq!(
            DESCRIPTOR.field(TOKEN).unwrap().account,
            Some("host-token-v1")
        );
        assert_eq!(
            DESCRIPTOR.field(TOKEN).unwrap().env,
            Some("DEGENVIEW_SNAPSHOT_TOKEN")
        );
    }
}

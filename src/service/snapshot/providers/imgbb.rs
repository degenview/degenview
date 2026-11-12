//! ImgBB (<https://imgbb.com>), a free image host with an API: the user's own API key, and an
//! image that can delete itself after a time they choose.
//!
//! The call is the one ImgBB documents (<https://api.imgbb.com>): a `POST` of the image as a
//! multipart form to `/1/upload`, the `key` and the optional `expiration` (seconds, 60 to
//! 15552000) in the query string as the documentation shows them. The key is therefore in the
//! URL: it travels over TLS, and this app never prints, logs or shows a request URL.
//!
//! Not stated by ImgBB's documentation, and so assumed here: the shape of an error answer (read
//! as `{"error": {"message": …}}`; anything else is read by its HTTP status alone).

use super::{
    Choice, FieldKind, FieldSpec, Link, Logo, ProviderDescriptor, Purpose, Request, Secrets,
    SnapshotProvider, common_failure,
};
use crate::model::chart::snapshot::{FieldValues, SharedSnapshot, UploadError, checked_link};
use crate::model::webhook::policy::url_encode;
use crate::service::net::multipart::Form;
use crate::service::snapshot::upload::HostResponse;
use eframe::egui;
use serde::Deserialize;
use std::borrow::Cow;

pub const ID: &str = "imgbb";
const KEY: &str = "key";
const EXPIRY: &str = "expiry";

/// Where uploads go. A constant, with a way to point a test at a local server.
const ENDPOINT: &str = "https://api.imgbb.com/1/upload";
/// What the user agrees to upload to.
const CONSENT_KEY: &str = "api.imgbb.com";

/// Where a developer or a machine with no secure storage gives the key.
pub const KEY_ENV: &str = "DEGENVIEW_IMGBB_KEY";

/// The shortest time ImgBB keeps an image that is to be deleted.
const TEST_SECONDS: u64 = 60;
/// The longest an API key is taken to be: far more than any key is likely to be, so a typo of
/// pasted text is caught without refusing a key ImgBB might issue longer.
const MAX_KEY_CHARS: usize = 128;

const DAY: u64 = 24 * 60 * 60;

static EXPIRIES: [Choice; 6] = [
    Choice {
        value: "1d",
        label: "1 day",
    },
    Choice {
        value: "7d",
        label: "1 week",
    },
    Choice {
        value: "30d",
        label: "30 days",
    },
    Choice {
        value: "90d",
        label: "90 days",
    },
    Choice {
        value: "180d",
        label: "180 days",
    },
    Choice {
        value: "never",
        label: "Never",
    },
];

/// How long an uploaded image is kept, by its saved choice; `None` is for ever.
fn expiry_seconds(choice: &str) -> Option<u64> {
    match choice {
        "1d" => Some(DAY),
        "7d" => Some(7 * DAY),
        "30d" => Some(30 * DAY),
        "90d" => Some(90 * DAY),
        "180d" => Some(180 * DAY),
        "never" => None,
        // A saved value this build does not know: the default, not for ever.
        _ => Some(30 * DAY),
    }
}

/// ImgBB's mark, drawn like the logos of data sources and brokers.
fn logo() -> egui::ImageSource<'static> {
    egui::include_image!("../../../../assets/brand/imgbb.png")
}

fn key_problem(text: &str) -> Option<&'static str> {
    let text = text.trim();
    (text.chars().count() > MAX_KEY_CHARS || !text.chars().all(|c| c.is_ascii_graphic()))
        .then_some("An API key has no spaces and is at most 128 plain characters.")
}

static DESCRIPTOR: ProviderDescriptor = ProviderDescriptor {
    id: ID,
    name: "ImgBB",
    summary: "Image hosting with your own API key",
    logo: Logo::Image(logo),
    setup_steps: &[
        "Sign in to ImgBB, or create a free account.",
        "Open the API page and copy your API key.",
        "Paste it below and turn ImgBB on.",
    ],
    links: &[
        Link {
            label: "Get an ImgBB API key",
            url: "https://api.imgbb.com/",
        },
        Link {
            label: "ImgBB terms of service",
            url: "https://imgbb.com/tos",
        },
        Link {
            label: "ImgBB privacy policy",
            url: "https://imgbb.com/privacy",
        },
    ],
    notice: "Images are uploaded to ImgBB with your API key. Anyone who has the link can open the image, and ImgBB's terms apply. ImgBB is asked to delete the image after the time you choose; with Never it keeps it.",
    fields: &[
        FieldSpec {
            key: KEY,
            title: "ImgBB API key",
            hint: "Kept in your system's secure storage, never in the settings or synced.",
            placeholder: "",
            kind: FieldKind::Secret,
            required: true,
            default: "",
            env: Some(KEY_ENV),
            account: None,
            needed: "Add your ImgBB API key in Settings ▸ Snapshots to share.",
            remove_label: "Remove key",
            validate: key_problem,
        },
        FieldSpec {
            key: EXPIRY,
            title: "Keep images for",
            hint: "ImgBB is asked to delete the image after this time, and the link stops working then.",
            placeholder: "",
            kind: FieldKind::Choice(&EXPIRIES),
            required: false,
            default: "30d",
            env: None,
            account: None,
            needed: "",
            remove_label: "",
            validate: super::no_check,
        },
    ],
    testable: true,
};

pub struct ImgBb {
    endpoint: &'static str,
}

pub static IMGBB: ImgBb = ImgBb { endpoint: ENDPOINT };

impl ImgBb {
    /// The provider pointed at another address: a test's local server.
    #[cfg(test)]
    pub fn at(endpoint: &'static str) -> Self {
        Self { endpoint }
    }
}

/// What ImgBB answers to an upload.
#[derive(Deserialize)]
struct Answer {
    data: Option<AnswerData>,
}

#[derive(Deserialize)]
struct AnswerData {
    url: Option<String>,
    display_url: Option<String>,
    delete_url: Option<String>,
    /// Seconds the image is kept, as a string ("0" for ever) or a number.
    expiration: Option<serde_json::Value>,
    /// When it was uploaded, in seconds since 1970, as a string or a number.
    time: Option<serde_json::Value>,
}

fn seconds(value: &serde_json::Value) -> Option<u64> {
    match value {
        serde_json::Value::String(text) => text.trim().parse().ok(),
        serde_json::Value::Number(number) => number.as_u64(),
        _ => None,
    }
}

/// `2026-12-11T12:00:00Z` for `epoch_seconds`.
fn utc_timestamp(epoch_seconds: u64) -> String {
    let seconds = i64::try_from(epoch_seconds).unwrap_or(i64::MAX / 2);
    let (year, month, day) = crate::format::civil_from_days(seconds.div_euclid(86_400));
    let in_day = seconds.rem_euclid(86_400);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        in_day / 3_600,
        in_day % 3_600 / 60,
        in_day % 60
    )
}

/// An error answer: `{"error": {"message": "Invalid API v1 key."}}`.
#[derive(Deserialize)]
struct Failure {
    error: Option<FailureDetail>,
}

#[derive(Deserialize)]
struct FailureDetail {
    message: Option<String>,
}

impl SnapshotProvider for ImgBb {
    fn descriptor(&self) -> &'static ProviderDescriptor {
        &DESCRIPTOR
    }

    fn consent_key(&self, _: &FieldValues) -> String {
        CONSENT_KEY.to_owned()
    }

    fn status_line(&self, settings: &FieldValues) -> String {
        let choice = DESCRIPTOR
            .field(EXPIRY)
            .and_then(|field| field.choice(settings.get(EXPIRY).map(String::as_str)));
        match choice {
            Some(choice) if choice.value == "never" => "Keeps images for ever".to_owned(),
            Some(choice) => format!("Keeps images for {}", choice.label),
            None => DESCRIPTOR.summary.to_owned(),
        }
    }

    fn request<'a>(
        &self,
        settings: &FieldValues,
        secrets: &Secrets,
        png: &'a [u8],
        purpose: Purpose,
    ) -> Result<Request<'a>, UploadError> {
        let key = secrets
            .get(KEY)
            .map(str::trim)
            .filter(|key| !key.is_empty() && key_problem(key).is_none())
            .ok_or(UploadError::Unauthorized)?;
        let expiration = match purpose {
            // A check asks for the shortest life ImgBB allows, so it is not left behind.
            Purpose::Test => Some(TEST_SECONDS),
            Purpose::Share => expiry_seconds(
                DESCRIPTOR
                    .field(EXPIRY)
                    .and_then(|field| field.choice(settings.get(EXPIRY).map(String::as_str)))
                    .map_or("30d", |choice| choice.value),
            ),
        };
        let mut url = format!("{}?key={}", self.endpoint, url_encode(key));
        if let Some(expiration) = expiration {
            url.push_str(&format!("&expiration={expiration}"));
        }
        let form = Form::new().file("image", "snapshot.png", "image/png", png);
        Ok(Request {
            url,
            headers: Vec::new(),
            content_type: form.content_type(),
            body: Cow::Owned(form.finish()),
        })
    }

    fn parse(&self, _: &FieldValues, body: &[u8]) -> Result<SharedSnapshot, UploadError> {
        let answer: Answer = serde_json::from_slice(body).map_err(|_| UploadError::BadResponse)?;
        let data = answer.data.ok_or(UploadError::BadResponse)?;
        // The direct link to the image: it opens, embeds and previews anywhere.
        let url = [&data.url, &data.display_url]
            .into_iter()
            .flatten()
            .find_map(|link| checked_link(link, false))
            .ok_or(UploadError::BadResponse)?;
        let expires_at = match (
            data.time.as_ref().and_then(seconds),
            data.expiration.as_ref().and_then(seconds),
        ) {
            (Some(time), Some(kept)) if kept > 0 => Some(utc_timestamp(time.saturating_add(kept))),
            _ => None,
        };
        Ok(SharedSnapshot {
            url,
            delete_url: data
                .delete_url
                .as_deref()
                .and_then(|link| checked_link(link, false)),
            expires_at,
        })
    }

    fn classify_failure(&self, response: &HostResponse) -> UploadError {
        let message = serde_json::from_slice::<Failure>(&response.body)
            .ok()
            .and_then(|failure| failure.error)
            .and_then(|error| error.message)
            .unwrap_or_default()
            .to_ascii_lowercase();
        // A key ImgBB does not know is a 400 that says so, not a 401.
        if matches!(response.status, 400 | 401 | 403) && message.contains("key") {
            return UploadError::Unauthorized;
        }
        if message.contains("too large") || (message.contains("max") && message.contains("size")) {
            return UploadError::RejectedAsTooLarge;
        }
        common_failure(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(expiry: &str) -> FieldValues {
        FieldValues::from([(EXPIRY.to_owned(), expiry.to_owned())])
    }

    fn secrets(key: &str) -> Secrets {
        let mut secrets = Secrets::default();
        secrets.insert(KEY, key.to_owned());
        secrets
    }

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nDATA";

    fn request(settings: &FieldValues, key: &str, purpose: Purpose) -> Request<'static> {
        IMGBB
            .request(settings, &secrets(key), PNG, purpose)
            .expect("a request")
    }

    #[test]
    fn the_image_is_a_binary_file_in_a_multipart_form_and_nothing_secret_is_in_it() {
        let request = request(&settings("7d"), "abc123", Purpose::Share);
        assert!(
            request
                .content_type
                .starts_with("multipart/form-data; boundary=")
        );
        let body = String::from_utf8_lossy(&request.body).into_owned();
        assert!(
            body.contains("name=\"image\"; filename=\"snapshot.png\""),
            "{body}"
        );
        assert!(body.contains("Content-Type: image/png"), "{body}");
        assert!(body.contains("DATA"), "{body}");
        assert!(!body.contains("abc123"), "the key is not in the body");
        assert!(request.headers.is_empty());
    }

    #[test]
    fn the_key_and_the_lifetime_ride_in_the_query_as_the_documentation_shows() {
        let kept = request(&settings("7d"), "abc123", Purpose::Share);
        assert_eq!(
            kept.url,
            "https://api.imgbb.com/1/upload?key=abc123&expiration=604800"
        );
        // The default is 30 days, and never leaves the parameter out.
        assert!(
            request(&FieldValues::new(), "k", Purpose::Share)
                .url
                .ends_with("&expiration=2592000")
        );
        assert!(
            request(&settings("garbage"), "k", Purpose::Share)
                .url
                .ends_with("&expiration=2592000")
        );
        assert_eq!(
            request(&settings("never"), "k", Purpose::Share).url,
            "https://api.imgbb.com/1/upload?key=k"
        );
    }

    #[test]
    fn every_choice_is_a_lifetime_imgbb_accepts() {
        for choice in &EXPIRIES {
            if let Some(seconds) = expiry_seconds(choice.value) {
                assert!((60..=15_552_000).contains(&seconds), "{}", choice.value);
            }
        }
        assert_eq!(expiry_seconds("180d"), Some(15_552_000));
    }

    #[test]
    fn a_test_leaves_nothing_behind() {
        let test = request(&settings("never"), "k", Purpose::Test);
        assert!(test.url.ends_with("&expiration=60"), "{}", test.url);
    }

    #[test]
    fn the_key_is_encoded_and_a_missing_or_odd_one_sends_nothing() {
        // Nothing a key could hold can add a parameter to the URL.
        let odd = IMGBB.request(&FieldValues::new(), &secrets("a&b=c"), PNG, Purpose::Share);
        assert!(odd.unwrap().url.contains("key=a%26b%3Dc"));
        for key in ["", "  ", "has space", "é"] {
            assert!(
                matches!(
                    IMGBB.request(&FieldValues::new(), &secrets(key), PNG, Purpose::Share),
                    Err(UploadError::Unauthorized)
                ),
                "{key:?}"
            );
        }
        assert!(matches!(
            IMGBB.request(
                &FieldValues::new(),
                &Secrets::default(),
                PNG,
                Purpose::Share
            ),
            Err(UploadError::Unauthorized)
        ));
    }

    const ANSWER: &str = r#"{
        "data": {
            "id": "2ndCYJK", "title": "snapshot",
            "url_viewer": "https://ibb.co/2ndCYJK",
            "url": "https://i.ibb.co/w04Prt6/c1f64245afb2.png",
            "display_url": "https://i.ibb.co/98W13PY/c1f64245afb2.png",
            "width": "1", "height": "1", "size": "42",
            "time": "1552042565", "expiration": "600",
            "delete_url": "https://ibb.co/2ndCYJK/670a7e48ddcb85ac340c717a41047e5c"
        },
        "success": true, "status": 200
    }"#;

    #[test]
    fn the_answer_gives_the_direct_link_the_delete_link_and_when_it_expires() {
        let shared = IMGBB.parse(&FieldValues::new(), ANSWER.as_bytes()).unwrap();
        assert_eq!(shared.url, "https://i.ibb.co/w04Prt6/c1f64245afb2.png");
        assert_eq!(
            shared.delete_url.as_deref(),
            Some("https://ibb.co/2ndCYJK/670a7e48ddcb85ac340c717a41047e5c")
        );
        // 2019-03-08 10:56:05 UTC + 600 s.
        assert_eq!(shared.expires_at.as_deref(), Some("2019-03-08T11:06:05Z"));
    }

    #[test]
    fn an_image_that_is_kept_for_ever_has_no_expiry_and_numbers_are_read_too() {
        let forever = ANSWER.replace("\"expiration\": \"600\"", "\"expiration\": \"0\"");
        assert_eq!(
            IMGBB
                .parse(&FieldValues::new(), forever.as_bytes())
                .unwrap()
                .expires_at,
            None
        );
        let numbers = ANSWER
            .replace("\"time\": \"1552042565\"", "\"time\": 1552042565")
            .replace("\"expiration\": \"600\"", "\"expiration\": 600");
        assert_eq!(
            IMGBB
                .parse(&FieldValues::new(), numbers.as_bytes())
                .unwrap()
                .expires_at
                .as_deref(),
            Some("2019-03-08T11:06:05Z")
        );
    }

    #[test]
    fn the_display_link_stands_in_and_a_link_that_is_not_https_is_no_link() {
        let only_display = r#"{"data":{"display_url":"https://i.ibb.co/x/y.png"}}"#;
        assert_eq!(
            IMGBB
                .parse(&FieldValues::new(), only_display.as_bytes())
                .unwrap()
                .url,
            "https://i.ibb.co/x/y.png"
        );
        for body in [
            r#"{"data":{"url":"http://i.ibb.co/x/y.png"}}"#,
            r#"{"data":{"url":"https://u:p@i.ibb.co/x/y.png"}}"#,
            r#"{"data":{}}"#,
            r#"{"success":false}"#,
            "<html>",
            "",
        ] {
            assert_eq!(
                IMGBB.parse(&FieldValues::new(), body.as_bytes()),
                Err(UploadError::BadResponse),
                "{body}"
            );
        }
        // A delete link that is not usable is dropped, the image link is kept.
        let bad_delete =
            r#"{"data":{"url":"https://i.ibb.co/x/y.png","delete_url":"javascript:alert(1)"}}"#;
        let shared = IMGBB
            .parse(&FieldValues::new(), bad_delete.as_bytes())
            .unwrap();
        assert_eq!(shared.delete_url, None);
    }

    fn failure(status: u16, body: &str) -> UploadError {
        IMGBB.classify_failure(&HostResponse {
            status,
            retry_after: Some(5),
            body: body.as_bytes().to_vec(),
        })
    }

    #[test]
    fn what_imgbb_says_is_told_apart() {
        let bad_key = r#"{"status_code":400,"error":{"message":"Invalid API v1 key.","code":100},"status_txt":"Bad Request"}"#;
        assert_eq!(failure(400, bad_key), UploadError::Unauthorized);
        assert_eq!(failure(403, bad_key), UploadError::Unauthorized);
        assert_eq!(failure(401, ""), UploadError::Unauthorized);
        let too_big = r#"{"error":{"message":"File too large."}}"#;
        assert_eq!(failure(400, too_big), UploadError::RejectedAsTooLarge);
        assert_eq!(failure(413, ""), UploadError::RejectedAsTooLarge);
        assert_eq!(
            failure(429, ""),
            UploadError::RateLimited {
                retry_after: Some(5)
            }
        );
        assert_eq!(failure(500, "oops"), UploadError::Server(500));
        // A 400 that is about something else is ImgBB's failure, not the key.
        assert_eq!(
            failure(400, r#"{"error":{"message":"Empty upload source."}}"#),
            UploadError::Server(400)
        );
        assert_eq!(failure(302, ""), UploadError::BadResponse);
    }

    #[test]
    fn the_row_says_how_long_images_are_kept() {
        assert_eq!(
            IMGBB.status_line(&FieldValues::new()),
            "Keeps images for 30 days"
        );
        assert_eq!(IMGBB.status_line(&settings("1d")), "Keeps images for 1 day");
        assert_eq!(
            IMGBB.status_line(&settings("never")),
            "Keeps images for ever"
        );
    }

    #[test]
    fn it_is_ready_with_a_key_and_asks_for_one_without() {
        use crate::service::snapshot::providers::{Readiness, SecretPresence};
        let mut present = SecretPresence::default();
        assert_eq!(
            IMGBB.readiness(&FieldValues::new(), &present),
            Readiness::Needs("Add your ImgBB API key in Settings ▸ Snapshots to share.".to_owned())
        );
        present.insert(KEY);
        assert_eq!(
            IMGBB.readiness(&FieldValues::new(), &present),
            Readiness::Ready
        );
        assert_eq!(IMGBB.consent_key(&FieldValues::new()), "api.imgbb.com");
        assert_eq!(IMGBB.label(&FieldValues::new()), "ImgBB");
    }

    #[test]
    fn the_key_is_checked_as_it_is_typed() {
        assert_eq!(key_problem("0123456789abcdef0123456789abcdef"), None);
        assert!(key_problem("has space").is_some());
        assert!(key_problem(&"a".repeat(129)).is_some());
        assert!(key_problem("é").is_some());
    }

    #[test]
    fn a_timestamp_is_utc() {
        assert_eq!(utc_timestamp(0), "1970-01-01T00:00:00Z");
        assert_eq!(utc_timestamp(1_552_042_565), "2019-03-08T10:56:05Z");
    }
}

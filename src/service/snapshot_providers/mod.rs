//! Where a shared snapshot is uploaded: the providers, and everything the rest of the app needs to
//! know about one without knowing which it is.
//!
//! A provider is a [`ProviderDescriptor`] (its name, how to set it up, the fields it asks for) and
//! a few pure functions: how to build the request for a PNG, how to read the answer. It does no
//! I/O. The upload job, the consent question, the camera menu, the Settings page, iCloud Sync and
//! the secret store all work from the descriptor and the trait, so **adding a provider is one new
//! file here and one line in [`registry`]**:
//!
//! 1. Write the descriptor: `id` (kept forever, it is saved), `name`, the setup steps, the
//!    `fields` (text, secret or a choice; every field title must be unique across providers),
//!    and a `notice` the user reads before the first upload.
//! 2. Implement [`SnapshotProvider`]: `request` builds the HTTP request, `parse` reads a success
//!    answer, `classify_failure` (optional) reads an error one, `consent_key` names what the user
//!    agrees to upload to.
//! 3. Add it to [`registry`], in the order the Settings page lists them.
//!
//! The Settings page draws one row per provider from its descriptor. Secrets go to the system's
//! secure storage under `"{provider}.{field}-v1"`; the other fields are saved with the setting
//! `snapshot.prefs`, which syncs.

pub mod imgbb;
pub mod self_hosted;

use crate::model::chart::snapshot::{FieldValues, SharedSnapshot, UploadError};
use crate::service::snapshot_upload::HostResponse;
use eframe::egui;
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};

// MARK: Descriptor

/// A neutral mark for a provider that has no logo of its own.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Glyph {
    /// A server the user runs.
    Server,
}

/// The mark in a provider's row.
#[derive(Clone, Copy)]
pub enum Logo {
    Glyph(Glyph),
    /// The service's own logo.
    Image(fn() -> egui::ImageSource<'static>),
}

/// One answer a choice field offers.
#[derive(Clone, Copy, Debug)]
pub struct Choice {
    /// What is saved.
    pub value: &'static str,
    /// What the user reads.
    pub label: &'static str,
}

#[derive(Clone, Copy, Debug)]
pub enum FieldKind {
    Text,
    /// Kept in the system's secure storage, never in the settings.
    Secret,
    /// One of a fixed list; saved the moment it is picked.
    Choice(&'static [Choice]),
}

/// A field a provider asks the user for.
pub struct FieldSpec {
    /// What it is saved under. Never changes once shipped.
    pub key: &'static str,
    /// The label over the field. Unique across providers: it also identifies the widget.
    pub title: &'static str,
    /// The line under the field.
    pub hint: &'static str,
    /// What an empty field shows.
    pub placeholder: &'static str,
    pub kind: FieldKind,
    /// Sharing cannot start without it.
    pub required: bool,
    /// The choice used until the user picks one.
    pub default: &'static str,
    /// An environment variable that stands in for a secret nobody saved.
    pub env: Option<&'static str>,
    /// The secure-storage account, where it is not the usual `"{provider}.{key}-v1"` (a field
    /// that existed before this scheme keeps the account it was saved under).
    pub account: Option<&'static str>,
    /// What the camera menu says while a required field is empty.
    pub needed: &'static str,
    /// The button that removes a saved secret ("Remove key"); only a secret has one.
    pub remove_label: &'static str,
    /// What is wrong with a typed value, in a sentence; `None` when it is fine. Empty is never
    /// checked: an empty field clears it.
    pub validate: fn(&str) -> Option<&'static str>,
}

/// A `validate` for a field anything may be typed into.
pub fn no_check(_: &str) -> Option<&'static str> {
    None
}

impl FieldSpec {
    pub fn is_secret(&self) -> bool {
        matches!(self.kind, FieldKind::Secret)
    }

    /// The saved choice, or the default when nothing valid is saved.
    pub fn choice<'a>(&'a self, saved: Option<&str>) -> Option<&'a Choice> {
        let FieldKind::Choice(choices) = &self.kind else {
            return None;
        };
        choices
            .iter()
            .find(|choice| Some(choice.value) == saved)
            .or_else(|| choices.iter().find(|choice| choice.value == self.default))
    }
}

/// A link in a provider's setup, always shown gray and underlined.
#[derive(Clone, Copy, Debug)]
pub struct Link {
    pub label: &'static str,
    pub url: &'static str,
}

pub struct ProviderDescriptor {
    /// Saved in the settings; never changes once shipped.
    pub id: &'static str,
    pub name: &'static str,
    /// The line under the name while the provider is off.
    pub summary: &'static str,
    pub logo: Logo,
    /// Numbered, in order.
    pub setup_steps: &'static [&'static str],
    pub links: &'static [Link],
    /// What the user should know before an image goes there: who can see it, whose terms apply.
    pub notice: &'static str,
    pub fields: &'static [FieldSpec],
    /// Whether the provider can check its settings with a real (tiny) upload.
    pub testable: bool,
}

impl ProviderDescriptor {
    pub fn field(&self, key: &str) -> Option<&FieldSpec> {
        self.fields.iter().find(|field| field.key == key)
    }

    pub fn secret_fields(&self) -> impl Iterator<Item = &FieldSpec> {
        self.fields.iter().filter(|field| field.is_secret())
    }
}

// MARK: Requests

/// A request for a provider to send: what goes on the wire, no more.
pub struct Request<'a> {
    pub url: String,
    pub headers: Vec<(&'static str, String)>,
    pub content_type: String,
    pub body: Cow<'a, [u8]>,
}

/// Why an upload is made.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Purpose {
    /// The user is sharing a chart: the image stays as long as the settings say.
    Share,
    /// The user is checking their settings with a tiny image, which a provider should not keep.
    Test,
}

/// A provider's secrets, read for one upload. Held in memory only as long as the upload.
#[derive(Clone, Default)]
pub struct Secrets(BTreeMap<String, String>);

impl Secrets {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(String::as_str)
    }

    pub fn insert(&mut self, key: &str, value: String) {
        self.0.insert(key.to_owned(), value);
    }
}

impl std::fmt::Debug for Secrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The keys only: a log line must never carry a value.
        f.debug_list().entries(self.0.keys()).finish()
    }
}

/// Which of a provider's secret fields have a value (saved, or given by the environment).
#[derive(Clone, Debug, Default)]
pub struct SecretPresence {
    present: BTreeSet<String>,
    /// The secure storage has not said yet whether some secret is saved.
    pending: bool,
}

impl SecretPresence {
    pub fn has(&self, key: &str) -> bool {
        self.present.contains(key)
    }

    pub fn insert(&mut self, key: &str) {
        self.present.insert(key.to_owned());
    }

    /// Whether some answer is still to come, so "no key" may only mean "not known yet".
    pub fn is_pending(&self) -> bool {
        self.pending
    }

    pub fn mark_pending(&mut self) {
        self.pending = true;
    }
}

/// Whether sharing can start, and if not what the user has to do.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Readiness {
    Ready,
    Needs(String),
}

// MARK: The trait

pub trait SnapshotProvider: Send + Sync {
    fn descriptor(&self) -> &'static ProviderDescriptor;

    /// Whether `settings` and the secrets present are enough to upload. The default asks for
    /// every required field; a provider that can say more (an address that does not parse)
    /// overrides it.
    fn readiness(&self, settings: &FieldValues, present: &SecretPresence) -> Readiness {
        let descriptor = self.descriptor();
        for field in descriptor.fields.iter().filter(|field| field.required) {
            let filled = if field.is_secret() {
                present.has(field.key)
            } else {
                settings
                    .get(field.key)
                    .is_some_and(|value| !value.trim().is_empty())
            };
            if !filled {
                return Readiness::Needs(field.needed.to_owned());
            }
        }
        Readiness::Ready
    }

    /// Where the image is going, as the user reads it ("ImgBB", "snapshots.example.com").
    fn label(&self, _settings: &FieldValues) -> String {
        self.descriptor().name.to_owned()
    }

    /// What the user agrees to upload to, remembered per destination: a service's address, or a
    /// host's `host[:port]`. Two settings with the same key share one consent.
    fn consent_key(&self, settings: &FieldValues) -> String;

    /// What else the user should be told before the first upload, given the secrets that exist:
    /// anything that is sent along with the image besides the image itself.
    fn consent_note(&self, _settings: &FieldValues, _present: &SecretPresence) -> Option<String> {
        None
    }

    /// The line under the name while the provider is on and ready.
    fn status_line(&self, _settings: &FieldValues) -> String {
        self.descriptor().summary.to_owned()
    }

    /// Builds the request that uploads `png`. Fails (without sending anything) for a secret or
    /// setting the provider cannot use.
    fn request<'a>(
        &self,
        settings: &FieldValues,
        secrets: &Secrets,
        png: &'a [u8],
        purpose: Purpose,
    ) -> Result<Request<'a>, UploadError>;

    /// Reads the answer to an upload that went through (HTTP 200 or 201).
    fn parse(&self, settings: &FieldValues, body: &[u8]) -> Result<SharedSnapshot, UploadError>;

    /// Reads any other answer. The default covers what every host says; a provider with its own
    /// error body (a refused key that comes as a 400) overrides it.
    fn classify_failure(&self, response: &HostResponse) -> UploadError {
        common_failure(response)
    }
}

/// What a status means whatever the host: no access, too big, slow down, a redirect (never
/// followed: the image goes only where the user agreed), or a failure on the host's side.
pub fn common_failure(response: &HostResponse) -> UploadError {
    match response.status {
        401 | 403 => UploadError::Unauthorized,
        413 => UploadError::RejectedAsTooLarge,
        429 => UploadError::RateLimited {
            retry_after: response.retry_after,
        },
        300..=399 => UploadError::BadResponse,
        other => UploadError::Server(other),
    }
}

// MARK: Registry

static REGISTRY: [&dyn SnapshotProvider; 2] = [&imgbb::IMGBB, &self_hosted::SELF_HOSTED_PROVIDER];

/// Every provider, in the order the Settings page lists them.
pub fn registry() -> &'static [&'static dyn SnapshotProvider] {
    &REGISTRY
}

pub fn provider(id: &str) -> Option<&'static dyn SnapshotProvider> {
    registry()
        .iter()
        .copied()
        .find(|provider| provider.descriptor().id == id)
}

// MARK: Destination

/// A provider with the settings to use it: where one upload goes.
#[derive(Clone)]
pub struct Destination {
    provider: &'static dyn SnapshotProvider,
    settings: FieldValues,
    /// What the consent question adds to the provider's notice.
    note: Option<String>,
}

impl Destination {
    pub fn new(provider: &'static dyn SnapshotProvider, settings: FieldValues) -> Self {
        Self {
            provider,
            settings,
            note: None,
        }
    }

    pub fn with_note(mut self, note: Option<String>) -> Self {
        self.note = note;
        self
    }

    /// Anything besides the image that goes with it, which the consent question names.
    pub fn note(&self) -> Option<&str> {
        self.note.as_deref()
    }

    pub fn provider(&self) -> &'static dyn SnapshotProvider {
        self.provider
    }

    pub fn settings(&self) -> &FieldValues {
        &self.settings
    }

    pub fn id(&self) -> &'static str {
        self.provider.descriptor().id
    }

    pub fn label(&self) -> String {
        self.provider.label(&self.settings)
    }

    pub fn consent_key(&self) -> String {
        self.provider.consent_key(&self.settings)
    }

    pub fn notice(&self) -> &'static str {
        self.provider.descriptor().notice
    }
}

impl std::fmt::Debug for Destination {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Destination")
            .field("provider", &self.id())
            .field("settings", &self.settings)
            .finish()
    }
}

impl PartialEq for Destination {
    fn eq(&self, other: &Self) -> bool {
        self.id() == other.id() && self.settings == other.settings
    }
}

impl Eq for Destination {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_provider_is_described_consistently() {
        let mut ids = BTreeSet::new();
        let mut titles = BTreeSet::new();
        for provider in registry() {
            let descriptor = provider.descriptor();
            assert!(
                !descriptor.id.is_empty()
                    && descriptor
                        .id
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                "{}: an id is lower-case letters, digits and _",
                descriptor.id
            );
            assert!(
                ids.insert(descriptor.id),
                "{} is listed twice",
                descriptor.id
            );
            assert!(!descriptor.name.is_empty() && !descriptor.summary.is_empty());
            assert!(
                !descriptor.notice.is_empty(),
                "{}: say what the user should know",
                descriptor.id
            );
            let mut keys = BTreeSet::new();
            for field in descriptor.fields {
                assert!(
                    keys.insert(field.key),
                    "{}.{} is declared twice",
                    descriptor.id,
                    field.key
                );
                assert!(
                    titles.insert(field.title),
                    "the title {:?} is used by two fields: it identifies the widget",
                    field.title
                );
                if field.required {
                    assert!(
                        !field.needed.is_empty(),
                        "{}.{} is required, so it needs a `needed` sentence",
                        descriptor.id,
                        field.key
                    );
                }
                if let FieldKind::Choice(choices) = field.kind {
                    assert!(
                        choices.iter().any(|choice| choice.value == field.default),
                        "{}.{}: the default is not one of the choices",
                        descriptor.id,
                        field.key
                    );
                }
                if let Some(env) = field.env {
                    assert!(
                        field.is_secret(),
                        "{env}: only a secret has an environment stand-in"
                    );
                    assert!(env.starts_with("DEGENVIEW_"), "{env}");
                }
            }
        }
    }

    #[test]
    fn a_provider_is_found_by_its_id() {
        assert!(provider("imgbb").is_some());
        assert!(provider(crate::model::chart::snapshot::SELF_HOSTED).is_some());
        assert!(provider("nope").is_none());
        assert_eq!(
            registry().first().map(|p| p.descriptor().id),
            Some("imgbb"),
            "ImgBB is listed first"
        );
    }

    #[test]
    fn a_destination_is_a_provider_with_its_settings() {
        let destination = Destination::new(provider("imgbb").expect("imgbb"), FieldValues::new());
        assert_eq!(destination.id(), "imgbb");
        assert_eq!(destination.label(), "ImgBB");
        assert_eq!(destination.consent_key(), "api.imgbb.com");
        let other = Destination::new(
            provider("imgbb").expect("imgbb"),
            FieldValues::from([("expiry".to_owned(), "7d".to_owned())]),
        );
        assert_ne!(destination, other, "its settings are part of it");
    }

    #[test]
    fn what_every_host_says_is_told_apart() {
        let answer = |status, retry_after| HostResponse {
            status,
            retry_after,
            body: Vec::new(),
        };
        assert_eq!(
            common_failure(&answer(401, None)),
            UploadError::Unauthorized
        );
        assert_eq!(
            common_failure(&answer(403, None)),
            UploadError::Unauthorized
        );
        assert_eq!(
            common_failure(&answer(413, None)),
            UploadError::RejectedAsTooLarge
        );
        assert_eq!(
            common_failure(&answer(429, Some(9))),
            UploadError::RateLimited {
                retry_after: Some(9)
            }
        );
        assert_eq!(common_failure(&answer(302, None)), UploadError::BadResponse);
        assert_eq!(common_failure(&answer(503, None)), UploadError::Server(503));
    }

    #[test]
    fn a_secret_is_never_printed() {
        let mut secrets = Secrets::default();
        secrets.insert("key", "hunter2".to_owned());
        assert!(!format!("{secrets:?}").contains("hunter2"));
        assert_eq!(secrets.get("key"), Some("hunter2"));
    }
}

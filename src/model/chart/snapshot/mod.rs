//! Chart snapshots as pure values: what can be asked of one, how big the image may be, what the
//! file is called, where a shared one is hosted and what the host may answer. The rendering, the
//! file and clipboard work and the upload live elsewhere; this is the part every one of them
//! agrees on.
//!
//! The Screenshot Studio's values are the children: what it is asked for ([`studio`]), how the
//! view is reframed ([`framing`]), the image's layout and size ([`composition`]) and the rectangle
//! the user selects ([`selection`]).

pub mod composition;
pub mod framing;
pub mod selection;
pub mod studio;

use self::studio::StudioPrefs;
use crate::format;
use crate::model::webhook::{self, policy::ParsedUrl};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The longest side of a snapshot, in pixels.
pub const MAX_SIDE_PX: u32 = 8192;
/// The most pixels a snapshot holds: 32 Mi pixels is 128 MiB of RGBA while it is drawn.
pub const MAX_PIXELS: u64 = 33_554_432;
/// The smallest scale a snapshot is drawn at; a chart so large it needs less is refused.
const MIN_SCALE: f32 = 0.25;
/// The most a PNG may weigh to be uploaded.
pub const MAX_UPLOAD_BYTES: usize = 24 * 1024 * 1024;
/// The most a PNG may weigh to be returned to an MCP client, which gets it base64-encoded (a third
/// larger) in one reply. An ordinary chart is a few hundred KiB; one this large holds a very
/// detailed image drawing, and the client is told to ask at `1x`.
pub const MAX_MCP_PNG_BYTES: usize = 8 * 1024 * 1024;
/// The longest answer from a snapshot host that is read.
pub const MAX_RESPONSE_BYTES: usize = 16 * 1024;
/// The longest link a host may hand back.
const MAX_LINK_CHARS: usize = 2048;
/// How long a part of a file name may be.
const MAX_NAME_PART: usize = 32;

/// What the camera menu offers.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SnapshotAction {
    Download,
    CopyImage,
    CopyLink,
    OpenInTab,
    ShareOnX,
}

impl SnapshotAction {
    pub const ALL: [Self; 5] = [
        Self::Download,
        Self::CopyImage,
        Self::CopyLink,
        Self::OpenInTab,
        Self::ShareOnX,
    ];

    /// The row in the menu.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Download => "Download image",
            Self::CopyImage => "Copy image",
            Self::CopyLink => "Copy link",
            Self::OpenInTab => "Open in new tab",
            Self::ShareOnX => "Tweet image",
        }
    }

    /// Whether the image has to be uploaded for this to work.
    pub const fn is_hosted(self) -> bool {
        matches!(self, Self::CopyLink | Self::OpenInTab | Self::ShareOnX)
    }
}

/// How many pixels an image holds per point of the chart.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum SnapshotScale {
    X1,
    #[default]
    X2,
}

impl SnapshotScale {
    pub const ALL: [Self; 2] = [Self::X1, Self::X2];

    pub const fn factor(self) -> f32 {
        match self {
            Self::X1 => 1.0,
            Self::X2 => 2.0,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::X1 => "Standard (1×)",
            Self::X2 => "High resolution (2×)",
        }
    }
}

/// What a snapshot includes beyond the chart itself.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SnapshotOptions {
    pub scale: SnapshotScale,
    /// Paper-trading orders and positions on the chart.
    pub include_trading: bool,
}

/// One snapshot asked for: which chart, what to do with the image, what it holds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SnapshotRequest {
    pub chart_id: u64,
    pub action: SnapshotAction,
    pub options: SnapshotOptions,
}

/// The size an image is drawn at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SnapshotDimensions {
    /// The scale the chart is drawn at, the asked one unless it would make the image too large.
    pub pixels_per_point: f32,
    pub width: u32,
    pub height: u32,
}

impl SnapshotDimensions {
    /// The image for a chart `width` × `height` points: drawn again at the scale, never
    /// stretched, and at a smaller one when the image would pass the limits. `None` for a chart
    /// with no size, or one so large even the smallest scale cannot hold it.
    pub fn fit(width: f32, height: f32, scale: SnapshotScale) -> Option<Self> {
        if !(width.is_finite() && height.is_finite()) || width < 1.0 || height < 1.0 {
            return None;
        }
        let (w, h) = (f64::from(width), f64::from(height));
        let by_side = f64::from(MAX_SIDE_PX) / w.max(h);
        let by_area = (MAX_PIXELS as f64 / (w * h)).sqrt();
        let fitted = f64::from(scale.factor()).min(by_side).min(by_area);
        if fitted < f64::from(MIN_SCALE) {
            return None;
        }
        // The scale is what the pixel counts are made of; rounding it down never passes a limit.
        let pixels_per_point = (fitted * 1000.0).floor() / 1000.0;
        let width_px = (w * pixels_per_point).round().max(1.0) as u32;
        let height_px = (h * pixels_per_point).round().max(1.0) as u32;
        (width_px <= MAX_SIDE_PX
            && height_px <= MAX_SIDE_PX
            && u64::from(width_px) * u64::from(height_px) <= MAX_PIXELS)
            .then_some(Self {
                pixels_per_point: pixels_per_point as f32,
                width: width_px,
                height: height_px,
            })
    }
}

// MARK: Names

/// One piece of a file name: letters, digits, `-` and `_` only, so it is safe on every file
/// system. Anything else (`/`, `:`, spaces, accents…) is dropped.
fn name_part(text: &str, fallback: &str) -> String {
    let part: String = text
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        .take(MAX_NAME_PART)
        .collect();
    if part.is_empty() {
        fallback.to_owned()
    } else {
        part
    }
}

/// `2026-10-19_1642`: the local date and time (`offset_secs` east of UTC) a file name carries.
fn stamp(epoch_ms: i64, offset_secs: i64) -> String {
    let local = epoch_ms.div_euclid(1_000) + offset_secs;
    let (year, month, day) = format::civil_from_days(local.div_euclid(86_400));
    let seconds = local.rem_euclid(86_400);
    format!(
        "{year:04}-{month:02}-{day:02}_{:02}{:02}",
        seconds / 3_600,
        seconds % 3_600 / 60,
    )
}

/// `DegenView_BTCUSDT_1H_2026-10-19_1642.png`: the chart's symbol and timeframe and the local
/// time (`offset_secs` east of UTC). A `1M` timeframe stays upper case so it is never read as
/// minutes.
pub fn file_name(symbol: &str, timeframe: &str, epoch_ms: i64, offset_secs: i64) -> String {
    format!(
        "DegenView_{}_{}_{}.png",
        name_part(symbol, "Chart"),
        name_part(timeframe, "Chart"),
        stamp(epoch_ms, offset_secs),
    )
}

/// `DegenView_Layout_2026-10-19_1642.png`: the name of an image of several charts, which has
/// no one symbol to carry.
pub fn layout_file_name(epoch_ms: i64, offset_secs: i64) -> String {
    format!("DegenView_Layout_{}.png", stamp(epoch_ms, offset_secs))
}

/// `name` with `.png` on it when it has no extension of its own.
pub fn with_png_extension(name: &str) -> String {
    if name.to_ascii_lowercase().ends_with(".png") {
        name.to_owned()
    } else {
        format!("{name}.png")
    }
}

/// The first of `name`, `name (2).png`, `name (3).png`… that `taken` does not claim.
pub fn unused_name(name: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(name) {
        return name.to_owned();
    }
    let stem = name.strip_suffix(".png").unwrap_or(name);
    (2..)
        .map(|n| format!("{stem} ({n}).png"))
        .find(|candidate| !taken(candidate))
        .expect("an unbounded range")
}

// MARK: Sharing on X

/// What the post says before the link.
pub fn share_text(title: &str, timeframe: &str) -> String {
    format!("{title} {timeframe} chart — DegenView")
}

/// X's composer, filled in with `text` and the snapshot's `link`. The link is passed as the
/// post's own link, so X shows it as a card; the image is not attached to the post.
pub fn x_compose_url(text: &str, link: &str) -> String {
    format!(
        "https://x.com/intent/post?text={}&url={}",
        webhook::policy::url_encode(text),
        webhook::policy::url_encode(link),
    )
}

// MARK: Hosting

/// Why a host address was refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostError {
    Empty,
    NotAUrl,
    /// Uploads go over HTTPS; plain HTTP is for a host on this computer only.
    NotHttps,
    /// A user name or password in the address would end up in logs and the settings file.
    Credentials,
    Fragment,
}

impl HostError {
    pub const fn message(self) -> &'static str {
        match self {
            Self::Empty => "Enter the address uploads are sent to.",
            Self::NotAUrl => "That is not a web address.",
            Self::NotHttps => "Use an https:// address. Plain http:// is only for this computer.",
            Self::Credentials => "Leave the user name and password out of the address.",
            Self::Fragment => "Remove the # part of the address.",
        }
    }
}

fn is_loopback(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1" || host == "::1"
}

/// The address uploaded snapshots are posted to, checked.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotHost {
    endpoint: String,
    parsed: ParsedUrl,
}

impl SnapshotHost {
    pub fn parse(text: &str) -> Result<Self, HostError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(HostError::Empty);
        }
        let parsed = webhook::policy::parse_url(text).ok_or(HostError::NotAUrl)?;
        if parsed.host.is_empty() {
            return Err(HostError::NotAUrl);
        }
        if parsed.userinfo.is_some() {
            return Err(HostError::Credentials);
        }
        if parsed.fragment.is_some() {
            return Err(HostError::Fragment);
        }
        match parsed.scheme.as_str() {
            "https" => {}
            "http" if is_loopback(&parsed.host) => {}
            _ => return Err(HostError::NotHttps),
        }
        Ok(Self {
            endpoint: text.to_owned(),
            parsed,
        })
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// `host` or `host:port`: what the user is shown and what a consent is remembered for.
    pub fn authority(&self) -> String {
        match self.parsed.port {
            Some(port) => format!("{}:{port}", self.parsed.host),
            None => self.parsed.host.clone(),
        }
    }

    /// Whether the host is this computer, where plain `http://` is accepted (for development).
    pub fn loopback(&self) -> bool {
        is_loopback(&self.parsed.host)
    }
}

/// A snapshot a host stored.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SharedSnapshot {
    /// Where anyone can open the image.
    pub url: String,
    /// Where the owner removes it, if the host offers that.
    pub delete_url: Option<String>,
    /// When the host says it forgets the image, as it wrote it.
    pub expires_at: Option<String>,
}

#[derive(Deserialize)]
struct HostAnswer {
    url: Option<String>,
    delete_url: Option<String>,
    expires_at: Option<serde_json::Value>,
}

/// A link a host hands back is something the user will open and send on: HTTPS (or plain HTTP
/// when `allow_http`, for a host on this computer), no credentials, no spaces, a sane length.
pub fn checked_link(link: &str, allow_http: bool) -> Option<String> {
    let link = link.trim();
    if link.is_empty() || link.chars().count() > MAX_LINK_CHARS {
        return None;
    }
    let parsed = webhook::policy::parse_url(link)?;
    let allowed = parsed.scheme == "https" || (allow_http && parsed.scheme == "http");
    (allowed && parsed.userinfo.is_none() && !parsed.host.is_empty()).then(|| link.to_owned())
}

/// Reads a host's answer to an upload: `{"url": "https://…"}`, with an optional `delete_url` and
/// `expires_at`. Anything else is no snapshot.
pub fn parse_upload_response(
    body: &[u8],
    host: &SnapshotHost,
) -> Result<SharedSnapshot, UploadError> {
    let answer: HostAnswer = serde_json::from_slice(body).map_err(|_| UploadError::BadResponse)?;
    let url = answer
        .url
        .as_deref()
        .and_then(|link| checked_link(link, host.loopback()))
        .ok_or(UploadError::BadResponse)?;
    Ok(SharedSnapshot {
        url,
        delete_url: answer
            .delete_url
            .as_deref()
            .and_then(|link| checked_link(link, host.loopback())),
        expires_at: answer.expires_at.and_then(|value| match value {
            serde_json::Value::String(text) => Some(text.chars().take(64).collect()),
            serde_json::Value::Number(number) => Some(number.to_string()),
            _ => None,
        }),
    })
}

/// Why an upload did not give a link.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UploadError {
    /// The image is over what a host may be sent.
    TooLarge,
    /// No connection to the host.
    Offline,
    Timeout,
    /// The host does not accept the token (or wants one).
    Unauthorized,
    /// The host asked for fewer uploads; `retry_after` is seconds if it said.
    RateLimited {
        retry_after: Option<u64>,
    },
    /// The host turned the image away as too big.
    RejectedAsTooLarge,
    Server(u16),
    /// The host answered with something that is no snapshot.
    BadResponse,
}

impl UploadError {
    /// What to tell the user. `label` is where the image was going ("ImgBB", "snap.example.com").
    pub fn message(&self, label: &str) -> String {
        match self {
            Self::TooLarge => "The image is too large to upload.".to_owned(),
            Self::Offline => format!("Could not reach {label}. Check your connection."),
            Self::Timeout => format!("{label} took too long to answer."),
            Self::Unauthorized => {
                format!("{label} refused the credentials. Check Settings ▸ Snapshots.")
            }
            Self::RateLimited {
                retry_after: Some(seconds),
            } => format!("Too many uploads. Try again in {seconds} s."),
            Self::RateLimited { retry_after: None } => {
                "Too many uploads. Try again in a moment.".to_owned()
            }
            Self::RejectedAsTooLarge => format!("{label} says the image is too large."),
            Self::Server(code) => format!("{label} failed (HTTP {code})."),
            Self::BadResponse => format!("{label} did not return a link."),
        }
    }

    /// Whether trying the same upload again can work.
    pub fn retryable(&self) -> bool {
        matches!(
            self,
            Self::Offline | Self::Timeout | Self::RateLimited { .. } | Self::Server(_)
        )
    }
}

// MARK: Settings

/// A provider's non-secret settings: its fields' keys and the values the user gave. Secrets are
/// never here; they live in the system's secure storage.
pub type FieldValues = BTreeMap<String, String>;

/// What the user chose about snapshots, saved as one setting.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(from = "StoredPrefs")]
pub struct SnapshotPrefs {
    /// The host hosted snapshots are uploaded to (a provider's id); none until the user turns one
    /// on, and only one at a time. An id this build does not know is kept, and inactive.
    pub provider: Option<String>,
    /// Each provider's settings, by provider id. Kept for a provider that is off, so turning it
    /// back on finds what was entered.
    pub providers: BTreeMap<String, FieldValues>,
    /// Orders, positions and their stops on the chart. Off: they show the account's holdings.
    pub include_trading: bool,
    pub scale: SnapshotScale,
    /// The look the Screenshot Studio opens in. Synced like `scale`.
    pub studio: StudioPrefs,
    /// What the user agreed to send images to: one entry per destination, as the provider names
    /// it (`host[:port]` for a host of their own).
    pub consented: Vec<String>,
}

/// The id of the provider that speaks DegenView's own protocol: what the one `host` of an older
/// save becomes.
pub const SELF_HOSTED: &str = "self_hosted";

/// How the setting is read: also the shape older versions wrote, where the one host was a
/// top-level `host` and sharing was on whenever it was set.
#[derive(Default, Deserialize)]
#[serde(default)]
struct StoredPrefs {
    host: Option<String>,
    /// `None` when the key is missing (an older save), `Some(None)` when it is `null` (every
    /// provider turned off on purpose).
    #[serde(deserialize_with = "present")]
    provider: Option<Option<String>>,
    providers: BTreeMap<String, FieldValues>,
    include_trading: bool,
    scale: SnapshotScale,
    /// Read part by part (see [`StudioPrefs`]); one that is not an object is the default look.
    studio: Option<serde_json::Value>,
    consented: Vec<String>,
}

fn present<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

impl From<StoredPrefs> for SnapshotPrefs {
    fn from(stored: StoredPrefs) -> Self {
        let mut prefs = Self {
            provider: stored.provider.clone().flatten(),
            providers: stored.providers,
            include_trading: stored.include_trading,
            scale: stored.scale,
            studio: stored
                .studio
                .and_then(|look| serde_json::from_value(look).ok())
                .unwrap_or_default(),
            consented: stored.consented,
        };
        // An older save: nobody who set a host loses it. Once the provider is written (even as
        // off) a stray `host`, such as one an older Mac syncs in, is ignored.
        // Only a host that is still a valid one: an address with a password in it, or plain
        // http to somewhere else, would not have been accepted by the page that saved it, and a
        // value that is not shared on must not be copied into the synced settings either.
        if stored.provider.is_none()
            && let Some(host) = stored
                .host
                .filter(|host| SnapshotHost::parse(host.trim()).is_ok())
        {
            prefs.provider = Some(SELF_HOSTED.to_owned());
            prefs
                .providers
                .entry(SELF_HOSTED.to_owned())
                .or_default()
                .entry("host".to_owned())
                .or_insert(host);
        }
        prefs
    }
}

impl SnapshotPrefs {
    pub const KEY: &'static str = "snapshot.prefs";

    pub fn options(&self) -> SnapshotOptions {
        SnapshotOptions {
            scale: self.scale,
            include_trading: self.include_trading,
        }
    }

    /// What a provider has been given (nothing, for one never set up).
    pub fn settings_of(&self, provider: &str) -> FieldValues {
        self.providers.get(provider).cloned().unwrap_or_default()
    }

    /// Turns `provider` on, which turns the one that was on off; `None` turns sharing off.
    pub fn activate(&mut self, provider: Option<&str>) {
        self.provider = provider.map(str::to_owned);
    }

    /// Sets one of a provider's fields; an empty value clears it.
    pub fn set_field(&mut self, provider: &str, key: &str, value: &str) {
        let value = value.trim();
        if value.is_empty() {
            if let Some(fields) = self.providers.get_mut(provider) {
                fields.remove(key);
                if fields.is_empty() {
                    self.providers.remove(provider);
                }
            }
        } else {
            self.providers
                .entry(provider.to_owned())
                .or_default()
                .insert(key.to_owned(), value.to_owned());
        }
    }

    pub fn has_consented(&self, destination: &str) -> bool {
        self.consented.iter().any(|entry| entry == destination)
    }

    pub fn consent(&mut self, destination: &str) {
        if !self.has_consented(destination) {
            self.consented.push(destination.to_owned());
        }
    }
}

#[cfg(test)]
mod composition_tests;
#[cfg(test)]
mod framing_tests;
#[cfg(test)]
mod selection_tests;
#[cfg(test)]
mod studio_tests;
#[cfg(test)]
mod tests;

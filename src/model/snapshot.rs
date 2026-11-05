//! Chart snapshots as pure values: what can be asked of one, how big the image may be, what the
//! file is called, where a shared one is hosted and what the host may answer. The rendering, the
//! file and clipboard work and the upload live elsewhere; this is the part every one of them
//! agrees on.

use crate::format;
use crate::model::webhook_policy::{self, ParsedUrl};
use serde::{Deserialize, Serialize};

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

/// `DegenView_BTCUSDT_1H_2026-10-19_1642.png`: the chart's symbol and timeframe and the local
/// time (`offset_secs` east of UTC). A `1M` timeframe stays upper case so it is never read as
/// minutes.
pub fn file_name(symbol: &str, timeframe: &str, epoch_ms: i64, offset_secs: i64) -> String {
    let local = epoch_ms.div_euclid(1_000) + offset_secs;
    let (year, month, day) = format::civil_from_days(local.div_euclid(86_400));
    let seconds = local.rem_euclid(86_400);
    format!(
        "DegenView_{}_{}_{year:04}-{month:02}-{day:02}_{:02}{:02}.png",
        name_part(symbol, "Chart"),
        name_part(timeframe, "Chart"),
        seconds / 3_600,
        seconds % 3_600 / 60,
    )
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
        webhook_policy::url_encode(text),
        webhook_policy::url_encode(link),
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
        let parsed = webhook_policy::parse_url(text).ok_or(HostError::NotAUrl)?;
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

    fn loopback(&self) -> bool {
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

/// A link a host hands back is something the user will open and send on: HTTPS (or this
/// computer, for a host that is), no credentials, no spaces, a sane length.
fn checked_link(link: &str, host: &SnapshotHost) -> Option<String> {
    let link = link.trim();
    if link.is_empty() || link.chars().count() > MAX_LINK_CHARS {
        return None;
    }
    let parsed = webhook_policy::parse_url(link)?;
    let allowed = parsed.scheme == "https" || (host.loopback() && parsed.scheme == "http");
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
        .and_then(|link| checked_link(link, host))
        .ok_or(UploadError::BadResponse)?;
    Ok(SharedSnapshot {
        url,
        delete_url: answer
            .delete_url
            .as_deref()
            .and_then(|link| checked_link(link, host)),
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
    pub fn message(&self) -> String {
        match self {
            Self::TooLarge => "The image is too large to upload.".to_owned(),
            Self::Offline => "Could not reach the snapshot host. Check your connection.".to_owned(),
            Self::Timeout => "The snapshot host took too long to answer.".to_owned(),
            Self::Unauthorized => {
                "The snapshot host refused the token. Check Settings ▸ Snapshots.".to_owned()
            }
            Self::RateLimited {
                retry_after: Some(seconds),
            } => format!("Too many uploads. Try again in {seconds} s."),
            Self::RateLimited { retry_after: None } => {
                "Too many uploads. Try again in a moment.".to_owned()
            }
            Self::RejectedAsTooLarge => "The snapshot host says the image is too large.".to_owned(),
            Self::Server(code) => format!("The snapshot host failed (HTTP {code})."),
            Self::BadResponse => "The snapshot host did not return a link.".to_owned(),
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

/// What the user chose about snapshots, saved as one setting.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct SnapshotPrefs {
    /// Where hosted snapshots are uploaded; none until the user sets one.
    pub host: Option<String>,
    /// Orders, positions and their stops on the chart. Off: they show the account's holdings.
    pub include_trading: bool,
    pub scale: SnapshotScale,
    /// The hosts (`host[:port]`) the user agreed to send images to.
    pub consented: Vec<String>,
}

impl SnapshotPrefs {
    pub const KEY: &'static str = "snapshot.prefs";

    /// The configured host, if it is a valid one.
    pub fn host(&self) -> Option<SnapshotHost> {
        SnapshotHost::parse(self.host.as_deref()?).ok()
    }

    pub fn options(&self) -> SnapshotOptions {
        SnapshotOptions {
            scale: self.scale,
            include_trading: self.include_trading,
        }
    }

    pub fn has_consented(&self, host: &SnapshotHost) -> bool {
        self.consented.contains(&host.authority())
    }

    pub fn consent(&mut self, host: &SnapshotHost) {
        if !self.has_consented(host) {
            self.consented.push(host.authority());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOON: i64 = 1_792_413_720_000; // 2026-10-19 12:42:00 UTC

    #[test]
    fn the_menu_has_five_rows_and_three_need_a_host() {
        assert_eq!(SnapshotAction::ALL.len(), 5);
        let hosted: Vec<_> = SnapshotAction::ALL
            .into_iter()
            .filter(|action| action.is_hosted())
            .map(SnapshotAction::label)
            .collect();
        assert_eq!(hosted, ["Copy link", "Open in new tab", "Tweet image"]);
    }

    #[test]
    fn a_snapshot_is_drawn_again_at_the_scale_not_stretched() {
        let size = SnapshotDimensions::fit(760.0, 520.0, SnapshotScale::X2).expect("fits");
        assert_eq!((size.width, size.height), (1520, 1040));
        assert_eq!(size.pixels_per_point, 2.0);
        let size = SnapshotDimensions::fit(760.0, 520.0, SnapshotScale::X1).expect("fits");
        assert_eq!((size.width, size.height), (760, 520));
        // The aspect ratio holds when the size is not a whole number of points.
        let size = SnapshotDimensions::fit(333.4, 200.2, SnapshotScale::X2).expect("fits");
        assert_eq!((size.width, size.height), (667, 400));
    }

    #[test]
    fn a_huge_chart_is_drawn_at_a_smaller_scale_within_the_limits() {
        let size = SnapshotDimensions::fit(6000.0, 4000.0, SnapshotScale::X2).expect("fits");
        assert!(size.pixels_per_point < 2.0);
        assert!(size.width <= MAX_SIDE_PX && size.height <= MAX_SIDE_PX);
        assert!(u64::from(size.width) * u64::from(size.height) <= MAX_PIXELS);
        // A side past the limit at scale 1 is drawn smaller rather than refused.
        let wide = SnapshotDimensions::fit(10_000.0, 100.0, SnapshotScale::X1).expect("fits");
        assert!(
            (8_000..=MAX_SIDE_PX).contains(&wide.width),
            "{}",
            wide.width
        );
        assert!(wide.pixels_per_point < 1.0);
        // Too large for even the smallest scale.
        assert_eq!(
            SnapshotDimensions::fit(40_000.0, 40_000.0, SnapshotScale::X1),
            None
        );
    }

    #[test]
    fn a_chart_without_a_size_has_no_snapshot() {
        for (w, h) in [
            (0.0, 100.0),
            (100.0, 0.5),
            (f32::NAN, 100.0),
            (100.0, f32::INFINITY),
        ] {
            assert_eq!(
                SnapshotDimensions::fit(w, h, SnapshotScale::X2),
                None,
                "{w}x{h}"
            );
        }
    }

    #[test]
    fn the_file_is_named_for_the_chart_and_the_local_time() {
        assert_eq!(
            file_name("BTCUSD", "1H", NOON, 0),
            "DegenView_BTCUSD_1H_2026-10-19_1242.png"
        );
        // Two hours east of UTC: the local clock and, across midnight, the local date.
        assert_eq!(
            file_name("BTCUSD", "1H", NOON, 2 * 3_600),
            "DegenView_BTCUSD_1H_2026-10-19_1442.png"
        );
        assert_eq!(
            file_name("BTCUSD", "1H", NOON + 12 * 3_600_000, 0),
            "DegenView_BTCUSD_1H_2026-10-20_0042.png"
        );
        assert_eq!(
            file_name("BTCUSD", "1H", NOON, -13 * 3_600),
            "DegenView_BTCUSD_1H_2026-10-18_2342.png"
        );
    }

    #[test]
    fn a_name_keeps_to_what_every_file_system_takes() {
        assert_eq!(
            file_name("BTC/USDT", "1M", NOON, 0),
            "DegenView_BTCUSDT_1M_2026-10-19_1242.png"
        );
        assert_eq!(
            file_name("a:b*c?d\"e<f>g|h\\i j", "1D", NOON, 0),
            "DegenView_abcdefghij_1D_2026-10-19_1242.png"
        );
        assert_eq!(
            file_name("../../etc/passwd\0", "1D", NOON, 0),
            "DegenView_etcpasswd_1D_2026-10-19_1242.png"
        );
        assert_eq!(
            file_name("日本語", "", NOON, 0),
            "DegenView_Chart_Chart_2026-10-19_1242.png"
        );
        let long = "X".repeat(200);
        let name = file_name(&long, "1D", NOON, 0);
        assert_eq!(name.matches('X').count(), MAX_NAME_PART);
    }

    #[test]
    fn a_name_gets_its_extension_and_a_free_number() {
        assert_eq!(with_png_extension("chart"), "chart.png");
        assert_eq!(with_png_extension("chart.PNG"), "chart.PNG");
        assert_eq!(unused_name("a.png", |_| false), "a.png");
        assert_eq!(unused_name("a.png", |n| n == "a.png"), "a (2).png");
        assert_eq!(
            unused_name("a.png", |n| n == "a.png" || n == "a (2).png"),
            "a (3).png"
        );
    }

    #[test]
    fn the_post_names_the_chart_and_carries_the_link() {
        assert_eq!(share_text("BTC/USD", "1H"), "BTC/USD 1H chart — DegenView");
        assert_eq!(
            x_compose_url(
                "BTC/USD 1H chart — DegenView",
                "https://s.example/a?b=1&c=2"
            ),
            "https://x.com/intent/post?text=BTC%2FUSD%201H%20chart%20%E2%80%94%20DegenView\
             &url=https%3A%2F%2Fs.example%2Fa%3Fb%3D1%26c%3D2"
        );
    }

    #[test]
    fn a_host_is_https_or_this_computer() {
        let host = SnapshotHost::parse(" https://snap.example.com/v1/snapshots ").expect("valid");
        assert_eq!(host.endpoint(), "https://snap.example.com/v1/snapshots");
        assert_eq!(host.authority(), "snap.example.com");
        assert_eq!(
            SnapshotHost::parse("https://snap.example.com:8443/up")
                .expect("valid")
                .authority(),
            "snap.example.com:8443"
        );
        assert!(SnapshotHost::parse("http://127.0.0.1:9000/up").is_ok());
        assert!(SnapshotHost::parse("http://localhost/up").is_ok());
        for (text, error) in [
            ("", HostError::Empty),
            ("   ", HostError::Empty),
            ("snap.example.com", HostError::NotAUrl),
            ("https://", HostError::NotAUrl),
            ("http://snap.example.com/up", HostError::NotHttps),
            ("ftp://snap.example.com/up", HostError::NotHttps),
            (
                "https://me:secret@snap.example.com/up",
                HostError::Credentials,
            ),
            ("https://snap.example.com/up#top", HostError::Fragment),
            ("https://snap.example.com/u p", HostError::NotAUrl),
        ] {
            assert_eq!(SnapshotHost::parse(text), Err(error), "{text:?}");
        }
    }

    fn host() -> SnapshotHost {
        SnapshotHost::parse("https://snap.example.com/up").expect("valid")
    }

    #[test]
    fn a_host_answers_with_a_link() {
        let shared = parse_upload_response(
            br#"{"url":"https://cdn.example.com/s/AbC123.png","delete_url":"https://snap.example.com/d/xyz","expires_at":"2027-01-01T00:00:00Z"}"#,
            &host(),
        )
        .expect("a snapshot");
        assert_eq!(shared.url, "https://cdn.example.com/s/AbC123.png");
        assert_eq!(
            shared.delete_url.as_deref(),
            Some("https://snap.example.com/d/xyz")
        );
        assert_eq!(shared.expires_at.as_deref(), Some("2027-01-01T00:00:00Z"));
        let plain = parse_upload_response(br#"{"url":"https://a.example/x"}"#, &host())
            .expect("only the link is needed");
        assert_eq!((plain.delete_url, plain.expires_at), (None, None));
    }

    #[test]
    fn anything_but_a_safe_link_is_no_snapshot() {
        for body in [
            &b""[..],
            b"not json",
            b"[]",
            b"{}",
            br#"{"url":null}"#,
            br#"{"url":""}"#,
            br#"{"url":"http://a.example/x"}"#,
            br#"{"url":"file:///etc/passwd"}"#,
            br#"{"url":"javascript:alert(1)"}"#,
            br#"{"url":"https://user:pw@a.example/x"}"#,
            br#"{"url":"https://a.example/with space"}"#,
            br#"{"url":42}"#,
        ] {
            assert_eq!(
                parse_upload_response(body, &host()),
                Err(UploadError::BadResponse),
                "{}",
                String::from_utf8_lossy(body)
            );
        }
        let long = format!(
            r#"{{"url":"https://a.example/{}"}}"#,
            "x".repeat(MAX_LINK_CHARS)
        );
        assert_eq!(
            parse_upload_response(long.as_bytes(), &host()),
            Err(UploadError::BadResponse)
        );
        // A bad delete link is dropped, not trusted and not fatal.
        let shared = parse_upload_response(
            br#"{"url":"https://a.example/x","delete_url":"javascript:1"}"#,
            &host(),
        )
        .expect("the link is fine");
        assert_eq!(shared.delete_url, None);
    }

    #[test]
    fn a_host_on_this_computer_may_answer_with_plain_http() {
        let local = SnapshotHost::parse("http://127.0.0.1:9000/up").expect("valid");
        assert!(parse_upload_response(br#"{"url":"http://127.0.0.1:9000/s/1"}"#, &local).is_ok());
        assert!(parse_upload_response(br#"{"url":"http://127.0.0.1:9000/s/1"}"#, &host()).is_err());
    }

    #[test]
    fn only_a_failure_that_can_pass_is_worth_a_retry() {
        for error in [
            UploadError::Offline,
            UploadError::Timeout,
            UploadError::RateLimited {
                retry_after: Some(3),
            },
            UploadError::Server(503),
        ] {
            assert!(error.retryable(), "{error:?}");
        }
        for error in [
            UploadError::TooLarge,
            UploadError::Unauthorized,
            UploadError::RejectedAsTooLarge,
            UploadError::BadResponse,
        ] {
            assert!(!error.retryable(), "{error:?}");
        }
        assert_eq!(
            UploadError::RateLimited {
                retry_after: Some(7)
            }
            .message(),
            "Too many uploads. Try again in 7 s."
        );
    }

    #[test]
    fn the_defaults_share_nothing_and_include_no_trading() {
        let prefs = SnapshotPrefs::default();
        assert_eq!(prefs.host(), None);
        assert!(!prefs.include_trading);
        assert_eq!(prefs.scale, SnapshotScale::X2);
        // An older or partial save still reads.
        let read: SnapshotPrefs =
            serde_json::from_str(r#"{"host":"https://s.example/up"}"#).unwrap();
        assert!(read.host().is_some());
        assert!(!read.include_trading);
    }

    #[test]
    fn consent_is_per_host() {
        let mut prefs = SnapshotPrefs::default();
        let a = SnapshotHost::parse("https://a.example/up").unwrap();
        let b = SnapshotHost::parse("https://b.example/up").unwrap();
        assert!(!prefs.has_consented(&a));
        prefs.consent(&a);
        prefs.consent(&a);
        assert!(prefs.has_consented(&a) && !prefs.has_consented(&b));
        assert_eq!(prefs.consented.len(), 1);
        // The same host on another path is the same consent; another port is not.
        let moved = SnapshotHost::parse("https://a.example/other").unwrap();
        let other_port = SnapshotHost::parse("https://a.example:8443/up").unwrap();
        assert!(prefs.has_consented(&moved) && !prefs.has_consented(&other_port));
    }
}

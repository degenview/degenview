//! The link the Post tool holds: an X post. Pure parsing and normalisation, no network and no
//! egui.
//!
//! A card keeps only what the link itself proves (an X handle and post id) plus, once the user asks
//! for it, the few fields a fetch returned. Nothing is guessed: a post's time is decoded from its
//! snowflake id.
//!
//! The parser is deliberately strict. It accepts `https` (and upgrades `http`), no user info, no
//! port, and only X's own hosts, so a pasted link can never point the app, or the preview fetch,
//! anywhere else.

use serde::{Deserialize, Serialize};

/// The longest link accepted, in bytes.
pub const MAX_URL_BYTES: usize = 2_048;
/// Limits on what a fetch may store on a card, in characters.
pub const MAX_TITLE_CHARS: usize = 300;
pub const MAX_AUTHOR_CHARS: usize = 100;
pub const MAX_EXCERPT_CHARS: usize = 800;

/// X's own epoch, the base of its snowflake ids: 2010-11-04T01:42:54.657Z.
const SNOWFLAKE_EPOCH_MS: i64 = 1_288_834_974_657;
/// Posts up to this id were numbered in sequence, before snowflake ids: they carry no time.
const LAST_SEQUENTIAL_POST: u64 = 29_700_859_247;

/// The service the cards are for.
pub const SERVICE: &str = "X";
/// The only host a preview fetch may contact.
pub const FETCH_HOST: &str = "publish.x.com";

/// Why a link was refused, in words for the person who pasted it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LinkError {
    Empty,
    TooLong,
    /// Whitespace, control characters or anything that is not plain ASCII.
    Malformed,
    /// Not `https` (or `http`).
    Scheme,
    /// A user name, password or port in the address.
    Authority,
    /// Not an X host.
    Host,
    /// An X host, but not the address of one post.
    Path,
}

impl LinkError {
    pub const fn message(self) -> &'static str {
        match self {
            Self::Empty => "Paste a link.",
            Self::TooLong => "That link is too long.",
            Self::Malformed => "That is not a valid link.",
            Self::Scheme => "Only https links are supported.",
            Self::Authority => "Links with a user name or port are not supported.",
            Self::Host => "That link is not from X.",
            Self::Path => "That link does not point to a single post.",
        }
    }
}

/// A link understood: the post it points at, without contacting anything. The handle is absent
/// for the `x.com/i/status/…` form.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Link {
    pub id: u64,
    pub handle: Option<String>,
}

impl Link {
    /// The link written one way, whatever way it was pasted.
    pub fn url(&self) -> String {
        match &self.handle {
            Some(handle) => format!("https://x.com/{handle}/status/{}", self.id),
            None => format!("https://x.com/i/status/{}", self.id),
        }
    }

    /// The post's time, from its snowflake id. `None` for a post numbered before snowflake ids
    /// existed.
    pub fn time_ms(&self) -> Option<i64> {
        post_time_ms(self.id)
    }

    /// What the link says about itself, for a card that has fetched nothing.
    pub fn headline(&self) -> String {
        match &self.handle {
            Some(handle) => format!("@{handle}"),
            None => "Post".to_owned(),
        }
    }
}

/// The time an X post was made, from its id; `None` for ids numbered before snowflakes.
pub fn post_time_ms(id: u64) -> Option<i64> {
    if id <= LAST_SEQUENTIAL_POST {
        return None;
    }
    i64::try_from(id >> 22)
        .ok()?
        .checked_add(SNOWFLAKE_EPOCH_MS)
}

/// What a preview fetch returned, kept on the card. Every field is as long as its cap allows and
/// no longer, and empty means the source gave nothing: nothing is invented to fill a card.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct LinkMeta {
    pub title: String,
    pub author: String,
    pub excerpt: String,
    pub fetched_at_ms: i64,
}

impl LinkMeta {
    pub fn is_empty(&self) -> bool {
        self.title.is_empty() && self.author.is_empty() && self.excerpt.is_empty()
    }

    /// The meta with every field cut to its cap and tidied of line breaks.
    pub fn sanitized(self) -> Self {
        let tidy = |text: &str, cap: usize| -> String {
            text.split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(cap)
                .collect()
        };
        Self {
            title: tidy(&self.title, MAX_TITLE_CHARS),
            author: tidy(&self.author, MAX_AUTHOR_CHARS),
            excerpt: tidy(&self.excerpt, MAX_EXCERPT_CHARS),
            fetched_at_ms: self.fetched_at_ms,
        }
    }
}

/// What the user has agreed to, kept on this Mac only (it is not a synced setting): fetching a
/// preview sends the link to X's own host, so it is asked once.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct ContentPrefs {
    /// The user allowed previews to be fetched from X.
    pub previews: bool,
}

impl ContentPrefs {
    pub const KEY: &'static str = "content.prefs";
}

/// A Post drawing's content: the link, what the link proves, and what a fetch added.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct LinkCard {
    pub link: Link,
    /// The card's size on screen, in logical pixels.
    pub size: [f32; 2],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<LinkMeta>,
}

/// The size a new card starts at, and the range the user may resize it to.
pub const DEFAULT_CARD_SIZE: [f32; 2] = [300.0, 132.0];
pub const CARD_SIZE_RANGE: ([f32; 2], [f32; 2]) = ([180.0, 96.0], [640.0, 480.0]);

impl LinkCard {
    pub fn new(link: Link) -> Self {
        Self {
            link,
            size: DEFAULT_CARD_SIZE,
            meta: None,
        }
    }

    /// The card with its size brought into range and its meta cut to the caps.
    pub fn sanitized(mut self) -> Self {
        let (low, high) = CARD_SIZE_RANGE;
        for axis in 0..2 {
            let value = self.size[axis];
            self.size[axis] = if value.is_finite() {
                value.clamp(low[axis], high[axis])
            } else {
                DEFAULT_CARD_SIZE[axis]
            };
        }
        self.meta = self
            .meta
            .map(LinkMeta::sanitized)
            .filter(|meta| !meta.is_empty());
        self
    }

    /// The card's size after its bottom-right corner is dragged by (`dx`, `dy`), held to the
    /// range a card may have.
    pub fn resize_by(&mut self, dx: f32, dy: f32) {
        self.size = [self.size[0] + dx, self.size[1] + dy];
        let (low, high) = CARD_SIZE_RANGE;
        for axis in 0..2 {
            self.size[axis] = if self.size[axis].is_finite() {
                self.size[axis].clamp(low[axis], high[axis])
            } else {
                DEFAULT_CARD_SIZE[axis]
            };
        }
    }

    /// What to call the card in a list: what a fetch returned if the user asked for it, else
    /// what the link itself says.
    pub fn summary(&self) -> String {
        if let Some(meta) = &self.meta {
            let headline = if meta.title.is_empty() {
                &meta.author
            } else {
                &meta.title
            };
            if !headline.is_empty() {
                return headline.clone();
            }
        }
        self.link.headline()
    }

    /// Whether the card is one a tool could have made.
    pub fn is_valid(&self) -> bool {
        self.size.iter().all(|side| side.is_finite()) && Link::parse(&self.link.url()).is_ok()
    }
}

/// The scheme-stripped address split into its host and path, or why not.
fn split(input: &str) -> Result<(String, Vec<&str>), LinkError> {
    let input = input.trim();
    if input.is_empty() {
        return Err(LinkError::Empty);
    }
    if input.len() > MAX_URL_BYTES {
        return Err(LinkError::TooLong);
    }
    if !input.is_ascii() || input.chars().any(|c| c.is_ascii_control() || c == ' ') {
        return Err(LinkError::Malformed);
    }
    let lower = input.to_ascii_lowercase();
    let rest = if lower.starts_with("https://") {
        &input["https://".len()..]
    } else if lower.starts_with("http://") {
        &input["http://".len()..]
    } else {
        return Err(LinkError::Scheme);
    };
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    if authority.is_empty() {
        return Err(LinkError::Malformed);
    }
    if authority.contains(['@', ':', '\\', '%']) {
        return Err(LinkError::Authority);
    }
    let path = tail.split(['?', '#']).next().unwrap_or("");
    let segments = path.split('/').filter(|part| !part.is_empty()).collect();
    Ok((
        authority
            .to_ascii_lowercase()
            .trim_end_matches('.')
            .to_owned(),
        segments,
    ))
}

fn is_handle(text: &str) -> bool {
    (1..=15).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

fn is_x_host(host: &str) -> bool {
    matches!(
        host,
        "x.com"
            | "www.x.com"
            | "mobile.x.com"
            | "twitter.com"
            | "www.twitter.com"
            | "mobile.twitter.com"
    )
}

impl Link {
    /// An X post's address.
    pub fn parse(input: &str) -> Result<Self, LinkError> {
        let (host, segments) = split(input)?;
        if !is_x_host(&host) {
            return Err(LinkError::Host);
        }
        match segments[..] {
            // `i` is no handle: these are the forms with none.
            ["i", "web", "status", id, ..] | ["i", "status", id, ..] => Ok(Self {
                id: parse_post_id(id)?,
                handle: None,
            }),
            [handle, "status" | "statuses", id, ..] if is_handle(handle) => Ok(Self {
                id: parse_post_id(id)?,
                handle: Some(handle.to_owned()),
            }),
            _ => Err(LinkError::Path),
        }
    }
}

fn parse_post_id(text: &str) -> Result<u64, LinkError> {
    if text.is_empty() || text.len() > 20 || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(LinkError::Path);
    }
    text.parse().map_err(|_| LinkError::Path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn post(text: &str) -> Link {
        Link::parse(text).unwrap_or_else(|error| panic!("{text}: {error:?}"))
    }

    #[test]
    fn every_way_of_writing_a_post_link_normalises_to_one() {
        let canonical = "https://x.com/jack/status/20";
        for text in [
            "https://x.com/jack/status/20",
            "https://twitter.com/jack/status/20",
            "http://twitter.com/jack/status/20",
            "https://www.twitter.com/jack/status/20/",
            "https://mobile.twitter.com/jack/status/20?s=20&t=abc",
            "  HTTPS://X.COM/jack/status/20#fragment  ",
            "https://x.com/jack/status/20/photo/1",
            "https://x.com/jack/statuses/20",
        ] {
            assert_eq!(post(text).url(), canonical, "{text}");
        }
        assert_eq!(
            post("https://x.com/i/web/status/1519480761749016577").url(),
            "https://x.com/i/status/1519480761749016577"
        );
        assert_eq!(
            post("https://x.com/i/status/5").url(),
            "https://x.com/i/status/5"
        );
        // The handle keeps its case; the host does not matter.
        assert_eq!(
            post("https://X.com/ElonMusk/status/1519480761749016577").headline(),
            "@ElonMusk"
        );
    }

    #[test]
    fn a_post_link_must_name_a_post_on_a_known_host_and_nothing_else() {
        let refused = [
            ("", LinkError::Empty),
            ("   ", LinkError::Empty),
            ("x.com/jack/status/20", LinkError::Scheme),
            ("ftp://x.com/jack/status/20", LinkError::Scheme),
            ("javascript:alert(1)", LinkError::Scheme),
            ("file:///etc/passwd", LinkError::Scheme),
            ("https://x.com.evil.com/jack/status/20", LinkError::Host),
            ("https://evilx.com/jack/status/20", LinkError::Host),
            (
                "https://x.com@evil.com/jack/status/20",
                LinkError::Authority,
            ),
            (
                "https://user:pass@x.com/jack/status/20",
                LinkError::Authority,
            ),
            ("https://x.com:8443/jack/status/20", LinkError::Authority),
            (
                "https://x.com%2eevil.com/jack/status/20",
                LinkError::Authority,
            ),
            (
                "https://x.com\\@evil.com/jack/status/20",
                LinkError::Authority,
            ),
            ("https://x.com/jack", LinkError::Path),
            ("https://x.com/jack/status", LinkError::Path),
            ("https://x.com/jack/status/abc", LinkError::Path),
            ("https://x.com/jack/status/", LinkError::Path),
            (
                "https://x.com/jack/status/123456789012345678901",
                LinkError::Path,
            ),
            (
                "https://x.com/jack/status/99999999999999999999",
                LinkError::Path,
            ),
            (
                "https://x.com/this_handle_is_too_long/status/20",
                LinkError::Path,
            ),
            ("https://x.com/ja-ck/status/20", LinkError::Path),
            ("https://x.com/home", LinkError::Path),
            ("https://x.com/i/web/status", LinkError::Path),
        ];
        for (text, error) in &refused {
            assert_eq!(Link::parse(text), Err(*error), "{text:?}");
        }
        assert!(Link::parse("https://x.com/jack/status/20 \n").is_ok());
        assert_eq!(
            Link::parse("https://x.com/jack/status/2 0"),
            Err(LinkError::Malformed)
        );
        assert_eq!(
            Link::parse("https://x.com/jäck/status/20"),
            Err(LinkError::Malformed)
        );
        assert_eq!(
            Link::parse(&format!("https://x.com/jack/status/{}", "9".repeat(3_000))),
            Err(LinkError::TooLong)
        );
    }

    #[test]
    fn a_post_carries_its_time_only_when_its_id_is_a_snowflake() {
        // "Next I'm buying Coca-Cola…", which X dates April 28, 2022.
        let id = 1_519_480_761_749_016_577;
        assert_eq!(post_time_ms(id), Some(1_651_107_418_845));
        // Posts numbered in sequence carry no time: "just setting up my twttr" is #20.
        assert_eq!(post_time_ms(20), None);
        assert_eq!(post_time_ms(LAST_SEQUENTIAL_POST), None);
        assert!(post_time_ms(LAST_SEQUENTIAL_POST + 1).is_some());
        assert_eq!(post("https://x.com/jack/status/20").time_ms(), None);
        assert_eq!(
            post("https://x.com/elonmusk/status/1519480761749016577").time_ms(),
            Some(1_651_107_418_845)
        );
        // The largest id must not panic.
        let _ = post_time_ms(u64::MAX);
    }

    #[test]
    fn a_stranger_is_refused_and_every_error_has_words() {
        assert_eq!(Link::parse("https://example.com/"), Err(LinkError::Host));
        assert_eq!(
            Link::parse("https://www.tradingview.com/i/Gj4niqTa/"),
            Err(LinkError::Host)
        );
        // A wrong path on X stays an X error, not "unsupported site".
        assert_eq!(Link::parse("https://x.com/home"), Err(LinkError::Path));
        for error in [
            LinkError::Empty,
            LinkError::TooLong,
            LinkError::Malformed,
            LinkError::Scheme,
            LinkError::Authority,
            LinkError::Host,
            LinkError::Path,
        ] {
            assert!(!error.message().is_empty());
        }
    }

    #[test]
    fn a_canonical_url_parses_back_to_the_same_link() {
        for link in [
            post("https://x.com/jack/status/20"),
            post("https://x.com/i/status/1519480761749016577"),
        ] {
            assert_eq!(Link::parse(&link.url()), Ok(link));
        }
    }

    #[test]
    fn meta_is_cut_to_its_caps_and_an_empty_one_is_dropped() {
        let long = "word ".repeat(400);
        let meta = LinkMeta {
            title: format!("  {long}\n"),
            author: "A\nB".into(),
            excerpt: long.clone(),
            fetched_at_ms: 7,
        }
        .sanitized();
        assert_eq!(meta.title.chars().count(), MAX_TITLE_CHARS);
        assert_eq!(meta.author, "A B");
        assert_eq!(meta.excerpt.chars().count(), MAX_EXCERPT_CHARS);
        assert_eq!(meta.fetched_at_ms, 7);
        let mut card = LinkCard::new(post("https://x.com/jack/status/20"));
        card.meta = Some(LinkMeta::default());
        card.size = [f32::NAN, 1.0e9];
        let card = card.sanitized();
        assert_eq!(card.meta, None);
        assert_eq!(card.size[0], DEFAULT_CARD_SIZE[0]);
        assert_eq!(card.size[1], CARD_SIZE_RANGE.1[1]);
        assert!(card.is_valid());
    }

    #[test]
    fn a_card_round_trips_and_omits_what_it_never_fetched() {
        let card = LinkCard::new(post("https://x.com/jack/status/20"));
        let json = serde_json::to_string(&card).unwrap();
        assert!(!json.contains("meta"), "{json}");
        assert_eq!(serde_json::from_str::<LinkCard>(&json).unwrap(), card);
        let mut fetched = card.clone();
        fetched.meta = Some(LinkMeta {
            title: "t".into(),
            fetched_at_ms: 9,
            ..LinkMeta::default()
        });
        let json = serde_json::to_string(&fetched).unwrap();
        assert_eq!(serde_json::from_str::<LinkCard>(&json).unwrap(), fetched);
    }
}

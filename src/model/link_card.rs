//! The links the Post and Idea tools hold: an X post, or a published TradingView idea. Pure
//! parsing and normalisation, no network and no egui.
//!
//! A card keeps only what the link itself proves (an X handle and post id, an idea's id, symbol
//! and slug) plus, once the user asks for it, the few fields a fetch returned. Nothing is guessed:
//! a post's time is decoded from its snowflake id, an idea's has no offline source.
//!
//! The parser is deliberately strict. It accepts `https` (and upgrades `http`), no user info, no
//! port, and only the hosts the tools are for, so a pasted link can never point the app, or the
//! preview fetch, anywhere else.

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

/// Which service a card refers to.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkKind {
    Post,
    Idea,
}

impl LinkKind {
    pub const fn service(self) -> &'static str {
        match self {
            Self::Post => "X",
            Self::Idea => "TradingView",
        }
    }

    /// The only hosts a fetch for this kind may contact.
    pub const fn fetch_host(self) -> &'static str {
        match self {
            Self::Post => "publish.x.com",
            Self::Idea => "www.tradingview.com",
        }
    }
}

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
    /// Not a host this tool is for.
    Host,
    /// A supported host, but not the address of one post or idea.
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
            Self::Host => "That link is not from a supported site.",
            Self::Path => "That link does not point to a single post or idea.",
        }
    }
}

/// A link understood: what it points at, without contacting anything.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Link {
    /// An X post. The handle is absent for the `x.com/i/status/…` form.
    Post { id: u64, handle: Option<String> },
    /// A TradingView idea. `symbol` and `slug` are absent for the short forms.
    Idea {
        id: String,
        symbol: Option<String>,
        slug: Option<String>,
    },
}

impl Link {
    pub const fn kind(&self) -> LinkKind {
        match self {
            Self::Post { .. } => LinkKind::Post,
            Self::Idea { .. } => LinkKind::Idea,
        }
    }

    /// The link written one way, whatever way it was pasted.
    pub fn url(&self) -> String {
        match self {
            Self::Post {
                id,
                handle: Some(handle),
            } => format!("https://x.com/{handle}/status/{id}"),
            Self::Post { id, handle: None } => format!("https://x.com/i/status/{id}"),
            Self::Idea {
                id,
                symbol: Some(symbol),
                slug: Some(slug),
            } => format!("https://www.tradingview.com/chart/{symbol}/{id}-{slug}/"),
            Self::Idea {
                id,
                symbol: Some(symbol),
                slug: None,
            } => format!("https://www.tradingview.com/chart/{symbol}/{id}/"),
            Self::Idea { id, .. } => format!("https://www.tradingview.com/chart/{id}/"),
        }
    }

    /// The post's time, from its snowflake id. `None` for an idea and for a post numbered
    /// before snowflake ids existed.
    pub fn time_ms(&self) -> Option<i64> {
        match self {
            Self::Post { id, .. } => post_time_ms(*id),
            Self::Idea { .. } => None,
        }
    }

    /// What the link says about itself, for a card that has fetched nothing: `@handle` for a
    /// post, the symbol for an idea.
    pub fn headline(&self) -> String {
        match self {
            Self::Post {
                handle: Some(handle),
                ..
            } => format!("@{handle}"),
            Self::Post { handle: None, .. } => "Post".to_owned(),
            Self::Idea {
                symbol: Some(symbol),
                ..
            } => symbol.clone(),
            Self::Idea { .. } => "Idea".to_owned(),
        }
    }

    /// The words of an idea's slug (`Add-Tweets-To-Your-Chart`), which the link itself carries.
    pub fn slug_words(&self) -> Option<String> {
        match self {
            Self::Idea {
                slug: Some(slug), ..
            } => Some(slug.replace(['-', '_'], " ")),
            _ => None,
        }
    }

    /// A short stable form of the id for a card's footer.
    pub fn short_id(&self) -> String {
        match self {
            Self::Post { id, .. } => id.to_string(),
            Self::Idea { id, .. } => id.clone(),
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
/// preview sends the link to the service's own host, so each service asks once.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct ContentPrefs {
    /// The services the user allowed previews from.
    pub previews: Vec<LinkKind>,
}

impl ContentPrefs {
    pub const KEY: &'static str = "content.prefs";

    pub fn allows(&self, kind: LinkKind) -> bool {
        self.previews.contains(&kind)
    }

    pub fn allow(&mut self, kind: LinkKind) {
        if !self.allows(kind) {
            self.previews.push(kind);
        }
    }
}

/// A Post or Idea drawing's content: the link, what the link proves, and what a fetch added.
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
        match self.link.slug_words() {
            Some(words) => format!("{} · {words}", self.link.headline()),
            None => self.link.headline(),
        }
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

fn is_idea_id(text: &str) -> bool {
    text.len() == 8 && text.bytes().all(|b| b.is_ascii_alphanumeric())
}

fn is_symbol(text: &str) -> bool {
    (1..=32).contains(&text.len())
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'!' | b'-'))
}

fn is_slug(text: &str) -> bool {
    (1..=200).contains(&text.len())
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
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

/// A TradingView host: the bare domain, `www` or one short language label (`kr`, `de`).
fn is_tradingview_host(host: &str) -> bool {
    if host == "tradingview.com" {
        return true;
    }
    host.strip_suffix(".tradingview.com").is_some_and(|prefix| {
        (1..=8).contains(&prefix.len()) && prefix.bytes().all(|b| b.is_ascii_lowercase())
    })
}

impl Link {
    /// An X post's address.
    pub fn parse_post(input: &str) -> Result<Self, LinkError> {
        let (host, segments) = split(input)?;
        if !is_x_host(&host) {
            return Err(LinkError::Host);
        }
        match segments[..] {
            // `i` is no handle: these are the forms with none.
            ["i", "web", "status", id, ..] | ["i", "status", id, ..] => Ok(Self::Post {
                id: parse_post_id(id)?,
                handle: None,
            }),
            [handle, "status" | "statuses", id, ..] if is_handle(handle) => Ok(Self::Post {
                id: parse_post_id(id)?,
                handle: Some(handle.to_owned()),
            }),
            _ => Err(LinkError::Path),
        }
    }

    /// A published TradingView idea's address: `/chart/SYMBOL/ID-slug/`, `/chart/ID/` or `/i/ID/`.
    pub fn parse_idea(input: &str) -> Result<Self, LinkError> {
        let (host, segments) = split(input)?;
        if !is_tradingview_host(&host) {
            return Err(LinkError::Host);
        }
        match segments[..] {
            // `/chart/NAME/` is also how a symbol's own chart is written, so a bare id has to look
            // like an id: those always mix in lower case, which a symbol never does.
            ["chart", id] if is_idea_id(id) && id.bytes().any(|b| b.is_ascii_lowercase()) => {
                Ok(Self::Idea {
                    id: id.to_owned(),
                    symbol: None,
                    slug: None,
                })
            }
            ["i", id] if is_idea_id(id) => Ok(Self::Idea {
                id: id.to_owned(),
                symbol: None,
                slug: None,
            }),
            ["chart", symbol, tail] if is_symbol(symbol) => {
                let (id, slug) = match tail.split_once('-') {
                    Some((id, slug)) => (id, Some(slug)),
                    None => (tail, None),
                };
                if !is_idea_id(id) || slug.is_some_and(|slug| !is_slug(slug)) {
                    return Err(LinkError::Path);
                }
                Ok(Self::Idea {
                    id: id.to_owned(),
                    symbol: Some(symbol.to_owned()),
                    slug: slug.map(str::to_owned),
                })
            }
            _ => Err(LinkError::Path),
        }
    }

    /// Whichever of the two the text is.
    pub fn parse(input: &str) -> Result<Self, LinkError> {
        match Self::parse_post(input) {
            Err(LinkError::Host) => Self::parse_idea(input),
            other => other,
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
        Link::parse_post(text).unwrap_or_else(|error| panic!("{text}: {error:?}"))
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
            assert_eq!(Link::parse_post(text), Err(*error), "{text:?}");
        }
        assert!(Link::parse_post("https://x.com/jack/status/20 \n").is_ok());
        assert_eq!(
            Link::parse_post("https://x.com/jack/status/2 0"),
            Err(LinkError::Malformed)
        );
        assert_eq!(
            Link::parse_post("https://x.com/jäck/status/20"),
            Err(LinkError::Malformed)
        );
        assert_eq!(
            Link::parse_post(&format!("https://x.com/jack/status/{}", "9".repeat(3_000))),
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
    fn idea_links_in_every_form_normalise_and_keep_what_they_prove() {
        let full = Link::parse_idea(
            "https://de.tradingview.com/chart/DOGE/Gj4niqTa-Add-Tweets-To-Your-Chart/?utm=1",
        )
        .unwrap();
        assert_eq!(
            full.url(),
            "https://www.tradingview.com/chart/DOGE/Gj4niqTa-Add-Tweets-To-Your-Chart/"
        );
        assert_eq!(full.headline(), "DOGE");
        assert_eq!(
            full.slug_words().as_deref(),
            Some("Add Tweets To Your Chart")
        );
        assert_eq!(full.short_id(), "Gj4niqTa");
        assert_eq!(full.time_ms(), None);
        assert_eq!(full.kind(), LinkKind::Idea);
        let short = Link::parse_idea("https://www.tradingview.com/i/Gj4niqTa/").unwrap();
        assert_eq!(short.url(), "https://www.tradingview.com/chart/Gj4niqTa/");
        let bare = Link::parse_idea("http://tradingview.com/chart/Gj4niqTa").unwrap();
        assert_eq!(bare.url(), "https://www.tradingview.com/chart/Gj4niqTa/");
        let no_slug =
            Link::parse_idea("https://www.tradingview.com/chart/BTCUSD/Gj4niqTa/").unwrap();
        assert_eq!(
            no_slug.url(),
            "https://www.tradingview.com/chart/BTCUSD/Gj4niqTa/"
        );
    }

    #[test]
    fn an_idea_link_must_name_one_idea_on_tradingview() {
        for (text, error) in [
            ("https://www.tradingview.com/", LinkError::Path),
            ("https://www.tradingview.com/chart/", LinkError::Path),
            // A symbol's own chart is not an idea, even when its name is eight characters long.
            (
                "https://www.tradingview.com/chart/USDCUSDT/",
                LinkError::Path,
            ),
            (
                "https://www.tradingview.com/symbols/BTCUSD/",
                LinkError::Path,
            ),
            (
                "https://www.tradingview.com/chart/BTCUSD/short/",
                LinkError::Path,
            ),
            (
                "https://www.tradingview.com/chart/BTCUSD/Gj4niqT-x/",
                LinkError::Path,
            ),
            (
                "https://www.tradingview.com/chart/BTCUSD/Gj4niqTa-bad slug/",
                LinkError::Malformed,
            ),
            (
                "https://www.tradingview.com/chart/BTCUSD/Gj4niqTa-bad$slug/",
                LinkError::Path,
            ),
            (
                "https://tradingview.com.evil.com/chart/DOGE/Gj4niqTa/",
                LinkError::Host,
            ),
            (
                "https://nottradingview.com/chart/DOGE/Gj4niqTa/",
                LinkError::Host,
            ),
            (
                "https://a.b.tradingview.com/chart/DOGE/Gj4niqTa/",
                LinkError::Host,
            ),
            (
                "https://www.tradingview.com@evil.com/chart/DOGE/Gj4niqTa/",
                LinkError::Authority,
            ),
            (
                "https://www.tradingview.com:444/chart/DOGE/Gj4niqTa/",
                LinkError::Authority,
            ),
            (
                "www.tradingview.com/chart/DOGE/Gj4niqTa/",
                LinkError::Scheme,
            ),
        ] {
            assert_eq!(Link::parse_idea(text), Err(error), "{text}");
        }
    }

    #[test]
    fn a_pasted_link_is_whichever_it_is_and_a_stranger_is_refused() {
        assert_eq!(
            Link::parse("https://x.com/jack/status/20").unwrap().kind(),
            LinkKind::Post
        );
        assert_eq!(
            Link::parse("https://www.tradingview.com/i/Gj4niqTa/")
                .unwrap()
                .kind(),
            LinkKind::Idea
        );
        assert_eq!(Link::parse("https://example.com/"), Err(LinkError::Host));
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
            Link::parse_idea("https://www.tradingview.com/chart/DOGE/Gj4niqTa-Add-Tweets/")
                .unwrap(),
            Link::parse_idea("https://www.tradingview.com/i/Gj4niqTa/").unwrap(),
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

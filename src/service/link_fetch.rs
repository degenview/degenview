//! The preview of a Post or Idea card, fetched only when the user asks for it.
//!
//! One request per click, to a fixed host (`publish.x.com` for an X post, `www.tradingview.com`
//! for an idea), over https, with no redirects, no cookies and no credentials, a short timeout and
//! a cap on how much of the answer is read. The answer is parsed as data: nothing in it is run,
//! and only a title, an author and a short text are kept, cut to the card's limits. What the
//! service does not say stays empty; nothing is filled in.

use crate::model::link_card::{Link, LinkKind, LinkMeta};
use std::io::Read;
use std::time::Duration;

/// How long the whole request may take.
const TIMEOUT: Duration = Duration::from_secs(12);
/// How much of an answer is read, in bytes. An idea's page carries its `<meta>` tags at the top.
const MAX_BODY_BYTES: u64 = 256 * 1024;

/// Why a preview could not be had, in words for the person who asked.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FetchError {
    /// No answer in time, or none at all.
    Network,
    /// The service answered that there is nothing there (a deleted post, a private idea).
    Unavailable,
    /// The service refused or failed, with this status.
    Refused(u16),
    /// An answer with nothing a card can show.
    Unreadable,
}

impl FetchError {
    pub fn message(self) -> String {
        match self {
            Self::Network => "The preview could not be fetched. Check your connection.".to_owned(),
            Self::Unavailable => {
                "The preview is unavailable: the link may be deleted or private.".to_owned()
            }
            Self::Refused(status) => format!("The service refused the request ({status})."),
            Self::Unreadable => "The service sent nothing a card can show.".to_owned(),
        }
    }
}

/// The address asked for `link`'s preview. Always `https` and always on the kind's own host.
pub fn request_url(link: &Link) -> String {
    match link.kind() {
        LinkKind::Post => format!(
            "https://{}/oembed?url={}&omit_script=true&dnt=true",
            LinkKind::Post.fetch_host(),
            percent_encode(&link.url())
        ),
        LinkKind::Idea => link.url(),
    }
}

/// Fetches the preview of `link`. Blocks; run it on a worker.
pub fn fetch(link: &Link, now_ms: i64) -> Result<LinkMeta, FetchError> {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .max_redirects(0)
        .max_redirects_will_error(false)
        .http_status_as_error(false)
        .https_only(true)
        .user_agent("DegenView")
        .build();
    let agent = ureq::Agent::new_with_config(config);
    let mut request = agent.get(request_url(link));
    if link.kind() == LinkKind::Idea {
        request = request.header("Range", format!("bytes=0-{}", MAX_BODY_BYTES - 1));
    }
    let mut response = request.call().map_err(|_| FetchError::Network)?;
    match response.status().as_u16() {
        200 | 206 => {}
        404 | 403 | 410 => return Err(FetchError::Unavailable),
        status => return Err(FetchError::Refused(status)),
    }
    let mut bytes = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(MAX_BODY_BYTES)
        .read_to_end(&mut bytes)
        .map_err(|_| FetchError::Network)?;
    let body = String::from_utf8_lossy(&bytes);
    let meta = match link.kind() {
        LinkKind::Post => parse_oembed(&body, link),
        LinkKind::Idea => parse_idea(&body),
    };
    meta.map(|meta| LinkMeta {
        fetched_at_ms: now_ms,
        ..meta
    })
    .ok_or(FetchError::Unreadable)
}

/// What X's oEmbed answer says: the author's name, their handle, and the post's own words.
pub fn parse_oembed(body: &str, link: &Link) -> Option<LinkMeta> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let name = value.get("author_name")?.as_str().unwrap_or_default();
    let handle = value
        .get("author_url")
        .and_then(serde_json::Value::as_str)
        .and_then(|url| url.trim_end_matches('/').rsplit('/').next())
        .filter(|handle| !handle.is_empty())
        .map(|handle| format!("@{handle}"))
        .or_else(|| match link {
            Link::Post {
                handle: Some(handle),
                ..
            } => Some(format!("@{handle}")),
            _ => None,
        })
        .unwrap_or_default();
    let html = value.get("html").and_then(serde_json::Value::as_str)?;
    // The words are the blockquote's first paragraph; what follows it is the byline.
    let start = html.find("<p")?;
    let paragraph = &html[html[start..].find('>')? + start + 1..];
    let paragraph = &paragraph[..paragraph.find("</p>")?];
    let meta = LinkMeta {
        title: name.to_owned(),
        author: handle,
        excerpt: decode_entities(&strip_tags(paragraph)),
        fetched_at_ms: 0,
    }
    .sanitized();
    (!meta.is_empty()).then_some(meta)
}

/// What a TradingView idea's page says about itself in its `<meta>` tags.
pub fn parse_idea(html: &str) -> Option<LinkMeta> {
    let mut title = String::new();
    let mut excerpt = String::new();
    let mut author = String::new();
    let mut rest = html;
    while let Some(start) = rest.find("<meta") {
        rest = &rest[start + 5..];
        let Some(end) = rest.find('>') else { break };
        let tag = &rest[..end];
        let key = attribute(tag, "property").or_else(|| attribute(tag, "name"));
        let content = attribute(tag, "content");
        if let (Some(key), Some(content)) = (key, content) {
            let slot = match key.as_str() {
                "og:title" => &mut title,
                "og:description" => &mut excerpt,
                "author" | "article:author" => &mut author,
                _ => continue,
            };
            if slot.is_empty() {
                *slot = decode_entities(&content);
            }
        }
        rest = &rest[end..];
    }
    let meta = LinkMeta {
        title,
        author,
        excerpt,
        fetched_at_ms: 0,
    }
    .sanitized();
    (!meta.is_empty()).then_some(meta)
}

/// The value of attribute `name` in the text of one tag, quoted either way.
fn attribute(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(found) = lower[from..].find(name) {
        let at = from + found;
        from = at + name.len();
        let before_ok = at == 0 || lower.as_bytes()[at - 1].is_ascii_whitespace();
        let rest = tag[at + name.len()..].trim_start();
        let Some(rest) = rest.strip_prefix('=') else {
            continue;
        };
        if !before_ok {
            continue;
        }
        let rest = rest.trim_start();
        let quote = rest.chars().next().filter(|c| matches!(c, '"' | '\''))?;
        let value = &rest[1..];
        return value.find(quote).map(|end| value[..end].to_owned());
    }
    None
}

/// The text of some HTML with its tags taken out. Line breaks become spaces.
fn strip_tags(html: &str) -> String {
    let mut text = String::with_capacity(html.len());
    let mut inside = false;
    for c in html.chars() {
        match c {
            '<' => {
                inside = true;
                text.push(' ');
            }
            '>' if inside => inside = false,
            _ if !inside => text.push(c),
            _ => {}
        }
    }
    text
}

/// Replaces the character references a page uses with the characters they stand for.
fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        let Some(end) = rest.find(';').filter(|end| *end <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let name = &rest[1..end];
        let decoded = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            "mdash" => Some('—'),
            "ndash" => Some('–'),
            "hellip" => Some('…'),
            _ => name
                .strip_prefix("#x")
                .or_else(|| name.strip_prefix("#X"))
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                .or_else(|| {
                    name.strip_prefix('#')
                        .and_then(|digits| digits.parse().ok())
                })
                .and_then(char::from_u32),
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// A value written into a query string.
fn percent_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 2);
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn post() -> Link {
        Link::parse("https://x.com/XDevelopers/status/1750000000000000000").unwrap()
    }

    #[test]
    fn a_request_goes_to_the_kinds_own_host_over_https() {
        let url = request_url(&post());
        assert_eq!(
            url,
            "https://publish.x.com/oembed?url=https%3A%2F%2Fx.com%2FXDevelopers%2Fstatus%2F1750000000000000000&omit_script=true&dnt=true"
        );
        let idea = Link::parse("https://www.tradingview.com/chart/BTCUSD/AbCdEf12-Setup/").unwrap();
        assert_eq!(
            request_url(&idea),
            "https://www.tradingview.com/chart/BTCUSD/AbCdEf12-Setup/"
        );
        for link in [post(), idea] {
            let url = request_url(&link);
            let host = url
                .strip_prefix("https://")
                .unwrap()
                .split('/')
                .next()
                .unwrap();
            assert_eq!(host, link.kind().fetch_host());
        }
    }

    #[test]
    fn an_oembed_answer_gives_the_name_the_handle_and_only_the_posts_words() {
        let body = r#"{"url":"https://twitter.com/XDevelopers/status/1750000000000000000","author_name":"Developers","author_url":"https://twitter.com/XDevelopers","html":"<blockquote class=\"twitter-tweet\"><p lang=\"en\" dir=\"ltr\">Fish &amp; chips<br>for <a href=\"https://t.co/x\">pic.x.com/1</a> &#8212; &quot;great&quot;</p>&mdash; Developers (@XDevelopers) <a href=\"https://twitter.com/XDevelopers/status/1\">January 1, 2024</a></blockquote>\n"}"#;
        let meta = parse_oembed(body, &post()).unwrap();
        assert_eq!(meta.title, "Developers");
        assert_eq!(meta.author, "@XDevelopers");
        assert_eq!(meta.excerpt, "Fish & chips for pic.x.com/1 — \"great\"");
    }

    #[test]
    fn an_oembed_answer_that_is_not_one_gives_nothing() {
        assert!(parse_oembed("not json", &post()).is_none());
        assert!(parse_oembed("{}", &post()).is_none());
        assert!(parse_oembed(r#"{"author_name":"A","html":"no paragraph"}"#, &post()).is_none());
    }

    #[test]
    fn an_ideas_meta_tags_give_its_title_and_description() {
        let html = r#"<html><head><title>x</title>
            <meta charset="utf-8">
            <meta property="og:title" content="BTC &amp; the range: a setup">
            <meta property='og:description' content="Breaks above &quot;resistance&quot;.">
            <meta property="og:image" content="https://s3.tradingview.com/x.png">
            <meta name="author" content="someone">
            <meta property="og:title" content="a later title is ignored">
            </head><body><script>var meta = '<meta property="og:title" content="no">'</script>"#;
        let meta = parse_idea(html).unwrap();
        assert_eq!(meta.title, "BTC & the range: a setup");
        assert_eq!(meta.excerpt, "Breaks above \"resistance\".");
        assert_eq!(meta.author, "someone");
        assert!(parse_idea("<html><head></head></html>").is_none());
    }

    #[test]
    fn what_is_kept_is_cut_to_the_cards_limits() {
        let html = format!(
            r#"<meta property="og:description" content="{}">"#,
            "word ".repeat(1_000)
        );
        let meta = parse_idea(&html).unwrap();
        assert_eq!(
            meta.excerpt.chars().count(),
            crate::model::link_card::MAX_EXCERPT_CHARS
        );
    }

    #[test]
    fn entities_decode_and_a_stray_ampersand_stays() {
        assert_eq!(
            decode_entities("a &amp; b &lt;c&gt; &#x41;&#66; &bogus; &"),
            "a & b <c> AB &bogus; &"
        );
        assert_eq!(
            decode_entities("&#1114112; &#xD800;"),
            "&#1114112; &#xD800;"
        );
    }
}

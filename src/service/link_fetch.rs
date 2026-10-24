//! The preview of a Post card, fetched when a card is placed or the user asks again.
//!
//! Requests go only to X's own hosts, over https, with no redirects, no cookies and no
//! credentials, a short timeout and a cap on how much of each answer is read: `publish.x.com` for
//! the post's name, handle and text (oEmbed), `cdn.syndication.twimg.com` for the author's picture
//! address (the endpoint X's own embeds use), and `pbs.twimg.com` for the picture itself. Answers
//! are parsed as data: nothing in them is run, and only a name, a handle, the post's text and a
//! picture are kept, cut to the card's limits and the picture checked and re-encoded like any
//! other. What X does not say stays empty; nothing is filled in. The picture is best effort: the
//! text is the preview, and a missing picture is not a failure.

use crate::model::link_card::{FETCH_HOST, Link, LinkMeta};
use crate::service::image_ingest::{self, Ingested};
use std::io::Read;
use std::time::Duration;

/// The host the author's picture address is asked of.
const SYNDICATION_HOST: &str = "cdn.syndication.twimg.com";
/// The only host a picture is downloaded from.
const AVATAR_HOST: &str = "pbs.twimg.com";
/// A picture larger than this is not read (a profile picture is a few kilobytes).
const MAX_AVATAR_BYTES: u64 = 1024 * 1024;

/// How long the whole request may take.
const TIMEOUT: Duration = Duration::from_secs(12);
/// How much of an answer is read, in bytes. An oEmbed answer is far smaller.
const MAX_BODY_BYTES: u64 = 256 * 1024;

/// Why a preview could not be had, in words for the person who asked.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FetchError {
    /// No answer in time, or none at all.
    Network,
    /// The service answered that there is nothing there (a deleted or protected post).
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

/// The address asked for `link`'s preview. Always `https` and always on X's own oEmbed host.
pub fn request_url(link: &Link) -> String {
    format!(
        "https://{FETCH_HOST}/oembed?url={}&omit_script=true&dnt=true",
        percent_encode(&link.url())
    )
}

/// What a fetch found: the preview, and the author's picture if it could be had. The picture is
/// checked and re-encoded but not yet stored: the caller stores it and names it on the preview.
pub struct Preview {
    pub meta: LinkMeta,
    pub avatar: Option<Ingested>,
}

fn agent() -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .max_redirects(0)
        .max_redirects_will_error(false)
        .http_status_as_error(false)
        .https_only(true)
        .user_agent("DegenView")
        .build();
    ureq::Agent::new_with_config(config)
}

/// Fetches the preview of `link`. Blocks; run it on a worker.
pub fn fetch(link: &Link, now_ms: i64) -> Result<Preview, FetchError> {
    let agent = agent();
    let meta = fetch_text(&agent, link, now_ms)?;
    let avatar = avatar_url(&agent, link).and_then(|url| download_avatar(&agent, &url));
    Ok(Preview { meta, avatar })
}

fn fetch_text(agent: &ureq::Agent, link: &Link, now_ms: i64) -> Result<LinkMeta, FetchError> {
    let mut response = agent
        .get(request_url(link))
        .call()
        .map_err(|_| FetchError::Network)?;
    match response.status().as_u16() {
        200 => {}
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
    parse_oembed(&body, link)
        .map(|meta| LinkMeta {
            fetched_at_ms: now_ms,
            ..meta
        })
        .ok_or(FetchError::Unreadable)
}

/// The token X's embed endpoint expects with a post id: the id over 10^15 times pi, in base 36,
/// with the dots and zeros taken out. It is not a secret, only what the endpoint asks for.
fn syndication_token(id: u64) -> String {
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let value = id as f64 / 1e15 * std::f64::consts::PI;
    let mut whole = value.trunc() as u64;
    let mut fraction = value.fract();
    let mut head = Vec::new();
    while whole > 0 {
        head.push(DIGITS[(whole % 36) as usize]);
        whole /= 36;
    }
    head.reverse();
    let mut text: Vec<u8> = head;
    for _ in 0..12 {
        fraction *= 36.0;
        let digit = fraction.trunc();
        text.push(DIGITS[digit as usize]);
        fraction -= digit;
    }
    text.retain(|byte| *byte != b'0');
    String::from_utf8(text).unwrap_or_default()
}

/// The address X's embed endpoint is asked for a post's author. Always `https` and on its own host.
pub fn syndication_url(link: &Link) -> String {
    format!(
        "https://{SYNDICATION_HOST}/tweet-result?id={}&lang=en&token={}",
        link.id,
        syndication_token(link.id)
    )
}

/// The address of the author's picture in an embed answer, if it is one this app will download:
/// `https` on X's own image host, under its profile pictures, with no user info or port. The
/// small size X names (`_normal`, 48 px) is swapped for the one a card draws (`_bigger`, 73 px).
pub fn parse_avatar_url(body: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let url = value
        .get("user")?
        .get("profile_image_url_https")?
        .as_str()?;
    let rest = url.strip_prefix(&format!("https://{AVATAR_HOST}/profile_images/"))?;
    let well_formed = !rest.is_empty()
        && rest.len() <= 200
        && rest
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'_' | b'-' | b'.'))
        && !rest.contains("..");
    well_formed.then(|| {
        format!(
            "https://{AVATAR_HOST}/profile_images/{}",
            rest.replacen("_normal.", "_bigger.", 1)
        )
    })
}

fn avatar_url(agent: &ureq::Agent, link: &Link) -> Option<String> {
    let mut response = agent.get(syndication_url(link)).call().ok()?;
    if response.status().as_u16() != 200 {
        return None;
    }
    let mut bytes = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(MAX_BODY_BYTES)
        .read_to_end(&mut bytes)
        .ok()?;
    parse_avatar_url(&String::from_utf8_lossy(&bytes))
}

fn download_avatar(agent: &ureq::Agent, url: &str) -> Option<Ingested> {
    let mut response = agent.get(url).call().ok()?;
    if response.status().as_u16() != 200 {
        return None;
    }
    let mut bytes = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(MAX_AVATAR_BYTES)
        .read_to_end(&mut bytes)
        .ok()?;
    // Checked, scaled and re-encoded like every picture: whatever the host sent is not kept as is.
    image_ingest::ingest(&bytes).ok()
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
        .or_else(|| link.handle.as_ref().map(|handle| format!("@{handle}")))
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
        avatar: None,
    }
    .sanitized();
    (!meta.is_empty()).then_some(meta)
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
    fn a_request_goes_to_xs_own_host_over_https() {
        let url = request_url(&post());
        assert_eq!(
            url,
            "https://publish.x.com/oembed?url=https%3A%2F%2Fx.com%2FXDevelopers%2Fstatus%2F1750000000000000000&omit_script=true&dnt=true"
        );
        for text in [
            "https://x.com/jack/status/20",
            "https://twitter.com/i/web/status/1519480761749016577",
        ] {
            let url = request_url(&Link::parse(text).unwrap());
            let host = url
                .strip_prefix("https://")
                .unwrap()
                .split('/')
                .next()
                .unwrap();
            assert_eq!(host, FETCH_HOST);
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
    fn what_is_kept_is_cut_to_the_cards_limits() {
        let body = format!(
            r#"{{"author_name":"A","html":"<blockquote><p>{}</p></blockquote>"}}"#,
            "word ".repeat(1_000)
        );
        let meta = parse_oembed(&body, &post()).unwrap();
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

    /// The shipped request path against the real service. Not run with the suite (it needs the
    /// network and X does not promise its answer): `cargo test --lib live_ -- --ignored
    /// --nocapture`.
    #[test]
    #[ignore = "contacts x.com"]
    fn live_a_real_post_gives_a_preview() {
        let post = Link::parse("https://x.com/Interior/status/463440424141459456").unwrap();
        let preview = fetch(&post, 1).expect("the post's preview");
        println!(
            "POST {:?} avatar {:?}",
            preview.meta,
            preview
                .avatar
                .as_ref()
                .map(|a| (a.width, a.height, a.bytes.len()))
        );
        assert!(!preview.meta.is_empty());
        assert!(preview.avatar.is_some(), "the author's picture");
    }

    #[test]
    fn the_embed_token_is_base_36_without_dots_or_zeros() {
        for id in [
            20,
            463_440_424_141_459_456,
            1_750_000_000_000_000_000,
            u64::MAX,
        ] {
            let token = syndication_token(id);
            assert!(
                !token.is_empty()
                    && token
                        .bytes()
                        .all(|b| b.is_ascii_digit() || b.is_ascii_lowercase()),
                "{id}: {token}"
            );
            assert!(!token.contains('0'));
        }
        assert_eq!(
            syndication_token(463_440_424_141_459_456),
            "14fxvks611f263w"
        );
    }

    #[test]
    fn the_embed_request_goes_to_xs_own_host() {
        let url = syndication_url(&post());
        assert!(
            url.starts_with(
                "https://cdn.syndication.twimg.com/tweet-result?id=1750000000000000000&"
            )
        );
    }

    #[test]
    fn only_a_profile_picture_on_xs_image_host_is_downloaded() {
        let answer = |url: &str| format!(r#"{{"user":{{"profile_image_url_https":"{url}"}}}}"#);
        assert_eq!(
            parse_avatar_url(&answer(
                "https://pbs.twimg.com/profile_images/432081479/DOI_LOGO_normal.jpg"
            )),
            Some("https://pbs.twimg.com/profile_images/432081479/DOI_LOGO_bigger.jpg".to_owned())
        );
        for refused in [
            "http://pbs.twimg.com/profile_images/1/a_normal.jpg",
            "https://evil.com/profile_images/1/a_normal.jpg",
            "https://pbs.twimg.com.evil.com/profile_images/1/a_normal.jpg",
            "https://pbs.twimg.com@evil.com/profile_images/1/a_normal.jpg",
            "https://pbs.twimg.com/media/1/a_normal.jpg",
            "https://pbs.twimg.com/profile_images/../media/a.jpg",
            "https://pbs.twimg.com/profile_images/1/a b.jpg",
            "https://pbs.twimg.com/profile_images/",
            "",
        ] {
            assert_eq!(parse_avatar_url(&answer(refused)), None, "{refused:?}");
        }
        assert_eq!(parse_avatar_url("not json"), None);
        assert_eq!(parse_avatar_url("{}"), None);
    }
}

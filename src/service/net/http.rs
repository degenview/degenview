//! Shared transport for idempotent market-data requests: GETs, and the JSON POSTs of providers whose
//! public queries are POSTed (Hyperliquid's `/info`). Provider parsing and rate limits remain at
//! their call sites; this only bounds network work and classifies failures.

use super::agent::FollowingAgent;
use std::sync::LazyLock;
use std::time::Duration;
use ureq::http::Response;

const TIMEOUT: Duration = Duration::from_secs(12);
const MAX_BODY_BYTES: u64 = 8 * 1024 * 1024;

static AGENT: LazyLock<FollowingAgent> = LazyLock::new(|| {
    FollowingAgent::new(|config| {
        config
            .timeout_global(Some(TIMEOUT))
            .http_status_as_error(false)
    })
});

/// Something that sends a request: the shared agent, which follows redirects hop by hop under the
/// proxy setting (`FollowingAgent::get_following`) for a GET, or a plain agent.
trait Transport {
    /// A GET, or a POST of `json` when it is given. A POST does not follow redirects.
    fn request(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        json: Option<&str>,
    ) -> Result<Response<ureq::Body>, ureq::Error>;
}

impl Transport for FollowingAgent {
    fn request(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        json: Option<&str>,
    ) -> Result<Response<ureq::Body>, ureq::Error> {
        match json {
            None => self.get_following(url, headers),
            Some(json) => post_json_with(&self.get_for(url), url, headers, json),
        }
    }
}

impl Transport for ureq::Agent {
    fn request(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        json: Option<&str>,
    ) -> Result<Response<ureq::Body>, ureq::Error> {
        match json {
            None => {
                let mut request = self.get(url);
                for &(name, value) in headers {
                    request = request.header(name, value);
                }
                request.call()
            }
            Some(json) => post_json_with(self, url, headers, json),
        }
    }
}

fn post_json_with(
    agent: &ureq::Agent,
    url: &str,
    headers: &[(&str, &str)],
    json: &str,
) -> Result<Response<ureq::Body>, ureq::Error> {
    let mut request = agent.post(url).header("content-type", "application/json");
    for &(name, value) in headers {
        request = request.header(name, value);
    }
    request.send(json)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Retry {
    /// The caller owns backoff, or another request would exceed a provider's limit.
    Never,
    /// One extra attempt for transport failures and 408/425/429/5xx responses.
    Transient,
}

/// What a request does besides what its [`Retry`] says.
#[derive(Clone, Copy, Debug)]
struct Policy {
    retry: Retry,
    /// Whether an error status is answered with the body the provider wrote ([`Error::Refused`]).
    keep_refusals: bool,
}

impl From<Retry> for Policy {
    fn from(retry: Retry) -> Self {
        Self {
            retry,
            keep_refusals: false,
        }
    }
}

/// The most of a refusal's body that is kept: an error message, not a page.
const REFUSAL_BODY_BYTES: u64 = 4 * 1024;

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum Error {
    #[error("HTTP {0}")]
    Status(u16),
    /// An error status with the body the provider wrote, from the requests that ask for it
    /// ([`get_text_reading_refusals`], [`post_json_reading_refusals`]) because that body tells what
    /// the status alone cannot (an unknown market is an HTTP 400 or 500 with an error code in it).
    #[error("HTTP {status}")]
    Refused { status: u16, body: String },
    #[error("response exceeds {0} bytes")]
    BodyTooLarge(u64),
    #[error("{0}")]
    Network(String),
}

impl Error {
    /// The HTTP status of a refusal, whether or not its body was kept.
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Status(status) | Self::Refused { status, .. } => Some(*status),
            Self::BodyTooLarge(_) | Self::Network(_) => None,
        }
    }
}

pub fn get_text(url: &str, headers: &[(&str, &str)], retry: Retry) -> Result<String, Error> {
    get_text_with_limit(url, headers, retry, MAX_BODY_BYTES)
}

/// A POST of the JSON text `body`, answered with the response text. For the public queries of a
/// provider that takes them as POSTs; the request is as safe to repeat as a GET, so `retry` applies
/// as it does there.
pub fn post_json(url: &str, body: &str, retry: Retry) -> Result<String, Error> {
    post_json_with_policy(url, body, retry.into())
}

/// [`post_json`] whose error statuses are [`Error::Refused`], with the provider's body.
pub fn post_json_reading_refusals(url: &str, body: &str, retry: Retry) -> Result<String, Error> {
    post_json_with_policy(
        url,
        body,
        Policy {
            retry,
            keep_refusals: true,
        },
    )
}

fn post_json_with_policy(url: &str, body: &str, policy: Policy) -> Result<String, Error> {
    fetch(
        &*AGENT,
        url,
        &[],
        Some(body),
        policy,
        std::thread::sleep,
        |response| {
            response
                .with_config()
                .limit(MAX_BODY_BYTES)
                .read_to_string()
        },
    )
}

pub fn head_status(url: &str) -> Result<u16, Error> {
    AGENT
        .head_following(url)
        .map(|response| response.status().as_u16())
        .map_err(|error| Error::Network(error.to_string()))
}

/// For known large endpoints, such as Kalshi's unpaginated series catalog.
pub fn get_text_with_limit(
    url: &str,
    headers: &[(&str, &str)],
    retry: Retry,
    max_body_bytes: u64,
) -> Result<String, Error> {
    get_with(
        &*AGENT,
        url,
        headers,
        retry,
        max_body_bytes,
        std::thread::sleep,
    )
}

fn retry_delay(retry_after: Option<&str>, attempt: usize, random: u32) -> Duration {
    if let Some(seconds) = retry_after.and_then(|value| value.trim().parse::<u64>().ok()) {
        return Duration::from_secs(seconds.min(5));
    }
    let base_ms = 250_u64.saturating_mul(1_u64 << attempt.min(4));
    let jitter = 75 + u64::from(random % 51); // 75–125% of the base delay.
    Duration::from_millis(base_ms * jitter / 100)
}

fn jitter_seed() -> u32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.subsec_nanos())
}

fn get_with(
    agent: &impl Transport,
    url: &str,
    headers: &[(&str, &str)],
    retry: Retry,
    max_body_bytes: u64,
    sleep: impl Fn(Duration),
) -> Result<String, Error> {
    fetch(agent, url, headers, None, retry, sleep, |body| {
        body.with_config().limit(max_body_bytes).read_to_string()
    })
}

/// A GET that returns the body as bytes, for images. `max_body_bytes` bounds what is read.
pub fn get_bytes(url: &str, max_body_bytes: u64) -> Result<Vec<u8>, Error> {
    fetch(
        &*AGENT,
        url,
        &[],
        None,
        Retry::Transient,
        std::thread::sleep,
        |body| body.with_config().limit(max_body_bytes).read_to_vec(),
    )
}

/// The request loop shared by every GET and POST: retry classification and `Retry-After` live
/// here, the caller only says how to read a successful body. `json` is the body of a POST.
fn fetch<T>(
    agent: &impl Transport,
    url: &str,
    headers: &[(&str, &str)],
    json: Option<&str>,
    policy: impl Into<Policy>,
    sleep: impl Fn(Duration),
    read: impl Fn(&mut ureq::Body) -> Result<T, ureq::Error>,
) -> Result<T, Error> {
    let policy = policy.into();
    let attempts = if policy.retry == Retry::Transient {
        2
    } else {
        1
    };
    for attempt in 0..attempts {
        let (result, retry_after) = match agent.request(url, headers, json) {
            Ok(mut response) => {
                let status = response.status().as_u16();
                if status >= 400 {
                    let retry_after = response
                        .headers()
                        .get("retry-after")
                        .and_then(|value| value.to_str().ok())
                        .map(str::to_owned);
                    let error = if policy.keep_refusals {
                        let body = response
                            .body_mut()
                            .with_config()
                            .limit(REFUSAL_BODY_BYTES)
                            .read_to_string()
                            .unwrap_or_default();
                        Error::Refused { status, body }
                    } else {
                        Error::Status(status)
                    };
                    (Err(error), retry_after)
                } else {
                    let body = read(response.body_mut()).map_err(|error| match error {
                        ureq::Error::BodyExceedsLimit(limit) => Error::BodyTooLarge(limit),
                        other => Error::Network(other.to_string()),
                    });
                    (body, None)
                }
            }
            Err(ureq::Error::StatusCode(code)) => (Err(Error::Status(code)), None),
            Err(error) => (Err(Error::Network(error.to_string())), None),
        };
        let should_retry = match &result {
            Err(Error::Network(_)) => true,
            Err(error) => matches!(error.status(), Some(408 | 425 | 429 | 500..=599)),
            Ok(_) => false,
        };
        if attempt + 1 == attempts || !should_retry {
            return result;
        }
        sleep(retry_delay(retry_after.as_deref(), attempt, jitter_seed()));
    }
    unreachable!("at least one attempt")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    fn serve(
        replies: &'static [(u16, &'static str)],
    ) -> (String, Arc<AtomicUsize>, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let count = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&count);
        let thread = std::thread::spawn(move || {
            for &(status, headers) in replies {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0_u8; 1024];
                let _ = stream.read(&mut request);
                seen.fetch_add(1, Ordering::SeqCst);
                let body = if status == 200 { "ok" } else { "error" };
                write!(stream, "HTTP/1.1 {status} test\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n{body}", body.len()).unwrap();
            }
        });
        (url, count, thread)
    }

    #[test]
    fn transient_server_error_gets_one_more_attempt() {
        let (url, count, thread) = serve(&[(503, ""), (200, "")]);
        let agent =
            ureq::Agent::new_with_config(ureq::Agent::config_builder().max_redirects(0).build());
        assert_eq!(
            get_with(&agent, &url, &[], Retry::Transient, MAX_BODY_BYTES, |_| {}).unwrap(),
            "ok"
        );
        thread.join().unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_refusal_keeps_the_body_only_when_asked() {
        let agent = ureq::Agent::new_with_config(
            ureq::Agent::config_builder()
                .http_status_as_error(false)
                .build(),
        );
        let (url, _, thread) = serve(&[(500, "")]);
        let kept = Policy {
            retry: Retry::Never,
            keep_refusals: true,
        };
        let read = |body: &mut ureq::Body| body.read_to_string();
        assert_eq!(
            fetch(&agent, &url, &[], None, kept, |_| {}, read),
            Err(Error::Refused {
                status: 500,
                body: "error".to_owned()
            })
        );
        thread.join().unwrap();
        let (url, _, thread) = serve(&[(500, "")]);
        assert_eq!(
            fetch(&agent, &url, &[], None, Retry::Never, |_| {}, read),
            Err(Error::Status(500))
        );
        thread.join().unwrap();
        assert_eq!(
            Error::Refused {
                status: 429,
                body: String::new()
            }
            .status(),
            Some(429)
        );
        assert_eq!(Error::Network("down".to_owned()).status(), None);
    }

    #[test]
    fn a_kept_refusal_is_retried_like_its_status() {
        let (url, count, thread) = serve(&[(503, ""), (200, "")]);
        let agent = ureq::Agent::new_with_config(
            ureq::Agent::config_builder()
                .http_status_as_error(false)
                .build(),
        );
        let kept = Policy {
            retry: Retry::Transient,
            keep_refusals: true,
        };
        let answer = fetch(
            &agent,
            &url,
            &[],
            None,
            kept,
            |_| {},
            |body| body.read_to_string(),
        );
        assert_eq!(answer, Ok("ok".to_owned()));
        thread.join().unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn rate_limit_is_left_to_caller_with_no_retry() {
        let (url, count, thread) = serve(&[(429, "")]);
        let agent = ureq::Agent::new_with_defaults();
        assert!(matches!(
            get_with(&agent, &url, &[], Retry::Never, MAX_BODY_BYTES, |_| {}),
            Err(Error::Status(429))
        ));
        thread.join().unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn transient_rate_limit_uses_retry_after() {
        let (url, count, thread) = serve(&[(429, "Retry-After: 2\r\n"), (200, "")]);
        let agent = ureq::Agent::new_with_config(
            ureq::Agent::config_builder()
                .http_status_as_error(false)
                .build(),
        );
        let delays = Mutex::new(Vec::new());
        assert_eq!(
            get_with(
                &agent,
                &url,
                &[],
                Retry::Transient,
                MAX_BODY_BYTES,
                |delay| { delays.lock().unwrap().push(delay) }
            )
            .unwrap(),
            "ok"
        );
        thread.join().unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 2);
        assert_eq!(*delays.lock().unwrap(), [Duration::from_secs(2)]);
    }

    #[test]
    fn response_body_limit_is_enforced() {
        let (url, _, thread) = serve(&[(200, "")]);
        let agent = ureq::Agent::new_with_defaults();
        assert!(matches!(
            get_with(&agent, &url, &[], Retry::Never, 1, |_| {}),
            Err(Error::BodyTooLarge(1))
        ));
        thread.join().unwrap();
    }

    #[test]
    fn bytes_are_read_whole_and_bounded() {
        let read =
            |limit: u64| move |body: &mut ureq::Body| body.with_config().limit(limit).read_to_vec();
        let agent = ureq::Agent::new_with_defaults();
        let (url, _, thread) = serve(&[(200, "")]);
        assert_eq!(
            fetch(
                &agent,
                &url,
                &[],
                None,
                Retry::Never,
                |_| {},
                read(MAX_BODY_BYTES)
            )
            .unwrap(),
            b"ok"
        );
        thread.join().unwrap();
        let (url, _, thread) = serve(&[(200, "")]);
        assert!(matches!(
            fetch(&agent, &url, &[], None, Retry::Never, |_| {}, read(1)),
            Err(Error::BodyTooLarge(1))
        ));
        thread.join().unwrap();
    }

    #[test]
    fn a_json_post_sends_its_body_with_its_content_type() {
        const BODY: &str = r#"{"type":"meta"}"#;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut chunk = [0_u8; 1024];
            while !request.ends_with(BODY.as_bytes()) {
                let read = stream.read(&mut chunk).unwrap();
                assert!(read > 0, "the request ended before its body");
                request.extend_from_slice(&chunk[..read]);
            }
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok");
            String::from_utf8(request).unwrap().to_lowercase()
        });
        let agent = ureq::Agent::new_with_defaults();
        let read =
            |body: &mut ureq::Body| body.with_config().limit(MAX_BODY_BYTES).read_to_string();
        let answer = fetch(&agent, &url, &[], Some(BODY), Retry::Never, |_| {}, read).unwrap();
        assert_eq!(answer, "ok");
        let request = server.join().unwrap();
        assert!(request.starts_with("post / http/1.1"), "{request}");
        assert!(
            request.contains("content-type: application/json"),
            "{request}"
        );
    }

    #[test]
    fn stalled_response_obeys_request_deadline() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request);
            std::thread::sleep(Duration::from_millis(100));
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok");
        });
        let agent = ureq::Agent::new_with_config(
            ureq::Agent::config_builder()
                .timeout_global(Some(Duration::from_millis(30)))
                .build(),
        );
        assert!(matches!(
            get_with(&agent, &url, &[], Retry::Never, MAX_BODY_BYTES, |_| {}),
            Err(Error::Network(_))
        ));
        server.join().unwrap();
    }

    #[test]
    fn jitter_and_retry_after_are_bounded() {
        assert_eq!(retry_delay(Some("999"), 0, 0), Duration::from_secs(5));
        assert_eq!(retry_delay(None, 0, 0), Duration::from_millis(187));
        assert_eq!(retry_delay(None, 0, 50), Duration::from_millis(312));
    }
}

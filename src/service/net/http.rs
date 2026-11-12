//! Shared transport for idempotent market-data GETs. Provider parsing and rate limits
//! remain at their call sites; this only bounds network work and classifies failures.

use std::sync::LazyLock;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(12);
const MAX_BODY_BYTES: u64 = 8 * 1024 * 1024;

static AGENT: LazyLock<ureq::Agent> = LazyLock::new(|| {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .http_status_as_error(false)
        .build();
    ureq::Agent::new_with_config(config)
});

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Retry {
    /// The caller owns backoff, or another request would exceed a provider's limit.
    Never,
    /// One extra attempt for transport failures and 408/425/429/5xx responses.
    Transient,
}

#[derive(Debug)]
pub enum Error {
    Status(u16),
    BodyTooLarge(u64),
    Network(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Status(code) => write!(f, "HTTP {code}"),
            Self::BodyTooLarge(limit) => write!(f, "response exceeds {limit} bytes"),
            Self::Network(message) => f.write_str(message),
        }
    }
}

pub fn get_text(url: &str, headers: &[(&str, &str)], retry: Retry) -> Result<String, Error> {
    get_text_with_limit(url, headers, retry, MAX_BODY_BYTES)
}

pub fn head_status(url: &str) -> Result<u16, Error> {
    AGENT
        .head(url)
        .call()
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
    get_with(&AGENT, url, headers, retry, max_body_bytes, |delay| {
        std::thread::sleep(delay)
    })
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
    agent: &ureq::Agent,
    url: &str,
    headers: &[(&str, &str)],
    retry: Retry,
    max_body_bytes: u64,
    sleep: impl Fn(Duration),
) -> Result<String, Error> {
    fetch(agent, url, headers, retry, sleep, |body| {
        body.with_config().limit(max_body_bytes).read_to_string()
    })
}

/// A GET that returns the body as bytes, for images. `max_body_bytes` bounds what is read.
pub fn get_bytes(url: &str, max_body_bytes: u64) -> Result<Vec<u8>, Error> {
    fetch(
        &AGENT,
        url,
        &[],
        Retry::Transient,
        std::thread::sleep,
        |body| body.with_config().limit(max_body_bytes).read_to_vec(),
    )
}

/// The request loop shared by every GET: retry classification and `Retry-After` live here, the
/// caller only says how to read a successful body.
fn fetch<T>(
    agent: &ureq::Agent,
    url: &str,
    headers: &[(&str, &str)],
    retry: Retry,
    sleep: impl Fn(Duration),
    read: impl Fn(&mut ureq::Body) -> Result<T, ureq::Error>,
) -> Result<T, Error> {
    let attempts = if retry == Retry::Transient { 2 } else { 1 };
    for attempt in 0..attempts {
        let mut request = agent.get(url);
        for &(name, value) in headers {
            request = request.header(name, value);
        }
        let (result, retry_after) = match request.call() {
            Ok(mut response) => {
                let status = response.status().as_u16();
                if status >= 400 {
                    let retry_after = response
                        .headers()
                        .get("retry-after")
                        .and_then(|value| value.to_str().ok())
                        .map(str::to_owned);
                    (Err(Error::Status(status)), retry_after)
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
        let should_retry = matches!(
            result,
            Err(Error::Status(408 | 425 | 429 | 500..=599)) | Err(Error::Network(_))
        );
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
            fetch(&agent, &url, &[], Retry::Never, |_| {}, read(1)),
            Err(Error::BodyTooLarge(1))
        ));
        thread.join().unwrap();
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

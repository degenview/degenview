//! The updater's download: HTTPS only, from GitHub's hosts only, on every hop.
//!
//! GitHub answers a release-asset URL with a 302 to `release-assets.githubusercontent.com`. The
//! download follows redirects itself so each hop can be checked; the SHA-256 the caller compares
//! afterwards is what makes the bytes trustworthy, this only keeps the request from being sent
//! somewhere else (plain HTTP, another host, an internal address).

use http::Uri;
use std::io::Read;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(600);
const MAX_HOPS: usize = 3;
const CHUNK_BYTES: usize = 64 * 1024;

/// Blocking: GETs `url`, following at most `MAX_HOPS` allowed redirects, and returns the body,
/// which must be `max_bytes` long or shorter.
/// Calls `progress` with the bytes received so far after every chunk.
pub fn get(url: &str, max_bytes: u64, progress: &dyn Fn(u64)) -> Result<Vec<u8>, String> {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .http_status_as_error(false)
        .max_redirects(0)
        .max_redirects_will_error(false)
        .https_only(true)
        .build();
    fetch(
        &ureq::Agent::new_with_config(config),
        url,
        max_bytes,
        &allowed_url,
        progress,
    )
}

fn fetch(
    agent: &ureq::Agent,
    url: &str,
    max_bytes: u64,
    allowed: &dyn Fn(&str) -> bool,
    progress: &dyn Fn(u64),
) -> Result<Vec<u8>, String> {
    let mut url = url.to_owned();
    for _ in 0..=MAX_HOPS {
        if !allowed(&url) {
            return Err("Download failed: unexpected download address.".to_owned());
        }
        let mut response = agent
            .get(&url)
            .header(
                "User-Agent",
                concat!("DegenView/", env!("CARGO_PKG_VERSION")),
            )
            .call()
            .map_err(|error| format!("Download failed: {error}"))?;
        match response.status().as_u16() {
            200 => {
                let reader = response
                    .body_mut()
                    .with_config()
                    .limit(max_bytes + 1)
                    .reader();
                return read_counting(reader, progress);
            }
            301 | 302 | 303 | 307 | 308 => {
                let location = response
                    .headers()
                    .get("location")
                    .and_then(|value| value.to_str().ok())
                    .ok_or("Download failed: redirect without a location.")?;
                url = resolve(&url, location)
                    .ok_or("Download failed: unexpected download address.")?;
            }
            status => return Err(format!("Download failed: HTTP {status}.")),
        }
    }
    Err("Download failed: too many redirects.".to_owned())
}

/// Reads `reader` to the end in chunks, reporting the running total.
fn read_counting(mut reader: impl Read, progress: &dyn Fn(u64)) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    let mut chunk = vec![0_u8; CHUNK_BYTES];
    loop {
        let read = reader
            .read(&mut chunk)
            .map_err(|error| format!("Download failed: {error}"))?;
        if read == 0 {
            return Ok(bytes);
        }
        bytes.extend_from_slice(&chunk[..read]);
        progress(bytes.len() as u64);
    }
}

/// `location` as an absolute URL: already absolute, or a path on `base`'s own origin.
fn resolve(base: &str, location: &str) -> Option<String> {
    if location.starts_with("https://") || location.starts_with("http://") {
        return Some(location.to_owned());
    }
    if location.starts_with('/') && !location.starts_with("//") {
        let base: Uri = base.parse().ok()?;
        return Some(format!(
            "{}://{}{location}",
            base.scheme_str()?,
            base.authority()?
        ));
    }
    None
}

/// `https://github.com/…` or `https://*.githubusercontent.com/…`, no credentials, default port.
fn allowed_url(url: &str) -> bool {
    let Ok(uri) = url.parse::<Uri>() else {
        return false;
    };
    let (Some("https"), Some(authority)) = (uri.scheme_str(), uri.authority()) else {
        return false;
    };
    let host = authority.host().to_ascii_lowercase();
    let known_host = host == "github.com" || host.ends_with(".githubusercontent.com");
    known_host && !authority.as_str().contains('@') && authority.port_u16().is_none_or(|p| p == 443)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    #[test]
    fn only_github_over_https_is_allowed() {
        for url in [
            "https://github.com/degenview/degenview/releases/download/v1/a.zip",
            "https://release-assets.githubusercontent.com/x?y=1",
            "https://objects.githubusercontent.com/x",
            "https://GitHub.com/x",
            "https://github.com:443/x",
        ] {
            assert!(allowed_url(url), "{url}");
        }
        for url in [
            "http://github.com/x",
            "https://evil.com/github.com",
            "https://evil.com/x.githubusercontent.com",
            "https://github.com.evil.com/x",
            "https://xgithubusercontent.com/x",
            "https://githubusercontent.com/x",
            "https://github.com@evil.com/x",
            "https://evil.com@github.com/x",
            "https://github.com:8443/x",
            "https://127.0.0.1/x",
            "ftp://github.com/x",
            "//github.com/x",
            "/x",
            "",
        ] {
            assert!(!allowed_url(url), "{url}");
        }
    }

    #[test]
    fn locations_resolve_against_their_own_origin_only() {
        assert_eq!(
            resolve("https://github.com/a/b", "/c/d?e=1").as_deref(),
            Some("https://github.com/c/d?e=1")
        );
        assert_eq!(
            resolve("https://github.com/a", "https://x.githubusercontent.com/z").as_deref(),
            Some("https://x.githubusercontent.com/z")
        );
        assert_eq!(resolve("https://github.com/a", "//evil.com/x"), None);
        assert_eq!(resolve("https://github.com/a", "relative"), None);
    }

    /// Serves `replies` in order on a local port; returns its base URL.
    fn serve(replies: Vec<String>) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let thread = std::thread::spawn(move || {
            for reply in replies {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let mut request = [0_u8; 1024];
                let _ = stream.read(&mut request);
                let _ = stream.write_all(reply.as_bytes());
            }
        });
        (base, thread)
    }

    fn ok(body: &str) -> String {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    fn redirect(location: &str) -> String {
        format!(
            "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )
    }

    fn agent() -> ureq::Agent {
        ureq::Agent::new_with_config(
            ureq::Agent::config_builder()
                .http_status_as_error(false)
                .max_redirects(0)
                .max_redirects_will_error(false)
                .build(),
        )
    }

    fn local_only(url: &str) -> bool {
        url.starts_with("http://127.0.0.1:")
    }

    #[test]
    fn a_checked_redirect_chain_is_followed() {
        let (base, thread) = serve(vec![redirect("/next"), ok("zip-bytes")]);
        let body = fetch(
            &agent(),
            &format!("{base}/start"),
            100,
            &local_only,
            &|_| {},
        )
        .unwrap();
        assert_eq!(body, b"zip-bytes");
        thread.join().unwrap();
    }

    #[test]
    fn a_redirect_to_another_host_is_refused_without_connecting() {
        let (base, thread) = serve(vec![redirect("http://169.254.169.254/latest")]);
        let error = fetch(
            &agent(),
            &format!("{base}/start"),
            100,
            &local_only,
            &|_| {},
        )
        .unwrap_err();
        assert!(error.contains("unexpected download address"), "{error}");
        thread.join().unwrap();
    }

    #[test]
    fn progress_counts_up_to_the_body_length() {
        let body = "x".repeat(200_000);
        let (base, thread) = serve(vec![ok(&body)]);
        let seen = std::cell::RefCell::new(Vec::new());
        let bytes = fetch(
            &agent(),
            &format!("{base}/a"),
            300_000,
            &local_only,
            &|done| seen.borrow_mut().push(done),
        )
        .unwrap();
        assert_eq!(bytes.len(), 200_000);
        let seen = seen.into_inner();
        assert!(seen.windows(2).all(|pair| pair[0] < pair[1]), "{seen:?}");
        assert_eq!(seen.last(), Some(&200_000));
        thread.join().unwrap();
    }

    #[test]
    fn redirect_loops_stop_after_a_few_hops() {
        let replies = (0..=MAX_HOPS).map(|_| redirect("/again")).collect();
        let (base, thread) = serve(replies);
        let error = fetch(
            &agent(),
            &format!("{base}/start"),
            100,
            &local_only,
            &|_| {},
        )
        .unwrap_err();
        assert!(error.contains("too many redirects"), "{error}");
        thread.join().unwrap();
    }

    #[test]
    fn errors_and_oversized_bodies_fail() {
        let (base, thread) = serve(vec![
            "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned(),
        ]);
        assert!(
            fetch(&agent(), &format!("{base}/a"), 100, &local_only, &|_| {})
                .unwrap_err()
                .contains("HTTP 404")
        );
        thread.join().unwrap();
        let (base, thread) = serve(vec![ok("0123456789")]);
        assert!(fetch(&agent(), &format!("{base}/a"), 3, &local_only, &|_| {}).is_err());
        thread.join().unwrap();
    }
}

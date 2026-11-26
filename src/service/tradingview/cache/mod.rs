//! A read-only reader of a Chromium HTTP cache, the two formats there are: "simple" (one file per
//! entry; macOS and Linux) and "blockfile" (an index and shared block files; Windows). Entries are
//! listed by address first, and only the ones asked for are read further, so nothing else the
//! app fetched is ever decoded.

mod blockfile;
mod encoding;
mod simple;
#[cfg(test)]
pub(crate) mod testing;

use std::io;
use std::path::Path;

/// A cached entry larger than this is skipped: nothing imported comes near it.
const MAX_ENTRY_BYTES: u64 = 32 * 1024 * 1024;

/// One cached response.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Response {
    /// The address it was fetched from, query included: it holds access tokens, so it is never
    /// logged or kept.
    pub url: String,
    /// When it was received, in epoch milliseconds.
    pub at_ms: i64,
    /// The body, decompressed.
    pub body: Vec<u8>,
}

/// Whether `dir` is a cache of either format.
pub fn is_cache(dir: &Path) -> bool {
    blockfile::is_blockfile(dir) || simple::is_simple(dir)
}

/// The successful responses of `dir` whose address `wanted` accepts. An entry that cannot be read
/// (being written, damaged, an encoding not known) is passed over.
pub fn read(dir: &Path, wanted: &dyn Fn(&str) -> bool) -> io::Result<Vec<Response>> {
    if blockfile::is_blockfile(dir) {
        blockfile::read(dir, wanted)
    } else {
        simple::read(dir, wanted)
    }
}

/// The address in a cache key. A key may start with the site the request was made for
/// (`1/0/https://…`, `_dk_site site https://…`).
fn url_of(key: &str) -> Option<&str> {
    let last = key.rsplit(' ').next()?;
    let start = last.find("https://")?;
    Some(&last[start..])
}

/// Microseconds from 1601, Chromium's epoch, to 1970.
const EPOCH_OFFSET_US: i64 = 11_644_473_600_000_000;
/// The bit of the header record's flags that says a second flags word follows.
const HAS_EXTRA_FLAGS: u32 = 1 << 31;

/// When the response was received, from the head of its stored headers: a length, flags (and
/// maybe more), then the request and response times. A refreshed response is written into the
/// entry it had, so neither the entry's creation nor the file's time is dependably this.
fn received_ms(headers: &[u8]) -> Option<i64> {
    let flags = u32::from_le_bytes(headers.get(4..8)?.try_into().ok()?);
    let at = if flags & HAS_EXTRA_FLAGS != 0 { 20 } else { 16 };
    let micros = i64::from_le_bytes(headers.get(at..at + 8)?.try_into().ok()?);
    // Anything before 1970 is not a time.
    (micros > EPOCH_OFFSET_US).then(|| (micros - EPOCH_OFFSET_US) / 1_000)
}

/// The body of a response from its stored headers and bytes; `None` unless it succeeded and
/// could be decompressed.
fn body_of(headers: &[u8], body: &[u8]) -> Option<Vec<u8>> {
    let headers = String::from_utf8_lossy(headers);
    // The headers are a pickled list of NUL-separated lines.
    let mut lines = headers.split('\0');
    let status = lines.find_map(|line| {
        let start = line.find("HTTP/")?;
        line[start..].split_whitespace().nth(1).map(str::to_owned)
    })?;
    if status != "200" {
        return None;
    }
    let coding = lines.find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("content-encoding")
            .then(|| value.trim().to_ascii_lowercase())
    });
    encoding::decode(coding.as_deref(), body)
}

#[cfg(test)]
mod tests;

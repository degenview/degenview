//! Chromium's "simple" cache backend: a file per entry named `<hash>_0`, holding a header, the
//! key, the body (stream 1), an end record, the response headers (stream 0), optionally the key's
//! SHA-256, and a second end record. See Chromium's `simple_entry_format.h`.

use super::{MAX_ENTRY_BYTES, Response, body_of, url_of};
use std::fs;
use std::io::{self, Read};
use std::path::Path;
use std::time::UNIX_EPOCH;

const HEADER_MAGIC: u64 = 0xfcfb_6d1b_a772_5c30;
const END_MAGIC: u64 = 0xf4fa_6f45_970d_41d8;
const HEADER_BYTES: usize = 24;
const END_BYTES: usize = 24;
/// The end record's flag for a key hash stored before it.
const HAS_KEY_SHA256: u32 = 2;
const SHA256_BYTES: usize = 32;
/// The longest key read while looking for the wanted entries.
const MAX_KEY_BYTES: usize = 16 * 1024;

pub fn is_simple(dir: &Path) -> bool {
    dir.join("index-dir").is_dir()
}

pub fn read(dir: &Path, wanted: &dyn Fn(&str) -> bool) -> io::Result<Vec<Response>> {
    let mut found = Vec::new();
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        let is_entry = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with("_0"));
        if !is_entry {
            continue;
        }
        // The app may delete or rewrite an entry while it runs.
        if let Some(response) = read_entry(&path, wanted) {
            found.push(response);
        }
    }
    Ok(found)
}

fn read_entry(path: &Path, wanted: &dyn Fn(&str) -> bool) -> Option<Response> {
    let mut file = fs::File::open(path).ok()?;
    let metadata = file.metadata().ok()?;
    if metadata.len() > MAX_ENTRY_BYTES {
        return None;
    }
    let mut header = [0u8; HEADER_BYTES];
    file.read_exact(&mut header).ok()?;
    if u64_at(&header, 0)? != HEADER_MAGIC {
        return None;
    }
    let key_length = u32_at(&header, 12)? as usize;
    if key_length > MAX_KEY_BYTES {
        return None;
    }
    let mut key = vec![0u8; key_length];
    file.read_exact(&mut key).ok()?;
    let url = url_of(std::str::from_utf8(&key).ok()?)?.to_owned();
    if !wanted(&url) {
        return None;
    }
    let mut rest = Vec::new();
    file.read_to_end(&mut rest).ok()?;
    let (headers, body) = streams(&rest)?;
    let at_ms = metadata
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_millis();
    Some(Response {
        body: body_of(headers, body)?,
        url,
        at_ms: i64::try_from(at_ms).ok()?,
    })
}

/// `(headers, body)` from what follows the key.
fn streams(rest: &[u8]) -> Option<(&[u8], &[u8])> {
    let last_end = rest.len().checked_sub(END_BYTES)?;
    if u64_at(rest, last_end)? != END_MAGIC {
        return None;
    }
    let flags = u32_at(rest, last_end + 8)?;
    let headers_length = u32_at(rest, last_end + 16)? as usize;
    let headers_end = if flags & HAS_KEY_SHA256 != 0 {
        last_end.checked_sub(SHA256_BYTES)?
    } else {
        last_end
    };
    let headers_start = headers_end.checked_sub(headers_length)?;
    let body_end = headers_start.checked_sub(END_BYTES)?;
    if u64_at(rest, body_end)? != END_MAGIC {
        return None;
    }
    Some((&rest[headers_start..headers_end], &rest[..body_end]))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn u64_at(bytes: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(bytes.get(at..at + 8)?.try_into().ok()?))
}

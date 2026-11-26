//! The content codings a cached body may carry.

use std::io::Read;

/// The most a body may decompress to.
const MAX_DECODED_BYTES: u64 = 64 * 1024 * 1024;

pub fn decode(coding: Option<&str>, body: &[u8]) -> Option<Vec<u8>> {
    match coding {
        None | Some("identity") => Some(body.to_vec()),
        Some("gzip" | "x-gzip") => bounded(flate2::read::MultiGzDecoder::new(body)),
        Some("deflate") => bounded(flate2::read::ZlibDecoder::new(body)),
        Some("br") => bounded(brotli_decompressor::Decompressor::new(body, 8 * 1024)),
        Some("zstd") => bounded(ruzstd::decoding::StreamingDecoder::new(body).ok()?),
        Some(_) => None,
    }
}

fn bounded(reader: impl Read) -> Option<Vec<u8>> {
    let mut decoded = Vec::new();
    reader
        .take(MAX_DECODED_BYTES + 1)
        .read_to_end(&mut decoded)
        .ok()?;
    (decoded.len() as u64 <= MAX_DECODED_BYTES).then_some(decoded)
}

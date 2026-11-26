//! The content codings a cached body may carry.

use ruzstd::decoding::StreamingDecoder;
use ruzstd::decoding::errors::{FrameDecoderError, ReadFrameHeaderError};
use std::io::Read;

/// The most a body may decompress to.
const MAX_DECODED_BYTES: u64 = 64 * 1024 * 1024;

pub fn decode(coding: Option<&str>, body: &[u8]) -> Option<Vec<u8>> {
    match coding {
        None | Some("identity") => Some(body.to_vec()),
        Some("gzip" | "x-gzip") => bounded(flate2::read::MultiGzDecoder::new(body)),
        Some("deflate") => bounded(flate2::read::ZlibDecoder::new(body)),
        Some("br") => bounded(brotli_decompressor::Decompressor::new(body, 8 * 1024)),
        Some("zstd") => zstd(body),
        Some(_) => None,
    }
}

/// Every frame of a zstd body: a response is often sent as several.
fn zstd(mut body: &[u8]) -> Option<Vec<u8>> {
    let mut decoded = Vec::new();
    while !body.is_empty() {
        match StreamingDecoder::new(&mut body) {
            Ok(mut frame) => {
                let room = MAX_DECODED_BYTES + 1 - decoded.len() as u64;
                frame.by_ref().take(room).read_to_end(&mut decoded).ok()?;
                if decoded.len() as u64 > MAX_DECODED_BYTES {
                    return None;
                }
            }
            // A frame of other data: its header is read, its content is passed over.
            Err(FrameDecoderError::ReadFrameHeaderError(ReadFrameHeaderError::SkipFrame {
                length,
                ..
            })) => body = body.get(length as usize..)?,
            Err(_) => return None,
        }
    }
    Some(decoded)
}

fn bounded(reader: impl Read) -> Option<Vec<u8>> {
    let mut decoded = Vec::new();
    reader
        .take(MAX_DECODED_BYTES + 1)
        .read_to_end(&mut decoded)
        .ok()?;
    (decoded.len() as u64 <= MAX_DECODED_BYTES).then_some(decoded)
}

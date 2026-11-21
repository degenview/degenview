//! Compressed frames: how a session reads a binary message that is text underneath.

use flate2::read::GzDecoder;
use std::io::{self, Read};
use std::string::FromUtf8Error;

/// The most a message may unpack to. Venues' messages are a few kilobytes; a compressed frame that
/// unpacks to more is refused, so a bad or hostile peer cannot fill memory.
const MAX_DECODED_BYTES: u64 = 8 * 1024 * 1024;

/// What a session does with the binary messages it reads.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Decode {
    /// Binary messages are handed over as they are (MEXC's Protocol Buffers).
    #[default]
    None,
    /// Binary messages are gzip-compressed text (HTX, BingX): they are unpacked and handed over as
    /// text messages, so heartbeats and the venue's parser see plain JSON.
    #[cfg_attr(not(test), allow(dead_code))] // until HTX or BingX lands
    Gzip,
}

/// Why a binary message could not be read as the session's text.
#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("the message is not gzip: {0}")]
    Gzip(#[from] io::Error),
    #[error("the message unpacks to more than {MAX_DECODED_BYTES} bytes")]
    TooLarge,
    #[error("the unpacked message is not text: {0}")]
    NotText(#[from] FromUtf8Error),
}

impl Decode {
    /// The text a binary message holds; `None` when this session leaves binary messages alone.
    pub fn text(self, bytes: &[u8]) -> Option<Result<String, DecodeError>> {
        match self {
            Self::None => None,
            Self::Gzip => Some(gunzip(bytes)),
        }
    }
}

fn gunzip(bytes: &[u8]) -> Result<String, DecodeError> {
    let mut unpacked = Vec::new();
    // One byte past the limit tells "exactly the limit" from "more".
    GzDecoder::new(bytes)
        .take(MAX_DECODED_BYTES + 1)
        .read_to_end(&mut unpacked)?;
    if unpacked.len() as u64 > MAX_DECODED_BYTES {
        return Err(DecodeError::TooLarge);
    }
    Ok(String::from_utf8(unpacked)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compression, write::GzEncoder};
    use std::io::Write;

    fn gzip(text: &[u8]) -> Vec<u8> {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
        encoder.write_all(text).unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn a_gzip_session_unpacks_binary_messages_to_text() {
        let packed = gzip(br#"{"ch":"market.btcusdt.kline.1min"}"#);
        let text = Decode::Gzip.text(&packed).unwrap().unwrap();
        assert_eq!(text, r#"{"ch":"market.btcusdt.kline.1min"}"#);
    }

    #[test]
    fn a_plain_session_leaves_binary_messages_alone() {
        assert!(Decode::None.text(&gzip(b"Ping")).is_none());
        assert_eq!(Decode::default(), Decode::None);
    }

    #[test]
    fn bytes_that_are_not_gzip_are_an_error_not_a_panic() {
        assert!(matches!(
            Decode::Gzip.text(b"not gzip at all").unwrap(),
            Err(DecodeError::Gzip(_))
        ));
        assert!(matches!(
            Decode::Gzip.text(&[]).unwrap(),
            Err(DecodeError::Gzip(_))
        ));
        // A truncated stream is no better.
        let packed = gzip(&vec![b'x'; 4096]);
        assert!(
            Decode::Gzip
                .text(&packed[..packed.len() / 2])
                .unwrap()
                .is_err()
        );
    }

    #[test]
    fn unpacked_bytes_that_are_not_utf8_are_an_error() {
        assert!(matches!(
            Decode::Gzip.text(&gzip(&[0xff, 0xfe])).unwrap(),
            Err(DecodeError::NotText(_))
        ));
    }

    #[test]
    fn a_message_that_unpacks_past_the_limit_is_refused() {
        let at_limit = gzip(&vec![b'a'; MAX_DECODED_BYTES as usize]);
        assert_eq!(
            Decode::Gzip.text(&at_limit).unwrap().unwrap().len() as u64,
            MAX_DECODED_BYTES
        );
        let past = gzip(&vec![b'a'; MAX_DECODED_BYTES as usize + 1]);
        assert!(matches!(
            Decode::Gzip.text(&past).unwrap(),
            Err(DecodeError::TooLarge)
        ));
    }
}

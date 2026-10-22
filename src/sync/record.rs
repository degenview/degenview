//! The wire envelope for one synced entity. A record's name is deterministic from the entity's
//! identity (`<kind>.<id>`), so uploading twice or downloading twice always lands on the same
//! logical object. The payload is opaque UTF-8 JSON here; `validate` decides whether it is safe.

use super::policy::EntityKind;
use sha2::{Digest, Sha256};

/// Bumped when a payload changes shape in a way an older app cannot read. Records with a higher
/// version are preserved untouched and applied after the user updates.
pub const SCHEMA_VERSION: u32 = 1;

/// CloudKit caps record names at 255 characters.
pub const MAX_RECORD_NAME: usize = 255;

/// Inline payload ceiling. CloudKit allows about 1 MB per record; the rest is headroom for the
/// plaintext fields and encryption overhead.
pub const MAX_PAYLOAD_BYTES: usize = 900_000;

/// The most a payload may hold once unpacked. A paper-trading history that does not fit one record
/// as written is deflated; this bounds what a record can expand to, so a small one cannot be made
/// to unpack into something huge.
pub const MAX_RAW_PAYLOAD_BYTES: usize = 24_000_000;

/// Starts a payload that is deflated. A payload is a JSON object and starts with `{`, so the two
/// cannot be mistaken for each other.
const DEFLATED: &[u8] = b"DVZ1";

/// The bytes to put in a record for `raw`: the payload itself when it fits, otherwise deflated
/// behind a marker. `None` when it does not fit even deflated (or is absurdly large to begin with).
pub fn to_wire(raw: &[u8]) -> Option<Vec<u8>> {
    use std::io::Write;
    if raw.len() <= MAX_PAYLOAD_BYTES {
        return Some(raw.to_vec());
    }
    if raw.len() > MAX_RAW_PAYLOAD_BYTES {
        return None;
    }
    let mut encoder =
        flate2::write::ZlibEncoder::new(DEFLATED.to_vec(), flate2::Compression::default());
    encoder.write_all(raw).ok()?;
    let packed = encoder.finish().ok()?;
    (packed.len() <= MAX_PAYLOAD_BYTES).then_some(packed)
}

/// The payload a record carries, unpacked if it was deflated. `None` for a damaged or oversize one.
pub fn from_wire(wire: Vec<u8>) -> Option<Vec<u8>> {
    use std::io::Read;
    let Some(body) = wire.strip_prefix(DEFLATED) else {
        return Some(wire);
    };
    let mut raw = Vec::new();
    flate2::read::ZlibDecoder::new(body)
        .take(MAX_RAW_PAYLOAD_BYTES as u64 + 1)
        .read_to_end(&mut raw)
        .ok()?;
    (raw.len() <= MAX_RAW_PAYLOAD_BYTES).then_some(raw)
}

/// Special ids that name a singleton rather than a database row.
pub const FAVORITES_ID: &str = "favorites";

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RecordName {
    pub kind: EntityKind,
    pub id: String,
}

impl RecordName {
    /// `None` when the id cannot form a legal record name.
    pub fn new(kind: EntityKind, id: impl Into<String>) -> Option<Self> {
        let name = Self {
            kind,
            id: id.into(),
        };
        name.is_legal().then_some(name)
    }

    pub fn layout_drawing(layout: u64, chart: u64, drawing: u64) -> Self {
        Self {
            kind: EntityKind::LayoutDrawing,
            id: format!("{layout}.{chart}.{drawing}"),
        }
    }

    pub fn render(&self) -> String {
        format!("{}.{}", self.kind.prefix(), self.id)
    }

    /// Splits at the first `.`; the id may itself contain dots (setting keys, composite ids).
    pub fn parse(text: &str) -> Option<Self> {
        let (prefix, id) = text.split_once('.')?;
        Self::new(EntityKind::from_prefix(prefix)?, id)
    }

    /// `(layout, chart, drawing)` of a `LayoutDrawing` name.
    pub fn layout_drawing_parts(&self) -> Option<(u64, u64, u64)> {
        if self.kind != EntityKind::LayoutDrawing {
            return None;
        }
        let mut parts = self.id.split('.');
        let parsed = (
            parts.next()?.parse().ok()?,
            parts.next()?.parse().ok()?,
            parts.next()?.parse().ok()?,
        );
        parts.next().is_none().then_some(parsed)
    }

    fn is_legal(&self) -> bool {
        !self.id.is_empty()
            && self.render().len() <= MAX_RECORD_NAME
            && self
                .id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    }
}

/// One entity as it travels to and from CloudKit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncRecord {
    pub name: RecordName,
    pub schema_version: u32,
    /// Increases with every accepted edit of this entity.
    pub revision: u64,
    pub origin_device: String,
    pub modified_ms: i64,
    pub payload: Vec<u8>,
}

impl SyncRecord {
    pub fn payload_sha256(&self) -> String {
        sha256_hex(&self.payload)
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{DEFLATED, MAX_PAYLOAD_BYTES, MAX_RAW_PAYLOAD_BYTES, from_wire, to_wire};

    /// JSON that looks like a long trading history: lots of near-identical rows.
    fn repetitive(bytes: usize) -> Vec<u8> {
        let row = r#"{"id":1,"account_id":7,"side":"buy","quantity":1.0,"price":100.0,"timestamp":1700000000000},"#;
        let mut out = String::from("{\"fills\":[");
        while out.len() < bytes {
            out.push_str(row);
        }
        out.push_str("0]}");
        out.into_bytes()
    }

    /// Bytes that do not compress: a simple xorshift stream, hex encoded (still JSON-safe).
    fn incompressible(bytes: usize) -> Vec<u8> {
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        let mut out = String::from("{\"noise\":\"");
        while out.len() < bytes {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            out.push_str(&format!("{state:016x}"));
        }
        out.push_str("\"}");
        out.into_bytes()
    }

    #[test]
    fn a_payload_that_fits_is_sent_as_it_is() {
        let raw = repetitive(MAX_PAYLOAD_BYTES / 2);
        assert_eq!(to_wire(&raw).as_deref(), Some(raw.as_slice()));
        assert_eq!(from_wire(raw.clone()), Some(raw));
    }

    #[test]
    fn an_oversize_payload_is_deflated_and_comes_back_unchanged() {
        let raw = repetitive(MAX_PAYLOAD_BYTES * 4);
        let wire = to_wire(&raw).expect("fits once deflated");
        assert!(wire.starts_with(DEFLATED));
        assert!(wire.len() <= MAX_PAYLOAD_BYTES, "{} bytes", wire.len());
        assert!(
            wire.len() * 5 < raw.len(),
            "repetitive history shrinks a lot"
        );
        assert_eq!(from_wire(wire), Some(raw));
    }

    #[test]
    fn a_payload_too_big_even_deflated_is_refused() {
        assert!(to_wire(&incompressible(MAX_PAYLOAD_BYTES * 3)).is_none());
        assert!(to_wire(&repetitive(MAX_RAW_PAYLOAD_BYTES + 1)).is_none());
    }

    #[test]
    fn a_damaged_or_bomb_payload_is_refused_not_unpacked() {
        let mut damaged = to_wire(&repetitive(MAX_PAYLOAD_BYTES * 2)).unwrap();
        let middle = damaged.len() / 2;
        damaged[middle] ^= 0xFF;
        damaged.truncate(middle + 10);
        assert!(from_wire(damaged).is_none());
        // A small record that would unpack to more than the ceiling.
        let too_big = {
            use std::io::Write;
            let mut encoder =
                flate2::write::ZlibEncoder::new(DEFLATED.to_vec(), flate2::Compression::default());
            encoder
                .write_all(&vec![b' '; MAX_RAW_PAYLOAD_BYTES + 1000])
                .unwrap();
            encoder.finish().unwrap()
        };
        assert!(too_big.len() < 100_000, "the bomb is small on the wire");
        assert!(from_wire(too_big).is_none());
    }

    use super::*;

    #[test]
    fn names_round_trip_for_every_kind() {
        for kind in EntityKind::ALL {
            let name = RecordName::new(kind, "12345").unwrap();
            assert_eq!(RecordName::parse(&name.render()), Some(name));
        }
    }

    #[test]
    fn ids_may_contain_dots() {
        let setting = RecordName::new(EntityKind::Setting, "drawing.defaults").unwrap();
        assert_eq!(setting.render(), "setting.drawing.defaults");
        assert_eq!(RecordName::parse("setting.drawing.defaults"), Some(setting));
    }

    #[test]
    fn composite_drawing_ids_decompose() {
        let name = RecordName::layout_drawing(10, 2, 7);
        assert_eq!(name.render(), "layoutdrawing.10.2.7");
        assert_eq!(name.layout_drawing_parts(), Some((10, 2, 7)));
        let parsed = RecordName::parse("layoutdrawing.10.2.7").unwrap();
        assert_eq!(parsed.layout_drawing_parts(), Some((10, 2, 7)));
    }

    #[test]
    fn malformed_composite_ids_do_not_decompose() {
        for id in ["1.2", "1.2.3.4", "a.b.c", "1.2.x"] {
            let name = RecordName::new(EntityKind::LayoutDrawing, id).unwrap();
            assert_eq!(name.layout_drawing_parts(), None, "{id}");
        }
        let other = RecordName::new(EntityKind::Layout, "1.2.3").unwrap();
        assert_eq!(other.layout_drawing_parts(), None);
    }

    #[test]
    fn illegal_names_are_rejected() {
        assert_eq!(RecordName::parse("layout"), None);
        assert_eq!(RecordName::parse("layout."), None);
        assert_eq!(RecordName::parse("tab.1"), None);
        assert_eq!(RecordName::parse("layout.a b"), None);
        assert_eq!(RecordName::parse("layout.é"), None);
        assert_eq!(RecordName::parse("layout.a/b"), None);
        assert!(RecordName::new(EntityKind::Layout, "x".repeat(300)).is_none());
        assert!(RecordName::new(EntityKind::Layout, "").is_none());
    }

    #[test]
    fn the_longest_legal_name_fits() {
        let id = "a".repeat(MAX_RECORD_NAME - "layout.".len());
        assert!(RecordName::new(EntityKind::Layout, id).is_some());
        let id = "a".repeat(MAX_RECORD_NAME - "layout.".len() + 1);
        assert!(RecordName::new(EntityKind::Layout, id).is_none());
    }

    #[test]
    fn payload_hash_is_stable_sha256() {
        let record = SyncRecord {
            name: RecordName::new(EntityKind::Layout, "1").unwrap(),
            schema_version: SCHEMA_VERSION,
            revision: 1,
            origin_device: "d".into(),
            modified_ms: 0,
            payload: b"abc".to_vec(),
        };
        assert_eq!(
            record.payload_sha256(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}

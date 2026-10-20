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

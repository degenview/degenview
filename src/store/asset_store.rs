//! The pictures drawings show, kept once each by the hash of their bytes.
//!
//! A drawing refers to its picture by that hash and never carries the bytes, so a layout, an undo
//! step and a synced drawing record stay small, and a picture used by ten drawings is stored
//! once. A row is `id` (the hash), and a JSON payload with the picture's type, size and bytes
//! (base64): the one shape the store keeps and the sync sends, so a Mac that receives it can
//! check it is what its name says.
//!
//! Writes add: a stored picture is never replaced or deleted by anything here, because a drawing
//! on another Mac, in a layout not synced yet, or on the undo stack may still show it. Rows are
//! read and written one at a time by id, never as a collection, so a store of many pictures costs
//! nothing until one is looked at.

use super::Store;
use crate::service::image_ingest::{self, Ingested, MAX_STORED_BYTES};
use crate::sync::native::{base64_decode, base64_encode};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};

pub const TABLE: &str = "drawing_asset";

/// The most side a stored picture may have: what ingest makes is far below it, and a record from
/// another Mac is held to it too.
pub const MAX_STORED_SIDE: u32 = 4_096;

/// A picture as it is stored and as it syncs.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StoredAsset {
    /// The hash of the picture's bytes, in lower-case hex.
    pub id: String,
    pub mime: String,
    pub width: u32,
    pub height: u32,
    /// The bytes, base64.
    pub data: String,
}

impl StoredAsset {
    pub fn from_ingested(picture: &Ingested) -> Self {
        Self {
            id: picture.id.clone(),
            mime: picture.mime.to_owned(),
            width: picture.width,
            height: picture.height,
            data: base64_encode(&picture.bytes),
        }
    }

    /// The picture's bytes, only when they are what this asset says: a PNG or a JPEG of the type
    /// and size it declares, within the limits, and hashing to its own name. Anything else
    /// (damage, a record from a stranger) gives `None`.
    pub fn verified_bytes(&self) -> Option<Vec<u8>> {
        if self.id.len() != 64
            || !self
                .id
                .bytes()
                .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        {
            return None;
        }
        if self.data.len() > MAX_STORED_BYTES.div_ceil(3) * 4 + 8 {
            return None;
        }
        let bytes = base64_decode(&self.data)?;
        if bytes.is_empty() || bytes.len() > MAX_STORED_BYTES {
            return None;
        }
        if image_ingest::hash(&bytes) != self.id {
            return None;
        }
        let (mime, width, height) = image_ingest::describe(&bytes)?;
        let sane =
            (1..=MAX_STORED_SIDE).contains(&width) && (1..=MAX_STORED_SIDE).contains(&height);
        (mime == self.mime && width == self.width && height == self.height && sane).then_some(bytes)
    }
}

impl Store {
    /// Keeps a picture. A picture already kept is left as it is. With sync on, a new one is
    /// queued for upload in the same step, on its own: nothing else in the table is read or
    /// rewritten. `true` when it was new.
    pub fn put_asset(&self, asset: &StoredAsset) -> rusqlite::Result<bool> {
        let payload = serde_json::to_string(asset)
            .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
        self.connection.execute_batch("SAVEPOINT put_asset")?;
        let result = (|| {
            let inserted = self.connection.execute(
                &format!(
                    "INSERT OR IGNORE INTO {TABLE} (id, position, payload) VALUES (?1, 0, ?2)"
                ),
                rusqlite::params![asset.id, payload],
            )? > 0;
            if inserted && self.sync_enabled() {
                self.track_documents(TABLE, &[], &[(asset.id.clone(), 0, payload)])?;
            }
            Ok(inserted)
        })();
        if result.is_err() {
            self.connection.execute_batch("ROLLBACK TO put_asset")?;
        }
        self.connection.execute_batch("RELEASE put_asset")?;
        result
    }

    /// Keeps a picture that came from the cloud: the same row, with nothing queued, since the
    /// cloud already has it. `true` when it was new here.
    pub fn put_asset_from_cloud(&self, asset: &StoredAsset) -> rusqlite::Result<bool> {
        let payload = serde_json::to_string(asset)
            .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
        Ok(self.connection.execute(
            &format!("INSERT OR IGNORE INTO {TABLE} (id, position, payload) VALUES (?1, 0, ?2)"),
            rusqlite::params![asset.id, payload],
        )? > 0)
    }

    /// The picture named `id`, if it is kept and decodes.
    pub fn asset(&self, id: &str) -> rusqlite::Result<Option<StoredAsset>> {
        let payload: Option<String> = self
            .connection
            .query_row(
                &format!("SELECT payload FROM {TABLE} WHERE id = ?1"),
                [id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(payload.and_then(|payload| serde_json::from_str(&payload).ok()))
    }

    /// Whether a picture is kept, without reading its bytes.
    pub fn has_asset(&self, id: &str) -> rusqlite::Result<bool> {
        Ok(self
            .connection
            .query_row(
                &format!("SELECT 1 FROM {TABLE} WHERE id = ?1"),
                [id],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    /// The ids of every kept picture. Reads no bytes.
    pub fn asset_ids(&self) -> rusqlite::Result<Vec<String>> {
        let mut statement = self
            .connection
            .prepare(&format!("SELECT id FROM {TABLE}"))?;
        statement
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture(seed: u8) -> Ingested {
        let image = image::RgbaImage::from_fn(30, 20, |x, y| {
            image::Rgba([seed, (x * 8) as u8, (y * 12) as u8, 255])
        });
        let mut png = Vec::new();
        image::ImageEncoder::write_image(
            image::codecs::png::PngEncoder::new(&mut png),
            image.as_raw(),
            30,
            20,
            image::ExtendedColorType::Rgba8,
        )
        .unwrap();
        image_ingest::ingest(&png).unwrap()
    }

    #[test]
    fn a_picture_is_kept_once_and_read_back_whole() {
        let store = Store::open_in_memory().unwrap();
        let kept = StoredAsset::from_ingested(&picture(1));
        assert!(store.put_asset(&kept).unwrap());
        assert!(
            !store.put_asset(&kept).unwrap(),
            "the second is the same picture"
        );
        assert_eq!(store.asset(&kept.id).unwrap(), Some(kept.clone()));
        assert!(store.has_asset(&kept.id).unwrap());
        assert_eq!(store.asset_ids().unwrap(), std::slice::from_ref(&kept.id));
        assert!(store.asset("0".repeat(64).as_str()).unwrap().is_none());
        assert!(!store.has_asset("nothing").unwrap());
        assert!(kept.verified_bytes().is_some());
    }

    #[test]
    fn the_same_bytes_from_the_cloud_do_not_replace_what_is_kept() {
        let store = Store::open_in_memory().unwrap();
        let kept = StoredAsset::from_ingested(&picture(2));
        assert!(store.put_asset_from_cloud(&kept).unwrap());
        assert!(!store.put_asset_from_cloud(&kept).unwrap());
        assert_eq!(store.asset_ids().unwrap().len(), 1);
    }

    #[test]
    fn a_stored_asset_is_trusted_only_when_it_is_what_it_says() {
        let good = StoredAsset::from_ingested(&picture(3));
        assert!(good.verified_bytes().is_some());
        let mut bad = good.clone();
        bad.id = "a".repeat(64);
        assert!(
            bad.verified_bytes().is_none(),
            "bytes that hash to another name"
        );
        let mut bad = good.clone();
        bad.id = good.id.to_uppercase();
        assert!(bad.verified_bytes().is_none(), "ids are lower case");
        let mut bad = good.clone();
        bad.mime = "image/png".into();
        assert!(bad.verified_bytes().is_none(), "the wrong type");
        let mut bad = good.clone();
        bad.width += 1;
        assert!(bad.verified_bytes().is_none(), "the wrong size");
        let mut bad = good.clone();
        bad.data = "AAAA".into();
        assert!(bad.verified_bytes().is_none(), "not a picture");
        let mut bad = good.clone();
        bad.data = "!!!!".into();
        assert!(bad.verified_bytes().is_none(), "not base64");
        let mut bad = good.clone();
        bad.id = "short".into();
        assert!(bad.verified_bytes().is_none());
        let mut bad = good.clone();
        bad.data = "A".repeat(2_000_000);
        assert!(bad.verified_bytes().is_none(), "past the size limit");
    }
}

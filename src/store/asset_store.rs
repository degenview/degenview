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
use crate::model::annotation::asset::StoredAsset;
use rusqlite::OptionalExtension;

pub const TABLE: &str = "drawing_asset";

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
    use crate::service::annotation::ingest::{self, Ingested};

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
        ingest::ingest(&png).unwrap()
    }

    #[test]
    fn a_picture_is_kept_once_and_read_back_whole() {
        let store = Store::open_in_memory().unwrap();
        let kept = picture(1).stored();
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
        assert!(ingest::verified_bytes(&kept).is_some());
    }

    #[test]
    fn the_same_bytes_from_the_cloud_do_not_replace_what_is_kept() {
        let store = Store::open_in_memory().unwrap();
        let kept = picture(2).stored();
        assert!(store.put_asset_from_cloud(&kept).unwrap());
        assert!(!store.put_asset_from_cloud(&kept).unwrap());
        assert_eq!(store.asset_ids().unwrap().len(), 1);
    }
}

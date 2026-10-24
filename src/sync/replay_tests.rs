//! Records an earlier launch had to set aside (a kind it did not know, a schema newer than it) are
//! looked at again by a launch that may read them, and what is still unreadable is left alone.

use super::apply::{NEWER_SCHEMA, UNKNOWN_TYPE};
use super::apply_tests::{pair, settle};
use super::core_tests::Mac;
use super::mock::name_of;
use super::policy::EntityKind;
use super::record::SCHEMA_VERSION;
use crate::service::image_ingest;
use crate::store::asset_store::StoredAsset;
use crate::store::sync::InboxRecord;

fn picture(seed: u8) -> StoredAsset {
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
    StoredAsset::from_ingested(&image_ingest::ingest(&png).unwrap())
}

/// A record as an inbox keeps it.
fn set_aside(name: &str, reason: &str, schema_version: u32, payload: Vec<u8>) -> InboxRecord {
    InboxRecord {
        record_name: name.to_owned(),
        entity_type: name.split('.').next().unwrap_or_default().to_owned(),
        schema_version,
        reason: reason.to_owned(),
        payload,
        system_fields: None,
        revision: 1,
        origin_device: "Mac C".into(),
        modified_ms: 5,
    }
}

fn waiting(mac: &Mac) -> Vec<(String, String)> {
    mac.store
        .lock()
        .unwrap()
        .inbox()
        .unwrap()
        .into_iter()
        .map(|entry| (entry.record_name, entry.reason))
        .collect()
}

#[test]
fn a_record_set_aside_as_an_unknown_kind_is_applied_once_a_launch_knows_the_kind() {
    let (_cloud, mut a, mut b) = pair();
    a.core.start();
    let kept = picture(1);
    let name = name_of(EntityKind::DrawingAsset, &kept.id);
    // What an older build left behind: the record, set aside as a kind it did not know.
    b.store
        .lock()
        .unwrap()
        .put_inbox(
            &set_aside(&name, UNKNOWN_TYPE, 1, serde_json::to_vec(&kept).unwrap()),
            1,
        )
        .unwrap();
    assert!(!b.store.lock().unwrap().has_asset(&kept.id).unwrap());
    b.core.start();
    settle(&mut a, &mut b);
    assert!(b.store.lock().unwrap().has_asset(&kept.id).unwrap());
    assert_eq!(waiting(&b), Vec::new(), "nothing is left waiting");
}

#[test]
fn a_record_set_aside_as_newer_is_applied_once_this_launch_reads_that_schema() {
    let (_cloud, mut a, mut b) = pair();
    a.core.start();
    let kept = picture(2);
    let name = name_of(EntityKind::DrawingAsset, &kept.id);
    // Written by a build with a schema this one now reads (the record's own version is not above it).
    b.store
        .lock()
        .unwrap()
        .put_inbox(
            &set_aside(
                &name,
                NEWER_SCHEMA,
                SCHEMA_VERSION,
                serde_json::to_vec(&kept).unwrap(),
            ),
            1,
        )
        .unwrap();
    b.core.start();
    settle(&mut a, &mut b);
    assert!(b.store.lock().unwrap().has_asset(&kept.id).unwrap());
    assert!(waiting(&b).is_empty());
}

#[test]
fn what_this_launch_still_cannot_read_stays_set_aside_untouched() {
    let (_cloud, mut a, mut b) = pair();
    a.core.start();
    let kept = picture(3);
    let known = name_of(EntityKind::DrawingAsset, &kept.id);
    let unknown_kind = b"{\"x\":1}".to_vec();
    let newer = set_aside(
        &known,
        NEWER_SCHEMA,
        SCHEMA_VERSION + 1,
        serde_json::to_vec(&kept).unwrap(),
    );
    {
        let store = b.store.lock().unwrap();
        store
            .put_inbox(
                &set_aside("hologram.3", UNKNOWN_TYPE, 1, unknown_kind.clone()),
                1,
            )
            .unwrap();
        store.put_inbox(&newer, 1).unwrap();
    }
    b.core.start();
    settle(&mut a, &mut b);
    assert!(!b.store.lock().unwrap().has_asset(&kept.id).unwrap());
    let kept_entries = b.store.lock().unwrap().inbox().unwrap();
    assert_eq!(kept_entries.len(), 2);
    let hologram = kept_entries
        .iter()
        .find(|entry| entry.record_name == "hologram.3")
        .unwrap();
    assert_eq!(hologram.payload, unknown_kind, "kept byte for byte");
    let still_newer = kept_entries
        .iter()
        .find(|entry| entry.record_name == known)
        .unwrap();
    assert_eq!(still_newer.schema_version, SCHEMA_VERSION + 1);
    assert_eq!(still_newer.payload, newer.payload);
}

#[test]
fn a_launch_with_nothing_set_aside_asks_for_no_extra_apply() {
    let (_cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    let effects = b.core.take_effects();
    assert!(
        effects
            .iter()
            .all(|effect| !matches!(effect, super::core::Effect::RemoteChanges(_))),
        "{effects:?}"
    );
    settle(&mut a, &mut b);
}

//! Pictures between two Macs: they travel whole, once, and never come back as a deletion.

use super::apply_tests::{outbox_len, pair, settle};
use super::mock::name_of;
use super::policy::EntityKind;
use crate::service::image_ingest::{self, Ingested};
use crate::store::asset_store::StoredAsset;

fn picture(seed: u8) -> StoredAsset {
    let image = image::RgbaImage::from_fn(36, 24, |x, y| {
        image::Rgba([seed, (x * 7) as u8, (y * 10) as u8, 255])
    });
    let mut png = Vec::new();
    image::ImageEncoder::write_image(
        image::codecs::png::PngEncoder::new(&mut png),
        image.as_raw(),
        36,
        24,
        image::ExtendedColorType::Rgba8,
    )
    .unwrap();
    let kept: Ingested = image_ingest::ingest(&png).unwrap();
    StoredAsset::from_ingested(&kept)
}

#[test]
fn a_picture_reaches_the_other_mac_whole_and_does_not_echo_back() {
    let (cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    let kept = picture(1);
    assert!(a.store.lock().unwrap().put_asset(&kept).unwrap());
    settle(&mut a, &mut b);
    assert_eq!(cloud.names(), [name_of(EntityKind::DrawingAsset, &kept.id)]);
    let on_b = b.store.lock().unwrap().asset(&kept.id).unwrap();
    assert_eq!(on_b.as_ref(), Some(&kept));
    assert!(
        on_b.unwrap().verified_bytes().is_some(),
        "the bytes are the picture"
    );
    assert_eq!(
        outbox_len(&a) + outbox_len(&b),
        0,
        "applying queues nothing"
    );
    assert_eq!(cloud.save_attempts(), 1);
}

#[test]
fn a_second_picture_uploads_alone_and_the_same_one_again_uploads_nothing() {
    let (cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    let (first, second) = (picture(1), picture(2));
    a.store.lock().unwrap().put_asset(&first).unwrap();
    settle(&mut a, &mut b);
    let before = cloud.save_attempts();
    assert!(a.store.lock().unwrap().put_asset(&second).unwrap());
    settle(&mut a, &mut b);
    assert_eq!(
        cloud.save_attempts(),
        before + 1,
        "only the new picture went"
    );
    // The same picture again is the same asset: nothing is stored or queued.
    assert!(!a.store.lock().unwrap().put_asset(&second).unwrap());
    assert_eq!(outbox_len(&a), 0);
    settle(&mut a, &mut b);
    assert_eq!(cloud.save_attempts(), before + 1);
    assert_eq!(b.store.lock().unwrap().asset_ids().unwrap().len(), 2);
}

#[test]
fn a_picture_the_cloud_is_told_to_delete_stays_and_goes_back_up() {
    let (cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    let kept = picture(3);
    a.store.lock().unwrap().put_asset(&kept).unwrap();
    settle(&mut a, &mut b);
    let name = name_of(EntityKind::DrawingAsset, &kept.id);
    cloud.delete_from_elsewhere(&name, "Mac C");
    settle(&mut a, &mut b);
    // Nothing deletes a picture: a drawing somewhere may still show it. Both Macs keep it, and
    // the cloud holds it again.
    assert!(a.store.lock().unwrap().has_asset(&kept.id).unwrap());
    assert!(b.store.lock().unwrap().has_asset(&kept.id).unwrap());
    assert!(cloud.names().contains(&name), "{:?}", cloud.names());
    assert_eq!(outbox_len(&a) + outbox_len(&b), 0);
}

#[test]
fn a_picture_this_mac_holds_that_the_cloud_lacks_is_found_by_its_id_alone() {
    let (cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    let kept = picture(4);
    // Stored without the write hook (as an older build, or before sync was on, would have).
    assert!(a.store.lock().unwrap().put_asset_from_cloud(&kept).unwrap());
    assert_eq!(outbox_len(&a), 0);
    let found = a.store.lock().unwrap().reconcile_stored().unwrap();
    assert_eq!(found.upserts, 1);
    assert_eq!(found.deletes, 0);
    settle(&mut a, &mut b);
    assert_eq!(b.store.lock().unwrap().asset(&kept.id).unwrap(), Some(kept));
    assert_eq!(cloud.save_attempts(), 1);
    // And once the cloud has it, looking again finds nothing to do.
    let again = a.store.lock().unwrap().reconcile_stored().unwrap();
    assert_eq!((again.upserts, again.deletes), (0, 0));
}

#[test]
fn a_picture_never_counts_as_deleted_when_a_layout_stops_using_it() {
    let (cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    let kept = picture(5);
    a.store.lock().unwrap().put_asset(&kept).unwrap();
    settle(&mut a, &mut b);
    // A reconcile with no layout naming the picture must not queue its deletion.
    let found = a.store.lock().unwrap().reconcile_stored().unwrap();
    assert_eq!(found.deletes, 0);
    settle(&mut a, &mut b);
    assert_eq!(cloud.delete_attempts(), 0);
    assert!(b.store.lock().unwrap().has_asset(&kept.id).unwrap());
}

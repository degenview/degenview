//! Pictures between two Macs: they travel whole, once, and never come back as a deletion.

use super::apply_tests::{layout_of, outbox_len, pair, put_layouts, rows, settle};
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

#[test]
fn every_text_note_and_content_tool_reaches_the_other_mac_exactly_as_drawn() {
    use crate::model::{Anchor, Drawing, Geometry, Placement, Tool, ToolGroup};
    let (cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    let anchors: Vec<Anchor> = (0..3)
        .map(|index| Anchor {
            time_ms: 1_700_000_000_000 + index * 3_600_000,
            price: 100.0 + index as f64 * 7.5,
        })
        .collect();
    let mut drawings: Vec<Drawing> = ToolGroup::NOTE_MENU
        .into_iter()
        .flat_map(Tool::in_group)
        .enumerate()
        .map(|(index, tool)| {
            let count = match tool.spec().placement {
                Placement::Points(count) => usize::from(count),
                _ => 1,
            };
            let mut drawing = Drawing::sample(
                tool,
                index as u64 + 1,
                &anchors[..count],
                tool.default_style(),
            )
            .unwrap_or_else(|| panic!("{tool:?}"));
            if tool.caps().has(crate::model::Caps::TEXT) {
                drawing.set_text("a note\nin two lines");
            }
            drawing
        })
        .collect();
    assert_eq!(drawings.len(), 13);
    for drawing in &mut drawings {
        match drawing.geometry_mut() {
            Some(Geometry::Table(_)) => {
                drawing.set_table_cell(1, 2, "a cell");
            }
            Some(Geometry::Card { card, .. }) => {
                card.meta = Some(crate::model::link_card::LinkMeta {
                    title: "A title".into(),
                    author: "@someone".into(),
                    excerpt: "A few words.".into(),
                    fetched_at_ms: 9,
                });
            }
            _ => {}
        }
    }
    put_layouts(&a, &[layout_of(10, "Notes", &drawings)]);
    settle(&mut a, &mut b);
    // The layout and one record per drawing; a picture is not part of a drawing's record.
    assert_eq!(
        cloud.names().len(),
        1 + drawings.len(),
        "{:?}",
        cloud.names()
    );
    let on_b = rows(&b, "saved_view");
    let received = on_b[0]["charts"][0]["drawings"].as_array().unwrap().clone();
    assert_eq!(received.len(), 13);
    for (sent, got) in drawings.iter().zip(&received) {
        assert_eq!(
            &serde_json::to_value(sent).unwrap(),
            got,
            "{:?}",
            sent.tool()
        );
        let back: Drawing = serde_json::from_value(got.clone()).unwrap();
        assert_eq!(&back, sent, "{:?}", sent.tool());
    }
    assert_eq!(outbox_len(&a) + outbox_len(&b), 0);
}

#[test]
fn a_drawing_that_arrives_before_its_picture_is_kept_and_the_picture_follows() {
    use crate::model::image_content::{Content, ImageContent};
    use crate::model::{Anchor, Drawing, Tool};
    let (_cloud, mut a, mut b) = pair();
    a.core.start();
    b.core.start();
    let kept = picture(3);
    let content = Content::Image(ImageContent {
        asset: kept.id.clone(),
        name: "levels".into(),
        pixels: [36, 24],
    });
    let at = |index: i64| Anchor {
        time_ms: 1_700_000_000_000 + index * 3_600_000,
        price: 100.0 + index as f64,
    };
    let drawing = Drawing::build_with(
        Tool::Image,
        1,
        &[at(0), at(5)],
        Tool::Image.default_style(),
        &content,
    )
    .unwrap();
    put_layouts(
        &a,
        &[layout_of(10, "Pictures", std::slice::from_ref(&drawing))],
    );
    settle(&mut a, &mut b);
    let on_b = rows(&b, "saved_view");
    assert_eq!(
        on_b[0]["charts"][0]["drawings"].as_array().unwrap().len(),
        1,
        "the drawing is there without its picture"
    );
    assert!(b.store.lock().unwrap().asset(&kept.id).unwrap().is_none());
    // The picture follows, and is then the very bytes the drawing names.
    a.store.lock().unwrap().put_asset(&kept).unwrap();
    settle(&mut a, &mut b);
    let arrived = b.store.lock().unwrap().asset(&kept.id).unwrap();
    assert_eq!(arrived.as_ref(), Some(&kept));
    assert!(arrived.unwrap().verified_bytes().is_some());
}

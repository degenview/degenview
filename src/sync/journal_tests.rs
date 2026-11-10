//! Trade Journal sync across two Macs: reviews, daily entries and playbooks through the real core,
//! apply layer and merge rules, on the in-memory CloudKit stand-in.

use super::core_tests::{Mac, mac};
use super::merge::OTHER_VERSION;
use super::mock::MockCloud;
use super::policy::Category;
use crate::model::journal::daily::DailyEntry;
use crate::model::journal::playbook::Playbook;
use crate::model::journal::reviews::{Review, review_row_id};

fn pair() -> (MockCloud, Mac, Mac) {
    let cloud = MockCloud::new();
    let mut a = mac(&cloud, "Mac A", "acct", Some("acct"));
    let mut b = mac(&cloud, "Mac B", "acct", Some("acct"));
    a.apply = true;
    b.apply = true;
    a.core.start();
    b.core.start();
    (cloud, a, b)
}

fn settle(a: &mut Mac, b: &mut Mac) {
    for _ in 0..6 {
        a.core.request_sync();
        a.run();
        b.core.request_sync();
        b.run();
    }
}

/// Edits are stamped with the wall clock: leave a moment so the later edit is later.
fn later() {
    std::thread::sleep(std::time::Duration::from_millis(5));
}

fn review(key: &str, notes: &str) -> Review {
    let mut review = Review::new(key, "paper:1");
    review.notes = notes.to_owned();
    review
}

fn save_review(mac: &Mac, review: &Review, now_ms: i64) -> Review {
    mac.store
        .lock()
        .unwrap()
        .journal_save_review(review, now_ms)
        .unwrap()
        .expect("stored")
}

fn reviews(mac: &Mac) -> Vec<Review> {
    mac.store.lock().unwrap().journal_reviews().unwrap()
}

fn outbox(a: &Mac, b: &Mac) -> usize {
    a.outbox().len() + b.outbox().len()
}

#[test]
fn a_review_written_on_one_mac_appears_on_the_other_without_echoing_back() {
    let (cloud, mut a, mut b) = pair();
    let mut r = review("paper:1:100:0", "Waited for the retest.");
    r.tags = vec!["retest".into()];
    r.rating = Some(4);
    r.images = vec!["a".repeat(64)];
    save_review(&a, &r, 1_000);
    settle(&mut a, &mut b);
    assert_eq!(
        cloud.names(),
        [format!("journalreview.{}", review_row_id("paper:1:100:0"))]
    );
    let on_b = reviews(&b);
    assert_eq!(on_b.len(), 1);
    assert_eq!(on_b[0].notes, "Waited for the retest.");
    assert_eq!(on_b[0].tags, ["retest"]);
    assert_eq!(on_b[0].images, ["a".repeat(64)]);
    assert_eq!(outbox(&a, &b), 0);
    // The row's own id travels with it and is what names it in the table.
    assert_eq!(reviews(&a), reviews(&b));
}

#[test]
fn editing_and_clearing_a_review_travel_and_clearing_deletes_it_everywhere() {
    let (_cloud, mut a, mut b) = pair();
    let first = save_review(&a, &review("k1", "one"), 1_000);
    settle(&mut a, &mut b);
    let mut edited = first.clone();
    edited.notes = "one, revised".into();
    save_review(&a, &edited, 2_000);
    settle(&mut a, &mut b);
    assert_eq!(reviews(&b)[0].notes, "one, revised");
    // Emptying it removes it, and the other Mac learns of the deletion.
    let mut emptied = Review::new("k1", "paper:1");
    emptied.updated_ms = 3_000;
    assert!(
        a.store
            .lock()
            .unwrap()
            .journal_save_review(&emptied, 3_000)
            .unwrap()
            .is_none()
    );
    settle(&mut a, &mut b);
    assert!(reviews(&a).is_empty());
    assert!(reviews(&b).is_empty());
    assert_eq!(outbox(&a, &b), 0);
}

#[test]
fn two_edits_of_one_review_keep_the_later_and_the_other_texts_after_it() {
    let (_cloud, mut a, mut b) = pair();
    let base = save_review(&a, &review("k1", "original"), 1_000);
    settle(&mut a, &mut b);
    let mut on_a = base.clone();
    on_a.notes = "Mac A thought this.".into();
    on_a.rating = Some(2);
    save_review(&a, &on_a, 2_000);
    later();
    let mut on_b = reviews(&b)[0].clone();
    on_b.notes = "Mac B thought that.".into();
    on_b.rating = Some(5);
    save_review(&b, &on_b, 3_000);
    settle(&mut a, &mut b);
    let (ra, rb) = (reviews(&a), reviews(&b));
    assert_eq!(ra, rb, "both Macs converge");
    assert_eq!(ra.len(), 1);
    // The later edit is the winner: its rating, and its notes first.
    assert_eq!(ra[0].rating, Some(5));
    assert!(
        ra[0].notes.starts_with("Mac B thought that."),
        "{}",
        ra[0].notes
    );
    // The other Mac's words are kept after a divider, not lost.
    assert!(ra[0].notes.contains(OTHER_VERSION));
    assert!(
        ra[0].notes.ends_with("Mac A thought this."),
        "{}",
        ra[0].notes
    );
    assert_eq!(outbox(&a, &b), 0);
    // Settling again changes nothing: the merge does not grow the text.
    let before = reviews(&a);
    settle(&mut a, &mut b);
    assert_eq!(reviews(&a), before);
}

#[test]
fn identical_words_written_on_both_macs_are_not_doubled() {
    let (_cloud, mut a, mut b) = pair();
    let base = save_review(&a, &review("k1", "same"), 1_000);
    settle(&mut a, &mut b);
    let mut x = base.clone();
    x.notes = "same, plus this".into();
    save_review(&a, &x, 2_000);
    let mut y = reviews(&b)[0].clone();
    y.notes = "same, plus this".into();
    y.tags = vec!["extra".into()];
    save_review(&b, &y, 3_000);
    settle(&mut a, &mut b);
    assert_eq!(reviews(&a), reviews(&b));
    assert_eq!(reviews(&a)[0].notes, "same, plus this");
}

#[test]
fn playbooks_and_daily_entries_sync_and_a_playbook_is_last_writer() {
    let (_cloud, mut a, mut b) = pair();
    let mut playbook = Playbook::new("pb1".into(), "Breakout");
    playbook.checklist = vec!["Volume".into(), "Above VWAP".into()];
    a.store
        .lock()
        .unwrap()
        .journal_save_playbook(&playbook, 1_000)
        .unwrap();
    let mut day = DailyEntry::new("2026-11-09");
    day.plan = "Open only.".into();
    a.store
        .lock()
        .unwrap()
        .journal_save_day(&day, 1_000)
        .unwrap();
    settle(&mut a, &mut b);
    let playbooks = b.store.lock().unwrap().journal_playbooks().unwrap();
    assert_eq!(playbooks.len(), 1);
    assert_eq!(playbooks[0].checklist, ["Volume", "Above VWAP"]);
    assert_eq!(
        b.store.lock().unwrap().journal_days().unwrap()[0].plan,
        "Open only."
    );

    // Renamed on both Macs: the later one wins whole, nothing is combined.
    let mut on_a = playbooks[0].clone();
    on_a.name = "Breakout (A)".into();
    a.store
        .lock()
        .unwrap()
        .journal_save_playbook(&on_a, 2_000)
        .unwrap();
    later();
    let mut on_b = playbooks[0].clone();
    on_b.name = "Breakout (B)".into();
    b.store
        .lock()
        .unwrap()
        .journal_save_playbook(&on_b, 3_000)
        .unwrap();
    settle(&mut a, &mut b);
    let names = |mac: &Mac| -> Vec<String> {
        mac.store
            .lock()
            .unwrap()
            .journal_playbooks()
            .unwrap()
            .into_iter()
            .map(|playbook| playbook.name)
            .collect()
    };
    assert_eq!(names(&a), names(&b));
    assert_eq!(names(&a), ["Breakout (B)"]);

    // A day's text from both Macs is kept.
    let mut day_a = a.store.lock().unwrap().journal_days().unwrap()[0].clone();
    day_a.lessons = "Wait for the retest.".into();
    a.store
        .lock()
        .unwrap()
        .journal_save_day(&day_a, 4_000)
        .unwrap();
    let mut day_b = b.store.lock().unwrap().journal_days().unwrap()[0].clone();
    day_b.lessons = "Size down on Fridays.".into();
    b.store
        .lock()
        .unwrap()
        .journal_save_day(&day_b, 5_000)
        .unwrap();
    settle(&mut a, &mut b);
    let (da, db) = (
        a.store.lock().unwrap().journal_days().unwrap(),
        b.store.lock().unwrap().journal_days().unwrap(),
    );
    assert_eq!(da, db);
    // One of the two is first and the other follows it: neither is lost.
    assert!(da[0].lessons.contains("Size down on Fridays."));
    assert!(da[0].lessons.contains("Wait for the retest."));
    assert!(da[0].lessons.contains(OTHER_VERSION));
    assert_eq!(outbox(&a, &b), 0);
}

#[test]
fn the_journal_switch_stops_uploads_and_downloads_until_it_is_turned_on() {
    let (cloud, mut a, mut b) = pair();
    a.store
        .lock()
        .unwrap()
        .set_category_enabled(Category::TradingJournal, false)
        .unwrap();
    save_review(&a, &review("k1", "private for now"), 1_000);
    settle(&mut a, &mut b);
    assert!(cloud.names().is_empty(), "nothing uploaded while it is off");
    assert!(reviews(&b).is_empty());

    // The other Mac turning it off keeps what arrives in the inbox, unapplied.
    a.store
        .lock()
        .unwrap()
        .set_category_enabled(Category::TradingJournal, true)
        .unwrap();
    // Turning a category on is followed by a reconcile, as the host does.
    a.store.lock().unwrap().reconcile_stored().unwrap();
    b.store
        .lock()
        .unwrap()
        .set_category_enabled(Category::TradingJournal, false)
        .unwrap();
    settle(&mut a, &mut b);
    assert_eq!(
        cloud.names(),
        [format!("journalreview.{}", review_row_id("k1"))]
    );
    assert!(reviews(&b).is_empty(), "held back while off");
    b.store
        .lock()
        .unwrap()
        .set_category_enabled(Category::TradingJournal, true)
        .unwrap();
    // As the host does: an empty apply retries what was held back.
    b.apply_to_store(&super::core::RemoteBatch {
        id: 901,
        records: vec![],
        deletions: vec![],
    });
    settle(&mut a, &mut b);
    assert_eq!(reviews(&b).len(), 1);
}

#[test]
fn a_review_for_a_trade_this_mac_does_not_have_yet_is_kept_not_dropped() {
    // A review syncs on its own: it does not need the trade (which is rebuilt from the paper
    // bundle that may arrive later) to exist here.
    let (_cloud, mut a, mut b) = pair();
    save_review(
        &a,
        &review("paper:999:1:0", "for a trade that is still on its way"),
        1_000,
    );
    settle(&mut a, &mut b);
    assert_eq!(reviews(&b)[0].trade_key, "paper:999:1:0");
}

#[test]
fn a_damaged_or_foreign_record_is_refused_and_never_applied() {
    use super::policy::EntityKind;
    use super::record::{RecordName, SCHEMA_VERSION, SyncRecord};
    use super::validate::{Verdict, validate};
    let record = |kind: EntityKind, id: &str, payload: serde_json::Value| SyncRecord {
        name: RecordName {
            kind,
            id: id.to_owned(),
        },
        schema_version: SCHEMA_VERSION,
        revision: 1,
        origin_device: "dev".into(),
        modified_ms: 1,
        payload: serde_json::to_vec(&payload).unwrap(),
    };
    let accepted = |record: &SyncRecord| matches!(validate(record), Verdict::Accept(_));
    let id = review_row_id("import:main|csv-1");
    let key = "import:main|csv-1";
    let good = serde_json::json!({"id": id, "trade_key": key, "account_uid": "a", "notes": "x"});
    assert!(accepted(&record(
        EntityKind::JournalReview,
        &id,
        good.clone()
    )));
    // Named for another trade, no key, bad screenshots, a huge text.
    assert!(!accepted(&record(EntityKind::JournalReview, "other", good)));
    let bad =
        |payload: serde_json::Value| !accepted(&record(EntityKind::JournalReview, &id, payload));
    assert!(bad(serde_json::json!({"account_uid": "a"})));
    assert!(bad(
        serde_json::json!({"trade_key": key, "account_uid": "a", "images": ["../../etc/passwd"]})
    ));
    assert!(bad(
        serde_json::json!({"trade_key": key, "account_uid": "a", "notes": "x".repeat(300_000)})
    ));
    assert!(accepted(&record(
        EntityKind::JournalDay,
        "2026-11-09",
        serde_json::json!({"date":"2026-11-09","plan":"p"})
    )));
    assert!(!accepted(&record(
        EntityKind::JournalDay,
        "2026-13-45",
        serde_json::json!({"date":"2026-13-45"})
    )));
    assert!(accepted(&record(
        EntityKind::JournalPlaybook,
        "pb",
        serde_json::json!({"id":"pb","name":"Breakout"})
    )));
    assert!(!accepted(&record(
        EntityKind::JournalPlaybook,
        "pb",
        serde_json::json!({"id":"pb","name":"  "})
    )));
}

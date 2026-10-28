//! A script edited outside the app, through the whole Script Manager over a real folder: what the
//! open editor does about it. Times are passed in, so nothing here waits.

use std::time::{Duration, Instant};

use super::debug_tests::Rig;
use super::model::CompileStatus;
use super::*;
use eframe::egui;

const NEW: &str = "//@version=6
indicator(\"Changed outside\")
plot(close)
";

const BROKEN: &str = "//@version=6
indicator(\"Broken\")
plot(close
";

fn at(base: Instant, ms: u64) -> Instant {
    base + Duration::from_millis(ms)
}

/// A rig whose editor is clean: its text is the file's.
fn clean_rig(tag: &str) -> Rig {
    let mut rig = Rig::new(tag);
    rig.manager.save_open();
    assert!(!rig.manager.open.as_ref().unwrap().editor.is_dirty());
    rig
}

fn file(rig: &mut Rig) -> std::path::PathBuf {
    let id = rig.manager.open.as_ref().unwrap().id.clone();
    rig.manager
        .script_store
        .as_mut()
        .unwrap()
        .file_path(&id)
        .unwrap()
        .unwrap()
}

fn on_disk(rig: &mut Rig) -> String {
    std::fs::read_to_string(file(rig)).unwrap()
}

fn source(rig: &Rig) -> String {
    rig.manager.open.as_ref().unwrap().editor.source.clone()
}

fn is_dirty(rig: &Rig) -> bool {
    rig.manager.open.as_ref().unwrap().editor.is_dirty()
}

fn conflict(rig: &Rig) -> bool {
    rig.manager
        .open
        .as_ref()
        .is_some_and(|open| open.watch.conflict.is_some())
}

/// One look at the folder at time `now`, as `begin_frame` does it.
fn poll(rig: &mut Rig, now: Instant) {
    if rig.manager.external_scan_due(now) {
        rig.manager.refresh();
        let ctx = rig.ctx.clone();
        rig.manager.reconcile_open(&ctx, now);
    }
}

fn undo(rig: &mut Rig) {
    rig.frame();
    rig.frame();
    rig.frame_with(vec![egui::Event::Key {
        key: egui::Key::Z,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::COMMAND,
    }]);
}

#[test]
fn a_clean_editor_shows_what_another_tool_wrote_once_the_file_has_settled() {
    let mut rig = clean_rig("ext-clean");
    let before = source(&rig);
    let t0 = Instant::now() + Duration::from_secs(10);
    // The first look finds nothing to do.
    poll(&mut rig, t0);
    assert_eq!(source(&rig), before);
    std::fs::write(file(&mut rig), NEW).unwrap();
    // Seen, but not yet settled.
    poll(&mut rig, at(t0, 500));
    assert_eq!(source(&rig), before, "not acted on at the first sighting");
    poll(&mut rig, at(t0, 1000));
    assert_eq!(source(&rig), NEW);
    assert!(!is_dirty(&rig), "the file's text is the saved text");
    assert!(
        rig.manager
            .open
            .as_ref()
            .unwrap()
            .editor
            .notice()
            .is_some_and(|text| text.starts_with("Reloaded"))
    );
    assert!(!conflict(&rig));
}

#[test]
fn a_reload_is_one_undo_step() {
    let mut rig = clean_rig("ext-undo");
    let before = source(&rig);
    let t0 = Instant::now() + Duration::from_secs(10);
    poll(&mut rig, t0);
    std::fs::write(file(&mut rig), NEW).unwrap();
    poll(&mut rig, at(t0, 500));
    poll(&mut rig, at(t0, 1000));
    assert_eq!(source(&rig), NEW);
    undo(&mut rig);
    assert_eq!(source(&rig), before, "Undo shows the previous text");
    assert!(is_dirty(&rig), "as an unsaved change from the file's text");
}

#[test]
fn a_file_that_is_still_being_written_is_never_loaded_half_written() {
    let mut rig = clean_rig("ext-half");
    let before = source(&rig);
    let t0 = Instant::now() + Duration::from_secs(10);
    poll(&mut rig, t0);
    // A tool writes the first half...
    std::fs::write(file(&mut rig), &NEW[..20]).unwrap();
    poll(&mut rig, at(t0, 500));
    assert_eq!(source(&rig), before);
    // ...and, 200 ms later, the whole file. The half never gets its time to settle.
    std::fs::write(file(&mut rig), NEW).unwrap();
    poll(&mut rig, at(t0, 1000));
    assert_eq!(source(&rig), before, "the whole file starts its own wait");
    poll(&mut rig, at(t0, 1500));
    assert_eq!(source(&rig), NEW);
}

#[test]
fn a_file_that_no_longer_compiles_shows_its_problems() {
    let mut rig = clean_rig("ext-broken");
    let t0 = Instant::now() + Duration::from_secs(10);
    poll(&mut rig, t0);
    std::fs::write(file(&mut rig), BROKEN).unwrap();
    poll(&mut rig, at(t0, 500));
    poll(&mut rig, at(t0, 1000));
    let open = rig.manager.open.as_ref().unwrap();
    assert_eq!(open.editor.source, BROKEN);
    assert_eq!(open.editor.status, CompileStatus::Error);
    assert!(!open.editor.diagnostics.is_empty());
}

#[test]
fn the_apps_own_save_is_not_an_outside_edit() {
    let mut rig = Rig::new("ext-own-save");
    rig.set_source(NEW);
    rig.manager.save_open();
    let t0 = Instant::now() + Duration::from_secs(10);
    for step in 0..6 {
        poll(&mut rig, at(t0, step * 500));
    }
    assert_eq!(source(&rig), NEW);
    assert!(!is_dirty(&rig));
    assert!(!conflict(&rig));
    assert!(
        rig.manager.open.as_ref().unwrap().editor.notice().is_none(),
        "nothing was reloaded"
    );
}

#[test]
fn a_file_without_its_final_newline_is_the_same_script() {
    let mut rig = clean_rig("ext-newline");
    let before = source(&rig);
    let t0 = Instant::now() + Duration::from_secs(10);
    poll(&mut rig, t0);
    std::fs::write(file(&mut rig), before.trim_end()).unwrap();
    poll(&mut rig, at(t0, 500));
    poll(&mut rig, at(t0, 1000));
    assert_eq!(source(&rig), before);
    assert!(rig.manager.open.as_ref().unwrap().editor.notice().is_none());
}

#[test]
fn typing_is_never_replaced_the_file_raises_a_question_and_save_waits() {
    let mut rig = clean_rig("ext-conflict");
    let typed = format!("{}// typed\n", source(&rig));
    rig.set_source(&typed);
    let t0 = Instant::now() + Duration::from_secs(10);
    poll(&mut rig, t0);
    std::fs::write(file(&mut rig), NEW).unwrap();
    poll(&mut rig, at(t0, 500));
    assert!(!conflict(&rig), "waits for the file to settle first");
    poll(&mut rig, at(t0, 1000));
    assert!(conflict(&rig));
    assert_eq!(source(&rig), typed, "the typing is untouched");
    // Save is on hold: the outside edit stays on disk.
    rig.manager.save_open();
    assert_eq!(on_disk(&mut rig), NEW);
    assert!(is_dirty(&rig));
    // The question is drawn.
    rig.frame();
}

#[test]
fn reload_takes_the_file_and_undo_brings_the_typing_back() {
    let mut rig = clean_rig("ext-reload");
    let typed = format!("{}// typed\n", source(&rig));
    rig.set_source(&typed);
    let t0 = Instant::now() + Duration::from_secs(10);
    poll(&mut rig, t0);
    std::fs::write(file(&mut rig), NEW).unwrap();
    poll(&mut rig, at(t0, 500));
    poll(&mut rig, at(t0, 1000));
    assert!(conflict(&rig));
    rig.manager.cmds.push(Cmd::DiskReload);
    rig.frame();
    assert_eq!(source(&rig), NEW);
    assert!(!is_dirty(&rig));
    assert!(!conflict(&rig));
    undo(&mut rig);
    assert_eq!(source(&rig), typed, "the typing is back");
}

#[test]
fn keeping_my_version_closes_the_question_until_the_file_changes_again() {
    let mut rig = clean_rig("ext-keep");
    let typed = format!("{}// typed\n", source(&rig));
    rig.set_source(&typed);
    let t0 = Instant::now() + Duration::from_secs(10);
    poll(&mut rig, t0);
    std::fs::write(file(&mut rig), NEW).unwrap();
    poll(&mut rig, at(t0, 500));
    poll(&mut rig, at(t0, 1000));
    assert!(conflict(&rig));
    rig.manager.cmds.push(Cmd::DiskKeep);
    rig.frame();
    assert!(!conflict(&rig));
    // The same file, looked at again and again: no new question. (The full rescan is due too.)
    for step in 2..8 {
        poll(&mut rig, at(t0, step * 500));
    }
    rig.manager.refresh();
    let ctx = rig.ctx.clone();
    rig.manager.reconcile_open(&ctx, at(t0, 9000));
    assert!(!conflict(&rig));
    assert_eq!(source(&rig), typed);
    // Saving now overwrites the file, as chosen.
    rig.manager.save_open();
    assert_eq!(on_disk(&mut rig), typed);
    assert!(!is_dirty(&rig));
    // A later outside edit asks again.
    let again = NEW.replace("Changed", "Changed again");
    std::fs::write(file(&mut rig), &again).unwrap();
    let typed_more = format!("{typed}// more\n");
    rig.set_source(&typed_more);
    poll(&mut rig, at(t0, 12_000));
    poll(&mut rig, at(t0, 12_500));
    assert!(conflict(&rig));
}

#[test]
fn a_change_that_lands_after_the_last_look_still_cannot_be_overwritten_by_save() {
    let mut rig = clean_rig("ext-race");
    let typed = format!("{}// typed\n", source(&rig));
    rig.set_source(&typed);
    let t0 = Instant::now() + Duration::from_secs(10);
    poll(&mut rig, t0);
    // The file changes and the person saves before the app has looked again.
    std::fs::write(file(&mut rig), NEW).unwrap();
    rig.manager.save_open();
    assert_eq!(on_disk(&mut rig), NEW, "the outside edit survived the save");
    assert!(is_dirty(&rig));
    // The next look asks at once, without waiting for the file to settle.
    poll(&mut rig, at(t0, 500));
    assert!(conflict(&rig));
}

#[test]
fn the_question_goes_away_when_the_typing_becomes_the_files_text_or_the_file_goes_back() {
    let mut rig = clean_rig("ext-resolve");
    let original = source(&rig);
    let typed = format!("{original}// typed\n");
    rig.set_source(&typed);
    let t0 = Instant::now() + Duration::from_secs(10);
    poll(&mut rig, t0);
    std::fs::write(file(&mut rig), NEW).unwrap();
    poll(&mut rig, at(t0, 500));
    poll(&mut rig, at(t0, 1000));
    assert!(conflict(&rig));
    // The file goes back to the text the editor read.
    std::fs::write(file(&mut rig), &original).unwrap();
    poll(&mut rig, at(t0, 1500));
    poll(&mut rig, at(t0, 2000));
    assert!(!conflict(&rig));
    assert_eq!(source(&rig), typed);
}

#[test]
fn the_cheap_look_only_asks_for_a_scan_when_the_file_moved() {
    let mut rig = clean_rig("ext-stat");
    let t0 = Instant::now() + Duration::from_secs(10);
    // The first look has nothing to compare with.
    assert!(rig.manager.external_scan_due(t0));
    // Paced: too soon to look again.
    assert!(!rig.manager.external_scan_due(at(t0, 100)));
    // Unchanged file: no scan.
    assert!(!rig.manager.external_scan_due(at(t0, 500)));
    std::fs::write(file(&mut rig), NEW).unwrap();
    assert!(rig.manager.external_scan_due(at(t0, 1000)));
}

#[test]
fn a_script_deleted_outside_closes_in_the_editor() {
    let mut rig = clean_rig("ext-delete");
    std::fs::remove_file(file(&mut rig)).unwrap();
    rig.manager.refresh();
    assert!(rig.manager.open.is_none());
}

#[test]
fn the_status_note_and_the_question_draw_in_a_small_window() {
    let mut rig = clean_rig("ext-draw");
    rig.screen = Some(egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(700.0, 420.0),
    ));
    let t0 = Instant::now() + Duration::from_secs(10);
    poll(&mut rig, t0);
    std::fs::write(file(&mut rig), NEW).unwrap();
    poll(&mut rig, at(t0, 500));
    poll(&mut rig, at(t0, 1000));
    rig.frame();
    let typed = format!("{}// typed\n", source(&rig));
    rig.set_source(&typed);
    std::fs::write(file(&mut rig), BROKEN).unwrap();
    poll(&mut rig, at(t0, 3000));
    poll(&mut rig, at(t0, 3500));
    assert!(conflict(&rig));
    rig.frame();
    rig.frame();
}

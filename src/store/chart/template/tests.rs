use super::*;
use crate::model::chart::canvas::BackgroundMode;
use crate::model::chart::template::BUILT_INS;
use std::sync::{Arc, Mutex};

fn shared() -> SharedStore {
    Arc::new(Mutex::new(Store::open_in_memory().expect("open")))
}

fn look() -> ChartAppearance {
    let mut look = ChartAppearance::default();
    look.canvas.background.mode = BackgroundMode::Gradient;
    look.canvas.margins.right = 21;
    look
}

#[test]
fn templates_persist_and_a_new_session_reads_them_back() {
    let store = shared();
    let mut templates = ChartTemplateStore::new(store.clone());
    let id = templates
        .save("My Dark Theme", "calm", &look())
        .expect("saved");
    let copy = templates.duplicate(id).expect("duplicated");
    templates.rename(copy, "Copy", "").expect("renamed");

    let restarted = ChartTemplateStore::new(store);
    let saved = restarted.book().saved();
    assert_eq!(saved.len(), 2);
    assert_eq!((saved[0].id, saved[0].name.as_str()), (id, "My Dark Theme"));
    assert_eq!(saved[0].description.as_deref(), Some("calm"));
    assert_eq!(saved[0].appearance, look());
    assert_eq!(saved[1].name, "Copy");
    assert!(restarted.load_error().is_none());
}

#[test]
fn an_edit_to_the_chart_does_not_reach_the_saved_template_until_updated() {
    let store = shared();
    let mut templates = ChartTemplateStore::new(store.clone());
    let id = templates.save("Look", "", &look()).expect("saved");
    let mut edited = look();
    edited.canvas.margins.top = 33;
    assert_eq!(
        ChartTemplateStore::new(store.clone()).book().saved()[0].appearance,
        look()
    );
    templates.update_appearance(id, &edited).expect("updated");
    let reread = ChartTemplateStore::new(store);
    assert_eq!(reread.book().saved()[0].appearance, edited);
    assert_eq!(reread.book().saved()[0].name, "Look");
}

#[test]
fn deleting_persists() {
    let store = shared();
    let mut templates = ChartTemplateStore::new(store.clone());
    let keep = templates.save("Keep", "", &look()).expect("saved");
    let drop = templates.save("Drop", "", &look()).expect("saved");
    templates.delete(drop).expect("deleted");
    let reread = ChartTemplateStore::new(store);
    assert_eq!(reread.book().saved().len(), 1);
    assert_eq!(reread.book().saved()[0].id, keep);
}

#[test]
fn presets_are_never_written_to_the_table() {
    let store = shared();
    let mut templates = ChartTemplateStore::new(store.clone());
    templates.save("Mine", "", &look()).expect("saved");
    assert_eq!(templates.book().entries().len(), BUILT_INS.len() + 1);
    let rows = store
        .lock()
        .expect("lock")
        .chart_templates_strict()
        .expect("read");
    assert_eq!(rows.len(), 1);
    // Reopening twice does not add anything either.
    drop(ChartTemplateStore::new(store.clone()));
    assert_eq!(
        store
            .lock()
            .expect("lock")
            .chart_templates_strict()
            .expect("read")
            .len(),
        1
    );
}

#[test]
fn a_refused_change_reports_why_and_writes_nothing() {
    let store = shared();
    let mut templates = ChartTemplateStore::new(store.clone());
    templates.save("Same", "", &look()).expect("saved");
    let error = templates.save("same", "", &look()).expect_err("duplicate");
    assert!(error.contains("already exists"), "{error}");
    let error = templates.save("   ", "", &look()).expect_err("empty");
    assert!(error.contains("name"), "{error}");
    assert!(templates.delete(BUILT_INS[0].id).is_err());
    assert_eq!(
        store
            .lock()
            .expect("lock")
            .chart_templates_strict()
            .expect("read")
            .len(),
        1
    );
}

#[test]
fn two_open_sheets_do_not_overwrite_each_other() {
    let store = shared();
    let mut first = ChartTemplateStore::new(store.clone());
    let mut second = ChartTemplateStore::new(store.clone());
    let a = first.save("From first", "", &look()).expect("saved");
    // `second` still shows nothing, yet its own save keeps the first sheet's template.
    assert!(second.book().saved().is_empty());
    second.save("From second", "", &look()).expect("saved");
    assert_eq!(second.book().saved().len(), 2);
    first.rename(a, "Renamed by first", "").expect("renamed");
    let reread = ChartTemplateStore::new(store);
    let names: Vec<&str> = reread
        .book()
        .saved()
        .iter()
        .map(|t| t.name.as_str())
        .collect();
    assert_eq!(names, ["Renamed by first", "From second"]);
}

#[test]
fn an_undecodable_row_is_reported_and_never_overwritten() {
    let store = shared();
    {
        let guard = store.lock().expect("lock");
        guard
            .connection
            .execute(
                "INSERT INTO chart_template (id, position, payload) VALUES ('7', 0, '{\"nonsense\":true}')",
                [],
            )
            .expect("insert");
    }
    let mut templates = ChartTemplateStore::new(store.clone());
    assert!(templates.load_error().is_some());
    assert!(templates.book().saved().is_empty());
    // The presets still list; saving is refused rather than replacing the bad row.
    assert_eq!(templates.book().entries().len(), BUILT_INS.len());
    let error = templates.save("New", "", &look()).expect_err("refused");
    assert!(error.contains("could not be read"), "{error}");
    let payload: String = store
        .lock()
        .expect("lock")
        .connection
        .query_row(
            "SELECT payload FROM chart_template WHERE id = '7'",
            [],
            |row| row.get(0),
        )
        .expect("row survives");
    assert_eq!(payload, "{\"nonsense\":true}");
}

#[test]
fn a_row_that_turns_bad_after_the_sheet_opened_blocks_the_next_change() {
    let store = shared();
    let mut templates = ChartTemplateStore::new(store.clone());
    templates.save("Fine", "", &look()).expect("saved");
    store
        .lock()
        .expect("lock")
        .connection
        .execute(
            "INSERT INTO chart_template (id, position, payload) VALUES ('8', 1, 'not json')",
            [],
        )
        .expect("insert");
    assert!(templates.save("More", "", &look()).is_err());
    assert!(templates.load_error().is_some());
    assert!(templates.delete(templates.book().saved()[0].id).is_err());
}

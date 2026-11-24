//! The Studio as the workspace runs it: opened by a real selection over a real grid, driven by
//! pointer and keyboard events and by the actions the window reports.

use super::super::select_tests::{HOUR, Rig, wait_until};
use super::*;
use crate::lock::lock;
use crate::model::Tool;
use crate::model::chart::snapshot::framing::{Dial, Step};
use crate::model::chart::snapshot::studio::{AspectRatio, Background, Include, StudioPrefs, Style};
use crate::model::chart::snapshot::{SnapshotPrefs, SnapshotScale, layout_file_name};
use crate::service::snapshot::export::ExportError;
use crate::store::SharedStore;
use crate::ui::shortcut;
use crate::workspace::test_support::rectangle;
use eframe::egui::{Event, Key, Modifiers, Pos2, vec2};
use std::path::{Path, PathBuf};
use std::sync::mpsc;

/// The file system, the Save panel and the network, standing in: it answers what the test says,
/// can hold the job until the test lets it go, and keeps what was written.
#[derive(Default)]
struct Fake {
    /// What the Save panel answers, once: nothing set means a file in a folder.
    choice: Mutex<Option<Result<Option<PathBuf>, ExportError>>>,
    names: Mutex<Vec<String>>,
    written: Mutex<Vec<(PathBuf, Vec<u8>)>>,
    write_error: Mutex<Option<ExportError>>,
    gate: Mutex<Option<mpsc::Receiver<()>>>,
}

impl job::Backend for Fake {
    fn choose_destination(&self, name: &str) -> Result<Option<PathBuf>, ExportError> {
        self.names.lock().unwrap().push(name.to_owned());
        if let Some(gate) = self.gate.lock().unwrap().take() {
            let _ = gate.recv();
        }
        self.choice
            .lock()
            .unwrap()
            .take()
            .unwrap_or_else(|| Ok(Some(PathBuf::from("/saved").join(name))))
    }

    fn write(&self, path: &Path, png: &[u8]) -> Result<(), ExportError> {
        if let Some(error) = self.write_error.lock().unwrap().take() {
            return Err(error);
        }
        self.written
            .lock()
            .unwrap()
            .push((path.to_owned(), png.to_vec()));
        Ok(())
    }

    fn upload(
        &self,
        _: &[u8],
    ) -> Result<
        crate::model::chart::snapshot::SharedSnapshot,
        crate::model::chart::snapshot::UploadError,
    > {
        unreachable!("the Studio never uploads")
    }
}

fn rig(symbols: &[&str]) -> Rig {
    Rig::new(symbols).with_studio()
}

/// A rig with a Studio open on a selection over the first chart.
fn opened(symbols: &[&str]) -> Rig {
    open_on(rig(symbols))
}

fn open_on(mut rig: Rig) -> Rig {
    rig.begin();
    rig.drag(rig.at(0, (0.1, 0.1)), rig.at(0, (0.9, 0.9)));
    rig.settle();
    assert!(
        rig.workspace.studio_open(),
        "the selection opened the Studio"
    );
    rig
}

/// Gives the rig's workspace a snapshot host over a store of its own, holding `look` if there is
/// one, as a window of the app has. Returns the store, to read what was saved.
fn remember(rig: &mut Rig, look: Option<StudioPrefs>) -> SharedStore {
    use crate::service::keychain::api_keys::MemoryBackend;
    use crate::service::keychain::snapshot_secrets::SnapshotSecrets;
    let store: SharedStore = Arc::new(Mutex::new(crate::store::Store::open_in_memory().unwrap()));
    if let Some(studio) = look {
        let saved = SnapshotPrefs {
            studio,
            ..SnapshotPrefs::default()
        };
        lock(&store)
            .set_setting(SnapshotPrefs::KEY, &saved)
            .unwrap();
    }
    let secrets = SnapshotSecrets::with(Box::new(MemoryBackend::default()), Box::new(|_| None));
    let host = SnapshotHost::load_with(store.clone(), Arc::new(secrets));
    rig.workspace.set_snapshots(&host);
    store
}

/// The look the store holds.
fn saved_look(store: &SharedStore) -> StudioPrefs {
    lock(store)
        .setting_strict::<SnapshotPrefs>(SnapshotPrefs::KEY)
        .unwrap()
        .unwrap_or_default()
        .studio
}

fn session(rig: &Rig) -> &StudioSession {
    rig.workspace.snapshot.studio.as_ref().expect("a session")
}

/// Runs frames until the picture of what is wanted has been made, and drawn.
fn picture(rig: &mut Rig) {
    wait_until("the picture was never made", || {
        rig.frame(Vec::new());
        let previews = &session(rig).previews;
        previews.shown().is_some() && !previews.is_busy()
    });
    // The picture can land after the frame that drew the preview, which then still showed
    // "Drawing the image…": one more frame draws it, so its widget is there to find.
    rig.frame(Vec::new());
}

/// Runs frames until the snapshot job ends.
fn finished(rig: &mut Rig) {
    wait_until("the snapshot never finished", || {
        rig.frame(Vec::new());
        rig.workspace.snapshot.running.is_none()
    });
}

fn fake(rig: &mut Rig) -> Arc<Fake> {
    let fake = Arc::new(Fake::default());
    rig.workspace.snapshot.backend = Some(fake.clone());
    fake
}

fn click(rig: &mut Rig, label: &str) {
    let at = rig.label_at(label);
    rig.press(at);
    rig.release(at);
}

fn notice(rig: &Rig) -> Option<(String, bool)> {
    rig.workspace
        .context
        .notice
        .as_ref()
        .map(|notice| (notice.text.clone(), notice.error))
}

/// The width and height a PNG says it has.
fn png_size(png: &[u8]) -> (u32, u32) {
    assert_eq!(&png[1..4], b"PNG");
    let at = |offset: usize| u32::from_be_bytes(png[offset..offset + 4].try_into().unwrap());
    (at(16), at(20))
}

/// What the live charts are, for comparing before and after.
fn live(rig: &Rig) -> (Vec<crate::ui::ViewStep>, usize, usize, bool, bool) {
    (
        (0..rig.ids.len())
            .map(|chart| rig.card_view(chart))
            .collect(),
        rig.workspace.charts.iter().map(|c| c.drawings.len()).sum(),
        rig.workspace.charts.iter().map(|c| c.candles.len()).sum(),
        rig.workspace.history.can_undo(),
        rig.workspace.is_dirty(),
    )
}

trait CardView {
    fn card_view(&self, chart: usize) -> crate::ui::ViewStep;
}

impl CardView for Rig {
    fn card_view(&self, chart: usize) -> crate::ui::ViewStep {
        let index = self.workspace.chart_index(self.ids[chart]).expect("chart");
        self.workspace.charts[index].chart.view_step()
    }
}

// MARK: Opening

#[test]
fn a_selection_opens_the_studio_styled_at_three_to_two_on_the_portion() {
    let rig = opened(&["BTCUSDT"]);
    let config = session(&rig).config;
    assert_eq!(config.area, Area::Portion);
    assert_eq!(config.style, Style::Styled);
    assert_eq!(config.effective_size(), AspectRatio::Landscape3x2);
    assert_eq!(config.background, Background::None);
    assert!(config.framing.is_default());
    assert!(rig.labels.contains_key("Close"));
    assert!(rig.words.iter().any(|word| word == "Studio"));
}

#[test]
fn the_areas_on_offer_follow_the_workspace() {
    let one = opened(&["BTCUSDT"]);
    assert!(one.labels.contains_key("Full chart") && one.labels.contains_key("Portion"));
    assert!(!one.labels.contains_key("Entire layout"));
    let two = opened(&["BTCUSDT", "ETHUSDT"]);
    assert!(two.labels.contains_key("Entire layout"));
}

#[test]
fn the_summary_is_the_exports_own_size() {
    let rig = opened(&["BTCUSDT"]);
    let session = session(&rig);
    let composition = composition::layout(session.capture.size(), &session.config);
    let dimensions = composition::dimensions(&composition, SnapshotScale::X2).expect("fits");
    let expected = composition::summary(Area::Portion, Style::Styled, &dimensions);
    assert!(
        rig.words.contains(&expected),
        "{expected:?} in {:?}",
        rig.words
    );
}

// MARK: Area

#[test]
fn portion_goes_back_to_the_charts_and_the_next_selection_reopens_with_the_look() {
    let mut rig = rig(&["BTCUSDT"]);
    remember(&mut rig, None);
    let mut rig = open_on(rig);
    click(&mut rig, "DegenView mark, on");
    click(&mut rig, "Radiant background");
    click(&mut rig, "Chart");
    let before = session(&rig).config;
    assert_eq!(
        (before.style, before.background, before.include.branding),
        (Style::Chart, Background::Radiant, false)
    );
    click(&mut rig, "Portion");
    assert!(!rig.workspace.studio_open());
    assert!(rig.selecting());

    rig.drag(rig.at(0, (0.2, 0.2)), rig.at(0, (0.6, 0.7)));
    rig.settle();
    let after = session(&rig).config;
    assert_eq!(after.area, Area::Portion);
    assert_eq!(
        (after.style, after.background, after.include),
        (before.style, before.background, before.include)
    );
    assert!(after.framing.is_default());
}

#[test]
fn a_selection_cancelled_on_the_way_still_remembers_the_look() {
    let mut rig = rig(&["BTCUSDT"]);
    remember(&mut rig, None);
    let mut rig = open_on(rig);
    click(&mut rig, "Radiant background");
    click(&mut rig, "Portion");
    assert!(rig.selecting());
    rig.escape();
    rig.settle();
    assert!(!rig.selecting() && !rig.workspace.studio_open());
    rig.begin();
    rig.drag(rig.at(0, (0.1, 0.1)), rig.at(0, (0.9, 0.9)));
    rig.settle();
    assert_eq!(session(&rig).config.background, Background::Radiant);
}

// MARK: Remembering the look

#[test]
fn a_studio_opens_in_the_look_that_was_remembered_with_the_framing_at_its_default() {
    let look = StudioPrefs {
        style: Style::Chart,
        size: AspectRatio::Portrait9x16,
        background: Background::Illuminated,
        include: Include {
            indicators: true,
            drawings: false,
            branding: false,
        },
    };
    let mut rig = rig(&["BTCUSDT"]);
    remember(&mut rig, Some(look));
    let rig = open_on(rig);
    let config = session(&rig).config;
    assert_eq!(config.area, Area::Portion, "the area is how it was opened");
    assert_eq!(config.look(), look);
    assert!(config.framing.is_default());
    assert!(!config.size_enabled(), "Chart keeps Size dimmed");
}

#[test]
fn changing_a_control_saves_the_look_and_a_frame_that_changes_nothing_saves_nothing() {
    let mut rig = rig(&["BTCUSDT"]);
    let store = remember(&mut rig, None);
    // A row this version would rewrite if it saved anything: it stays while nothing changes.
    let marked = serde_json::json!({"provider": null, "marker": 1});
    lock(&store)
        .set_setting(SnapshotPrefs::KEY, &marked)
        .unwrap();
    let mut rig = open_on(rig);
    for _ in 0..5 {
        rig.frame(Vec::new());
    }
    assert_eq!(
        lock(&store)
            .raw_setting(SnapshotPrefs::KEY)
            .unwrap()
            .unwrap(),
        marked,
        "opening and drawing frames writes nothing"
    );

    click(&mut rig, "Spectrum background");
    assert_eq!(saved_look(&store).background, Background::Spectrum);
    click(&mut rig, "9:16");
    click(&mut rig, "DegenView mark, on");
    let saved = saved_look(&store);
    assert_eq!(saved.size, AspectRatio::Portrait9x16);
    assert!(!saved.include.branding);
    assert_eq!(saved, session(&rig).config.look());
    assert!(
        lock(&store)
            .raw_setting(SnapshotPrefs::KEY)
            .unwrap()
            .unwrap()
            .get("marker")
            .is_none(),
        "a change writes the settings"
    );
}

#[test]
fn the_framing_and_the_area_are_never_saved() {
    let mut rig = rig(&["BTCUSDT"]);
    let store = remember(&mut rig, None);
    let mut rig = open_on(rig);
    click(&mut rig, "Spectrum background");
    let before = lock(&store).raw_setting(SnapshotPrefs::KEY).unwrap();
    {
        let mut taken = rig.workspace.snapshot.studio.take().expect("a session");
        taken.config.framing.width = 1.4;
        rig.workspace.snapshot.studio = Some(taken);
    }
    for _ in 0..3 {
        rig.frame(Vec::new());
    }
    click(&mut rig, "Full chart");
    assert_eq!(
        lock(&store).raw_setting(SnapshotPrefs::KEY).unwrap(),
        before,
        "reframing and changing the area leave the saved look alone"
    );
    let keys: Vec<_> = before.unwrap()["studio"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    assert!(
        keys.iter()
            .all(|key| ["style", "size", "background", "include"].contains(&key.as_str())),
        "{keys:?}"
    );
}

#[test]
fn a_toggle_that_is_unavailable_does_not_overwrite_what_was_remembered() {
    // The chart has no indicators and no drawings, so those two toggles are dimmed. The look
    // that was remembered has them off; switching the mark must not turn them on.
    let remembered = StudioPrefs {
        include: Include {
            indicators: false,
            drawings: false,
            branding: true,
        },
        ..StudioPrefs::default()
    };
    let mut rig = rig(&["BTCUSDT"]);
    let store = remember(&mut rig, Some(remembered));
    let mut rig = open_on(rig);
    assert!(!session(&rig).capture.has_indicators() && !session(&rig).capture.has_drawings());
    click(&mut rig, "Indicators, off");
    click(&mut rig, "Drawings, off");
    assert_eq!(saved_look(&store), remembered, "dimmed toggles do nothing");
    click(&mut rig, "DegenView mark, on");
    let saved = saved_look(&store).include;
    assert_eq!(
        (saved.indicators, saved.drawings, saved.branding),
        (false, false, false)
    );
}

#[test]
fn a_look_that_could_not_be_saved_is_said_once_and_does_not_hold_up_the_studio() {
    let mut rig = rig(&["BTCUSDT"]);
    let store = remember(&mut rig, None);
    let mut rig = open_on(rig);
    // The database is gone: the lock is poisoned, as after a crash while it was held.
    let poisoner = store.clone();
    let _ = std::thread::spawn(move || {
        let _held = poisoner.lock().unwrap();
        panic!("poisons the lock");
    })
    .join();
    click(&mut rig, "Spectrum background");
    assert_eq!(session(&rig).config.background, Background::Spectrum);
    assert!(rig.workspace.studio_open());
    assert!(
        notice(&rig).is_some_and(|(_, error)| error),
        "the user is told, {:?}",
        notice(&rig)
    );
    for _ in 0..5 {
        rig.frame(Vec::new());
    }
    assert_eq!(
        session(&rig).remembered.background,
        Background::Spectrum,
        "it is not tried again every frame"
    );
}

#[test]
fn full_chart_takes_the_focused_chart_again_with_the_look_kept_and_the_framing_reset() {
    let mut rig = opened(&["BTCUSDT", "ETHUSDT"]);
    rig.workspace.focused = Some(rig.ids[1]);
    click(&mut rig, "Spectrum background");
    {
        let mut taken = rig.workspace.snapshot.studio.take().expect("a session");
        taken.config.framing.width = 1.4;
        rig.workspace.snapshot.studio = Some(taken);
    }
    click(&mut rig, "Full chart");
    let session = session(&rig);
    let Capture::Single(chart) = &session.capture else {
        panic!("one chart");
    };
    assert_eq!(chart.chart_id(), rig.ids[1], "the focused chart");
    let index = rig.workspace.chart_index(rig.ids[1]).expect("chart");
    assert_eq!(
        chart.size(),
        rig.workspace.charts[index].chart.snapshot_size()
    );
    assert_eq!(session.config.area, Area::FullChart);
    assert_eq!(session.config.background, Background::Spectrum);
    assert!(session.config.framing.is_default());
}

#[test]
fn entire_layout_takes_every_market_chart_as_arranged() {
    let mut rig = opened(&["BTCUSDT", "ETHUSDT"]);
    click(&mut rig, "Entire layout");
    let session = session(&rig);
    let Capture::Layout(layout) = &session.capture else {
        panic!("a layout");
    };
    assert_eq!(layout.charts.len(), 2);
    assert!(layout.crop.is_none());
    assert_eq!(session.config.area, Area::EntireLayout);
    // Several charts cannot be reframed: the rulers do nothing and Reset view stays away.
    assert!(!rig.labels.contains_key("Reset view"));
}

#[test]
fn the_studio_of_a_picture_of_several_charts_ignores_the_framing_actions() {
    let mut rig = opened(&["BTCUSDT", "ETHUSDT"]);
    click(&mut rig, "Entire layout");
    let mut taken = rig.workspace.snapshot.studio.take().expect("a session");
    let ctx = rig.ctx.clone();
    for action in [
        Action::DragDial(Dial::Width, 1.5),
        Action::DragPreview(vec2(-80.0, 20.0)),
        Action::NudgePreview { across: 1, down: 0 },
        Action::StepDial(Dial::Height, Step::Coarse, 1),
    ] {
        rig.workspace.apply_studio_action(&ctx, &mut taken, action);
    }
    assert_eq!(taken.config.framing.dx, 0.0);
    assert_eq!(taken.config.framing.dy, 0.0);
    assert_eq!(taken.config.framing.height, 1.0);
}

// MARK: Look

#[test]
fn the_controls_set_the_look_and_size_waits_for_styled() {
    let mut rig = opened(&["BTCUSDT"]);
    click(&mut rig, "16:9");
    assert_eq!(session(&rig).config.size, AspectRatio::Wide16x9);
    click(&mut rig, "Chart");
    assert_eq!(session(&rig).config.style, Style::Chart);
    // Size is disabled in the Chart style: a click on it changes nothing.
    let at = rig.rect_at("studio-size", 3);
    rig.press(at);
    rig.release(at);
    assert_eq!(session(&rig).config.size, AspectRatio::Wide16x9);
    click(&mut rig, "Styled");
    assert_eq!(
        session(&rig).config.size,
        AspectRatio::Landscape3x2,
        "Styled opens at 3:2"
    );
    click(&mut rig, "DegenView mark, on");
    let include = session(&rig).config.include;
    assert!(!include.branding && include.indicators && include.drawings);
}

trait RectAt {
    fn rect_at(&self, id: &str, index: usize) -> Pos2;
}

impl RectAt for Rig {
    fn rect_at(&self, id: &str, index: usize) -> Pos2 {
        self.ctx
            .read_response(Id::new(id).with(index))
            .expect("a widget")
            .rect
            .center()
    }
}

// MARK: Framing

#[test]
fn the_dials_snap_near_one_and_reset_view_keeps_the_rest_of_the_look() {
    let mut rig = opened(&["BTCUSDT"]);
    click(&mut rig, "Dots background");
    click(&mut rig, "1:1");
    let mut taken = rig.workspace.snapshot.studio.take().expect("a session");
    let ctx = rig.ctx.clone();
    let apply = |rig: &mut Rig, taken: &mut StudioSession, action| {
        rig.workspace.apply_studio_action(&ctx, taken, action);
    };
    apply(&mut rig, &mut taken, Action::DragDial(Dial::Width, 1.02));
    assert_eq!(taken.config.framing.width, 1.02);
    apply(&mut rig, &mut taken, Action::ReleaseDial);
    assert_eq!(taken.config.framing.width, 1.0, "snapped");
    apply(&mut rig, &mut taken, Action::DragDial(Dial::Width, 1.6));
    apply(&mut rig, &mut taken, Action::ReleaseDial);
    assert_eq!(taken.config.framing.width, 1.6, "far from one stays");
    apply(
        &mut rig,
        &mut taken,
        Action::StepDial(Dial::Height, Step::Fine, 1),
    );
    assert_eq!(taken.config.framing.height, 1.02);
    apply(&mut rig, &mut taken, Action::ResetDial(Dial::Height));
    assert_eq!(taken.config.framing.height, 1.0);

    apply(
        &mut rig,
        &mut taken,
        Action::NudgePreview { across: 1, down: 0 },
    );
    assert!(
        taken.config.framing.dx < 0.0,
        "the picture goes right: earlier bars"
    );
    apply(&mut rig, &mut taken, Action::ResetPosition);
    assert_eq!(
        (taken.config.framing.dx, taken.config.framing.width),
        (0.0, 1.6),
        "the position goes back, the dials stay"
    );

    apply(&mut rig, &mut taken, Action::DragPreview(vec2(-30.0, 12.0)));
    assert!(
        taken.config.framing.dx > 0.0,
        "dragging left shows later bars"
    );
    apply(&mut rig, &mut taken, Action::ResetView);
    assert!(taken.config.framing.is_default());
    assert_eq!(taken.config.background, Background::Dots);
    assert_eq!(taken.config.size, AspectRatio::Square);
    assert_eq!(taken.config.style, Style::Styled);
}

#[test]
fn a_drag_is_held_to_its_limits_and_settles_on_release() {
    let mut rig = opened(&["BTCUSDT"]);
    let mut taken = rig.workspace.snapshot.studio.take().expect("a session");
    let ctx = rig.ctx.clone();
    rig.workspace.apply_studio_action(
        &ctx,
        &mut taken,
        Action::DragPreview(vec2(-9_000.0, 9_000.0)),
    );
    let pulled = taken.config.framing;
    rig.workspace
        .apply_studio_action(&ctx, &mut taken, Action::ReleasePreview);
    let settled = taken.config.framing;
    let (base, plot) = taken.framing_basis().expect("one chart");
    let bounds = settled.bounds(&base, plot);
    assert!(
        pulled.dx >= settled.dx && pulled.dy >= settled.dy,
        "{pulled:?} {settled:?}"
    );
    assert!(settled.dx >= bounds.dx.0 - 1e-9 && settled.dx <= bounds.dx.1 + 1e-9);
    assert!(settled.dy >= bounds.dy.0 - 1e-9 && settled.dy <= bounds.dy.1 + 1e-9);
    assert!(taken.drag.is_none());
}

// MARK: The picture

#[test]
fn the_first_picture_arrives_and_later_ones_replace_it_in_the_same_texture() {
    let mut rig = opened(&["BTCUSDT"]);
    assert!(rig.words.iter().any(|w| w == "Drawing the image…"));
    picture(&mut rig);
    let first = session(&rig).previews.shown().expect("a picture");
    assert!(first.pixels[0] > 100 && first.pixels[1] > 100);
    click(&mut rig, "Luminous background");
    picture(&mut rig);
    let second = session(&rig).previews.shown().expect("a picture");
    assert_eq!(second.texture, first.texture);
    assert!(second.generation > first.generation);
}

#[test]
fn a_drag_is_drawn_at_half_the_pixels_and_the_full_picture_follows_the_release() {
    let mut rig = opened(&["BTCUSDT"]);
    picture(&mut rig);
    let full = session(&rig).previews.shown().expect("a picture").pixels;
    let at = rig.label_at("Preview. Drag to move the picture; arrow keys move it, Home resets.");
    rig.press(at);
    rig.drag_to(at, at + vec2(-40.0, 10.0));
    picture(&mut rig);
    let coarse = session(&rig).previews.shown().expect("a picture").pixels;
    assert!(
        coarse[0] * 2 <= full[0] + 2 && coarse[0] * 2 + 2 >= full[0] * 9 / 10,
        "{coarse:?} against {full:?}"
    );
    rig.release(at + vec2(-40.0, 10.0));
    picture(&mut rig);
    let after = session(&rig).previews.shown().expect("a picture").pixels;
    assert_eq!(after, full, "the same picture at full size");
    assert!(session(&rig).config.framing.dx != 0.0);
}

// MARK: Export

#[test]
fn download_writes_a_png_of_exactly_the_footers_size_named_for_the_chart() {
    let mut rig = opened(&["BTCUSDT"]);
    let fake = fake(&mut rig);
    let (expected, config) = {
        let session = session(&rig);
        let composition = composition::layout(session.capture.size(), &session.config);
        (
            composition::dimensions(&composition, SnapshotScale::X2).expect("fits"),
            session.config,
        )
    };
    click(&mut rig, "Download PNG");
    assert_eq!(
        rig.workspace.studio_export(),
        Some(window::Export::Download),
        "busy while it runs"
    );
    finished(&mut rig);
    let written = fake.written.lock().unwrap();
    let (path, png) = written.first().expect("a file");
    assert_eq!(png_size(png), (expected.width, expected.height));
    let names = fake.names.lock().unwrap();
    assert!(
        names[0].starts_with("DegenView_BTCUSDT_") && names[0].ends_with(".png"),
        "{names:?}"
    );
    assert_eq!(path, &PathBuf::from("/saved").join(&names[0]));
    let (text, error) = notice(&rig).expect("a message");
    assert_eq!((text, error), (format!("Saved {}", names[0]), false));
    assert!(rig.workspace.studio_open(), "the Studio stays open");
    assert_eq!(session(&rig).config, config, "and as it was");
}

#[test]
fn the_export_is_the_look_that_was_set() {
    let mut rig = opened(&["BTCUSDT"]);
    let fake = fake(&mut rig);
    click(&mut rig, "9:16");
    click(&mut rig, "Spectrum background");
    click(&mut rig, "Download PNG");
    finished(&mut rig);
    let written = fake.written.lock().unwrap();
    let (width, height) = png_size(&written[0].1);
    let ratio = width as f32 / height as f32;
    assert!((ratio - 9.0 / 16.0).abs() < 0.01, "{width} x {height}");
}

#[test]
fn an_image_of_the_layout_is_named_for_the_layout() {
    let mut rig = opened(&["BTCUSDT", "ETHUSDT"]);
    let fake = fake(&mut rig);
    click(&mut rig, "Entire layout");
    click(&mut rig, "Download PNG");
    finished(&mut rig);
    let names = fake.names.lock().unwrap();
    assert!(names[0].starts_with("DegenView_Layout_"), "{names:?}");
    // The same stamp as the helper makes, to the minute.
    let stamp = layout_file_name(now_ms(), timezone::local_utc_offset_secs(now_ms()));
    assert_eq!(names[0].len(), stamp.len());
}

#[test]
fn copy_image_reports_only_after_the_picture_is_made() {
    let mut rig = opened(&["BTCUSDT"]);
    let (sender, receiver) = mpsc::channel();
    let fake = fake(&mut rig);
    *fake.gate.lock().unwrap() = Some(receiver);
    click(&mut rig, "Copy image");
    // The copy has no Save panel, so the gate is not reached: it ends by itself.
    drop(sender);
    finished(&mut rig);
    assert_eq!(notice(&rig), Some(("Image copied".to_owned(), false)));
    assert!(fake.written.lock().unwrap().is_empty(), "nothing is saved");
    assert!(rig.workspace.studio_open());
}

#[test]
fn a_cancelled_save_panel_says_nothing_and_leaves_the_studio_open() {
    let mut rig = opened(&["BTCUSDT"]);
    let fake = fake(&mut rig);
    *fake.choice.lock().unwrap() = Some(Ok(None));
    click(&mut rig, "Download PNG");
    finished(&mut rig);
    assert_eq!(notice(&rig), None);
    assert!(fake.written.lock().unwrap().is_empty());
    assert!(rig.workspace.studio_open());
}

#[test]
fn a_failed_save_is_reported_as_every_snapshot_failure_is() {
    let mut rig = opened(&["BTCUSDT"]);
    let fake = fake(&mut rig);
    *fake.write_error.lock().unwrap() = Some(ExportError::Write("disk full".to_owned()));
    click(&mut rig, "Download PNG");
    finished(&mut rig);
    let (text, error) = notice(&rig).expect("a message");
    assert!(error, "{text}");
    assert!(rig.workspace.studio_open());
}

#[test]
fn one_export_at_a_time_and_the_buttons_wait() {
    let mut rig = opened(&["BTCUSDT"]);
    let (release, gate) = mpsc::channel();
    let fake = fake(&mut rig);
    *fake.gate.lock().unwrap() = Some(gate);
    click(&mut rig, "Download PNG");
    for _ in 0..10 {
        rig.frame(Vec::new());
    }
    assert_eq!(
        rig.workspace.studio_export(),
        Some(window::Export::Download)
    );
    click(&mut rig, "Copy image");
    assert_eq!(
        rig.workspace.snapshot.running.as_ref().map(|r| r.action),
        Some(SnapshotAction::Download),
        "the first still runs and no second began"
    );
    release.send(()).expect("the job waits");
    finished(&mut rig);
    assert_eq!(rig.workspace.studio_export(), None);
    assert_eq!(fake.written.lock().unwrap().len(), 1);
}

#[test]
fn an_image_too_large_to_make_says_so_and_cannot_be_exported() {
    let mut rig = opened(&["BTCUSDT"]);
    let index = rig.workspace.chart_index(rig.ids[0]).expect("chart");
    // A chart as large as no screen is, taken again before a frame draws it at its own size.
    rig.workspace.charts[index]
        .chart
        .set_snapshot_size_for_tests(vec2(30_000.0, 20_000.0));
    let mut taken = rig.workspace.snapshot.studio.take().expect("a session");
    let ctx = rig.ctx.clone();
    rig.workspace
        .apply_studio_action(&ctx, &mut taken, Action::SetArea(Area::FullChart));
    rig.workspace.snapshot.studio = Some(taken);
    rig.settle();
    let message = ComposeError::TooLarge.to_string();
    assert!(rig.words.contains(&message), "{:?}", rig.words);
    let fake = fake(&mut rig);
    click(&mut rig, "Download PNG");
    rig.settle();
    assert!(rig.workspace.snapshot.running.is_none());
    assert!(fake.names.lock().unwrap().is_empty());
}

#[test]
fn a_result_is_drawn_over_the_window_even_after_the_window_is_clicked() {
    let mut rig = opened(&["BTCUSDT"]);
    let _fake = fake(&mut rig);
    click(&mut rig, "Download PNG");
    finished(&mut rig);
    let order = |rig: &Rig| {
        let at = |text: &str| rig.words.iter().position(|word| word.starts_with(text));
        (at("Download PNG"), at("Saved "))
    };
    rig.frame(Vec::new());
    let (window, saved) = order(&rig);
    assert!(saved > window && saved.is_some(), "{:?}", rig.words);
    // A click in the window brings it to the front: the result must still be above it.
    click(&mut rig, "Dots background");
    let (window, saved) = order(&rig);
    assert!(saved > window && saved.is_some(), "{:?}", rig.words);
}

// MARK: Closing and the chart

#[test]
fn closing_drops_the_session_and_frees_the_picture() {
    let mut rig = opened(&["BTCUSDT"]);
    picture(&mut rig);
    let texture = session(&rig).previews.shown().expect("a picture").texture;
    click(&mut rig, "Close");
    assert!(!rig.workspace.studio_open());
    rig.settle();
    assert!(rig.freed.contains(&texture), "{:?}", rig.freed);
}

#[test]
fn escape_closes_and_is_not_the_workspaces() {
    let mut rig = opened(&["BTCUSDT"]);
    rig.workspace.toggle_tool(Tool::TrendLine);
    assert_eq!(rig.workspace.tools.active_tool(), Tool::TrendLine);
    rig.escape();
    rig.settle();
    assert!(!rig.workspace.studio_open());
    assert_eq!(
        rig.workspace.tools.active_tool(),
        Tool::TrendLine,
        "the tool was not disarmed by the same key"
    );
}

#[test]
fn a_session_ends_when_its_chart_goes() {
    let mut rig = opened(&["BTCUSDT", "ETHUSDT"]);
    let gone = rig.ids[0];
    rig.workspace.charts.retain(|chart| chart.id != gone);
    rig.frame(Vec::new());
    assert!(!rig.workspace.studio_open());
}

#[test]
fn a_session_ends_when_one_chart_of_a_layout_goes() {
    let mut rig = opened(&["BTCUSDT", "ETHUSDT"]);
    click(&mut rig, "Entire layout");
    let gone = rig.ids[1];
    rig.workspace.charts.retain(|chart| chart.id != gone);
    rig.frame(Vec::new());
    assert!(!rig.workspace.studio_open());
}

#[test]
fn a_whole_session_leaves_the_live_charts_as_they_were() {
    let mut rig = rig(&["BTCUSDT", "ETHUSDT"]);
    let index = rig.workspace.chart_index(rig.ids[0]).expect("chart");
    rig.workspace.charts[index].drawings = vec![rectangle(1, 100 * HOUR)];
    rig.settle();
    rig.workspace.mark_clean();
    let before = live(&rig);
    rig.begin();
    rig.drag(rig.at(0, (0.1, 0.1)), rig.at(0, (0.9, 0.9)));
    rig.settle();
    let fake = fake(&mut rig);
    picture(&mut rig);
    let at = rig.label_at("Preview. Drag to move the picture; arrow keys move it, Home resets.");
    rig.press(at);
    rig.drag_to(at, at + vec2(-60.0, 30.0));
    rig.release(at + vec2(-60.0, 30.0));
    click(&mut rig, "Spectrum background");
    click(&mut rig, "Entire layout");
    click(&mut rig, "Download PNG");
    finished(&mut rig);
    assert_eq!(fake.written.lock().unwrap().len(), 1);
    click(&mut rig, "Close");
    rig.settle();
    assert_eq!(live(&rig), before);
}

// MARK: The keyboard

fn snapshot_download_keys() -> Event {
    let mac = shortcut::is_mac();
    Event::Key {
        key: Key::S,
        physical_key: Some(Key::S),
        pressed: true,
        repeat: false,
        modifiers: Modifiers {
            alt: true,
            ctrl: !mac,
            shift: false,
            mac_cmd: mac,
            command: true,
        },
    }
}

#[test]
fn the_snapshot_shortcut_works_without_the_studio_and_waits_under_it() {
    // The control: the same keys on the same workspace with no Studio take a snapshot.
    let mut control = rig(&["BTCUSDT"]);
    let (_hold, gate) = mpsc::channel();
    *fake(&mut control).gate.lock().unwrap() = Some(gate);
    control.frame(vec![snapshot_download_keys()]);
    assert!(control.workspace.snapshot_busy(), "the shortcut is live");

    let mut rig = opened(&["BTCUSDT"]);
    let fake = fake(&mut rig);
    rig.frame(vec![snapshot_download_keys()]);
    rig.settle();
    assert!(!rig.workspace.snapshot_busy());
    assert!(fake.names.lock().unwrap().is_empty());
}

#[test]
fn the_workspaces_keys_wait_under_the_studio() {
    let mut rig = opened(&["BTCUSDT"]);
    let key = |key, modifiers| Event::Key {
        key,
        physical_key: Some(key),
        pressed: true,
        repeat: false,
        modifiers,
    };
    let command = if shortcut::is_mac() {
        Modifiers::MAC_CMD
    } else {
        Modifiers::CTRL
    };
    rig.frame(vec![key(Key::K, command)]);
    assert!(
        !rig.workspace.symbol_picker.is_open(),
        "the symbol search stays shut"
    );
    rig.frame(vec![key(Key::Z, command)]);
    rig.settle();
    assert!(rig.workspace.studio_open());
}

#[test]
fn clicks_on_the_charts_under_the_window_do_not_reach_them() {
    let mut rig = opened(&["BTCUSDT"]);
    let before = live(&rig);
    // A press and drag over the chart's body, which the window covers.
    let from = rig.at(0, (0.5, 0.5));
    rig.drag(from, from + vec2(-200.0, 0.0));
    rig.settle();
    assert_eq!(live(&rig), before);
    assert!(rig.workspace.studio_open());
}

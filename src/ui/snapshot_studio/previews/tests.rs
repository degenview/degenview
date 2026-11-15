use super::*;
use crate::model::chart::snapshot::studio::{Area, Background};
use eframe::egui::{Color32, vec2};
use std::cell::RefCell;
use std::sync::mpsc::{self, Sender};

type Answer = Result<ColorImage, RasterError>;

/// A pipeline whose compose and raster steps the test controls: every raster job is a channel the
/// test answers by hand.
struct Rig {
    ctx: Context,
    previews: Previews,
    composed: RefCell<Vec<Key>>,
    jobs: RefCell<Vec<Sender<Answer>>>,
    fail_compose: bool,
}

fn key(background: Background) -> Key {
    let mut config = StudioConfig::new(Area::FullChart);
    config.background = background;
    Key {
        config,
        max_px: (800, 600),
        coarse: false,
    }
}

fn picture(width: usize) -> Answer {
    Ok(ColorImage::filled([width, 3], Color32::RED))
}

impl Rig {
    fn new() -> Self {
        Self {
            ctx: Context::default(),
            previews: Previews::default(),
            composed: RefCell::default(),
            jobs: RefCell::default(),
            fail_compose: false,
        }
    }

    fn tick(&mut self) {
        let (composed, jobs, fail) = (&self.composed, &self.jobs, self.fail_compose);
        self.previews.tick(
            &self.ctx,
            |key| {
                composed.borrow_mut().push(key.clone());
                if fail {
                    return Err(ComposeError::TooLarge);
                }
                Ok(Composed {
                    scene: Scene::new(
                        Vec::new(),
                        eframe::egui::TexturesDelta::default(),
                        1.0,
                        (4, 3),
                        Color32::BLACK,
                    ),
                    composition: vec2(4.0, 3.0),
                })
            },
            |_| {
                let (sender, receiver) = mpsc::channel();
                jobs.borrow_mut().push(sender);
                Pending::from_receiver(receiver)
            },
        );
    }

    /// Answers the oldest job still waiting.
    fn finish(&mut self, answer: Answer) {
        let sender = self.jobs.borrow_mut().remove(0);
        sender.send(answer).expect("the pipeline still listens");
    }

    fn composed(&self) -> Vec<Background> {
        self.composed
            .borrow()
            .iter()
            .map(|key| key.config.background)
            .collect()
    }
}

#[test]
fn asking_for_the_same_picture_twice_asks_once() {
    let mut rig = Rig::new();
    rig.previews.want(key(Background::Dots));
    rig.previews.want(key(Background::Dots));
    assert_eq!(rig.previews.generation(), 1);
    rig.tick();
    rig.tick();
    assert_eq!(rig.composed(), [Background::Dots]);
    assert_eq!(rig.jobs.borrow().len(), 1);
}

#[test]
fn nothing_is_shown_until_the_first_picture_arrives() {
    let mut rig = Rig::new();
    assert!(rig.previews.shown().is_none() && !rig.previews.is_busy());
    rig.previews.want(key(Background::None));
    assert!(rig.previews.is_busy());
    rig.tick();
    assert!(rig.previews.shown().is_none() && rig.previews.in_flight());
    rig.finish(picture(5));
    rig.tick();
    let shown = rig.previews.shown().expect("a picture");
    assert_eq!(shown.pixels, [5, 3]);
    assert_eq!(shown.composition, vec2(4.0, 3.0));
    assert!(!rig.previews.is_busy());
}

#[test]
fn at_most_one_job_runs_and_the_newest_wish_is_started_when_it_ends() {
    let mut rig = Rig::new();
    rig.previews.want(key(Background::Dots));
    rig.tick();
    // Three more wishes while the first is drawn: only the last is kept.
    for background in [
        Background::Luminous,
        Background::Radiant,
        Background::Spectrum,
    ] {
        rig.previews.want(key(background));
        rig.tick();
        assert_eq!(rig.jobs.borrow().len(), 1, "one job at a time");
    }
    assert_eq!(
        rig.composed(),
        [Background::Dots],
        "nothing composed for the overtaken"
    );
    rig.finish(picture(5));
    rig.tick();
    assert_eq!(
        rig.composed(),
        [Background::Dots, Background::Spectrum],
        "latest wins"
    );
    assert_eq!(rig.jobs.borrow().len(), 1);
    rig.finish(picture(6));
    rig.tick();
    assert_eq!(rig.previews.shown().map(|shown| shown.pixels), Some([6, 3]));
    assert!(!rig.previews.is_busy());
    assert_eq!(rig.composed().len(), 2, "nothing left to start");
}

#[test]
fn a_picture_that_finishes_during_a_drag_is_shown_before_the_last_one() {
    let mut rig = Rig::new();
    rig.previews.want(key(Background::Dots));
    rig.tick();
    rig.previews.want(key(Background::Luminous));
    rig.finish(picture(5));
    rig.tick();
    assert_eq!(rig.previews.shown().map(|shown| shown.pixels), Some([5, 3]));
    assert!(rig.previews.in_flight(), "and the newest is under way");
}

#[test]
fn the_old_picture_stays_until_a_newer_one_arrives_in_the_same_texture() {
    let mut rig = Rig::new();
    rig.previews.want(key(Background::Dots));
    rig.tick();
    rig.finish(picture(5));
    rig.tick();
    let first = rig.previews.shown().expect("a picture");

    rig.previews.want(key(Background::Luminous));
    rig.tick();
    assert_eq!(
        rig.previews.shown(),
        Some(first),
        "unchanged while it is drawn"
    );
    rig.finish(picture(7));
    rig.tick();
    let second = rig.previews.shown().expect("a picture");
    assert_eq!(second.texture, first.texture, "one texture, replaced");
    assert_eq!(second.pixels, [7, 3]);
    assert!(second.generation > first.generation);
}

#[test]
fn a_picture_from_before_a_restart_is_dropped() {
    let mut rig = Rig::new();
    rig.previews.want(key(Background::Dots));
    rig.tick();
    rig.previews.restart();
    rig.previews.want(key(Background::Dots));
    rig.tick();
    assert_eq!(
        rig.jobs.borrow().len(),
        1,
        "the old job still holds the worker"
    );
    rig.finish(picture(5));
    rig.tick();
    assert!(
        rig.previews.shown().is_none(),
        "the old capture's picture is not shown"
    );
    // The new wish was started when the old job ended.
    assert!(rig.previews.in_flight());
    assert_eq!(rig.composed(), [Background::Dots, Background::Dots]);
    rig.finish(picture(9));
    rig.tick();
    assert_eq!(rig.previews.shown().map(|shown| shown.pixels), Some([9, 3]));
}

#[test]
fn a_restart_keeps_the_picture_on_screen_until_the_new_one() {
    let mut rig = Rig::new();
    rig.previews.want(key(Background::Dots));
    rig.tick();
    rig.finish(picture(5));
    rig.tick();
    let before = rig.previews.shown();
    rig.previews.restart();
    rig.previews.want(key(Background::Dots));
    rig.tick();
    assert_eq!(rig.previews.shown(), before);
}

#[test]
fn a_scene_that_cannot_be_composed_is_a_failure_that_the_next_good_one_clears() {
    let mut rig = Rig::new();
    rig.fail_compose = true;
    rig.previews.want(key(Background::Dots));
    rig.tick();
    assert_eq!(rig.previews.failure(), Some(ComposeError::TooLarge));
    assert!(rig.jobs.borrow().is_empty() && !rig.previews.is_busy());

    rig.fail_compose = false;
    rig.previews.want(key(Background::Luminous));
    rig.tick();
    assert_eq!(rig.previews.failure(), None);
    assert!(rig.previews.in_flight());
}

#[test]
fn a_picture_the_rasterizer_refuses_or_a_job_that_vanishes_is_a_failure() {
    let mut rig = Rig::new();
    rig.previews.want(key(Background::Dots));
    rig.tick();
    rig.finish(Err(RasterError::TooLarge));
    rig.tick();
    assert_eq!(rig.previews.failure(), Some(ComposeError::TooLarge));
    assert!(rig.previews.shown().is_none());

    rig.previews.want(key(Background::Luminous));
    rig.tick();
    drop(rig.jobs.borrow_mut().remove(0));
    rig.tick();
    assert_eq!(
        rig.previews.failure(),
        Some(ComposeError::TooLarge),
        "lost, not hung"
    );
    assert!(!rig.previews.is_busy());
}

#[test]
fn a_coarse_picture_asks_for_half_the_pixels_each_way() {
    let mut full = key(Background::None);
    assert_eq!(full.target_px(), (800, 600));
    full.coarse = true;
    assert_eq!(full.target_px(), (400, 300));
    full.max_px = (1, 1);
    assert_eq!(full.target_px(), (1, 1));
    // Coarse and full are different wishes, so leaving a drag draws the full one again.
    let mut rig = Rig::new();
    let mut coarse = key(Background::None);
    coarse.coarse = true;
    rig.previews.want(coarse);
    rig.previews.want(key(Background::None));
    assert_eq!(rig.previews.generation(), 2);
}

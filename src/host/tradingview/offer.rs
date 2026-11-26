//! The offer to import from TradingView, made once: on the first launch that finds something of
//! TradingView's to bring over. After that the import is only opened by hand.

use crate::lock::lock;
use crate::model::tradingview::library::Contents;
use crate::service::tradingview;
use crate::service::work_queue::{self, Pending};
use crate::store::SharedStore;
use crate::ui::tradingview::offer::{self as sheet, Choice};
use eframe::egui;
use std::sync::{Arc, Mutex};

/// `setting`: the offer was made, or the import was opened by hand. This computer's own, like the
/// data it is about.
const OFFERED: &str = "tradingview.offered";

/// Remembers that the import needs no introduction any more.
pub fn mark_offered(store: &SharedStore) {
    // A write that fails leaves the offer to be made again, which loses nothing.
    let _ = lock(store).set_setting(OFFERED, &true);
}

fn offered(store: &SharedStore) -> bool {
    lock(store).setting(OFFERED).unwrap_or(false)
}

enum State {
    /// Reading TradingView's data, off the frame.
    Looking(Pending<Option<Contents>>),
    /// Something was found; no window has shown the offer yet.
    Found(Contents),
    /// On screen.
    Asking(Contents),
    Over,
}

/// Shared by every window; the focused one draws it.
#[derive(Clone)]
pub struct ImportOffer {
    store: SharedStore,
    state: Arc<Mutex<State>>,
}

impl ImportOffer {
    /// Looks for something to offer, unless the offer was made before.
    pub fn new(store: SharedStore, ctx: &egui::Context) -> Self {
        Self::with_look(store, ctx, || {
            tradingview::discover().map(|library| library.contents())
        })
    }

    fn with_look(
        store: SharedStore,
        ctx: &egui::Context,
        look: impl FnOnce() -> Option<Contents> + Send + 'static,
    ) -> Self {
        let state = if offered(&store) {
            State::Over
        } else {
            State::Looking(work_queue::SNAPSHOTS.start(Some(ctx), look))
        };
        Self {
            store,
            state: Arc::new(Mutex::new(state)),
        }
    }

    /// Takes in what the look found; `true` once, when there is now an offer for a window to
    /// show.
    pub fn poll(&self) -> bool {
        let mut state = lock(&self.state);
        let State::Looking(job) = &mut *state else {
            return false;
        };
        let Some(found) = job.poll_or(|| None) else {
            return false;
        };
        *state = found.map_or(State::Over, State::Found);
        matches!(*state, State::Found(_))
    }

    /// Ends the offer: the import is being opened some other way.
    pub fn withdraw(&self) {
        *lock(&self.state) = State::Over;
    }

    /// Draws the offer, if there is one, in the window calling; `true` when the import is wanted.
    /// It counts as made as soon as it is on screen.
    pub fn show(&self, ctx: &egui::Context) -> bool {
        let mut state = lock(&self.state);
        if let State::Found(found) = &mut *state {
            let found = std::mem::take(found);
            // The import may have been opened by hand since the look began.
            *state = if offered(&self.store) {
                State::Over
            } else {
                mark_offered(&self.store);
                State::Asking(found)
            };
        }
        let State::Asking(found) = &*state else {
            return false;
        };
        let Some(choice) = sheet::show(ctx, found) else {
            return false;
        };
        *state = State::Over;
        choice == Choice::Import
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    fn found() -> Contents {
        Contents {
            layouts: vec!["Main".into()],
            drawings: 2,
            ..Contents::default()
        }
    }

    fn store() -> SharedStore {
        Arc::new(Mutex::new(Store::open_in_memory().unwrap()))
    }

    /// An offer whose look has answered.
    fn looked(store: &SharedStore, found: Option<Contents>) -> ImportOffer {
        let offer = ImportOffer::with_look(store.clone(), &egui::Context::default(), move || found);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while matches!(*lock(&offer.state), State::Looking(_)) {
            offer.poll();
            assert!(std::time::Instant::now() < deadline, "the look never ended");
            std::thread::yield_now();
        }
        offer
    }

    fn key(key: egui::Key) -> Vec<egui::Event> {
        vec![egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }]
    }

    fn click(at: egui::Pos2) -> Vec<egui::Event> {
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        vec![egui::Event::PointerMoved(at), button(true), button(false)]
    }

    /// Draws one frame; the offer's answer, and whether a sheet was on it.
    fn frame(ctx: &egui::Context, offer: &ImportOffer, events: Vec<egui::Event>) -> (bool, bool) {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200.0, 800.0),
            )),
            events,
            ..Default::default()
        };
        let mut wanted = false;
        let mut output = ctx.run_ui(raw, |ui| wanted = offer.show(ui.ctx()));
        output.textures_delta.clear();
        (
            wanted,
            ctx.memory(|memory| memory.top_modal_layer()).is_some(),
        )
    }

    #[test]
    fn what_is_found_is_offered_once() {
        let store = store();
        let offer = looked(&store, Some(found()));
        let ctx = egui::Context::default();
        assert!(!offered(&store), "nothing was on screen yet");
        frame(&ctx, &offer, Vec::new());
        let (_, open) = frame(&ctx, &offer, Vec::new());
        assert!(open);
        // Made once it is on screen, even if the app quits before an answer.
        assert!(offered(&store));
        assert!(matches!(
            *lock(&looked(&store, Some(found())).state),
            State::Over
        ));
    }

    /// An offer already on screen.
    fn on_screen(ctx: &egui::Context) -> ImportOffer {
        let offer = looked(&store(), Some(found()));
        frame(ctx, &offer, Vec::new());
        frame(ctx, &offer, Vec::new());
        offer
    }

    #[test]
    fn it_stays_until_answered_not_by_a_stray_enter_or_a_click_beside_it() {
        let ctx = egui::Context::default();
        let offer = on_screen(&ctx);
        let (wanted, open) = frame(&ctx, &offer, key(egui::Key::Enter));
        assert!(!wanted && open);
        let (wanted, open) = frame(&ctx, &offer, click(egui::pos2(5.0, 5.0)));
        assert!(!wanted && open);
        let (wanted, _) = frame(&ctx, &offer, key(egui::Key::Escape));
        assert!(!wanted);
        assert_eq!(frame(&ctx, &offer, Vec::new()), (false, false));
    }

    #[test]
    fn the_keyboard_can_take_the_offer() {
        let ctx = egui::Context::default();
        let offer = on_screen(&ctx);
        frame(&ctx, &offer, key(egui::Key::Tab));
        frame(&ctx, &offer, Vec::new());
        let (wanted, _) = frame(&ctx, &offer, key(egui::Key::Enter));
        assert!(wanted);
    }

    #[test]
    fn nothing_found_offers_nothing_and_asks_again_next_launch() {
        let store = store();
        let offer = looked(&store, None);
        assert_eq!(
            frame(&egui::Context::default(), &offer, Vec::new()),
            (false, false)
        );
        assert!(!offered(&store));
    }

    #[test]
    fn an_import_opened_by_hand_first_ends_the_offer() {
        let store = store();
        let offer = looked(&store, Some(found()));
        mark_offered(&store);
        assert_eq!(
            frame(&egui::Context::default(), &offer, Vec::new()),
            (false, false)
        );

        let withdrawn = looked(&self::store(), Some(found()));
        withdrawn.withdraw();
        assert_eq!(
            frame(&egui::Context::default(), &withdrawn, Vec::new()),
            (false, false)
        );
    }
}

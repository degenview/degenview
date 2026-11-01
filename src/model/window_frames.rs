//! Remembers where each window sits for each arrangement of displays, so it can be put back when
//! the OS moves it.
//!
//! When the Mac sleeps its displays drop out one at a time. macOS moves the windows onto the ones
//! left and clamps them to that display's visible area, and when the displays come back it does not
//! return every window to its place and size. A window's frame is therefore saved only while the
//! arrangement has been steady, under the arrangement it was seen in, and restored when that
//! arrangement returns (or after a wake, when the displays may have come and gone unseen).
//!
//! The settle times and the drift tolerance are inferred, not measured on real hardware.

use crate::model::tab_windows::WindowId;
use std::collections::HashMap;

/// The screens as `[x, y, width, height]` in whole points, sorted. Empty means the platform
/// cannot tell, and nothing is remembered.
pub type Layout = Vec<[i32; 4]>;

/// A window's frame in points: where its outer rect starts and how big its content is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub position: [f32; 2],
    pub size: [f32; 2],
}

impl Frame {
    fn differs_from(&self, other: &Frame) -> bool {
        self.position
            .iter()
            .chain(&self.size)
            .zip(other.position.iter().chain(&other.size))
            .any(|(a, b)| (a - b).abs() > DRIFT)
    }
}

/// A window as seen this tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub id: WindowId,
    pub frame: Frame,
    /// Minimized, maximized, fullscreen or following the cursor in a tab drag: its frame is not
    /// one the user chose to keep, and it is neither saved nor restored.
    pub transient: bool,
}

/// More than this gap between two ticks means the Mac slept (the clock is wall time: a monotonic
/// one stops while asleep).
const WAKE_GAP_MS: u64 = 3_000;
/// The arrangement must have been steady this long before a saved frame is put back, so the OS
/// has finished moving things.
const RESTORE_SETTLE_MS: u64 = 1_000;
/// How long after that a restore keeps correcting windows the OS moves again.
const RESTORE_WINDOW_MS: u64 = 6_000;
/// The arrangement must have been steady this long before frames are saved.
const SAVE_SETTLE_MS: u64 = 2_000;
/// A frame closer than this to the saved one counts as the same.
const DRIFT: f32 = 1.0;

struct Pending {
    frames: HashMap<WindowId, Frame>,
    /// When the arrangement changed or the Mac woke.
    since: u64,
    /// A restore was sent and the windows then matched.
    settled: bool,
}

#[derive(Default)]
pub struct FrameMemory {
    layout: Layout,
    layout_since: u64,
    last_tick: Option<u64>,
    saved: HashMap<Layout, HashMap<WindowId, Frame>>,
    pending: Option<Pending>,
    /// A restore was sent for the current `pending`.
    sent: bool,
}

impl FrameMemory {
    /// One tick at wall-clock time `now` (milliseconds). Returns the windows to move back.
    pub fn observe(
        &mut self,
        now: u64,
        layout: Layout,
        windows: &[Sample],
    ) -> Vec<(WindowId, Frame)> {
        if layout.is_empty() {
            return Vec::new();
        }
        let woke = self
            .last_tick
            .is_some_and(|last| now.saturating_sub(last) > WAKE_GAP_MS);
        self.last_tick = Some(now);
        if layout != self.layout || woke {
            self.layout = layout;
            self.layout_since = now;
            self.sent = false;
            self.pending = self.saved.get(&self.layout).map(|frames| Pending {
                frames: frames.clone(),
                since: now,
                settled: false,
            });
        }
        let steady = now.saturating_sub(self.layout_since);

        if let Some(pending) = &mut self.pending {
            if steady < RESTORE_SETTLE_MS {
                return Vec::new();
            }
            let moves: Vec<(WindowId, Frame)> = windows
                .iter()
                .filter(|window| !window.transient)
                .filter_map(|window| {
                    let saved = pending.frames.get(&window.id)?;
                    saved
                        .differs_from(&window.frame)
                        .then_some((window.id, *saved))
                })
                .collect();
            if !moves.is_empty() {
                self.sent = true;
            } else if self.sent {
                pending.settled = true;
            }
            // Windows that match are not enough to stop: one that has not redrawn since the wake
            // still reports where it was before.
            if pending.settled || now.saturating_sub(pending.since) > RESTORE_WINDOW_MS {
                self.pending = None;
            }
            return moves;
        }

        if steady >= SAVE_SETTLE_MS {
            let saved = self.saved.entry(self.layout.clone()).or_default();
            for window in windows.iter().filter(|window| !window.transient) {
                saved.insert(window.id, window.frame);
            }
        }
        for frames in self.saved.values_mut() {
            frames.retain(|id, _| windows.iter().any(|window| window.id == *id));
        }
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(x: f32, w: f32) -> Frame {
        Frame {
            position: [x, 40.0],
            size: [w, 900.0],
        }
    }

    fn window(id: WindowId, frame: Frame) -> Sample {
        Sample {
            id,
            frame,
            transient: false,
        }
    }

    fn three() -> Layout {
        vec![
            [0, 0, 1080, 1920],
            [1080, 0, 3840, 2160],
            [4920, 0, 1512, 982],
        ]
    }

    fn one() -> Layout {
        vec![[0, 0, 1512, 982]]
    }

    /// Settles `memory` on `layout` with `windows`, as a run of ticks 250 ms apart.
    fn settle(memory: &mut FrameMemory, from: u64, layout: &Layout, windows: &[Sample]) -> u64 {
        let mut now = from;
        while now < from + 3_000 {
            assert!(memory.observe(now, layout.clone(), windows).is_empty());
            now += 250;
        }
        now
    }

    #[test]
    fn displaced_windows_go_back_when_the_displays_return() {
        let mut memory = FrameMemory::default();
        let home = [
            window(1, frame(100.0, 1000.0)),
            window(2, frame(1200.0, 1400.0)),
        ];
        let now = settle(&mut memory, 0, &three(), &home);
        // Sleep: the displays drop and macOS crowds both windows onto the one left.
        let crowded = [window(1, frame(0.0, 900.0)), window(2, frame(30.0, 900.0))];
        let now = settle(&mut memory, now + 100, &one(), &crowded);
        // They return, and the windows are not put back.
        let shrunk = [
            window(1, frame(100.0, 980.0)),
            window(2, frame(1190.0, 1380.0)),
        ];
        let moves = (0..8)
            .map(|step| memory.observe(now + 100 + step * 250, three(), &shrunk))
            .find(|moves| !moves.is_empty())
            .expect("a restore");
        assert_eq!(moves.len(), 2);
        assert!(moves.contains(&(1, frame(100.0, 1000.0))));
        assert!(moves.contains(&(2, frame(1200.0, 1400.0))));
    }

    #[test]
    fn what_the_os_moved_is_not_saved_as_a_choice() {
        let mut memory = FrameMemory::default();
        let home = [window(1, frame(100.0, 1000.0))];
        let now = settle(&mut memory, 0, &three(), &home);
        // The displays vanish; the window is crowded onto the rest and stays there.
        let crowded = [window(1, frame(0.0, 900.0))];
        let now = settle(&mut memory, now, &one(), &crowded);
        // Back on all three, the saved frame is still the original.
        let moves = memory.observe(now + 1_000, three(), &crowded);
        assert!(moves.is_empty(), "restores wait for the layout to settle");
        let moves = memory.observe(now + 2_100, three(), &crowded);
        assert_eq!(moves, vec![(1, frame(100.0, 1000.0))]);
    }

    #[test]
    fn a_wake_with_the_same_displays_still_restores() {
        let mut memory = FrameMemory::default();
        let home = [window(1, frame(100.0, 1000.0))];
        let now = settle(&mut memory, 0, &three(), &home);
        // Asleep for an hour; the displays came back as they were but the window did not.
        let woke = now + 3_600_000;
        let shrunk = [window(1, frame(110.0, 980.0))];
        assert!(memory.observe(woke, three(), &shrunk).is_empty());
        let moves = memory.observe(woke + 1_100, three(), &shrunk);
        assert_eq!(moves, vec![(1, frame(100.0, 1000.0))]);
        // Once it matches again the restore ends and the next frame the user picks is saved.
        let back = [window(1, frame(100.0, 1000.0))];
        assert!(memory.observe(woke + 1_350, three(), &back).is_empty());
        let moved = [window(1, frame(300.0, 1000.0))];
        let now = settle(&mut memory, woke + 1_600, &three(), &moved);
        assert_eq!(memory.saved[&three()][&1], frame(300.0, 1000.0));
        assert!(memory.observe(now, three(), &moved).is_empty());
    }

    #[test]
    fn a_window_that_has_not_redrawn_does_not_end_the_restore() {
        let mut memory = FrameMemory::default();
        let home = [window(1, frame(100.0, 1000.0))];
        let now = settle(&mut memory, 0, &three(), &home);
        let woke = now + 600_000;
        // Hidden through the wake: it reports its old frame until it redraws.
        assert!(memory.observe(woke, three(), &home).is_empty());
        assert!(memory.observe(woke + 1_500, three(), &home).is_empty());
        assert!(memory.observe(woke + 3_000, three(), &home).is_empty());
        let shrunk = [window(1, frame(110.0, 980.0))];
        let moves = memory.observe(woke + 3_250, three(), &shrunk);
        assert_eq!(moves, vec![(1, frame(100.0, 1000.0))]);
    }

    #[test]
    fn a_frame_the_user_picked_is_kept() {
        let mut memory = FrameMemory::default();
        let now = settle(&mut memory, 0, &three(), &[window(1, frame(100.0, 1000.0))]);
        let moved = [window(1, frame(500.0, 1200.0))];
        let now = settle(&mut memory, now, &three(), &moved);
        // The layout changes and returns: the moved frame is the one that comes back.
        let now = settle(&mut memory, now, &one(), &[window(1, frame(0.0, 900.0))]);
        let moves = memory.observe(now + 1_100, three(), &[window(1, frame(0.0, 900.0))]);
        assert!(moves.is_empty());
        let moves = memory.observe(now + 2_200, three(), &[window(1, frame(0.0, 900.0))]);
        assert_eq!(moves, vec![(1, frame(500.0, 1200.0))]);
    }

    #[test]
    fn transient_windows_are_neither_saved_nor_restored() {
        let mut memory = FrameMemory::default();
        let mut maximized = window(1, frame(0.0, 3840.0));
        maximized.transient = true;
        let now = settle(&mut memory, 0, &three(), &[window(1, frame(100.0, 1000.0))]);
        let now = settle(&mut memory, now, &three(), &[maximized]);
        // A wake: the maximized window is left alone, and its size was never saved.
        let woke = now + 100_000;
        assert!(memory.observe(woke, three(), &[maximized]).is_empty());
        assert!(
            memory
                .observe(woke + 1_100, three(), &[maximized])
                .is_empty()
        );
    }

    #[test]
    fn a_window_within_the_tolerance_is_not_moved() {
        let mut memory = FrameMemory::default();
        let now = settle(&mut memory, 0, &three(), &[window(1, frame(100.0, 1000.0))]);
        let woke = now + 100_000;
        let nudged = [window(1, frame(100.5, 1000.5))];
        assert!(memory.observe(woke, three(), &nudged).is_empty());
        assert!(memory.observe(woke + 1_100, three(), &nudged).is_empty());
    }

    #[test]
    fn closed_windows_are_forgotten_and_an_unknown_platform_does_nothing() {
        let mut memory = FrameMemory::default();
        let now = settle(&mut memory, 0, &three(), &[window(1, frame(100.0, 1000.0))]);
        memory.observe(now, three(), &[window(2, frame(0.0, 800.0))]);
        assert!(!memory.saved[&three()].contains_key(&1));
        assert!(memory.observe(now + 10, Layout::new(), &[]).is_empty());
    }
}

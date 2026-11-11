//! Tab Linking's viewport path through real pointer events: two tabs, one chart each, drawn the way
//! the workspace draws a tab's focused chart, with the app's hub carrying what the user does in one
//! to the other. The workspace's own wiring (`render_grid`) is the same calls in the same order.

use super::*;
use crate::model::chart::sync::{SyncHub, TimeAxis, TimeRange};
use crate::model::chart::viewport::HorizontalViewport;
use crate::model::tab::link::{
    LinkColor, LinkHub, LinkSignal, LinkTable, LinkToggles, Member as LinkMember,
};
use crate::model::{IndicatorConfig, Tool};
use crate::ui::replay::overlay::ReplayChartFrame;
use eframe::egui::Modifiers;

const HOUR: i64 = 3_600_000;
const T0: i64 = 1_700_000_000_000;
/// The "chart" the group navigates as.
const ORIGIN: u64 = u64::MAX;

fn series(start: i64, step: i64, count: usize) -> Vec<Kline> {
    (0..count)
        .map(|index| {
            let close = 100.0 + (index as f64 * 0.3).sin() * 20.0;
            Kline {
                open_time: start + index as i64 * step,
                close_time: start + (index as i64 + 1) * step - 1,
                open: close - 1.0,
                high: close + 2.0,
                low: close - 3.0,
                close,
                volume: 10.0,
            }
        })
        .collect()
}

const fn toggles(time: bool, date_range: bool) -> LinkToggles {
    LinkToggles {
        symbol: false,
        interval: false,
        time,
        date_range,
    }
}

struct Tab {
    card: ChartCard,
    candles: Vec<Kline>,
    hub: SyncHub,
    left: f32,
    width: f32,
    rect: Rect,
    /// What the tab would announce this frame.
    said: Vec<LinkSignal>,
    announced: usize,
}

struct Tabs {
    ctx: egui::Context,
    tabs: Vec<Tab>,
    app: LinkHub,
    table: LinkTable,
    toggles: LinkToggles,
    time: f64,
}

impl Tabs {
    fn new(toggles: LinkToggles, bars: [Vec<Kline>; 2]) -> Self {
        let mut table = LinkTable::default();
        table.set(LinkColor::Orange, toggles);
        let [first, second] = bars;
        let tab = |candles, left, width| {
            let mut card = ChartCard::default();
            card.set_timeframe(Timeframe::Hour);
            Tab {
                card,
                candles,
                hub: SyncHub::default(),
                left,
                width,
                rect: Rect::NOTHING,
                said: Vec::new(),
                announced: 0,
            }
        };
        let mut tabs = Self {
            ctx: egui::Context::default(),
            tabs: vec![tab(first, 0.0, 700.0), tab(second, 710.0, 420.0)],
            app: LinkHub::default(),
            table,
            toggles,
            time: 0.0,
        };
        for _ in 0..3 {
            tabs.frame(Vec::new());
        }
        tabs
    }

    fn pair(toggles: LinkToggles) -> Self {
        let bars = series(T0, HOUR, 800);
        Self::new(toggles, [bars.clone(), bars])
    }

    /// One frame of each tab's window, then the app's tick.
    fn frame(&mut self, events: Vec<egui::Event>) {
        self.time += 0.016;
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1140.0, 520.0))),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let settings = self.toggles.viewport();
        let tabs = &mut self.tabs;
        let mut output = self.ctx.run_ui(input, |ui| {
            for tab in tabs.iter_mut() {
                let rect =
                    Rect::from_min_size(Pos2::new(tab.left, 0.0), Vec2::new(tab.width, 520.0));
                ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                    // A chart with no bars keeps the group's event for when it has some.
                    if !tab.candles.is_empty() {
                        tab.card.follow_sync(
                            &mut tab.hub,
                            1,
                            settings,
                            DataSource::Binance,
                            &tab.candles,
                        );
                    }
                    let mut frame = DrawingFrame::new(&[], Tool::Cursor, None, 100);
                    let mut replay = ReplayChartFrame {
                        selecting: false,
                        active: false,
                        picked_ms: None,
                        picked_range: None,
                    };
                    let pointer = tab.card.body(
                        ui,
                        "BTCUSDT",
                        &tab.candles,
                        false,
                        None,
                        PriceScaleMode::Regular,
                        DataSource::Binance,
                        &IndicatorConfig::default(),
                        None,
                        1,
                        &mut frame,
                        &mut replay,
                    );
                    tab.rect = pointer.rect.unwrap_or(Rect::NOTHING);
                    if let Some(change) = tab.card.link_change() {
                        tab.said.push(LinkSignal::View(change));
                        tab.announced += 1;
                    }
                    // As the workspace does: a chart with no bars was not asked to follow, so the
                    // frame does not count towards ending a zoom burst.
                    if !tab.candles.is_empty() {
                        tab.hub.end_frame();
                    }
                });
            }
        });
        output.textures_delta.clear();
        self.tick();
    }

    /// `DegenViewApp::tick_tab_links` for two tabs.
    fn tick(&mut self) {
        let members: Vec<LinkMember> = (0..self.tabs.len())
            .map(|index| LinkMember {
                tab: index as u64 + 1,
                color: Some(LinkColor::Orange),
                market: None,
                timeframe: Timeframe::Hour,
            })
            .collect();
        let mut signals = Vec::new();
        for (index, tab) in self.tabs.iter_mut().enumerate() {
            // Newest wins, as the outbox keeps it.
            if let Some(last) = tab.said.pop() {
                signals.push((index as u64 + 1, vec![last]));
            }
            tab.said.clear();
        }
        for delivery in self.app.tick(&members, &self.table, signals) {
            if let LinkSignal::View(change) = delivery.signal {
                self.tabs[delivery.target as usize - 1]
                    .hub
                    .publish(ORIGIN, change);
            }
        }
    }

    fn frames(&mut self, count: usize) {
        for _ in 0..count {
            self.frame(Vec::new());
        }
    }

    fn button(at: Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        }
    }

    fn drag(&mut self, from: Pos2, delta: Vec2) {
        self.frame(vec![egui::Event::PointerMoved(from)]);
        self.frame(vec![Self::button(from, true)]);
        for step in 1..=6 {
            self.frame(vec![egui::Event::PointerMoved(
                from + delta * (step as f32 / 6.0),
            )]);
        }
        self.frame(vec![Self::button(from + delta, false)]);
        self.frames(4);
    }

    fn wheel(&mut self, at: Pos2, delta: Vec2) {
        self.frame(vec![egui::Event::PointerMoved(at)]);
        self.frame(vec![egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta,
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
        }]);
        self.frames(6);
    }

    fn in_plot(&self, tab: usize) -> Pos2 {
        let rect = self.tabs[tab].rect;
        Pos2::new(rect.left() + rect.width() * 0.4, rect.top() + 180.0)
    }

    fn view(&self, tab: usize) -> HorizontalViewport {
        self.tabs[tab].card.horizontal()
    }

    fn set_view(&mut self, tab: usize, visible: usize, right_offset: f64) {
        self.tabs[tab].card.viewport.horizontal = HorizontalViewport {
            visible,
            right_offset,
        };
    }

    fn range(&self, tab: usize) -> TimeRange {
        let tab = &self.tabs[tab];
        TimeAxis::new(&tab.candles)
            .range_of(&tab.card.horizontal())
            .expect("bars")
    }
}

fn near(a: f64, b: f64, tolerance: f64) -> bool {
    (a - b).abs() <= tolerance
}

#[test]
fn time_on_a_pan_in_one_tab_puts_the_other_at_the_same_time_and_keeps_its_zoom() {
    let mut tabs = Tabs::pair(toggles(true, false));
    tabs.set_view(1, 300, 0.0);
    let zoom = tabs.view(1).visible;
    tabs.drag(tabs.in_plot(0), Vec2::new(150.0, 0.0));
    assert!(tabs.view(0).right_offset > 0.0, "the dragged tab moved");
    assert_eq!(tabs.range(0).right, tabs.range(1).right);
    assert_eq!(tabs.view(1).visible, zoom, "its zoom stays its own");
}

#[test]
fn with_time_and_date_range_off_nothing_crosses() {
    let mut tabs = Tabs::pair(toggles(false, false));
    tabs.drag(tabs.in_plot(0), Vec2::new(150.0, 0.0));
    assert!(tabs.view(0).right_offset > 0.0);
    assert_eq!(tabs.view(1).right_offset, 0.0);
    assert_eq!(tabs.tabs[1].announced, 0);
}

#[test]
fn date_range_on_alone_carries_the_zoom_and_keeps_the_position() {
    let mut tabs = Tabs::pair(toggles(false, true));
    tabs.set_view(1, 200, 20.0);
    let position = tabs.range(1).right;
    let before = tabs.view(1).visible;
    tabs.wheel(tabs.in_plot(0), Vec2::new(0.0, 60.0));
    assert_ne!(tabs.view(0).visible, 120, "the wheel zoomed the leader");
    assert_ne!(tabs.view(1).visible, before, "the follower zoomed");
    assert!(
        near(tabs.range(1).right, position, 1.0),
        "its position is its own"
    );
}

#[test]
fn time_and_date_range_together_show_the_same_time_range_after_a_zoom() {
    let mut tabs = Tabs::pair(toggles(true, true));
    tabs.wheel(tabs.in_plot(0), Vec2::new(0.0, 60.0));
    let (a, b) = (tabs.range(0), tabs.range(1));
    assert!(near(a.right, b.right, 1.0), "{a:?} {b:?}");
    let slot = 60_000.0 * 60.0;
    assert!(near(a.left, b.left, slot), "{a:?} {b:?}");
}

#[test]
fn a_follower_never_announces_what_it_was_told_so_nothing_echoes() {
    let mut tabs = Tabs::pair(toggles(true, true));
    tabs.drag(tabs.in_plot(0), Vec2::new(150.0, 0.0));
    let announced = tabs.tabs[1].announced;
    assert_eq!(announced, 0, "tab 2 only followed");
    // Left alone, neither tab moves and neither says anything, however many frames pass.
    let (a, b) = (tabs.view(0), tabs.view(1));
    let said = tabs.tabs[0].announced;
    tabs.frames(200);
    assert_eq!((tabs.view(0), tabs.view(1)), (a, b));
    assert_eq!(tabs.tabs[0].announced, said);
    assert_eq!(tabs.tabs[1].announced, 0);
}

#[test]
fn either_tab_can_lead() {
    let mut tabs = Tabs::pair(toggles(true, false));
    tabs.drag(tabs.in_plot(0), Vec2::new(150.0, 0.0));
    tabs.drag(tabs.in_plot(1), Vec2::new(-90.0, 0.0));
    assert_eq!(tabs.range(0).right, tabs.range(1).right);
}

#[test]
fn a_chart_still_loading_keeps_the_event_until_it_has_bars() {
    let leader = series(T0, HOUR, 800);
    let mut tabs = Tabs::new(toggles(true, false), [leader, Vec::new()]);
    tabs.drag(tabs.in_plot(0), Vec2::new(150.0, 0.0));
    let target = tabs.range(0).right;
    // The other tab's candles arrive afterwards: it goes to the group's time without any new gesture.
    tabs.tabs[1].candles = series(T0, HOUR, 800);
    tabs.frames(4);
    assert!(near(tabs.range(1).right, target, 1.0));
}

#[test]
fn a_different_interval_matches_by_time_not_by_bar() {
    let hourly = series(T0, HOUR, 800);
    let daily = series(T0, 24 * HOUR, 200);
    let mut tabs = Tabs::new(toggles(true, false), [hourly, daily]);
    tabs.drag(tabs.in_plot(0), Vec2::new(150.0, 0.0));
    let (a, b) = (tabs.range(0).right, tabs.range(1).right);
    assert!(near(a, b, 24.0 * HOUR as f64), "{a} vs {b}");
}

#[test]
fn a_flood_of_frames_stays_bounded() {
    let mut tabs = Tabs::pair(toggles(true, true));
    tabs.drag(tabs.in_plot(0), Vec2::new(300.0, 0.0));
    tabs.wheel(tabs.in_plot(0), Vec2::new(0.0, 40.0));
    tabs.wheel(tabs.in_plot(0), Vec2::new(0.0, -40.0));
    assert!(tabs.tabs[0].said.is_empty() && tabs.tabs[1].said.is_empty());
}

#[test]
fn a_zoom_made_while_the_other_tab_is_loading_reaches_it_when_its_bars_arrive() {
    let leader = series(T0, HOUR, 800);
    let mut tabs = Tabs::new(toggles(false, true), [leader, Vec::new()]);
    tabs.wheel(tabs.in_plot(0), Vec2::new(0.0, 60.0));
    let leader_zoom = tabs.view(0).visible;
    assert_ne!(
        leader_zoom,
        crate::model::chart::viewport::DEFAULT_VISIBLE,
        "the leader zoomed"
    );
    // Many frames pass with the other tab still empty.
    tabs.frames(30);
    tabs.tabs[1].candles = series(T0, HOUR, 800);
    tabs.frames(4);
    assert_ne!(
        tabs.view(1).visible,
        crate::model::chart::viewport::DEFAULT_VISIBLE,
        "the follower took the zoom"
    );
}

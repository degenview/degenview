//! The four price-scale modes on a chart: what moves (nothing, but on a logarithmic scale), what
//! the axis says, and that every price a chart hands out stays a real one.

use super::canvas_tests::{Harness, axis_texts, series};
use super::*;
use crate::model::ChartCanvas;
use eframe::egui::vec2;

fn plot() -> Rect {
    Rect::from_min_size(Pos2::ZERO, vec2(600.0, 400.0))
}

fn view(card: &ChartCard, candles: &[Kline], mode: PriceScaleMode) -> View {
    card.view(candles, plot(), mode, ChartType::Candles)
        .expect("a view")
}

fn close(left: f64, right: f64) -> bool {
    (left - right).abs() < 1e-6 * left.abs().max(right.abs()).max(1.0)
}

#[test]
fn percent_and_indexed_move_no_pixel_and_regular_stays_what_it_was() {
    let (card, candles) = (ChartCard::default(), series(300));
    let regular = view(&card, &candles, PriceScaleMode::Regular);
    for mode in [PriceScaleMode::Percent, PriceScaleMode::IndexedTo100] {
        let other = view(&card, &candles, mode);
        assert_eq!((regular.start, regular.end), (other.start, other.end));
        assert_eq!(regular.scale.minimum, other.scale.minimum, "{mode:?}");
        assert_eq!(regular.scale.maximum, other.scale.maximum, "{mode:?}");
        assert!(!other.scale.logarithmic, "{mode:?} is not a logarithm");
        for price in [60.0, 99.5, 123.4, 200.0] {
            assert_eq!(
                regular.scale.y(price),
                other.scale.y(price),
                "{mode:?} {price}"
            );
        }
    }
}

#[test]
fn every_mode_hands_back_the_real_price_under_the_pointer() {
    let (card, candles) = (ChartCard::default(), series(300));
    for mode in PriceScaleMode::ALL {
        let scale = view(&card, &candles, mode).scale;
        for price in [scale.minimum + 1.0, 123.4, scale.maximum - 1.0] {
            let y = scale.y(price);
            assert!(
                close(scale.price(y), price),
                "{mode:?}: {price} came back as {}",
                scale.price(y)
            );
        }
    }
}

#[test]
fn a_logarithmic_scale_gives_equal_ratios_equal_heights() {
    let scale = PriceScale {
        logarithmic: true,
        mode: PriceScaleMode::Logarithmic,
        ..PriceScale::linear(10.0, 40_000.0, 0.0, 400.0)
    };
    let heights: Vec<f32> = [10.0, 100.0, 1_000.0]
        .into_iter()
        .map(|low| scale.y(low) - scale.y(low * 2.0))
        .collect();
    assert!((heights[0] - heights[1]).abs() < 1e-3, "{heights:?}");
    assert!((heights[1] - heights[2]).abs() < 1e-3, "{heights:?}");
    // Not the linear map: a unit at the bottom is a larger share than a unit at the top.
    assert!(scale.y(10.0) - scale.y(11.0) > scale.y(39_000.0) - scale.y(39_001.0));
    for price in [10.0, 12.5, 3_141.0, 39_999.0] {
        assert!(close(scale.price(scale.y(price)), price));
    }
}

#[test]
fn the_reference_is_the_first_bar_shown_and_follows_a_pan() {
    let (mut card, candles) = (ChartCard::default(), series(300));
    let newest = view(&card, &candles, PriceScaleMode::Percent);
    assert_eq!(newest.scale.reference, Some(candles[newest.start].close));
    card.viewport.horizontal.right_offset = 60.0;
    let panned = view(&card, &candles, PriceScaleMode::Percent);
    assert!(panned.start < newest.start, "the pan reaches older bars");
    assert_eq!(panned.scale.reference, Some(candles[panned.start].close));
    assert_ne!(panned.scale.reference, newest.scale.reference);
    // The reference is the same whichever mode asks for it.
    let indexed = view(&card, &candles, PriceScaleMode::IndexedTo100);
    assert_eq!(indexed.scale.reference, panned.scale.reference);
}

#[test]
fn a_first_bar_with_no_price_leaves_the_axis_reading_prices() {
    let (card, mut candles) = (ChartCard::default(), series(300));
    let start = view(&card, &candles, PriceScaleMode::Percent).start;
    candles[start].close = 0.0;
    let scale = view(&card, &candles, PriceScaleMode::Percent).scale;
    assert_eq!(scale.reference, None);
    let format = card.appearance.price_format();
    let AxisTicks { prices, step } = scale.axis_ticks(8);
    assert!(!prices.is_empty());
    for price in &prices {
        let label = scale.tick_label(format, *price, step);
        assert!(!label.contains('%'), "{label}");
        assert_eq!(label, format.format_tick(*price, step));
    }
    // And it comes back as soon as there is a price to measure from.
    candles[start].close = 150.0;
    let back = view(&card, &candles, PriceScaleMode::Percent).scale;
    assert_eq!(back.reference, Some(150.0));
}

#[test]
fn a_logarithmic_scale_is_drawn_linear_while_a_price_is_at_or_below_zero() {
    let (card, mut candles) = (ChartCard::default(), series(300));
    let positive = view(&card, &candles, PriceScaleMode::Logarithmic);
    assert!(positive.scale.logarithmic);
    assert_eq!(positive.scale.mode, PriceScaleMode::Logarithmic);

    let at = positive.start + 3;
    for floor in [0.0, -12.5] {
        candles[at].low = floor;
        let scale = view(&card, &candles, PriceScaleMode::Logarithmic).scale;
        assert!(!scale.logarithmic, "low {floor}");
        assert_eq!(
            scale.mode,
            PriceScaleMode::Logarithmic,
            "the choice is kept"
        );
        for price in [floor, 1.0, 100.0, scale.maximum] {
            let y = scale.y(price);
            assert!(y.is_finite(), "low {floor}: y({price}) = {y}");
        }
        assert!(scale.minimum <= floor, "the range reaches the price");
    }
}

#[test]
fn percent_ticks_are_round_in_percent_and_zero_sits_on_the_reference() {
    let (card, candles) = (ChartCard::default(), series(300));
    let scale = view(&card, &candles, PriceScaleMode::Percent).scale;
    let reference = scale.reference.expect("a reference");
    let format = card.appearance.price_format();
    let AxisTicks { prices, step } = scale.axis_ticks(10);
    assert!(prices.len() >= 3 && step > 0.0);

    let labels: Vec<String> = prices
        .iter()
        .map(|price| scale.tick_label(format, *price, step))
        .collect();
    assert!(
        labels.iter().all(|label| label.ends_with('%')),
        "{labels:?}"
    );
    assert!(
        labels.iter().all(|label| label != "-0%" && label != "+0%"),
        "{labels:?}"
    );
    let mut unique = labels.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(
        unique.len(),
        labels.len(),
        "no two ticks read alike: {labels:?}"
    );
    // Each tick is a multiple of the step in percent, by the maths written out here.
    for price in &prices {
        let percent = 100.0 * (price - reference) / reference;
        let steps = percent / step;
        assert!(
            (steps - steps.round()).abs() < 1e-6,
            "{percent}% is not a multiple of {step}"
        );
    }
    // The 0% tick, when the range has one, is drawn at the reference's own height.
    if let Some(index) = labels.iter().position(|label| label == "0%") {
        assert_eq!(scale.y(prices[index]), scale.y(reference));
    }
}

#[test]
fn indexed_ticks_read_a_hundred_at_the_reference() {
    let (card, candles) = (ChartCard::default(), series(300));
    let scale = view(&card, &candles, PriceScaleMode::IndexedTo100).scale;
    let reference = scale.reference.expect("a reference");
    let format = card.appearance.price_format();
    let AxisTicks { prices, step } = scale.axis_ticks(10);
    let labels: Vec<String> = prices
        .iter()
        .map(|price| scale.tick_label(format, *price, step))
        .collect();
    assert!(
        labels.iter().all(|label| !label.contains('%')),
        "{labels:?}"
    );
    // The reference is a hundred, so the tick for a hundred sits at its height.
    let hundred = labels.iter().position(|label| label == "100");
    let index = hundred.expect("the range holds the reference, so it holds a hundred");
    assert!(close(prices[index], reference));
    // Ten percent up is 110: independent of Percent's +10%.
    assert_eq!(
        scale.tag_label(format, reference * 1.1),
        "110.00",
        "indexed is the percent plus a hundred"
    );
}

#[test]
fn a_tag_reads_as_the_axis_does_and_a_regular_tag_is_unchanged() {
    let (card, candles) = (ChartCard::default(), series(300));
    let format = card.appearance.price_format();
    let percent = view(&card, &candles, PriceScaleMode::Percent).scale;
    let reference = percent.reference.expect("a reference");
    assert_eq!(percent.tag_label(format, reference), "0%");
    assert_eq!(percent.tag_label(format, reference * 1.1), "+10.00%");
    assert_eq!(percent.tag_label(format, reference * 0.75), "-25.00%");
    for mode in [PriceScaleMode::Regular, PriceScaleMode::Logarithmic] {
        let scale = view(&card, &candles, mode).scale;
        assert_eq!(
            scale.tag_label(format, 123.456),
            format.format(123.456),
            "{mode:?}"
        );
    }
}

#[test]
fn a_chart_in_percent_prints_percentages_on_its_axis_and_prices_in_regular() {
    let mut regular = Harness::new(ChartCanvas::default());
    let (drawn, _) = regular.draw();
    assert!(
        axis_texts(&drawn).iter().all(|text| !text.contains('%')),
        "{:?}",
        axis_texts(&drawn)
    );

    for mode in [PriceScaleMode::Percent, PriceScaleMode::IndexedTo100] {
        let mut harness = Harness::new(ChartCanvas::default());
        harness.scale_mode = mode;
        let (drawn, _) = harness.draw();
        let texts = axis_texts(&drawn);
        let percent = texts.iter().filter(|text| text.ends_with('%')).count();
        if mode == PriceScaleMode::Percent {
            assert!(
                percent >= 4,
                "ticks and the last price read in percent: {texts:?}"
            );
            assert!(
                texts
                    .iter()
                    .any(|text| text == "0%" || text.starts_with('+')),
                "{texts:?}"
            );
        } else {
            assert_eq!(percent, 0, "an index has no percent sign: {texts:?}");
            assert!(texts.iter().any(|text| text == "100"), "{texts:?}");
        }
    }
}

#[test]
fn the_axis_is_wide_enough_for_the_widest_percentage() {
    // From a penny to a hundred thousand: +10,000,000% needs far more than a price's width.
    let mut harness = Harness::new(ChartCanvas::default());
    harness.scale_mode = PriceScaleMode::Percent;
    harness.candles = (0..300)
        .map(|index| {
            let close = 0.01 * 1.06f64.powi(index);
            Kline {
                open_time: i64::from(index) * super::canvas_tests::HOUR,
                close_time: (i64::from(index) + 1) * super::canvas_tests::HOUR - 1,
                open: close,
                high: close * 1.01,
                low: close * 0.99,
                close,
                volume: 1.0,
            }
        })
        .collect();
    let (drawn, _) = harness.draw();
    let texts = axis_texts(&drawn);
    assert!(texts.iter().all(|text| !text.is_empty()), "{texts:?}");
    assert!(texts.iter().any(|text| text.ends_with('%')), "{texts:?}");
}

/// Where each time label of the last frame sits, as `(left, right)`, and the plot's right edge.
fn time_label_extents(harness: &mut Harness) -> (Vec<(f32, f32)>, f32) {
    fn walk(shape: &egui::Shape, below: f32, into: &mut Vec<(f32, f32)>) {
        match shape {
            egui::Shape::Text(text)
                if text.pos.y >= below
                    && text.galley.text().chars().filter(|c| *c == ':').count() == 1 =>
            {
                into.push((text.pos.x, text.pos.x + text.galley.size().x));
            }
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| walk(shape, below, into)),
            _ => {}
        }
    }
    let mut last = harness.frame();
    for _ in 0..3 {
        last = harness.frame();
    }
    let plot = harness.card.probe.expect("a frame was drawn").plot;
    let mut extents = Vec::new();
    for clipped in &last.0 {
        walk(&clipped.shape, plot.bottom(), &mut extents);
    }
    (extents, plot.right())
}

#[test]
fn no_time_label_reaches_over_the_plots_right_edge_into_the_corner_button() {
    let mut harness = Harness::new(ChartCanvas::default());
    let mut checked = 0;
    // Pan across the chart: a label slides under the edge, and waits there until it fits.
    for step in 0..40 {
        harness.card.viewport.horizontal.right_offset = f64::from(step) * 0.9;
        let (labels, plot_right) = time_label_extents(&mut harness);
        for (left, right) in labels {
            checked += 1;
            assert!(
                right <= plot_right + 0.5,
                "offset {}: a time label spans {left}..{right}, past the plot's edge at {plot_right}",
                harness.card.viewport.horizontal.right_offset
            );
        }
    }
    assert!(checked > 100, "the labels were looked at: {checked}");
}

/// A chart card in a window with a drawing tool armed, driven by pointer and key events through
/// `body` like the app does, so what the card and its drawing layer do with a key or a click
/// while the scale menu is open is what the user would get.
struct Live {
    screen: crate::ui::render_tests::Screen,
    card: ChartCard,
    candles: Vec<Kline>,
    tool: crate::model::Tool,
    /// Drawing commands the layer returned over all frames since the last `take`.
    commands: usize,
    /// Whether an Escape or an Enter was still unspent after the card, i.e. what the workspace's
    /// own Escape handling and any later reader would see.
    escape_left: bool,
    enter_left: bool,
    drawing_escape: bool,
}

impl Live {
    fn new(tool: crate::model::Tool) -> Self {
        let mut card = ChartCard::default();
        card.set_timeframe(Timeframe::Hour);
        let mut live = Self {
            screen: crate::ui::render_tests::Screen::new(vec2(520.0, 340.0)),
            card,
            candles: series(300),
            tool,
            commands: 0,
            escape_left: false,
            enter_left: false,
            drawing_escape: false,
        };
        for _ in 0..6 {
            live.frame(Vec::new());
        }
        live
    }

    fn frame(&mut self, events: Vec<egui::Event>) {
        let (card, candles, tool) = (&mut self.card, &self.candles, self.tool);
        let (mut commands, mut escape_left, mut enter_left, mut drawing_escape) =
            (0, false, false, false);
        self.screen.run(events, |ui| {
            let mut drawing =
                DrawingFrame::new(&[], tool, None, 100).with_view(Timeframe::Hour, false);
            let mut replay = ReplayChartFrame {
                selecting: false,
                active: false,
                picked_ms: None,
            };
            card.body(
                ui,
                "BTCUSDT",
                candles,
                false,
                None,
                PriceScaleMode::Regular,
                DataSource::Binance,
                &IndicatorConfig::default(),
                None,
                1,
                &mut drawing,
                &mut replay,
            );
            commands = drawing.commands.len();
            drawing_escape = drawing.escape_consumed;
            (escape_left, enter_left) = ui.input(|input| {
                (
                    input.key_pressed(egui::Key::Escape),
                    input.key_pressed(egui::Key::Enter),
                )
            });
        });
        self.commands += commands;
        self.escape_left |= escape_left;
        self.enter_left |= enter_left;
        self.drawing_escape |= drawing_escape;
    }

    fn click(&mut self, at: Pos2) {
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        self.frame(vec![egui::Event::PointerMoved(at)]);
        self.frame(Vec::new());
        self.frame(vec![button(true)]);
        self.frame(vec![button(false)]);
        self.frame(Vec::new());
    }

    fn press(&mut self, key: egui::Key) {
        let event = |pressed| egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        self.frame(vec![event(true), event(false)]);
        self.frame(Vec::new());
    }

    fn plot(&self) -> Rect {
        self.card.probe.expect("a frame was drawn").plot
    }

    fn corner(&self) -> Pos2 {
        let size = self.screen.size;
        scale_menu::corner(self.plot(), Rect::from_min_size(Pos2::ZERO, size))
            .expect("a corner")
            .center()
    }

    fn open_menu(&mut self) {
        let corner = self.corner();
        self.click(corner);
        assert!(
            scale_menu::is_open(&self.screen.ctx, 1),
            "the button opens the menu"
        );
    }

    /// The middle of row `index` of the open menu.
    fn row(&self, index: usize) -> Pos2 {
        let menu = self
            .screen
            .ctx
            .memory(|memory| memory.area_rect(scale_menu::id(1).with("popup")))
            .expect("the menu is shown");
        Pos2::new(
            menu.center().x,
            menu.top() + 6.0 + 24.0 * (index as f32 + 0.5),
        )
    }

    fn spent(&mut self) -> (usize, bool, bool, bool) {
        let all = (
            self.commands,
            self.escape_left,
            self.enter_left,
            self.drawing_escape,
        );
        (
            self.commands,
            self.escape_left,
            self.enter_left,
            self.drawing_escape,
        ) = (0, false, false, false);
        all
    }
}

#[test]
fn a_click_on_a_menu_row_with_a_tool_armed_picks_the_row_and_draws_nothing() {
    let mut live = Live::new(crate::model::Tool::TrendLine);
    live.open_menu();
    live.spent();
    let percent = live.row(1);
    live.click(percent);
    assert_eq!(
        live.card.take_scale_request(),
        Some(PriceScaleMode::Percent)
    );
    assert!(
        !scale_menu::is_open(&live.screen.ctx, 1),
        "a pick closes the menu"
    );
    let (commands, ..) = live.spent();
    assert_eq!(commands, 0, "the click that picked a row placed nothing");
    // And it started no draft: with one, Escape would be spent cancelling it.
    live.press(egui::Key::Escape);
    assert!(!live.spent().3, "no draft was waiting");
}

#[test]
fn the_keys_belong_to_the_open_menu_and_reach_nothing_behind_it() {
    let mut live = Live::new(crate::model::Tool::TrendLine);
    // A first point of a trend line is down: a draft is in progress.
    let start = Pos2::new(plot_x(&live, 0.3), plot_y(&live, 0.4));
    live.click(start);
    live.spent();

    // Escape closes the menu, is spent, and the draft survives it.
    live.open_menu();
    live.spent();
    live.press(egui::Key::Escape);
    assert!(
        !scale_menu::is_open(&live.screen.ctx, 1),
        "Escape closes the menu"
    );
    let (_, escape_left, _, drawing_escape) = live.spent();
    assert!(
        !escape_left,
        "Escape was spent by the menu, not left for the workspace"
    );
    assert!(
        !drawing_escape,
        "the draft was not cancelled behind the menu"
    );
    live.click(Pos2::new(plot_x(&live, 0.7), plot_y(&live, 0.6)));
    assert_eq!(live.spent().0, 1, "the draft was still there to finish");
    assert_eq!(live.card.take_scale_request(), None);

    // The arrows and Enter pick a row, and are not left for anything else either.
    live.open_menu();
    live.spent();
    live.press(egui::Key::ArrowDown);
    live.press(egui::Key::ArrowDown);
    live.press(egui::Key::Enter);
    assert_eq!(
        live.card.take_scale_request(),
        Some(PriceScaleMode::Percent)
    );
    assert!(!scale_menu::is_open(&live.screen.ctx, 1));
    let (commands, _, enter_left, _) = live.spent();
    assert_eq!(commands, 0);
    assert!(!enter_left, "Enter was the menu's");
}

fn plot_x(live: &Live, share: f32) -> f32 {
    let plot = live.plot();
    plot.left() + plot.width() * share
}

fn plot_y(live: &Live, share: f32) -> f32 {
    let plot = live.plot();
    plot.top() + plot.height() * share
}

/// Pictures of the four modes and of the open menu, to look at by hand:
/// `DEGENVIEW_GALLERY_DIR=<dir> cargo test --lib gallery_of_the_scale -- --ignored`.
#[test]
#[ignore = "writes pictures; run by hand"]
fn gallery_of_the_scale_modes() {
    use crate::ui::render_tests::{Screen, gallery_dir};
    use crate::ui::scale_menu;
    if gallery_dir().is_none() {
        return;
    }
    let size = vec2(520.0, 340.0);
    let candles = series(300);
    let button = |pressed, at: Pos2| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    let frame = |screen: &mut Screen, card: &mut ChartCard, mode, events| {
        screen.run(events, |ui| {
            let mut drawing = DrawingFrame::new(&[], crate::model::Tool::Cursor, None, 1)
                .with_view(Timeframe::Hour, false);
            let mut replay = ReplayChartFrame {
                selecting: false,
                active: false,
                picked_ms: None,
            };
            card.body(
                ui,
                "BTCUSDT",
                &candles,
                false,
                None,
                mode,
                DataSource::Binance,
                &IndicatorConfig::default(),
                None,
                1,
                &mut drawing,
                &mut replay,
            );
        })
    };
    for (name, mode) in [
        ("regular", PriceScaleMode::Regular),
        ("percent", PriceScaleMode::Percent),
        ("indexed", PriceScaleMode::IndexedTo100),
        ("logarithmic", PriceScaleMode::Logarithmic),
    ] {
        let mut screen = Screen::new(size);
        let mut card = ChartCard::default();
        card.set_timeframe(Timeframe::Hour);
        let mut last = frame(&mut screen, &mut card, mode, Vec::new());
        for _ in 0..5 {
            last = frame(&mut screen, &mut card, mode, Vec::new());
        }
        screen.save(&format!("scale-{name}"), last);

        if mode == PriceScaleMode::Percent {
            // The corner button under the axis, then its menu open.
            let plot = card.probe.expect("a frame was drawn").plot;
            let cell =
                scale_menu::corner(plot, Rect::from_min_size(Pos2::ZERO, size)).expect("a corner");
            let at = cell.center();
            frame(
                &mut screen,
                &mut card,
                mode,
                vec![egui::Event::PointerMoved(at)],
            );
            let hover = frame(&mut screen, &mut card, mode, Vec::new());
            screen.save("scale-button-hover", hover);
            frame(&mut screen, &mut card, mode, vec![button(true, at)]);
            frame(&mut screen, &mut card, mode, vec![button(false, at)]);
            let mut open = frame(&mut screen, &mut card, mode, Vec::new());
            for _ in 0..3 {
                open = frame(&mut screen, &mut card, mode, Vec::new());
            }
            screen.save("scale-menu-open", open);
        }
    }
}

#[test]
fn a_click_on_the_axis_or_the_plot_hands_a_menu_the_real_price_in_every_mode() {
    use super::context::{Spot, request};
    let (card, candles) = (ChartCard::default(), series(300));
    let ctx = egui::Context::default();
    for mode in PriceScaleMode::ALL {
        let view = view(&card, &candles, mode);
        let projection = view.projection(plot(), &candles, card.appearance.price_format(), &ctx);
        // 123.4 sits at some height; a click there is a price, never "+x%" or an index.
        let at = Pos2::new(plot().right() + 20.0, view.scale.y(123.4));
        let axis = request(at, Spot::PriceAxis, &projection, &view.scale, &[]);
        let price = axis.price.expect("a price on the axis");
        assert!(
            close(price, 123.4),
            "{mode:?}: the axis menu was handed {price}"
        );
        let at = Pos2::new(plot().center().x, view.scale.y(123.4));
        let plotted = request(at, Spot::Plot, &projection, &view.scale, &[]);
        assert!(
            close(plotted.price.expect("a price in the plot"), 123.4),
            "{mode:?}"
        );
    }
}

use super::coordinates::anchor_label;
use super::rows::toggle_row;
use super::*;
use crate::format;
use crate::model::Placement;
use crate::model::Timeframe;
use crate::model::drawings::style::MAX_STROKE_WIDTH;
use crate::model::{Anchor, Caps, Drawing, Geometry, PitchforkVariant, Tool};
use crate::store::drawing_template::DrawingTemplateStore;
use crate::ui::icons::Icon;
use crate::ui::{kit, settings::card};
use eframe::egui::{self, Id, Key, vec2};
use std::sync::{Arc, Mutex};

fn templates() -> DrawingTemplateStore {
    DrawingTemplateStore::new(Arc::new(Mutex::new(
        crate::store::Store::open_in_memory().expect("store"),
    )))
}

fn drawing(tool: Tool) -> Drawing {
    let anchors: Vec<Anchor> = (0..7)
        .map(|index| Anchor {
            time_ms: 1_700_000_000_000 + index * 3_600_000,
            price: 100.0 + index as f64 * 7.0,
        })
        .collect();
    let count = match tool.spec().placement {
        Placement::Points(count) => usize::from(count),
        Placement::Single => 1,
        _ => 3,
    };
    Drawing::sample(tool, 1, &anchors[..count], tool.default_style()).expect("builds")
}

fn frame(
    ctx: &egui::Context,
    dialog: &mut DrawingProperties,
    store: &mut DrawingTemplateStore,
    events: Vec<egui::Event>,
) -> Outcome {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            vec2(1200.0, 900.0),
        )),
        events,
        ..Default::default()
    };
    let mut outcome = Outcome::Open;
    let mut output = ctx.run_ui(input, |ui| outcome = dialog.show(ui.ctx(), store));
    output.textures_delta.clear();
    outcome
}

fn key(key: Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

#[test]
fn a_ruler_and_an_unreadable_row_have_no_properties() {
    let ruler = Drawing::Ruler {
        id: 1,
        a: Anchor {
            time_ms: 0,
            price: 1.0,
        },
        b: Anchor {
            time_ms: 1,
            price: 2.0,
        },
    };
    assert!(DrawingProperties::new(1, &ruler).is_none());
    assert!(DrawingProperties::new(1, &Drawing::Unreadable(serde_json::Value::Null)).is_none());
}

#[test]
fn a_dialog_offers_only_the_pages_its_tool_supports() {
    for tool in Tool::GROUPED {
        let dialog = DrawingProperties::new(1, &drawing(tool)).expect("dialog");
        let tabs = dialog.tabs();
        let caps = tool.caps();
        assert_eq!(
            tabs.contains(&Tab::Style),
            has_style_page(caps),
            "{tool:?} style"
        );
        assert_eq!(
            tabs.contains(&Tab::Text),
            caps.has(Caps::TEXT) || caps.has(Caps::AUTO_TEXT),
            "{tool:?} text"
        );
        assert_eq!(
            tabs.contains(&Tab::Levels),
            caps.has(Caps::LEVELS) || has_fib_levels(tool),
            "{tool:?} levels"
        );
        assert_eq!(
            tabs.contains(&Tab::Inputs),
            has_inputs_page(caps),
            "{tool:?} inputs"
        );
        assert_eq!(
            tabs.contains(&Tab::Coordinates),
            !matches!(tool, Tool::Brush | Tool::Highlighter),
            "{tool:?} coordinates"
        );
        assert!(tabs.contains(&Tab::Visibility), "{tool:?}");
        assert_eq!(dialog.tab, tabs[0], "{tool:?} opens on its first page");
    }
    // The marks have a colour and text but no thickness, pattern or fill to offer.
    let mark = Tool::ArrowMarkUp.caps();
    assert!(mark.has(Caps::COLOR) && !mark.has(Caps::WIDTH) && !mark.has(Caps::FILL));
}

#[test]
fn every_page_of_every_tool_draws() {
    let ctx = egui::Context::default();
    let mut store = templates();
    for tool in Tool::GROUPED {
        let mut dialog = DrawingProperties::new(1, &drawing(tool)).expect("dialog");
        for tab in dialog.tabs() {
            dialog.tab = tab;
            for _ in 0..2 {
                assert_eq!(
                    frame(&ctx, &mut dialog, &mut store, Vec::new()),
                    Outcome::Open
                );
            }
        }
    }
}

#[test]
fn an_image_has_a_picture_page_and_replacing_the_picture_changes_only_the_picture() {
    use crate::model::annotation::image_content::ImageContent;
    let mut dialog = DrawingProperties::new(1, &drawing(Tool::Image)).expect("dialog");
    assert_eq!(dialog.tabs()[0], Tab::Content, "it opens on the picture");
    assert!(!dialog.tabs().contains(&Tab::Text));
    let Some(Geometry::Image(before)) = dialog.draft().geometry().cloned() else {
        panic!("an image");
    };
    let replacement = ImageContent::new("b".repeat(64), "other.png", [64, 64]);
    dialog.replace_picture(&replacement);
    let Some(Geometry::Image(after)) = dialog.draft().geometry() else {
        panic!("an image");
    };
    assert_eq!(after.asset, replacement.asset);
    assert_eq!(after.pixels, [64, 64]);
    assert_eq!(after.at, before.at, "it stays where it was");
    assert_eq!(after.size[0], before.size[0], "and as wide");
    assert!(
        (after.size[1] - after.size[0]).abs() < 0.01,
        "the square picture has a square shape: {:?}",
        after.size
    );
    assert_eq!(after.name, before.name, "the user's name for it is kept");
    assert!(!dialog.take_replace_request());
    dialog.replace_requested = true;
    assert!(dialog.take_replace_request());
    assert!(!dialog.take_replace_request(), "a request is taken once");
}

#[test]
fn every_page_leaves_the_card_spacing_as_it_found_it() {
    let ctx = egui::Context::default();
    let tools = Tool::GROUPED
        .into_iter()
        .chain([Tool::TrendLine, Tool::FibRetracement])
        .collect::<Vec<_>>();
    for tool in tools {
        let mut dialog = DrawingProperties::new(1, &drawing(tool)).expect("dialog");
        for tab in dialog.tabs() {
            let mut spacing = 0.0;
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.spacing_mut().item_spacing.y = CARD_SPACING;
                match tab {
                    Tab::Style => dialog.style_page(ui),
                    Tab::Inputs => dialog.inputs_page(ui),
                    Tab::Levels => dialog.levels_page(ui),
                    Tab::Text => dialog.text_page(ui),
                    Tab::Content => dialog.content_page(ui),
                    Tab::Link => dialog.link_page(ui),
                    Tab::Coordinates => dialog.coordinates_page(ui),
                    Tab::Visibility => dialog.visibility_page(ui),
                }
                spacing = ui.spacing().item_spacing.y;
            });
            output.textures_delta.clear();
            assert_eq!(spacing, CARD_SPACING, "{tool:?} {tab:?}");
        }
    }
}

#[test]
fn a_toggle_without_a_hint_is_as_tall_as_a_card_can_be_without_one() {
    let ctx = egui::Context::default();
    let mut with_empty = 0.0;
    let mut without = 0.0;
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        let mut on = false;
        let top = ui.cursor().top();
        toggle_row(ui, Icon::Eye, "Title", "", &mut on);
        with_empty = ui.cursor().top() - top;
        let top = ui.cursor().top();
        card::card(ui, Icon::Eye, "Title", None, |ui| {
            card::switch(ui, &mut on);
        });
        without = ui.cursor().top() - top;
    });
    output.textures_delta.clear();
    assert_eq!(with_empty, without);
}

#[test]
fn escape_cancels_and_return_applies_the_working_copy() {
    let ctx = egui::Context::default();
    let mut store = templates();
    let mut dialog = DrawingProperties::new(3, &drawing(Tool::Rectangle)).expect("dialog");
    frame(&ctx, &mut dialog, &mut store, Vec::new());
    assert_eq!(
        frame(&ctx, &mut dialog, &mut store, vec![key(Key::Escape)]),
        Outcome::Cancel
    );
    let mut dialog = DrawingProperties::new(3, &drawing(Tool::Rectangle)).expect("dialog");
    dialog.draft.set_text("edited");
    frame(&ctx, &mut dialog, &mut store, Vec::new());
    let outcome = frame(&ctx, &mut dialog, &mut store, vec![key(Key::Enter)]);
    let Outcome::Apply(applied) = outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!(applied.text(), "edited");
    assert_eq!(applied.id(), 1);
}

#[test]
fn the_pitchfork_style_switches_the_construction_and_keeps_the_pivots_and_the_look() {
    let mut dialog = DrawingProperties::new(1, &drawing(Tool::Pitchfork)).expect("dialog");
    let pivots = dialog.draft().anchors();
    let mut style = dialog.style();
    style.pitchfork_levels[0].line.enabled = true;
    style.stroke.color = [1, 2, 3, 255];
    dialog.set_style(style);
    for variant in PitchforkVariant::ALL {
        dialog.switch_variant(variant);
        assert_eq!(dialog.tool, variant.tool(), "the dialog follows the tool");
        assert_eq!(dialog.draft().tool(), Some(variant.tool()));
        assert_eq!(dialog.draft().anchors(), pivots);
        assert_eq!(dialog.style(), style, "every level and colour is kept");
    }
    // The dialog still draws every page after the switch.
    let ctx = egui::Context::default();
    let mut store = templates();
    for tab in dialog.tabs() {
        dialog.tab = tab;
        frame(&ctx, &mut dialog, &mut store, Vec::new());
    }
}

#[test]
fn the_levels_and_inputs_pages_edit_the_style_the_drawing_keeps() {
    let mut fork = DrawingProperties::new(1, &drawing(Tool::Pitchfork)).expect("dialog");
    let mut style = fork.style();
    style.pitchfork_levels[1].line.enabled = true;
    style.pitchfork_levels[1].ratio = 0.7;
    style.use_one_color = true;
    fork.set_style(style);
    let json = serde_json::to_string(fork.draft()).unwrap();
    let back: Drawing = serde_json::from_str(&json).unwrap();
    assert_eq!(back.style().unwrap().pitchfork_levels[1].ratio, 0.7);
    assert!(back.style().unwrap().use_one_color);
    // Absurd ratios and deviations are brought into range, never stored.
    let mut regression =
        DrawingProperties::new(1, &drawing(Tool::RegressionTrend)).expect("dialog");
    let mut style = regression.style();
    style.regression.upper_dev = f32::NAN;
    style.regression.lower_dev = -1.0e9;
    style.pitchfork_levels[0].ratio = f32::INFINITY;
    regression.set_style(style);
    let kept = regression.style();
    assert_eq!(kept.regression.upper_dev, 2.0);
    assert_eq!(
        kept.regression.lower_dev,
        -crate::model::drawings::style::MAX_DEVIATION
    );
    assert!(kept.pitchfork_levels[0].ratio.is_finite());
}

#[test]
fn the_coordinates_tab_leaves_out_what_an_anchor_does_not_read() {
    let ctx = egui::Context::default();
    let mut store = templates();
    // A horizontal line has a price and no time; a vertical line the opposite.
    for (tool, price, time) in [
        (Tool::HorizontalLine, true, false),
        (Tool::VerticalLine, false, true),
        (Tool::HorizontalRay, true, true),
        (Tool::RegressionTrend, false, true),
    ] {
        let dialog = DrawingProperties::new(1, &drawing(tool)).expect("dialog");
        let geometry = dialog.draft().geometry().expect("a geometry");
        assert_eq!(geometry.reads(0), (time, price), "{tool:?}");
        let mut dialog = dialog;
        dialog.tab = Tab::Coordinates;
        assert_eq!(
            frame(&ctx, &mut dialog, &mut store, Vec::new()),
            Outcome::Open
        );
    }
    assert_eq!(anchor_label(Tool::HorizontalLine, 0), "Price level");
    assert_eq!(anchor_label(Tool::DisjointChannel, 3), "End of second edge");
    assert_eq!(anchor_label(Tool::ParallelChannel, 2), "Offset point");
    assert_eq!(anchor_label(Tool::SchiffPitchfork, 2), "Point 3");
}

#[test]
fn a_trend_line_takes_text_in_the_dialog_like_any_other_line() {
    let mut dialog = DrawingProperties::new(1, &drawing(Tool::TrendLine)).expect("dialog");
    dialog.draft.set_text("support");
    assert_eq!(dialog.draft().text(), "support");
    assert_eq!(dialog.draft().tool(), Some(Tool::TrendLine));
    assert_eq!(dialog.tool, Tool::TrendLine);
    assert!(dialog.tabs().contains(&Tab::Text));
    dialog.draft.set_text("");
    assert_eq!(dialog.draft().text(), "");
}

#[test]
fn editing_a_coordinate_moves_that_anchor_and_refreshes_its_text() {
    let mut dialog = DrawingProperties::new(1, &drawing(Tool::Triangle)).expect("dialog");
    let before = dialog.draft().anchors();
    let moved = Anchor {
        time_ms: 1_800_000_000_000,
        price: -12.5,
    };
    dialog.move_anchor(1, moved);
    let after = dialog.draft().anchors();
    assert_eq!(after[0], before[0]);
    assert_eq!(after[1], moved, "negative prices are allowed");
    assert_eq!(after[2], before[2]);
    assert_eq!(dialog.coordinates[1].time, format::timestamp(moved.time_ms));
}

#[test]
fn a_picked_time_reads_back_as_the_same_utc_minute() {
    use crate::model::portfolio::editor::date_text;
    // The picker writes `date_text(.., 0)`; the dialog reads it with `parse_timestamp`.
    let picked = 1_800_000_060_000;
    assert_eq!(format::parse_timestamp(&date_text(picked, 0)), Some(picked));
}

#[test]
fn the_dialog_never_stores_an_out_of_range_style() {
    let mut dialog = DrawingProperties::new(1, &drawing(Tool::Rectangle)).expect("dialog");
    let mut style = dialog.style();
    style.stroke.width = 1.0e9;
    style.text.size = f32::NAN;
    dialog.set_style(style);
    let kept = dialog.style();
    assert_eq!(kept.stroke.width, MAX_STROKE_WIDTH);
    assert!(kept.text.size.is_finite());
    // Text is capped, and a drawing without a text field refuses it.
    dialog.draft.set_text(&"x".repeat(10_000));
    assert!(dialog.draft().text().chars().count() <= dialog.draft().max_text());
    let mut brush = DrawingProperties::new(1, &drawing(Tool::Brush)).expect("brush dialog");
    brush.draft.set_text("ignored");
    assert_eq!(brush.draft().text(), "");
}

#[test]
fn visibility_edits_keep_at_least_one_timeframe() {
    let mut dialog = DrawingProperties::new(1, &drawing(Tool::Circle)).expect("dialog");
    let mut visibility = dialog.draft().visibility();
    for timeframe in Timeframe::ALL {
        visibility = visibility.with(timeframe, false);
    }
    dialog.draft.set_visibility(visibility);
    let shown = Timeframe::ALL
        .into_iter()
        .filter(|timeframe| dialog.draft().visibility().shows(*timeframe))
        .count();
    assert_eq!(shown, 1, "the last one cannot be switched off");
}

#[test]
fn a_saved_style_applies_to_the_draft_and_can_become_the_default() {
    let mut store = templates();
    let mut dialog = DrawingProperties::new(1, &drawing(Tool::Rectangle)).expect("dialog");
    let mut thick = dialog.style();
    thick.stroke.width = 9.0;
    dialog.set_style(thick);
    // Save as a template, as the prompt does.
    let id = store
        .save("Thick zone", Tool::Rectangle, &dialog.style())
        .unwrap();
    // Back to the built-in look, then apply the template to the draft.
    dialog.set_style(Tool::Rectangle.default_style());
    assert!(dialog.style().stroke.width != 9.0);
    dialog.set_style(store.book().get(id).expect("saved").style);
    assert_eq!(dialog.style().stroke.width, 9.0);
    // Making it the default changes what a new rectangle starts with, nothing already drawn.
    store.set_default(Tool::Rectangle, &dialog.style()).unwrap();
    assert_eq!(
        crate::model::drawings::template::style_for(Tool::Rectangle, store.defaults())
            .stroke
            .width,
        9.0
    );
    assert_eq!(
        drawing(Tool::Rectangle).style().unwrap().stroke.width,
        Tool::Rectangle.default_style().stroke.width
    );
}

/// The window is exactly as wide as the sheet says on every tab of every tool, and the sheet is
/// wide enough for its tab bar: five tabs used to push the window past the page, cutting the bar
/// and the content off.
#[test]
fn every_tool_s_sheet_is_as_wide_as_its_tab_bar_and_its_window() {
    use crate::ui::render_tests::Screen;
    let mut widest = 0.0_f32;
    let mut screen = Screen::new(vec2(1200.0, 900.0));
    for tool in Tool::GROUPED {
        let Some(mut dialog) = DrawingProperties::new(1, &drawing(tool)) else {
            continue;
        };
        let mut store = templates();
        let tabs = dialog.tabs();
        for tab in tabs {
            dialog.tab = tab;
            for _ in 0..2 {
                screen.run(Vec::new(), |ui| {
                    dialog.show(ui.ctx(), &mut store);
                });
            }
            let window = screen
                .ctx
                .memory(|memory| memory.area_rect(Id::new(("drawing-properties", 1_u64))))
                .expect("the dialog is shown");
            assert!(
                // The frame's hairline is one point either side.
                (window.width() - dialog.width - 2.0).abs() < 1.0,
                "{tool:?} {tab:?}: the window is {} wide, the sheet {}",
                window.width(),
                dialog.width
            );
        }
        let items: Vec<kit::TabItem<Tab>> = dialog
            .tabs()
            .iter()
            .map(|&tab| dialog.tab_item(tab))
            .collect();
        if items.len() > 1 {
            screen.run(Vec::new(), |ui| {
                let bar = kit::tab_bar_regular_width(ui, &items);
                assert!(
                    bar + 2.0 * TAB_BAR_MARGIN <= dialog.width + 0.5,
                    "{tool:?}: a {bar} wide tab bar in a {} wide sheet",
                    dialog.width
                );
            });
        }
        widest = widest.max(dialog.width);
    }
    assert!(
        widest > WIDTH,
        "no tool needed a wider sheet: is the check live?"
    );
}

/// The new tools' pages as pictures. Run by hand with `DEGENVIEW_GALLERY_DIR` set.
#[test]
#[ignore = "writes images for a person to look at"]
fn gallery_of_the_new_pages() {
    use crate::ui::render_tests::{Screen, gallery_dir};
    if gallery_dir().is_none() {
        return;
    }
    for (name, tool, tab) in [
        ("dialog-pitchfork-style", Tool::SchiffPitchfork, Tab::Style),
        ("dialog-pitchfork-levels", Tool::Pitchfork, Tab::Levels),
        (
            "dialog-regression-inputs",
            Tool::RegressionTrend,
            Tab::Inputs,
        ),
        ("dialog-regression-style", Tool::RegressionTrend, Tab::Style),
        ("dialog-channel-style", Tool::ParallelChannel, Tab::Style),
        ("dialog-trend-line-text", Tool::TrendLine, Tab::Text),
        ("dialog-vertical-text", Tool::VerticalLine, Tab::Text),
        ("dialog-horizontal-style", Tool::HorizontalLine, Tab::Style),
        ("dialog-info-style", Tool::InfoLine, Tab::Style),
        (
            "dialog-flat-coordinates",
            Tool::FlatChannel,
            Tab::Coordinates,
        ),
        ("dialog-fib-style", Tool::FibRetracement, Tab::Style),
        ("dialog-fib-levels", Tool::FibRetracement, Tab::Levels),
        ("dialog-fan-levels", Tool::FibSpeedFan, Tab::Levels),
        ("dialog-fan-style", Tool::FibSpeedFan, Tab::Style),
        ("dialog-circles-style", Tool::FibCircles, Tab::Style),
        ("dialog-spiral-style", Tool::FibSpiral, Tab::Style),
        ("dialog-gann-square-levels", Tool::GannSquare, Tab::Levels),
        ("dialog-gann-square-style", Tool::GannSquare, Tab::Style),
        ("dialog-gann-fan-levels", Tool::GannFan, Tab::Levels),
        ("dialog-gann-box-style", Tool::GannBox, Tab::Style),
        (
            "dialog-fib-coordinates",
            Tool::FibRetracement,
            Tab::Coordinates,
        ),
        ("dialog-xabcd-style", Tool::XabcdPattern, Tab::Style),
        ("dialog-xabcd-inputs", Tool::XabcdPattern, Tab::Inputs),
        ("dialog-xabcd-text", Tool::XabcdPattern, Tab::Text),
        ("dialog-impulse-inputs", Tool::ElliottImpulse, Tab::Inputs),
        ("dialog-impulse-style", Tool::ElliottImpulse, Tab::Style),
        (
            "dialog-head-shoulders-coordinates",
            Tool::HeadAndShoulders,
            Tab::Coordinates,
        ),
        ("dialog-time-cycles-style", Tool::TimeCycles, Tab::Style),
        ("dialog-long-inputs", Tool::LongPosition, Tab::Inputs),
        ("dialog-long-style", Tool::LongPosition, Tab::Style),
        (
            "dialog-forecast-inputs",
            Tool::PositionForecast,
            Tab::Inputs,
        ),
        ("dialog-bars-inputs", Tool::BarsPattern, Tab::Inputs),
        ("dialog-ghost-inputs", Tool::GhostFeed, Tab::Inputs),
        ("dialog-vwap-inputs", Tool::AnchoredVwap, Tab::Inputs),
        (
            "dialog-profile-inputs",
            Tool::FixedRangeVolumeProfile,
            Tab::Inputs,
        ),
        (
            "dialog-measure-inputs",
            Tool::DateAndPriceRange,
            Tab::Inputs,
        ),
        (
            "dialog-price-range-coordinates",
            Tool::PriceRange,
            Tab::Coordinates,
        ),
    ] {
        let mut dialog = DrawingProperties::new(1, &drawing(tool)).expect("dialog");
        dialog.tab = tab;
        let mut store = templates();
        let mut screen = Screen::new(vec2(640.0, 760.0));
        let mut last = None;
        for _ in 0..4 {
            last = Some(screen.run(Vec::new(), |ui| {
                dialog.show(ui.ctx(), &mut store);
            }));
        }
        screen.save(name, last.expect("a frame"));
    }
}

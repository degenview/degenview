//! The window driven by pointer and keyboard events: every control says what it asks for, and
//! what cannot be used says why.

use super::rig::{Rig, Scenario, Shown};
use super::toolbar::lines;
use super::*;
use crate::model::chart::snapshot::framing::Framing;
use eframe::egui::{Pos2, TextureId};

const SCREEN: Vec2 = vec2(1440.0, 900.0);
const COMPOSITION: Vec2 = vec2(1200.0, 800.0);
const THREE: [Area; 3] = [Area::FullChart, Area::EntireLayout, Area::Portion];

fn styled() -> StudioConfig {
    StudioConfig::new(Area::Portion)
}

fn chart_style() -> StudioConfig {
    let mut config = StudioConfig::new(Area::FullChart);
    config.set_style(Style::Chart);
    config
}

/// A scenario with a picture standing in for the preview.
fn with_picture(config: StudioConfig) -> Scenario<'static> {
    let mut scenario = Scenario::new(config);
    scenario.preview = Shown::Picture(Image {
        texture: TextureId::Managed(0),
        pixels: [1200, 800],
        composition: COMPOSITION,
    });
    scenario
}

fn rig() -> Rig {
    Rig::new(SCREEN, false)
}

/// A rig that has drawn `scenario` enough times to know where everything is.
fn ready(scenario: &Scenario<'_>) -> Rig {
    let mut rig = rig();
    rig.settle(scenario);
    rig
}

fn centre(rig: &Rig, label: &str) -> Pos2 {
    rig.find(label).center()
}

// MARK: Area, Style and Size

#[test]
fn a_workspace_with_one_chart_offers_full_chart_and_portion() {
    let scenario = Scenario::new(styled());
    let rig = ready(&scenario);
    assert!(rig.has("Full chart") && rig.has("Portion"));
    assert!(!rig.has("Entire layout"));
}

#[test]
fn a_workspace_with_several_charts_also_offers_the_entire_layout() {
    let mut scenario = Scenario::new(styled());
    scenario.areas = &THREE;
    let rig = ready(&scenario);
    assert!(rig.has("Full chart") && rig.has("Entire layout") && rig.has("Portion"));
}

#[test]
fn picking_an_area_asks_for_it() {
    let mut scenario = Scenario::new(styled());
    scenario.areas = &THREE;
    let mut rig = ready(&scenario);
    let at = centre(&rig, "Full chart");
    assert_eq!(rig.click(&scenario, at), [Action::SetArea(Area::FullChart)]);
    let at = centre(&rig, "Entire layout");
    assert_eq!(
        rig.click(&scenario, at),
        [Action::SetArea(Area::EntireLayout)]
    );
}

#[test]
fn the_area_that_is_picked_already_asks_for_nothing_except_portion_which_selects_again() {
    let mut scenario = Scenario::new(StudioConfig::new(Area::FullChart));
    let mut rig = ready(&scenario);
    let at = centre(&rig, "Full chart");
    assert!(rig.click(&scenario, at).is_empty());

    scenario.config.area = Area::Portion;
    let mut rig = ready(&scenario);
    let at = centre(&rig, "Portion");
    assert_eq!(rig.click(&scenario, at), [Action::SetArea(Area::Portion)]);
}

#[test]
fn the_style_switch_asks_for_the_other_look_only() {
    let scenario = Scenario::new(styled());
    let mut rig = ready(&scenario);
    let at = centre(&rig, "Styled");
    assert!(rig.click(&scenario, at).is_empty());
    let at = centre(&rig, "Chart");
    assert_eq!(rig.click(&scenario, at), [Action::SetStyle(Style::Chart)]);
}

#[test]
fn sizes_are_picked_in_the_styled_look() {
    let scenario = Scenario::new(styled());
    let mut rig = ready(&scenario);
    for (label, size) in [
        ("Original", AspectRatio::Original),
        ("16:9", AspectRatio::Wide16x9),
        ("1:1", AspectRatio::Square),
        ("9:16", AspectRatio::Portrait9x16),
    ] {
        let at = centre(&rig, label);
        assert_eq!(rig.click(&scenario, at), [Action::SetSize(size)], "{label}");
    }
}

#[test]
fn size_is_disabled_in_the_chart_style_and_says_why() {
    let scenario = Scenario::new(chart_style());
    let mut rig = ready(&scenario);
    let at = centre_of_size(&rig, "16:9");
    let words = rig.hover(&scenario, at);
    assert!(
        words.iter().any(|w| w == "Size applies to the Styled look"),
        "{words:?}"
    );
    for label in ["Original", "16:9", "3:2", "1:1", "9:16"] {
        let at = centre_of_size(&rig, label);
        assert!(rig.click(&scenario, at).is_empty(), "{label}");
    }
}

/// A size segment's place: disabled ones are not widgets a screen reader sees, so by id.
fn centre_of_size(rig: &Rig, label: &str) -> Pos2 {
    let index = AspectRatio::ALL
        .iter()
        .position(|size| size.label() == label)
        .expect("a size");
    rig.rect_of(Id::new("studio-size").with(index)).center()
}

#[test]
fn the_size_control_shows_the_proportions_that_apply() {
    // Chart style keeps the capture's own proportions, whatever Size was left at.
    let mut config = chart_style();
    config.size = AspectRatio::Square;
    assert_eq!(config.effective_size(), AspectRatio::Original);
    let scenario = Scenario::new(config);
    let rig = ready(&scenario);
    assert!(rig.has("Original"));
}

// MARK: Background and Include

#[test]
fn each_swatch_picks_its_background_and_names_itself_on_hover() {
    let scenario = Scenario::new(styled());
    let mut rig = ready(&scenario);
    for (index, background) in Background::ALL.into_iter().enumerate() {
        let at = rig.rect_of(Id::new(("studio-background", index))).center();
        let words = rig.hover(&scenario, at);
        assert!(
            words.iter().any(|word| word == background.label()),
            "{background:?}: {words:?}"
        );
        let expected = if background == Background::None {
            Vec::new() // the one that is picked already
        } else {
            vec![Action::SetBackground(background)]
        };
        assert_eq!(rig.click(&scenario, at), expected, "{background:?}");
    }
    assert!(rig.has("Spectrum background"));
}

#[test]
fn each_include_toggle_flips_its_part_alone() {
    let scenario = Scenario::new(styled());
    let mut rig = ready(&scenario);
    let off = |include: Include| Action::SetInclude(include);
    let all = Include::default();
    let at = centre(&rig, "Indicators, on");
    assert_eq!(
        rig.click(&scenario, at),
        [off(Include {
            indicators: false,
            ..all
        })]
    );
    let at = centre(&rig, "Drawings, on");
    assert_eq!(
        rig.click(&scenario, at),
        [off(Include {
            drawings: false,
            ..all
        })]
    );
    let at = centre(&rig, "DegenView mark, on");
    assert_eq!(
        rig.click(&scenario, at),
        [off(Include {
            branding: false,
            ..all
        })]
    );
}

#[test]
fn a_part_that_is_off_is_switched_on_by_its_toggle() {
    let mut config = styled();
    config.include.drawings = false;
    let scenario = Scenario::new(config);
    let mut rig = ready(&scenario);
    let at = centre(&rig, "Drawings, off");
    assert_eq!(
        rig.click(&scenario, at),
        [Action::SetInclude(Include::default())]
    );
}

#[test]
fn indicators_and_drawings_cannot_be_toggled_when_there_are_none_and_say_so() {
    let mut scenario = Scenario::new(styled());
    scenario.has_indicators = false;
    scenario.has_drawings = false;
    let mut rig = ready(&scenario);
    let at = rig.rect_of(Id::new(("studio-include", 0))).center();
    assert!(rig.click(&scenario, at).is_empty());
    let words = rig.hover(&scenario, at);
    assert!(
        words.iter().any(|w| w == "No indicators on this chart"),
        "{words:?}"
    );
    let at = rig.rect_of(Id::new(("studio-include", 1))).center();
    assert!(rig.click(&scenario, at).is_empty());
    let words = rig.hover(&scenario, at);
    assert!(
        words.iter().any(|w| w == "No visible drawings"),
        "{words:?}"
    );
    // The mark still works.
    let at = rig.rect_of(Id::new(("studio-include", 2))).center();
    assert_eq!(rig.click(&scenario, at).len(), 1);
}

#[test]
fn the_mark_cannot_be_toggled_in_the_chart_style() {
    let scenario = Scenario::new(chart_style());
    let mut rig = ready(&scenario);
    let at = rig.rect_of(Id::new(("studio-include", 2))).center();
    assert!(rig.click(&scenario, at).is_empty());
    let words = rig.hover(&scenario, at);
    assert!(
        words
            .iter()
            .any(|w| w == "The mark belongs to the Styled look"),
        "{words:?}"
    );
}

// MARK: Rulers

#[test]
fn dragging_a_ruler_left_raises_its_dial_and_letting_go_asks_for_the_snap() {
    let scenario = with_picture(styled());
    let mut rig = ready(&scenario);
    let at = rig.rect_of(Id::new(("studio-dial", 0))).center();
    let actions = rig.drag(&scenario, at, vec2(-30.0, 0.0));
    let Some(Action::DragDial(Dial::Width, value)) = actions
        .iter()
        .rev()
        .find(|a| matches!(a, Action::DragDial(..)))
        .copied()
    else {
        panic!("a width drag: {actions:?}");
    };
    assert!(value > 1.0, "{value}");
    assert_eq!(actions.last(), Some(&Action::ReleaseDial));
    assert!(
        !actions
            .iter()
            .any(|a| matches!(a, Action::DragDial(Dial::Height, _)))
    );
}

#[test]
fn the_height_ruler_asks_for_the_height_dial() {
    let scenario = with_picture(styled());
    let mut rig = ready(&scenario);
    let at = rig.rect_of(Id::new(("studio-dial", 1))).center();
    let actions = rig.drag(&scenario, at, vec2(30.0, 0.0));
    let last = actions
        .iter()
        .rev()
        .find(|a| matches!(a, Action::DragDial(..)))
        .copied();
    assert!(
        matches!(last, Some(Action::DragDial(Dial::Height, v)) if v < 1.0),
        "{last:?}"
    );
}

#[test]
fn a_focused_ruler_steps_with_the_arrows_and_pages_and_resets_with_home_and_zero() {
    let scenario = with_picture(styled());
    let mut rig = ready(&scenario);
    let at = rig.rect_of(Id::new(("studio-dial", 0))).center();
    rig.click(&scenario, at);
    assert_eq!(
        rig.key(&scenario, egui::Key::ArrowRight),
        [Action::StepDial(Dial::Width, Step::Fine, 1)]
    );
    assert_eq!(
        rig.key(&scenario, egui::Key::ArrowLeft),
        [Action::StepDial(Dial::Width, Step::Fine, -1)]
    );
    assert_eq!(
        rig.key(&scenario, egui::Key::PageUp),
        [Action::StepDial(Dial::Width, Step::Coarse, 1)]
    );
    assert_eq!(
        rig.key(&scenario, egui::Key::PageDown),
        [Action::StepDial(Dial::Width, Step::Coarse, -1)]
    );
    assert_eq!(
        rig.key(&scenario, egui::Key::Home),
        [Action::ResetDial(Dial::Width)]
    );
    assert_eq!(
        rig.key(&scenario, egui::Key::Num0),
        [Action::ResetDial(Dial::Width)]
    );
}

#[test]
fn a_double_click_on_a_ruler_resets_that_dial() {
    let mut config = styled();
    config.framing.height = 1.5;
    let scenario = with_picture(config);
    let mut rig = ready(&scenario);
    let at = rig.rect_of(Id::new(("studio-dial", 1))).center();
    let mut actions = rig.click(&scenario, at);
    actions.extend(rig.click(&scenario, at));
    assert!(
        actions.contains(&Action::ResetDial(Dial::Height)),
        "{actions:?}"
    );
}

#[test]
fn rulers_are_dimmed_and_explain_themselves_for_several_charts() {
    let mut scenario = with_picture(StudioConfig::new(Area::EntireLayout));
    scenario.reframable = false;
    scenario.areas = &THREE;
    let mut rig = ready(&scenario);
    let at = rig.rect_of(Id::new(("studio-dial", 0))).center();
    assert!(rig.drag(&scenario, at, vec2(-40.0, 0.0)).is_empty());
    let words = rig.hover(&scenario, at);
    assert!(
        words
            .iter()
            .any(|w| w == "Not available for several charts"),
        "{words:?}"
    );
}

#[test]
fn a_ruler_tells_its_value_in_words() {
    let mut config = styled();
    config.framing.width = 1.28;
    config.framing.height = 0.75;
    let scenario = with_picture(config);
    let rig = ready(&scenario);
    assert!(rig.words.iter().any(|w| w == "1.28×"), "{:?}", rig.words);
    assert!(rig.words.iter().any(|w| w == "0.75×"), "{:?}", rig.words);
}

// MARK: The preview

fn preview_centre(rig: &Rig) -> Pos2 {
    rig.rect_of(Id::new("studio-preview")).center()
}

#[test]
fn dragging_the_preview_reports_the_travel_in_points_of_the_image() {
    let scenario = with_picture(styled());
    let mut rig = ready(&scenario);
    let rect = rig.rect_of(Id::new("studio-preview"));
    let scale = COMPOSITION.x / rect.width();
    let actions = rig.drag(&scenario, rect.center(), vec2(-40.0, 20.0));
    let Some(Action::DragPreview(travel)) = actions
        .iter()
        .rev()
        .find(|a| matches!(a, Action::DragPreview(_)))
        .copied()
    else {
        panic!("a drag: {actions:?}");
    };
    assert!((travel.x + 40.0 * scale).abs() < 1.0, "{travel:?}");
    assert!((travel.y - 20.0 * scale).abs() < 1.0, "{travel:?}");
    assert_eq!(actions.last(), Some(&Action::ReleasePreview));
}

#[test]
fn the_preview_reports_that_a_drag_is_going_on_until_it_ends() {
    let scenario = with_picture(styled());
    let mut rig = ready(&scenario);
    let at = preview_centre(&rig);
    let moved = rig.move_to(at);
    rig.frame(&scenario, vec![moved]);
    rig.frame(&scenario, vec![rig.button(at, true)]);
    let moved = rig.move_to(at + vec2(-30.0, 0.0));
    let (output, _) = rig.frame(&scenario, vec![moved]);
    assert!(output.interacting);
    let (output, _) = rig.frame(&scenario, vec![rig.button(at + vec2(-30.0, 0.0), false)]);
    assert!(!output.interacting || output.actions.contains(&Action::ReleasePreview));
    let (output, _) = rig.frame(&scenario, Vec::new());
    assert!(!output.interacting);
}

#[test]
fn a_focused_preview_moves_with_the_arrows_and_goes_home_with_home_zero_or_a_double_click() {
    let scenario = with_picture(styled());
    let mut rig = ready(&scenario);
    let at = preview_centre(&rig);
    rig.click(&scenario, at);
    for (key, across, down) in [
        (egui::Key::ArrowLeft, -1, 0),
        (egui::Key::ArrowRight, 1, 0),
        (egui::Key::ArrowUp, 0, -1),
        (egui::Key::ArrowDown, 0, 1),
    ] {
        assert_eq!(
            rig.key(&scenario, key),
            [Action::NudgePreview { across, down }],
            "{key:?}"
        );
    }
    assert_eq!(rig.key(&scenario, egui::Key::Home), [Action::ResetPosition]);
    assert_eq!(rig.key(&scenario, egui::Key::Num0), [Action::ResetPosition]);

    let actions = rig.double_click(&scenario, at);
    assert!(actions.contains(&Action::ResetPosition), "{actions:?}");
}

#[test]
fn the_arrows_do_nothing_unless_the_preview_has_the_keyboard() {
    let scenario = with_picture(styled());
    let mut rig = ready(&scenario);
    assert!(rig.key(&scenario, egui::Key::ArrowLeft).is_empty());
}

#[test]
fn several_charts_cannot_be_moved() {
    let mut scenario = with_picture(StudioConfig::new(Area::EntireLayout));
    scenario.reframable = false;
    let mut rig = ready(&scenario);
    let at = preview_centre(&rig);
    assert!(rig.drag(&scenario, at, vec2(-50.0, 10.0)).is_empty());
    rig.click(&scenario, at);
    assert!(rig.key(&scenario, egui::Key::ArrowLeft).is_empty());
    let words = rig.hover(&scenario, at);
    assert!(
        words
            .iter()
            .any(|w| w.contains("not available for several charts")),
        "{words:?}"
    );
}

#[test]
fn the_preview_reports_the_room_it_has_in_pixels() {
    let scenario = Scenario::new(styled());
    let mut rig = ready(&scenario);
    let (output, _) = rig.frame(&scenario, Vec::new());
    let (width, height) = output.preview_pixels;
    let size = window_size(SCREEN);
    // The window's width less the margins and the well's inset, at the screen's 2 pixels a point.
    let expected_width = (size.x - 2.0 * PAD - 24.0) * 2.0;
    assert_eq!(width as f32, expected_width.floor());
    assert!(height > 200 && (height as f32) < size.y * 2.0, "{height}");
}

#[test]
fn the_preview_says_what_it_waits_for_or_why_there_is_nothing() {
    let scenario = Scenario::new(styled());
    let rig = ready(&scenario);
    assert!(rig.words.iter().any(|w| w == "Drawing the image…"));

    let mut failed = Scenario::new(styled());
    failed.preview = Shown::Failed("This image is too large to draw.");
    let rig = ready(&failed);
    assert!(
        rig.words
            .iter()
            .any(|w| w == "This image is too large to draw.")
    );
    assert!(!rig.words.iter().any(|w| w == "Drawing the image…"));
}

// MARK: Reset view, the footer and closing

#[test]
fn reset_view_is_there_only_when_the_framing_is_not_the_captures() {
    let scenario = with_picture(styled());
    let rig = ready(&scenario);
    assert!(!rig.has("Reset view"));

    for framing in [
        Framing {
            width: 1.2,
            ..Framing::default()
        },
        Framing {
            height: 0.9,
            ..Framing::default()
        },
        Framing {
            dx: 3.0,
            ..Framing::default()
        },
        Framing {
            dy: -0.1,
            ..Framing::default()
        },
    ] {
        let mut config = styled();
        config.framing = framing;
        let scenario = with_picture(config);
        let mut rig = ready(&scenario);
        let at = centre(&rig, "Reset view");
        assert_eq!(rig.click(&scenario, at), [Action::ResetView], "{framing:?}");
    }
}

#[test]
fn reset_view_is_not_offered_for_several_charts() {
    let mut config = StudioConfig::new(Area::EntireLayout);
    config.framing.width = 1.5;
    let mut scenario = with_picture(config);
    scenario.reframable = false;
    let rig = ready(&scenario);
    assert!(!rig.has("Reset view"));
}

#[test]
fn the_footer_tells_the_image_and_exports_it_two_ways() {
    let scenario = with_picture(styled());
    let mut rig = ready(&scenario);
    assert!(
        rig.words
            .iter()
            .any(|w| w == "Selected portion · Styled · 1,800 × 1,200 · PNG")
    );
    let at = centre(&rig, "Download PNG");
    assert_eq!(rig.click(&scenario, at), [Action::Export(Export::Download)]);
    let at = centre(&rig, "Copy image");
    assert_eq!(
        rig.click(&scenario, at),
        [Action::Export(Export::CopyImage)]
    );
}

#[test]
fn the_download_button_is_the_main_action_at_the_right_end() {
    let scenario = with_picture(styled());
    let rig = ready(&scenario);
    let (copy, download) = (rig.find("Copy image"), rig.find("Download PNG"));
    assert!(copy.right() < download.left());
    let size = window_size(SCREEN);
    let window_right = (SCREEN.x + size.x) / 2.0;
    assert!(
        (download.right() - (window_right - PAD)).abs() < 1.0,
        "{download:?}"
    );
}

#[test]
fn nothing_is_exported_while_an_export_runs_and_the_running_button_is_marked() {
    let mut scenario = with_picture(styled());
    scenario.exporting = Some(Export::Download);
    let mut rig = ready(&scenario);
    for label in ["Download PNG", "Copy image"] {
        let at = centre(&rig, label);
        assert!(rig.click(&scenario, at).is_empty(), "{label}");
    }
}

#[test]
fn a_picture_that_cannot_be_made_says_so_in_the_footer_and_blocks_both_buttons() {
    let mut scenario = Scenario::new(styled());
    scenario.summary = Err("This image is too large to draw.".to_owned());
    scenario.can_export = false;
    let mut rig = ready(&scenario);
    assert!(
        rig.words
            .iter()
            .any(|w| w == "This image is too large to draw.")
    );
    for label in ["Download PNG", "Copy image"] {
        let at = centre(&rig, label);
        assert!(rig.click(&scenario, at).is_empty(), "{label}");
    }
}

#[test]
fn escape_and_the_close_button_close() {
    let scenario = Scenario::new(styled());
    let mut rig = ready(&scenario);
    assert_eq!(rig.key(&scenario, egui::Key::Escape), [Action::Close]);
    let at = centre(&rig, "Close");
    assert_eq!(rig.click(&scenario, at), [Action::Close]);
}

#[test]
fn a_frame_with_nothing_done_asks_for_nothing() {
    let scenario = with_picture(styled());
    let mut rig = ready(&scenario);
    let (output, _) = rig.frame(&scenario, Vec::new());
    assert!(output.actions.is_empty() && !output.interacting);
}

// MARK: Size and wrapping

#[test]
fn the_window_is_most_of_the_screen_up_to_its_largest() {
    assert_eq!(
        window_size(vec2(3000.0, 2000.0)),
        vec2(MAX_WIDTH, MAX_HEIGHT)
    );
    assert_eq!(window_size(vec2(1000.0, 700.0)), vec2(920.0, 644.0));
    // The smallest a chart window gets.
    let smallest = window_size(vec2(960.0, 620.0));
    assert!(smallest.x >= 880.0 && smallest.y >= 560.0, "{smallest:?}");
}

#[test]
fn at_the_smallest_window_everything_is_inside_it_and_the_preview_keeps_room() {
    let scenario = with_picture(styled());
    let mut rig = Rig::new(vec2(960.0, 620.0), false);
    let (output, _) = rig.settle(&scenario);
    let window = window_size(vec2(960.0, 620.0));
    let left = (960.0 - window.x) / 2.0;
    let right = left + window.x;
    for label in [
        "Full chart",
        "Portion",
        "Chart",
        "Styled",
        "Original",
        "9:16",
        "Spectrum background",
        "Download PNG",
        "Copy image",
        "Close",
    ] {
        if !rig.has(label) {
            continue;
        }
        let rect = rig.find(label);
        assert!(
            rect.left() >= left && rect.right() <= right,
            "{label}: {rect:?}"
        );
    }
    let preview = rig.rect_of(Id::new("studio-preview"));
    assert!(preview.height() > 150.0, "{preview:?}");
    assert!(output.preview_pixels.1 > 300);
    let bottom = rig.find("Download PNG").bottom();
    assert!(bottom <= (620.0 + window.y) / 2.0, "{bottom}");
}

#[test]
fn groups_that_do_not_fit_wrap_and_every_group_is_somewhere() {
    // Five groups of the widths the window has them at the smallest window.
    let widths = [124.0, 102.0, 213.0, 168.0, 98.0];
    let wide = lines(&widths, 1_080.0, 29.0);
    assert_eq!(wide, vec![vec![0, 1, 2, 3, 4]]);
    let narrow = lines(&widths, 700.0, 29.0);
    assert!(narrow.len() >= 2);
    let mut seen: Vec<usize> = narrow.concat();
    seen.sort_unstable();
    assert_eq!(seen, [0, 1, 2, 3, 4]);
}

// MARK: Languages

/// The window in the pseudo-locale at its smallest size, in both themes: every word of it goes
/// through `tr!`, so none shows without its brackets, and the longer text still has room: no label
/// is cut. With
/// `DEGENVIEW_GALLERY_DIR` set the pictures are written out to look at.
#[test]
fn the_window_draws_in_the_pseudo_locale() {
    use crate::i18n::{Locale, tr, with_locale};
    use crate::ui::render_tests::{Screen, gallery_dir};
    let _restore = crate::ui::theme::RestoreDefaultTheme;
    with_locale(Locale::PSEUDO, || {
        for light in [false, true] {
            let scenario = Scenario::new(styled());
            let mut rig = Rig::new(vec2(960.0, 620.0), light);
            let (_, full) = rig.settle(&scenario);
            for word in [
                tr!("Studio"),
                tr!("Include"),
                tr!("Background"),
                tr!("Width"),
                tr!("Height"),
                tr!("Download PNG"),
                tr!("Copy image"),
            ] {
                assert!(word.starts_with('['), "{word} is not in the pseudo-locale");
                assert!(
                    rig.words.iter().any(|shown| shown == word),
                    "{word} is not drawn: {:?}",
                    rig.words
                );
            }
            assert_eq!(
                Screen::truncated_text(&full),
                Vec::<String>::new(),
                "a label was cut"
            );
            if gallery_dir().is_some() {
                let theme = if light { "light" } else { "dark" };
                rig.screen.save(&format!("studio-pseudo-{theme}"), full);
            }
        }
    });
}

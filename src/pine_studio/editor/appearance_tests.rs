//! The editor paints what its theme says, and a theme change touches nothing but paint.

use super::*;
use crate::pine::token::{DiagnosticCategory, SourcePosition, SourceRange};
use crate::ui::editor_appearance::RestorePublished;
use crate::ui::theme::{self, AppTheme, RestoreDefaultTheme};
use eframe::egui::epaint::ClippedShape;

const SCRIPT: &str = "//@version=6\nindicator(\"x\")\nlen = 14\nplot(close)\n";

/// Every colour the shapes fill or stroke with, flattened out of nested shapes.
fn colours(shapes: &[ClippedShape]) -> Vec<Color32> {
    fn walk(shape: &Shape, out: &mut Vec<Color32>) {
        match shape {
            Shape::Vec(shapes) => shapes.iter().for_each(|shape| walk(shape, out)),
            Shape::Rect(rect) => {
                out.push(rect.fill);
                out.push(rect.stroke.color);
            }
            Shape::LineSegment { stroke, .. } => out.push(stroke.color),
            Shape::Mesh(mesh) => out.extend(mesh.vertices.iter().map(|v| v.color)),
            Shape::Text(text) => {
                out.push(text.fallback_color);
                out.extend(text.galley.job.sections.iter().map(|s| s.format.color));
                // The field adds the selection to the galley's own mesh.
                for placed in &text.galley.rows {
                    out.extend(placed.row.visuals.mesh.vertices.iter().map(|v| v.color));
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for clipped in shapes {
        walk(&clipped.shape, &mut out);
    }
    out
}

fn error_at_plot() -> Diagnostic {
    let offset = SCRIPT.find("plot").unwrap();
    Diagnostic {
        code: "PINE3001".into(),
        severity: DiagnosticSeverity::Error,
        category: DiagnosticCategory::Semantic,
        message: "message".into(),
        range: SourceRange::new(
            SourcePosition::new(4, 1, offset),
            SourcePosition::new(4, 5, offset + 4),
        ),
    }
}

/// Runs the editor for a few frames, jumping the caret to the error on the second.
fn paint(ctx: &egui::Context, state: &mut EditorState) -> Vec<Color32> {
    let mut last = Vec::new();
    for frame in 0..4 {
        if frame == 1 {
            let diagnostic = state.diagnostics[0].clone();
            state.reveal(&diagnostic);
        }
        let raw = egui::RawInput {
            time: Some(f64::from(frame) * 0.1),
            screen_rect: Some(Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                Vec2::new(800.0, 400.0),
            )),
            ..Default::default()
        };
        let mut output = ctx.run_ui(raw, |ui| {
            show(ui, state);
        });
        last = colours(&output.shapes);
        output.textures_delta.clear();
    }
    last
}

#[test]
fn the_editor_paints_the_surfaces_of_the_appearance() {
    for app_theme in [AppTheme::Dark, AppTheme::Light] {
        let _restore = RestoreDefaultTheme;
        let ctx = egui::Context::default();
        theme::install_offscreen(&ctx);
        theme::apply(&ctx, app_theme);
        let look = editor_appearance::current();
        let mut state = EditorState::new("s", SCRIPT, "");
        state.diagnostics = vec![error_at_plot()];
        let painted = paint(&ctx, &mut state);
        for (surface, what) in [
            (SurfaceColor::Background, "editor background"),
            (SurfaceColor::GutterBackground, "gutter"),
            (SurfaceColor::CurrentLine, "current line"),
            (SurfaceColor::DiagnosticError, "error underline"),
        ] {
            assert!(
                painted.contains(&look.surface(surface)),
                "{app_theme:?}: no {what} ({:?})",
                look.surface(surface)
            );
        }
        // The gutter's line numbers and the syntax colours of the script come through too.
        assert!(painted.contains(&look.surface(SurfaceColor::LineNumber)));
        for role in [
            SyntaxRole::Directive,
            SyntaxRole::BuiltinFunction,
            SyntaxRole::String,
            SyntaxRole::Number,
            SyntaxRole::BuiltinVariable,
        ] {
            assert!(
                painted.contains(&look.role(role)),
                "{app_theme:?}: no {role:?}"
            );
        }
    }
}

#[test]
fn a_theme_change_repaints_without_touching_the_script() {
    let _restore = RestoreDefaultTheme;
    let ctx = egui::Context::default();
    theme::install_offscreen(&ctx);
    theme::apply(&ctx, AppTheme::Dark);
    let mut state = EditorState::new("s", SCRIPT, SCRIPT);
    state.diagnostics = vec![error_at_plot()];
    let dark = paint(&ctx, &mut state);
    let caret = TextEditState::load(&ctx, state.id).and_then(|edit| edit.cursor.char_range());
    let spans = state
        .highlighted
        .as_ref()
        .map(|(_, spans)| Arc::as_ptr(spans));
    assert!(spans.is_some());

    theme::apply(&ctx, AppTheme::Light);
    let light = paint(&ctx, &mut state);

    assert_ne!(dark, light, "the colours changed");
    assert!(light.contains(&editor_appearance::current().surface(SurfaceColor::Background)));
    assert_eq!(state.source, SCRIPT, "the script is untouched");
    assert!(!state.is_dirty());
    // The caret stays where the first run put it (the second run's jump put it there again).
    assert_eq!(
        TextEditState::load(&ctx, state.id).and_then(|edit| edit.cursor.char_range()),
        caret
    );
    // Nothing was classified again: spans carry roles, not colours.
    assert_eq!(
        state
            .highlighted
            .as_ref()
            .map(|(_, spans)| Arc::as_ptr(spans)),
        spans
    );
}

#[test]
fn the_selection_and_the_caret_wear_the_theme() {
    let _restore = RestoreDefaultTheme;
    let ctx = egui::Context::default();
    theme::install_offscreen(&ctx);
    theme::apply(&ctx, AppTheme::Dark);
    let look = editor_appearance::current();
    let mut state = EditorState::new("s", SCRIPT, "");
    state.diagnostics = vec![error_at_plot()];
    // Frame 0 lays the text out; select `len` before frame 1 so the field paints the selection.
    let _ = paint(&ctx, &mut state);
    let mut edit = TextEditState::load(&ctx, state.id).unwrap_or_default();
    let at = SCRIPT.find("len").unwrap();
    edit.cursor.set_char_range(Some(CCursorRange::two(
        CCursor::new(at),
        CCursor::new(at + 3),
    )));
    edit.store(&ctx, state.id);
    ctx.memory_mut(|memory| memory.request_focus(state.id));
    let mut output = ctx.run_ui(
        egui::RawInput {
            time: Some(1.0),
            screen_rect: Some(Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                Vec2::new(800.0, 400.0),
            )),
            ..Default::default()
        },
        |ui| {
            show(ui, &mut state);
        },
    );
    output.textures_delta.clear();
    let painted = colours(&output.shapes);
    assert!(painted.contains(&look.surface(SurfaceColor::Selection)));
    assert!(painted.contains(&look.surface(SurfaceColor::Caret)));
}

#[test]
fn typed_text_is_classified_once_and_kept_for_the_next_frame() {
    let _restore = RestoreDefaultTheme;
    let ctx = egui::Context::default();
    theme::install_offscreen(&ctx);
    let mut state = EditorState::new("s", SCRIPT, SCRIPT);
    let _ = paint_plain(&ctx, &mut state, 0.0, Vec::new());
    ctx.memory_mut(|memory| memory.request_focus(state.id));
    let _ = paint_plain(&ctx, &mut state, 0.1, Vec::new());
    let before = state.highlighted.as_ref().map(|(key, _)| *key);
    let _ = paint_plain(
        &ctx,
        &mut state,
        0.2,
        vec![egui::Event::Text("x".to_owned())],
    );
    assert_ne!(state.source, SCRIPT, "the key reached the text");
    let (key, _) = state.highlighted.clone().expect("classified");
    assert_ne!(Some(key), before, "the new text was classified");
    assert_eq!(key, text_hash(&state.source), "and kept for the next frame");
    let (table_key, _) = state.nav.table.clone().expect("analysed");
    assert_eq!(table_key, key, "with the symbols that went with it");
}

fn paint_plain(
    ctx: &egui::Context,
    state: &mut EditorState,
    time: f64,
    events: Vec<egui::Event>,
) -> Vec<Color32> {
    let mut output = ctx.run_ui(
        egui::RawInput {
            time: Some(time),
            events,
            screen_rect: Some(Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                Vec2::new(800.0, 400.0),
            )),
            ..Default::default()
        },
        |ui| {
            show(ui, state);
        },
    );
    let painted = colours(&output.shapes);
    output.textures_delta.clear();
    painted
}

fn font(size: f32, line_height: crate::model::editor_theme::LineHeight) -> EditorFont {
    EditorFont {
        custom: false,
        size,
        line_height,
    }
}

#[test]
fn the_gutter_grows_with_the_font_and_the_script_but_never_shrinks() {
    let ctx = egui::Context::default();
    theme::install_offscreen(&ctx);
    ctx.run_ui(egui::RawInput::default(), |_| {})
        .textures_delta
        .clear();
    let mut widths = Vec::new();
    ctx.run_ui(egui::RawInput::default(), |ui| {
        use crate::model::editor_theme::LineHeight::Default;
        for (size, lines) in [
            (9.0, 10),
            (13.0, 10),
            (13.0, 99_999),
            (32.0, 10),
            (32.0, 99_999),
        ] {
            widths.push(gutter_width(ui, &font(size, Default), lines));
        }
    })
    .textures_delta
    .clear();
    assert_eq!(
        widths[0], GUTTER_WIDTH,
        "a small font keeps the gutter it had"
    );
    assert_eq!(widths[1], GUTTER_WIDTH, "so does 13 points: nothing moved");
    assert!(
        widths[2] > widths[1],
        "five digits need more room: {widths:?}"
    );
    assert!(widths[3] > widths[1], "a large font needs more: {widths:?}");
    assert!(widths[4] > widths[3] && widths[4] > widths[2], "{widths:?}");
}

#[test]
fn line_numbers_follow_the_editor_size_two_points_smaller() {
    use crate::model::editor_theme::LineHeight::Default;
    assert_eq!(line_number_size(&font(13.0, Default)), LINE_NUMBER_SIZE);
    assert_eq!(line_number_size(&font(20.0, Default)), 18.0);
    assert!(line_number_size(&font(9.0, Default)) >= 7.0);
}

#[test]
fn the_editor_draws_at_the_smallest_and_largest_size_with_every_line_height() {
    use crate::model::editor_theme::LineHeight;
    for size in [9.0, 13.0, 32.0] {
        for height in LineHeight::ALL {
            let _theme = RestoreDefaultTheme;
            let _published = RestorePublished;
            editor_appearance::publish(None, font(size, height));
            let ctx = egui::Context::default();
            theme::install_offscreen(&ctx);
            let mut state = EditorState::new("s", SCRIPT, SCRIPT);
            state.diagnostics = vec![error_at_plot()];
            let painted = paint(&ctx, &mut state);
            assert!(!painted.is_empty(), "{size} {height:?}");
            assert_eq!(state.source, SCRIPT);
        }
    }
}

#[test]
fn a_taller_line_height_makes_taller_rows_and_the_default_changes_nothing() {
    use crate::model::editor_theme::LineHeight;
    let ctx = egui::Context::default();
    theme::install_offscreen(&ctx);
    ctx.run_ui(egui::RawInput::default(), |_| {})
        .textures_delta
        .clear();
    let look = Appearance::of(crate::model::editor_theme::BuiltInTheme::Dark);
    let mut heights = Vec::new();
    ctx.run_ui(egui::RawInput::default(), |ui| {
        for height in LineHeight::ALL {
            let code = font(14.0, height);
            let job = syntax::layout_job(
                "a = 1\nb = 2\n",
                &[],
                code.code(ui.ctx()),
                code.line_height_points(),
                &look,
                look.role(SyntaxRole::Default),
            );
            let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
            heights.push(galley.rows[0].height());
        }
    })
    .textures_delta
    .clear();
    // Compact < Default (the font's own) < Relaxed < Loose, in points.
    assert!(
        heights[1] < heights[2] && heights[2] < heights[3],
        "{heights:?}"
    );
    let own = {
        let mut own = 0.0;
        ctx.run_ui(egui::RawInput::default(), |ui| {
            let id = egui::FontId::monospace(14.0);
            own = ui.fonts_mut(|fonts| fonts.row_height(&id));
        })
        .textures_delta
        .clear();
        own
    };
    assert!(
        (heights[1] - own).abs() < 1.0,
        "Default is the font's own height: {heights:?} {own}"
    );
    assert!((heights[3] - 14.0 * 1.75).abs() < 0.6, "{heights:?}");
}

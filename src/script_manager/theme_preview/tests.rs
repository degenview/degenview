use super::*;
use crate::model::editor_theme::{SurfaceColor, SyntaxRole};
use crate::script_manager::symbols;
use crate::script_manager::syntax;
use crate::ui::editor_appearance;
use crate::ui::theme::{self, AppTheme, RestoreDefaultTheme};
use std::collections::HashSet;

/// The sample as a script that is meant to compile: without the two lines that carry diagnostics.
fn clean_sample() -> &'static str {
    let at = SAMPLE
        .find("// Problems are underlined")
        .expect("the problem lines");
    &SAMPLE[..at]
}

#[test]
fn the_sample_is_valid_pine_apart_from_its_two_marked_lines() {
    let program = crate::pine::compiler::compile(
        clean_sample(),
        crate::pine::token::Limits::default(),
        &crate::pine::compiler::NoLibraries,
    );
    assert!(program.diagnostics.is_empty(), "{:?}", program.diagnostics);
    // The line the preview marks as an error really is one.
    let broken = crate::pine::compiler::compile(
        SAMPLE,
        crate::pine::token::Limits::default(),
        &crate::pine::compiler::NoLibraries,
    );
    assert!(
        broken
            .diagnostics
            .iter()
            .any(|d| d.severity == DiagnosticSeverity::Error),
        "the typo is not a name"
    );
}

#[test]
fn the_sample_shows_every_syntax_role() {
    let spans = syntax::classify(SAMPLE, Some(&symbols::analyze(SAMPLE)));
    let shown: HashSet<SyntaxRole> = spans.iter().map(|span| span.role).collect();
    for role in SyntaxRole::ALL {
        // Plain text has no span: it is the colour of everything between them.
        if role == SyntaxRole::Default {
            continue;
        }
        assert!(shown.contains(&role), "the sample has nothing for {role:?}");
    }
}

#[test]
fn the_marked_lines_carry_one_error_and_one_warning() {
    let preview = ThemePreview::new();
    let kinds: Vec<_> = preview
        .state
        .diagnostics
        .iter()
        .map(|d| d.severity)
        .collect();
    assert_eq!(
        kinds,
        [DiagnosticSeverity::Error, DiagnosticSeverity::Warning]
    );
    for diagnostic in &preview.state.diagnostics {
        let offset = diagnostic.range.start.offset;
        let line = SAMPLE[..offset].matches('\n').count() + 1;
        assert_eq!(diagnostic.range.start.line, line);
    }
}

#[test]
fn the_preview_is_the_real_editor_in_the_apps_appearance() {
    for app_theme in [AppTheme::Dark, AppTheme::Light] {
        let _restore = RestoreDefaultTheme;
        let ctx = egui::Context::default();
        theme::install_offscreen(&ctx);
        theme::apply(&ctx, app_theme);
        let look = editor_appearance::current();
        let mut preview = ThemePreview::new();
        let mut painted = Vec::new();
        for frame in 0..4 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    time: Some(f64::from(frame) * 0.1),
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::pos2(0.0, 0.0),
                        Vec2::new(700.0, 500.0),
                    )),
                    ..Default::default()
                },
                |ui| preview.show(ui, Vec2::new(640.0, 360.0)),
            );
            painted = output
                .shapes
                .iter()
                .map(|clipped| format!("{:?}", clipped.shape))
                .collect();
            output.textures_delta.clear();
        }
        let flat = painted.join("\n");
        // The editor's own background and current-line band are in what was drawn.
        let background = look.surface(SurfaceColor::Background);
        assert!(
            flat.contains(&format!("{background:?}")),
            "{app_theme:?}: no editor background"
        );
        assert_eq!(preview.text(), SAMPLE, "drawing it never edits it");
    }
}

#[test]
fn the_caret_starts_where_a_name_is_used_twice() {
    let ctx = egui::Context::default();
    theme::install_offscreen(&ctx);
    let mut preview = ThemePreview::new();
    for frame in 0..2 {
        let mut output = ctx.run_ui(
            egui::RawInput {
                time: Some(f64::from(frame) * 0.1),
                ..Default::default()
            },
            |ui| preview.show(ui, Vec2::new(500.0, 300.0)),
        );
        output.textures_delta.clear();
    }
    let at = preview.state.caret_index(&ctx).expect("a caret");
    let word: String = SAMPLE.chars().skip(at.saturating_sub(3)).take(7).collect();
    assert_eq!(word, "average", "{word:?}");
    assert!(SAMPLE.matches("average").count() >= 2);
}

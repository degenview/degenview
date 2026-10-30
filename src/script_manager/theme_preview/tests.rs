use super::*;
use crate::model::editor_theme::{SurfaceColor, SyntaxRole};
use crate::pine::token::DiagnosticSeverity;
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
    let diagnostics = &preview.state.diagnostics;
    assert_eq!(diagnostics.len(), 2);
    // They are the compiler's, and sit on the lines that cause them.
    let line_of = |needle: &str| SAMPLE[..SAMPLE.find(needle).unwrap()].matches('\n').count() + 1;
    let on = |severity| {
        let found: Vec<_> = diagnostics
            .iter()
            .filter(|d| d.severity == severity)
            .collect();
        assert_eq!(found.len(), 1, "{severity:?}");
        found[0].range.start.line
    };
    assert_eq!(on(DiagnosticSeverity::Error), line_of("plot(missing)"));
    assert_eq!(on(DiagnosticSeverity::Warning), line_of("hidden ="));
    assert_eq!(preview.state.status, CompileStatus::Error);
}

/// The text the first problem underlines, as the editor finds it.
fn underlined(preview: &ThemePreview, severity: DiagnosticSeverity) -> Vec<String> {
    let text: Vec<char> = preview.state.source.chars().collect();
    preview
        .state
        .diagnostics
        .iter()
        .filter(|d| d.severity == severity)
        .map(|d| {
            let from = crate::script_manager::syntax::compiled_utf16_to_char_index(
                &preview.state.source,
                d.range.start.offset,
            );
            let to = crate::script_manager::syntax::compiled_utf16_to_char_index(
                &preview.state.source,
                d.range.end.offset,
            );
            text[from..to].iter().collect()
        })
        .collect()
}

#[test]
fn the_problems_follow_the_code_when_lines_are_added_or_removed() {
    let mut preview = ThemePreview::new();
    assert_eq!(underlined(&preview, DiagnosticSeverity::Error), ["missing"]);
    let line_before = preview.state.diagnostics[0].range.start.line;

    // Lines added above: the underline moves down with the code.
    preview
        .state
        .source
        .insert_str(0, "// one\n// two\n// three\n");
    preview.recompile();
    assert_eq!(
        preview.state.diagnostics[0].range.start.line,
        line_before + 3,
        "the error is three lines further down"
    );
    assert_eq!(underlined(&preview, DiagnosticSeverity::Error), ["missing"]);
    assert_eq!(underlined(&preview, DiagnosticSeverity::Warning).len(), 1);

    // Typing inside the line moves the rest of it, and the underline with it.
    preview.state.source = preview
        .state
        .source
        .replace("plot(missing)", "plot(  missing)");
    preview.recompile();
    assert_eq!(underlined(&preview, DiagnosticSeverity::Error), ["missing"]);

    // Fixing it removes it; the warning is left; breaking something else adds a new one.
    preview.state.source = preview
        .state
        .source
        .replace("plot(  missing)", "plot(close)");
    preview.recompile();
    assert!(underlined(&preview, DiagnosticSeverity::Error).is_empty());
    assert_eq!(preview.state.status, CompileStatus::Warning);
    preview.state.source.push_str("plot(nothing_here)\n");
    preview.recompile();
    assert_eq!(
        underlined(&preview, DiagnosticSeverity::Error),
        ["nothing_here"]
    );

    // Everything fixed: nothing is underlined and the status says so.
    preview.state.source = clean_sample().to_owned();
    preview.recompile();
    assert!(preview.state.diagnostics.is_empty());
    assert_eq!(preview.state.status, CompileStatus::Valid);
}

#[test]
fn the_text_is_compiled_only_when_it_changes() {
    let mut preview = ThemePreview::new();
    let first = preview.compiled;
    preview.recompile();
    assert_eq!(
        preview.compiled, first,
        "the same text is not compiled again"
    );
    // A diagnostic list handed in by hand survives a call that has nothing to do.
    preview.state.diagnostics.clear();
    preview.recompile();
    assert!(preview.state.diagnostics.is_empty());
    preview.state.source.push('\n');
    preview.recompile();
    assert_ne!(preview.compiled, first);
    assert!(!preview.state.diagnostics.is_empty());
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

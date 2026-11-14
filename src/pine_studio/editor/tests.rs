use super::completion_view::{COMPLETION_ROW_HEIGHT, COMPLETION_ROWS, accept_completion};
use super::diff::ChangeKind;
use super::gutter::changes::apply_hunk_edit;
use super::indent_guides::indent_levels;
use super::status::status_chip;
use super::*;
use crate::pine::syntax::token::DiagnosticSeverity;
use crate::pine::syntax::token::{DiagnosticCategory, SourcePosition, SourceRange};
use eframe::egui::Color32;

fn diagnostic(
    severity: DiagnosticSeverity,
    line: usize,
    column: usize,
    offset: usize,
) -> Diagnostic {
    Diagnostic {
        code: "PINE3001".into(),
        severity,
        category: DiagnosticCategory::Semantic,
        message: "message".into(),
        range: SourceRange::new(
            SourcePosition::new(line, column, offset),
            SourcePosition::new(line, column + 1, offset + 1),
        ),
    }
}

#[test]
fn the_chip_reads_like_swift() {
    let error = diagnostic(DiagnosticSeverity::Error, 1, 1, 0);
    let warning = diagnostic(DiagnosticSeverity::Warning, 1, 1, 0);
    assert_eq!(
        status_chip(CompileStatus::NotCompiled, &[]).2,
        "Not compiled"
    );
    assert_eq!(status_chip(CompileStatus::Valid, &[]).2, "No problems");
    assert_eq!(
        status_chip(CompileStatus::Warning, std::slice::from_ref(&warning)).2,
        "1 warning"
    );
    assert_eq!(
        status_chip(CompileStatus::Warning, &[warning.clone(), warning]).2,
        "2 warnings"
    );
    assert_eq!(
        status_chip(CompileStatus::Error, &[error.clone(), error]).2,
        "2 errors"
    );
}

#[test]
fn blank_lines_keep_only_shared_guides() {
    let levels = indent_levels("a\n    b\n        c\n\n    d\n\n");
    assert_eq!(levels, [0, 1, 2, 1, 1, 0, 0]);
}

#[test]
fn a_compile_waits_for_typing_to_pause() {
    let mut state = EditorState::new("s", "plot(close)", "plot(close)");
    assert!(!state.compile_pending());
    state.request_compile();
    let start = state.compile_requested.unwrap();
    assert!(state.compile_pending());
    assert!(!state.compile_due(start + Duration::from_millis(100)));
    assert!(state.compile_due(start + COMPILE_DELAY));
    // Another keystroke starts the wait again.
    state.request_compile();
    let again = state.compile_requested.unwrap();
    assert!(!state.compile_due(again + COMPILE_DELAY - Duration::from_millis(1)));
    state.compiled();
    assert!(!state.compile_pending());
    assert!(!state.compile_due(again + COMPILE_DELAY));
}

#[test]
fn dirty_tracking_and_draft_debounce() {
    let mut state = EditorState::new("s", "plot(close)", "plot(close)");
    assert!(!state.is_dirty());
    state.source.push('\n');
    assert!(state.is_dirty());
    let start = Instant::now();
    state.last_edit = Some(start);
    assert!(!state.take_draft_due(start + Duration::from_millis(400)));
    assert!(state.take_draft_due(start + DRAFT_DELAY + Duration::from_millis(10)));
    assert!(
        !state.take_draft_due(start + Duration::from_secs(30)),
        "once per edit"
    );
    state.mark_saved();
    assert!(!state.is_dirty());
}

#[test]
fn the_editor_draws_headlessly_with_problems_and_a_jump() {
    let mut state = EditorState::new("s", "//@version=6\nindicator(\"x\")\nplot(close)\n", "");
    state.diagnostics = vec![diagnostic(DiagnosticSeverity::Error, 3, 1, 26)];
    state.status = CompileStatus::Error;
    let ctx = egui::Context::default();
    for frame in 0..3 {
        if frame == 1 {
            let d = state.diagnostics[0].clone();
            state.reveal(&d);
        }
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let result = show(ui, &mut state);
            assert!(!result.save);
        });
        output.textures_delta.clear();
    }
}

#[test]
fn a_diagnostic_tooltip_hangs_off_the_underlined_text() {
    let mut state = EditorState::new("s", "//@version=6\nindicator(\"x\")\nplot(close)\n", "");
    // `plot` on the third line.
    state.diagnostics = vec![diagnostic(DiagnosticSeverity::Error, 3, 1, 26)];
    let ctx = egui::Context::default();
    ctx.all_styles_mut(|style| {
        style.interaction.tooltip_delay = 0.0;
        style.interaction.show_tooltips_only_when_still = false;
    });
    let screen = Rect::from_min_size(egui::pos2(0.0, 0.0), Vec2::new(900.0, 700.0));
    let mut time = 0.0;
    let mut frame = |state: &mut EditorState, pointer: Option<egui::Pos2>| {
        time += 0.1;
        let mut input = egui::RawInput {
            screen_rect: Some(screen),
            time: Some(time),
            ..Default::default()
        };
        input.events.push(match pointer {
            Some(pos) => egui::Event::PointerMoved(pos),
            None => egui::Event::PointerGone,
        });
        let mut output = ctx.run_ui(input, |ui| {
            show(ui, state);
        });
        output.textures_delta.clear();
    };
    let tooltip_top = |ctx: &egui::Context| {
        ctx.memory(|memory| {
            memory
                .layer_ids()
                .filter(|layer| layer.order == egui::Order::Tooltip)
                .find_map(|layer| memory.area_rect(layer.id))
        })
    };
    // Sweep the pointer over the top rows until the underline shows its message.
    let mut found = None;
    'sweep: for y in (10..40).step_by(3) {
        for x in (100..200).step_by(4) {
            let pos = egui::pos2(x as f32, y as f32);
            for _ in 0..3 {
                frame(&mut state, Some(pos));
            }
            if let Some(rect) = tooltip_top(&ctx) {
                found = Some((pos, rect));
                break 'sweep;
            }
        }
    }
    let (pointer, tooltip) = found.expect("hovering the underline shows its message");
    assert!(
        tooltip.min.y < pointer.y + 40.0,
        "tooltip at y={} for a pointer at y={}",
        tooltip.min.y,
        pointer.y
    );
}

#[test]
fn the_find_bar_steps_through_matches_and_draws() {
    let mut state = EditorState::new("s", "plot(close)\nplot(open)\nplot(high)\n", "");
    state.find.open = true;
    state.find.query = "PLOT".into();
    step_match(&mut state, true);
    assert_eq!(state.find.current, 1);
    assert_eq!(state.select_to, Some(12..16));
    step_match(&mut state, true);
    step_match(&mut state, true);
    assert_eq!(state.find.current, 0, "wraps past the last match");
    step_match(&mut state, false);
    assert_eq!(state.find.current, 2, "and back");
    state.find.query = "nothing".into();
    step_match(&mut state, true);
    assert_eq!(state.find.current, 2, "no matches: nowhere to go");

    state.find.query = "plot".into();
    let ctx = egui::Context::default();
    for _ in 0..3 {
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            show(ui, &mut state);
        });
        output.textures_delta.clear();
    }
}

#[test]
fn accepting_a_suggestion_replaces_the_typed_prefix() {
    let mut state = EditorState::new("s", "x = ta.pi", "");
    let caret = state.source.chars().count();
    let completion = completion::complete(&state.source, caret, false).expect("suggestions");
    assert_eq!(completion.items[0].label, "ta.pivot_point_levels");
    state.completion = Some(CompletionView {
        completion,
        selected: 2,
        scroll_to_selected: false,
    });
    let ctx = egui::Context::default();
    assert!(accept_completion(&ctx, &mut state));
    assert_eq!(state.source, "x = ta.pivotlow()");
    assert!(state.completion.is_none());
    assert!(
        !accept_completion(&ctx, &mut state),
        "nothing left to accept"
    );
}

#[test]
fn the_editor_draws_with_suggestions_open() {
    let mut state = EditorState::new("s", "plot(cl", "");
    state.completion_wanted = true;
    let ctx = egui::Context::default();
    for _ in 0..3 {
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            show(ui, &mut state);
        });
        output.textures_delta.clear();
    }
}

/// Accepts the suggestion `label` for `source` (caret at its end); returns the new text and caret.
fn accept(source: &str, label: &str) -> (String, usize) {
    let mut state = EditorState::new("s", source, "");
    let caret = source.chars().count();
    let completion = completion::complete(source, caret, true).expect("suggestions");
    let selected = completion
        .items
        .iter()
        .position(|item| item.label == label)
        .expect("the suggestion is offered");
    state.completion = Some(CompletionView {
        completion,
        selected,
        scroll_to_selected: false,
    });
    let ctx = egui::Context::default();
    TextEditState::default().store(&ctx, state.id);
    assert!(accept_completion(&ctx, &mut state));
    let caret = TextEditState::load(&ctx, state.id)
        .and_then(|edit| edit.cursor.char_range())
        .map(|range| range.primary.index.0);
    (state.source, caret.unwrap_or(usize::MAX))
}

#[test]
fn accepting_a_function_adds_parentheses_with_the_caret_between() {
    assert_eq!(accept("x = ta.sm", "ta.sma"), ("x = ta.sma()".into(), 11));
    assert_eq!(
        accept("c = color.ne", "color.new"),
        ("c = color.new()".into(), 14)
    );
    let own = "half(x) => x / 2\nha";
    assert_eq!(accept(own, "half"), ("half(x) => x / 2\nhalf()".into(), 22));
}

#[test]
fn accepting_anything_else_adds_no_parentheses() {
    assert_eq!(
        accept("c = color.re", "color.red"),
        ("c = color.red".into(), 13)
    );
    assert_eq!(accept("x = ba", "barstate"), ("x = barstate".into(), 12));
    assert_eq!(accept("x = clo", "close"), ("x = close".into(), 9));
    // The call is already there: only the name is replaced.
    let (text, caret) = accept("ta.sm", "ta.sma");
    assert_eq!((text.as_str(), caret), ("ta.sma()", 7));
    let ctx = egui::Context::default();
    let mut state = EditorState::new("s", "ta.sm(close, 3)", "");
    let completion = completion::complete(&state.source, 5, true).expect("suggestions");
    let selected = completion
        .items
        .iter()
        .position(|i| i.label == "ta.sma")
        .unwrap();
    state.completion = Some(CompletionView {
        completion,
        selected,
        scroll_to_selected: false,
    });
    assert!(accept_completion(&ctx, &mut state));
    assert_eq!(state.source, "ta.sma(close, 3)");
}

/// Draws the editor with `source` focused and the caret at its end, `frames` times, sending
/// `events` on the first of them; returns the last frame's shapes.
fn draw_suggestions(
    ctx: &egui::Context,
    state: &mut EditorState,
    clock: &mut f64,
    events: Vec<Event>,
) -> Vec<egui::epaint::ClippedShape> {
    let mut shapes = Vec::new();
    for frame in 0..3 {
        let raw = egui::RawInput {
            time: Some(*clock),
            events: if frame == 0 {
                events.clone()
            } else {
                Vec::new()
            },
            ..Default::default()
        };
        let mut output = ctx.run_ui(raw, |ui| {
            show(ui, state);
        });
        output.textures_delta.clear();
        shapes = output.shapes;
        // Long scrolls animate: leave time for them to land.
        *clock += 1.0;
    }
    shapes
}

/// The rect of the highlighted suggestion and the clip it is drawn through.
fn highlighted_row(shapes: &[egui::epaint::ClippedShape]) -> Option<(Rect, Rect)> {
    shapes.iter().find_map(|clipped| match &clipped.shape {
        egui::Shape::Rect(rect)
            if rect.fill == Color32::from_rgb(0x58, 0x62, 0x69)
                && rect.rect.height() == COMPLETION_ROW_HEIGHT =>
        {
            Some((rect.rect, clipped.clip_rect))
        }
        _ => None,
    })
}

#[test]
fn the_suggestion_list_shows_ten_rows_and_follows_the_arrow_keys() {
    let ctx = egui::Context::default();
    let mut state = EditorState::new("s", "color.", "");
    let mut clock = 0.0;
    let mut edit = TextEditState::default();
    edit.cursor
        .set_char_range(Some(CCursorRange::one(CCursor::new(6usize))));
    edit.store(&ctx, state.id);
    ctx.memory_mut(|memory| memory.request_focus(state.id));
    state.completion_wanted = true;
    state.last_caret = Some(6);
    draw_suggestions(&ctx, &mut state, &mut clock, Vec::new());
    let count = state
        .completion
        .as_ref()
        .expect("suggestions")
        .completion
        .items
        .len();
    assert!(count > COMPLETION_ROWS, "color. offers {count}");
    let area = ctx
        .memory(|memory| memory.area_rect(Id::new("script-completion")))
        .expect("the list was laid out");
    let rows = COMPLETION_ROWS as f32 * COMPLETION_ROW_HEIGHT;
    assert!(
        area.height() >= rows && area.height() < rows + 40.0,
        "the list is {} tall for {rows} of rows",
        area.height()
    );

    // Down through more rows than fit: the highlight stays inside what the list shows.
    for step in 1..=count + 2 {
        let shapes = draw_suggestions(
            &ctx,
            &mut state,
            &mut clock,
            vec![key_press(Key::ArrowDown)],
        );
        let (row, clip) = highlighted_row(&shapes).expect("a highlighted row");
        assert!(
            clip.contains_rect(row),
            "step {step}: the highlight {row:?} is outside the list {clip:?}"
        );
    }
    // Up wraps to the last row and scrolls back through the list.
    for step in 1..=count + 2 {
        let shapes = draw_suggestions(&ctx, &mut state, &mut clock, vec![key_press(Key::ArrowUp)]);
        let (row, clip) = highlighted_row(&shapes).expect("a highlighted row");
        assert!(
            clip.contains_rect(row),
            "up step {step}: the highlight {row:?} is outside the list {clip:?}"
        );
    }
}

fn key_press(key: Key) -> Event {
    Event::Key {
        key,
        physical_key: Some(key),
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    }
}

#[test]
fn the_editor_draws_a_signature_panel_inside_a_call() {
    let mut state = EditorState::new("s", "plot(close, ", "");
    let ctx = egui::Context::default();
    // Put the caret at the end of the text and focus the editor, then draw a few frames.
    let mut edit = TextEditState::default();
    edit.cursor
        .set_char_range(Some(CCursorRange::one(CCursor::new(12usize))));
    edit.store(&ctx, state.id);
    ctx.memory_mut(|memory| memory.request_focus(state.id));
    for _ in 0..3 {
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            show(ui, &mut state);
        });
        output.textures_delta.clear();
    }
    assert!(signature::help(&state.source, 12).is_some());
    assert!(
        ctx.memory(|memory| memory.area_rect(Id::new("script-signature")))
            .is_some(),
        "the panel was laid out"
    );
}

/// The signature panel's rect for a caret inside `source`, in a window `width` wide.
fn signature_panel(source: &str, width: f32) -> Rect {
    let mut state = EditorState::new("s", source, "");
    let ctx = egui::Context::default();
    let caret = source.chars().count();
    let mut edit = TextEditState::default();
    edit.cursor
        .set_char_range(Some(CCursorRange::one(CCursor::new(caret))));
    edit.store(&ctx, state.id);
    ctx.memory_mut(|memory| memory.request_focus(state.id));
    for _ in 0..4 {
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                Vec2::new(width, 700.0),
            )),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            show(ui, &mut state);
        });
        output.textures_delta.clear();
    }
    ctx.memory(|memory| memory.area_rect(Id::new("script-signature")))
        .expect("the panel was laid out")
}

#[test]
fn a_long_signature_stays_on_one_line_while_the_window_has_room() {
    // `strategy(...)` is ~135 characters: far past the old 540-point wrap.
    let wide = signature_panel("strategy(", 1600.0);
    assert!(wide.width() > 540.0, "{}", wide.width());
    assert!(wide.height() < 40.0, "one line, got {}", wide.height());
    assert!(wide.right() <= 1600.0 - 8.0 + 0.5, "inside the window");
}

#[test]
fn a_signature_wraps_only_when_the_window_is_too_narrow_and_stays_inside_it() {
    let narrow = signature_panel("strategy(", 480.0);
    assert!(narrow.height() > 40.0, "wrapped, got {}", narrow.height());
    assert!(
        narrow.left() >= 7.5 && narrow.right() <= 480.0 - 7.5,
        "{narrow:?}"
    );
}

#[test]
fn the_signature_panel_hides_when_the_caret_is_scrolled_out_of_the_editor() {
    let mut source = "plot(close)\n".repeat(120);
    source.push_str("plot(close, ");
    let mut state = EditorState::new("s", &source, "");
    let ctx = egui::Context::default();
    let caret = source.chars().count();
    let mut edit = TextEditState::default();
    edit.cursor
        .set_char_range(Some(CCursorRange::one(CCursor::new(caret))));
    edit.store(&ctx, state.id);
    ctx.memory_mut(|memory| memory.request_focus(state.id));
    let time = std::cell::Cell::new(0.0);
    let frame = |events: Vec<Event>, state: &mut EditorState| {
        time.set(time.get() + 0.1);
        let input = egui::RawInput {
            time: Some(time.get()),
            screen_rect: Some(Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                Vec2::new(900.0, 320.0),
            )),
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            show(ui, state);
        });
        output.textures_delta.clear();
    };
    for _ in 0..30 {
        frame(Vec::new(), &mut state);
    }
    let layer = egui::LayerId::new(egui::Order::Foreground, Id::new("script-signature"));
    let drawn = |ctx: &egui::Context| ctx.memory(|m| m.areas().visible_last_frame(&layer));
    assert!(drawn(&ctx), "the panel is up while the caret is in view");
    let panel = ctx
        .memory(|memory| memory.area_rect(Id::new("script-signature")))
        .expect("laid out");
    let visible = state.text_visible.expect("set by show");
    assert!(
        visible.contains_rect(panel),
        "the panel {panel:?} stays inside the text {visible:?}"
    );
    assert!(
        !state.caret_hidden,
        "the caret was scrolled into view: {:?} scroll {}",
        state.caret_rect, state.scroll_y
    );
    // The wheel scrolls the text but not the caret: it leaves the editor, and the panel with it.
    let wheel = Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: Vec2::new(0.0, 3000.0),
        phase: egui::TouchPhase::Move,
        modifiers: Modifiers::NONE,
    };
    frame(
        vec![egui::Event::PointerMoved(egui::pos2(300.0, 150.0))],
        &mut state,
    );
    for _ in 0..4 {
        frame(vec![wheel.clone()], &mut state);
    }
    assert!(state.caret_hidden, "the caret is below the editor now");
    assert!(!drawn(&ctx), "so the panel is not drawn over what is below");
}

#[test]
fn showing_a_line_flashes_all_of_it_and_a_jump_only_its_spot() {
    let mut state = EditorState::new("s", "a\nbb\nccc", "");
    state.show_line(2);
    let flash = state.flash.expect("flashing");
    assert!(flash.whole_line);
    assert_eq!(flash.index, 2, "the start of the second line");
    assert!(flash.strength() > 0.9, "strongest when it starts");
    assert!(Flash::at(0).strength() <= 1.0 && !Flash::at(0).whole_line);
}

// MARK: Changes against the saved text

fn run_frame(ctx: &egui::Context, state: &mut EditorState, time: f64, events: Vec<Event>) {
    let raw = egui::RawInput {
        time: Some(time),
        events,
        ..Default::default()
    };
    let mut output = ctx.run_ui(raw, |ui| {
        show(ui, state);
    });
    output.textures_delta.clear();
}

fn undo_key(redo: bool) -> Event {
    Event::Key {
        key: Key::Z,
        physical_key: Some(Key::Z),
        pressed: true,
        repeat: false,
        modifiers: Modifiers {
            shift: redo,
            ..Modifiers::COMMAND
        },
    }
}

#[test]
fn typing_marks_changes_and_save_clears_them() {
    let mut state = EditorState::new("s", "a\nb\n", "a\nb\n");
    assert!(state.changes().is_clean());
    state.source.push_str("c\nd\n");
    let changes = state.changes();
    assert_eq!(changes.hunks().len(), 1);
    assert_eq!(changes.hunks()[0].kind, ChangeKind::Added);
    assert_eq!((changes.lines_added(), changes.lines_removed()), (2, 0));
    assert!(
        Arc::ptr_eq(&changes, &state.changes()),
        "computed once per change"
    );

    // A draft write never moves the baseline.
    state.last_edit = Some(Instant::now() - DRAFT_DELAY * 2);
    assert!(state.take_draft_due(Instant::now()));
    assert_eq!(state.changes().lines_added(), 2);

    state.mark_saved();
    assert!(state.changes().is_clean());
    assert!(!state.is_dirty());
    state.source.push_str("e\n");
    assert_eq!(
        state.changes().lines_added(),
        1,
        "counts restart from the save"
    );
}

#[test]
fn a_save_that_did_not_happen_keeps_every_marker() {
    // `save_open` only calls `mark_saved` once the store accepted the text.
    let mut state = EditorState::new("s", "a\nX\nc\n", "a\nb\nc\n");
    let before = state.changes();
    assert_eq!(before.hunks().len(), 1);
    assert_eq!(state.changes().hunks(), before.hunks());
    assert_eq!(state.saved_source(), "a\nb\nc\n");
}

#[test]
fn markers_are_against_the_saved_text_even_for_invalid_pine() {
    // A reopened draft: `source` is the draft, the baseline is the saved script.
    let mut state = EditorState::new("s", "plot(close\nplot(open)\n", "plot(close)\nplot(open)\n");
    crate::pine_studio::compile_into(&mut state);
    assert_eq!(state.status, CompileStatus::Error);
    let changes = state.changes();
    assert_eq!(changes.hunks().len(), 1);
    assert_eq!(changes.hunks()[0].kind, ChangeKind::Modified);
    assert_eq!(changes.hunks()[0].new_lines, 0..1);
}

#[test]
fn an_external_replace_updates_the_baseline_only_when_saved() {
    let mut state = EditorState::new("s", "a\n", "a\n");
    state.replace_source("a\nb\n", false);
    assert_eq!(state.changes().lines_added(), 1);
    state.replace_source("a\nb\n", true);
    assert!(state.changes().is_clean());
}

#[test]
fn hunk_edits_reject_stale_hunks_without_touching_the_text() {
    let mut state = EditorState::new("s", "a\nX\nc\n", "a\nb\nc\n");
    let hunk = state.changes().hunks()[0].clone();
    state.source = "a\nXY\nc\n".into();
    let ctx = egui::Context::default();
    state.preview = Some(hunk.clone());
    assert!(!apply_hunk_edit(&ctx, &mut state, &hunk));
    assert_eq!(state.source, "a\nXY\nc\n");
    assert!(state.preview.is_none(), "the stale comparison closes");
}

#[test]
fn an_open_comparison_closes_when_its_change_is_edited_away() {
    let mut state = EditorState::new("s", "a\nX\nc\n", "a\nb\nc\n");
    state.preview = Some(state.changes().hunks()[0].clone());
    let ctx = egui::Context::default();
    run_frame(&ctx, &mut state, 0.0, vec![]);
    run_frame(&ctx, &mut state, 0.1, vec![]);
    assert!(state.preview.is_some(), "still the same change");
    state.source = "a\nb\nc\n".into();
    run_frame(&ctx, &mut state, 0.2, vec![]);
    assert!(state.preview.is_none());
}

#[test]
fn a_revert_is_one_undo_step_and_redo_replays_it() {
    let saved = "plot(close)\n";
    let typed = "fast = ta.ema(close, 9)\nslow = ta.ema(close, 21)\nfast2 = 1\nplot(close)\n";
    let mut state = EditorState::new("s", saved, saved);
    let ctx = egui::Context::default();
    ctx.memory_mut(|memory| memory.request_focus(state.id));
    run_frame(&ctx, &mut state, 0.0, vec![]);
    // Typed just now: egui has not yet closed that run of edits.
    state.source = typed.to_owned();
    run_frame(&ctx, &mut state, 0.1, vec![]);
    run_frame(&ctx, &mut state, 0.2, vec![]);
    let hunk = state.changes().hunks()[0].clone();
    assert_eq!(hunk.new_lines, 0..3);
    assert!(apply_hunk_edit(&ctx, &mut state, &hunk));
    assert_eq!(state.source, saved);
    assert!(state.changes().is_clean());
    run_frame(&ctx, &mut state, 0.3, vec![]);
    run_frame(&ctx, &mut state, 0.4, vec![]);
    assert_eq!(state.source, saved);

    run_frame(&ctx, &mut state, 0.5, vec![undo_key(false)]);
    assert_eq!(state.source, typed, "one undo brings every line back");
    assert_eq!(state.changes().lines_added(), 3, "and the gutter with it");
    run_frame(&ctx, &mut state, 0.6, vec![undo_key(true)]);
    assert_eq!(state.source, saved, "redo reverts again");
    assert!(state.changes().is_clean());
}

#[test]
fn an_applied_script_is_one_undo_step_and_redo_replays_it() {
    let before = "//@version=6\nplot(close)\n";
    let after = "//@version=6\nfast = ta.ema(close, 9)\nplot(fast)\nplot(close)\n";
    let mut state = EditorState::new("s", before, before);
    let ctx = egui::Context::default();
    ctx.memory_mut(|memory| memory.request_focus(state.id));
    run_frame(&ctx, &mut state, 0.0, vec![]);
    // Typed just now: egui has not yet closed that run of edits.
    state.source = format!("{before}// note\n");
    run_frame(&ctx, &mut state, 0.1, vec![]);
    let typed = state.source.clone();

    apply_script(&ctx, &mut state, after);
    assert_eq!(state.source, after);
    run_frame(&ctx, &mut state, 0.2, vec![]);
    run_frame(&ctx, &mut state, 0.3, vec![]);
    assert_eq!(state.source, after);

    run_frame(&ctx, &mut state, 0.4, vec![undo_key(false)]);
    assert_eq!(state.source, typed, "one undo brings back what was there");
    run_frame(&ctx, &mut state, 0.5, vec![undo_key(true)]);
    assert_eq!(state.source, after, "redo applies the script again");
}

#[test]
fn an_applied_script_puts_the_caret_on_the_first_changed_line() {
    let mut state = EditorState::new("s", "a = 1\nb = 2\n", "a = 1\nb = 2\n");
    let ctx = egui::Context::default();
    apply_script(&ctx, &mut state, "a = 1\nb = 3\nc = 4\n");
    let caret = TextEditState::load(&ctx, state.id)
        .and_then(|edit| edit.cursor.char_range())
        .map(|range| range.primary.index.0);
    assert_eq!(caret, Some("a = 1\nb = ".chars().count()));
    // Identical text applies cleanly too.
    apply_script(&ctx, &mut state, "a = 1\nb = 3\nc = 4\n");
    assert_eq!(state.source, "a = 1\nb = 3\nc = 4\n");
}

#[test]
fn restoring_deleted_lines_puts_the_caret_after_them() {
    let mut state = EditorState::new("s", "a = 1\nc = 3\n", "a = 1\nb = 2\nc = 3\n");
    let ctx = egui::Context::default();
    let hunk = state.changes().hunks()[0].clone();
    assert!(apply_hunk_edit(&ctx, &mut state, &hunk));
    assert_eq!(state.source, "a = 1\nb = 2\nc = 3\n");
    let caret = TextEditState::load(&ctx, state.id)
        .and_then(|edit| edit.cursor.char_range())
        .map(|range| range.primary.index.0);
    assert_eq!(caret, Some("a = 1\nb = 2\n".chars().count()));
}

#[test]
fn the_gutter_draws_every_kind_of_change() {
    let saved: String = (0..60).map(|i| format!("v{i} = {i}\n")).collect();
    let current = saved
        .replace("v5 = 5\n", "")
        .replace("v20 = 20\n", "v20 = 21\n")
        .replace("v40 = 40\n", "v40 = 40\nnew = 1\nnew2 = 2\n");
    {
        let mut state = EditorState::new("s", &current, &saved);
        let ctx = egui::Context::default();
        assert_eq!(state.changes().hunks().len(), 3);
        state.preview = Some(state.changes().hunks()[1].clone());
        for step in 0..4 {
            run_frame(&ctx, &mut state, f64::from(step) * 0.1, vec![]);
        }
        assert!(state.preview.is_some());
    }
}

// MARK: Profile column

const PROFILED: &str =
    "//@version=6\nindicator(\"T\")\nt = 0.0\nfor i = 0 to 9\n    t += i\nplot(t)\n";

fn profile_marks() -> std::sync::Arc<crate::pine_studio::profiler::marks::ProfileMarks> {
    let report = crate::pine::profile::support::report(
        "t = 0.0\nfor i = 0 to 9\n    t += i\nplot(t)\n",
        20,
        1_000,
    );
    std::sync::Arc::new(crate::pine_studio::profiler::marks::ProfileMarks::build(
        &report, PROFILED, 0,
    ))
}

fn move_to(ctx: &egui::Context, state: &mut EditorState, x: f32, y: f32, time: f64) {
    run_frame(
        ctx,
        state,
        time,
        vec![Event::PointerMoved(egui::pos2(x, y))],
    );
    run_frame(ctx, state, time + 0.01, vec![]);
}

#[test]
fn the_profile_column_takes_room_only_while_the_profiler_is_on() {
    let mut state = EditorState::new("s", PROFILED, PROFILED);
    let ctx = egui::Context::default();
    let text_left = |state: &mut EditorState, ctx: &egui::Context, time: f64| {
        let mut left = 0.0;
        let raw = egui::RawInput {
            time: Some(time),
            ..Default::default()
        };
        let mut output = ctx.run_ui(raw, |ui| {
            left = show(ui, state).text_area.expect("drawn").min.x;
        });
        output.textures_delta.clear();
        left
    };
    let plain = text_left(&mut state, &ctx, 0.0);
    state.set_profile_column(Some(ProfileColumn::Empty("Run the profiler.")));
    assert_eq!(text_left(&mut state, &ctx, 0.1), plain + PROFILE_WIDTH);
    state.set_profile_column(None);
    assert_eq!(
        text_left(&mut state, &ctx, 0.2),
        plain,
        "and back when it is off"
    );
}

#[test]
fn hovering_a_line_in_the_profile_column_washes_the_block_it_covers() {
    let mut state = EditorState::new("s", PROFILED, PROFILED);
    state.set_profile_column(Some(ProfileColumn::Marks(profile_marks())));
    let ctx = egui::Context::default();
    for step in 0..3 {
        run_frame(&ctx, &mut state, f64::from(step) * 0.1, vec![]);
    }
    assert_eq!(state.profile_hover, None);
    let mut found = None;
    let mut y = 4.0;
    while y < 140.0 && found.is_none() {
        move_to(&ctx, &mut state, 30.0, y, 1.0 + f64::from(y));
        found = state.profile_hover;
        y += 3.0;
    }
    assert!(found.is_some(), "the pointer found a measured line");
    // Leaving the column ends it.
    move_to(&ctx, &mut state, 400.0, 60.0, 500.0);
    assert_eq!(state.profile_hover, None);
}

#[test]
fn an_empty_column_says_why_and_hovers_nothing() {
    let mut state = EditorState::new("s", PROFILED, PROFILED);
    state.set_profile_column(Some(ProfileColumn::Empty(
        "This profile is of earlier text.",
    )));
    let ctx = egui::Context::default();
    for step in 0..3 {
        run_frame(&ctx, &mut state, f64::from(step) * 0.1, vec![]);
    }
    move_to(&ctx, &mut state, 30.0, 40.0, 5.0);
    assert_eq!(state.profile_hover, None);
}

use super::*;
use crate::script_manager::symbols;
use crate::ui::theme::{self, AppTheme, RestoreDefaultTheme};
use eframe::egui;

type Found = Vec<(String, SyntaxRole)>;

fn found_with(source: &str, table: Option<&SymbolTable>) -> Found {
    classify(source, table)
        .iter()
        .map(|s| (source[s.range.clone()].to_owned(), s.role))
        .collect()
}

/// Roles the way the editor assigns them, from the symbol table.
fn found(source: &str) -> Found {
    found_with(source, Some(&symbols::analyze(source)))
}

/// The spans the editor lays out.
fn spans(source: &str) -> Vec<Span> {
    classify(source, Some(&symbols::analyze(source)))
}

/// Roles with no symbol table, as a code snippet in a chat message gets them.
fn lexical(source: &str) -> Found {
    found_with(source, None)
}

fn has(found: &[(String, SyntaxRole)], text: &str, role: SyntaxRole) -> bool {
    found.iter().any(|(t, r)| t == text && *r == role)
}

fn roles_of_text(found: &[(String, SyntaxRole)], text: &str) -> Vec<SyntaxRole> {
    found
        .iter()
        .filter(|(t, _)| t == text)
        .map(|(_, role)| *role)
        .collect()
}

#[test]
fn compiler_offsets_skip_the_carriage_return_of_a_crlf() {
    // The compiler sees "a\nb\nc"; the editor holds "a\r\nb\r\nc".
    let crlf = "a\r\nb\r\nc";
    assert_eq!(compiled_utf16_to_char_index(crlf, 0), 0);
    assert_eq!(compiled_utf16_to_char_index(crlf, 2), 3);
    assert_eq!(compiled_utf16_to_char_index(crlf, 4), 6);
    // Without CRLF it is the plain conversion, astral characters counting two units.
    let plain = "a😀b\nc";
    for offset in 0..=6 {
        assert_eq!(
            compiled_utf16_to_char_index(plain, offset),
            utf16_to_char_index(plain, offset)
        );
    }
    // A lone CR was folded into a line feed by the compiler, one unit as it is here.
    assert_eq!(compiled_utf16_to_char_index("a\rb", 2), 2);
}

#[test]
fn a_diagnostic_in_a_crlf_script_is_found_where_it_is() {
    let source = "//@version=6\r\nindicator(\"T\")\r\n// a comment\r\n\r\n\r\nlog.hi(\"hiii\")\r\n";
    let program = crate::pine::compiler::compile(
        source,
        crate::pine::token::Limits::default(),
        &crate::pine::compiler::NoLibraries,
    );
    let diagnostic = program
        .diagnostics
        .iter()
        .find(|d| d.code == "PINE3051")
        .expect("log.hi is not a function");
    let index = compiled_utf16_to_char_index(source, diagnostic.range.start.offset);
    let rest: String = source.chars().skip(index).collect();
    assert!(rest.starts_with("log.hi"), "{rest:?}");
    // The plain conversion lands in the line above, which is what clicking the problem did.
    let wrong = utf16_to_char_index(source, diagnostic.range.start.offset);
    assert!(
        !source
            .chars()
            .skip(wrong)
            .collect::<String>()
            .starts_with("log.hi")
    );
}

#[test]
fn builtins_are_matched_as_whole_chains() {
    let source = "//@version=6\nindicator(\"x\", overlay=true)\nlen = 14\nx = ta.sma(close, len)\nplot(x, color=color.red)\nif barstate.isfirst\n    na\n";
    for f in [found(source), lexical(source)] {
        assert!(has(&f, "//@version=6", SyntaxRole::Directive), "{f:?}");
        assert!(has(&f, "indicator", SyntaxRole::BuiltinFunction));
        assert!(has(&f, "\"x\"", SyntaxRole::String));
        assert!(has(&f, "true", SyntaxRole::Literal));
        assert!(has(&f, "14", SyntaxRole::Number));
        assert!(has(&f, "ta", SyntaxRole::Namespace));
        assert!(has(&f, "sma", SyntaxRole::BuiltinFunction));
        assert!(has(&f, "close", SyntaxRole::BuiltinVariable));
        assert!(has(&f, "red", SyntaxRole::BuiltinConstant));
        assert!(has(&f, "if", SyntaxRole::ControlKeyword));
        assert!(has(&f, "isfirst", SyntaxRole::BuiltinVariable));
        assert!(has(&f, "na", SyntaxRole::Literal));
        // A named argument of a builtin is not the script's name.
        assert_eq!(roles_of_text(&f, "overlay"), [], "{f:?}");
    }
}

#[test]
fn the_script_s_own_names_have_their_own_roles() {
    let source = "length = input.int(14, \"Period\")\ncalculateAverage(src, period) =>\n    ta.sma(src, period)\naverage = calculateAverage(close, length)\nfor i = 0 to 3\n    average += i\n";
    let f = found(source);
    assert!(has(&f, "length", SyntaxRole::UserVariable), "{f:?}");
    assert!(has(&f, "average", SyntaxRole::UserVariable));
    assert_eq!(
        roles_of_text(&f, "calculateAverage"),
        [SyntaxRole::UserFunction, SyntaxRole::UserFunction],
        "definition and call"
    );
    assert_eq!(
        roles_of_text(&f, "src"),
        [SyntaxRole::Parameter, SyntaxRole::Parameter]
    );
    assert_eq!(
        roles_of_text(&f, "period"),
        [SyntaxRole::Parameter, SyntaxRole::Parameter]
    );
    assert!(has(&f, "i", SyntaxRole::UserVariable), "loop variable");
    // The builtin `close` and the namespace stay builtin.
    assert!(has(&f, "close", SyntaxRole::BuiltinVariable));
    assert!(has(&f, "ta", SyntaxRole::Namespace));
    assert!(has(&f, "sma", SyntaxRole::BuiltinFunction));
    assert!(has(&f, "to", SyntaxRole::ControlKeyword));
}

#[test]
fn a_named_argument_to_the_script_s_function_is_its_parameter() {
    let f = found("scale(value, factor) =>\n    value * factor\nx = scale(2, factor=3)\n");
    assert_eq!(
        roles_of_text(&f, "factor"),
        [
            SyntaxRole::Parameter,
            SyntaxRole::Parameter,
            SyntaxRole::Parameter
        ],
        "{f:?}"
    );
}

#[test]
fn without_a_table_declared_names_are_still_the_script_s() {
    let f = lexical("close = 1\nplot(close)\nmy(a) =>\n    a\nmy(2)\n");
    assert_eq!(
        roles_of_text(&f, "close"),
        [SyntaxRole::UserVariable, SyntaxRole::UserVariable]
    );
    assert_eq!(
        roles_of_text(&f, "my"),
        [SyntaxRole::UserFunction, SyntaxRole::UserFunction]
    );
    // The lexical pass cannot place a parameter.
    assert_eq!(roles_of_text(&f, "a"), []);
}

#[test]
fn user_declarations_shadow_builtins() {
    let f = found("close = 1\nplot(close)\nmy(a) =>\n    a\nmy(2)\n");
    assert_eq!(
        roles_of_text(&f, "close"),
        [SyntaxRole::UserVariable, SyntaxRole::UserVariable],
        "{f:?}"
    );
    assert!(!has(&f, "close", SyntaxRole::BuiltinVariable));
}

#[test]
fn a_local_shadow_does_not_hide_the_builtin_outside_its_scope() {
    let f = found("f(x) =>\n    hl2 = x\n    hl2\nplot(hl2)\n");
    assert_eq!(
        roles_of_text(&f, "hl2"),
        [
            SyntaxRole::UserVariable,
            SyntaxRole::UserVariable,
            SyntaxRole::BuiltinVariable
        ],
        "{f:?}"
    );
}

#[test]
fn a_builtin_read_before_it_is_shadowed_is_the_builtin() {
    let f = found("plot(close)\nclose = 1\nplot(close)\n");
    assert_eq!(
        roles_of_text(&f, "close"),
        [
            SyntaxRole::BuiltinVariable,
            SyntaxRole::UserVariable,
            SyntaxRole::UserVariable
        ],
        "{f:?}"
    );
}

#[test]
fn types_colour_literals_and_operators() {
    let f = found("var float x = na\ny = #FF8800 + 1\n");
    assert!(has(&f, "var", SyntaxRole::Keyword), "{f:?}");
    assert!(has(&f, "float", SyntaxRole::Type));
    assert!(has(&f, "#FF8800", SyntaxRole::ColorLiteral));
    assert!(has(&f, "+", SyntaxRole::Operator));
    assert!(has(&f, "=", SyntaxRole::Operator));
    assert!(has(&f, "na", SyntaxRole::Literal));
}

#[test]
fn declaration_keywords_and_control_flow_are_apart() {
    let f = found("var a = 1\nif a > 0\n    a := 2\nelse\n    a := 3\nwhile a < 9\n    a += 1\n");
    assert!(has(&f, "var", SyntaxRole::Keyword), "{f:?}");
    for word in ["if", "else", "while"] {
        assert!(has(&f, word, SyntaxRole::ControlKeyword), "{word}");
    }
    let f = found(
        "import user/lib/1 as helper\nmethod m(float x) =>\n    x\ntype Point\n    float x\n",
    );
    for word in ["import", "as", "method", "type"] {
        assert!(has(&f, word, SyntaxRole::Keyword), "{word}: {f:?}");
    }
}

#[test]
fn punctuation_has_a_role() {
    let f = found("x = array.from(1, 2)\ny = x.get(0)\nz = x[0]\n");
    assert!(has(&f, "(", SyntaxRole::Punctuation), "{f:?}");
    assert!(has(&f, ")", SyntaxRole::Punctuation));
    assert!(has(&f, ",", SyntaxRole::Punctuation));
    assert!(has(&f, "[", SyntaxRole::Punctuation));
    assert!(has(&f, "]", SyntaxRole::Punctuation));
    assert!(has(&f, ".", SyntaxRole::Punctuation), "the dot in a chain");
}

#[test]
fn escapes_are_cut_out_of_strings() {
    let source = "x = \"a\\nb \\\"q\\\" c\"\n";
    let f = found(source);
    assert!(has(&f, "\\n", SyntaxRole::Escape), "{f:?}");
    assert_eq!(
        f.iter().filter(|(_, r)| *r == SyntaxRole::Escape).count(),
        3,
        "{f:?}"
    );
    let string: String = f
        .iter()
        .filter(|(_, r)| matches!(r, SyntaxRole::String | SyntaxRole::Escape))
        .map(|(t, _)| t.as_str())
        .collect();
    assert_eq!(
        string, "\"a\\nb \\\"q\\\" c\"",
        "the pieces rebuild the literal"
    );
    // A trailing lone backslash and a multibyte character after one do not break the cut.
    let _ = found("x = \"a\\\"\n");
    let f = found("x = \"é\\é\"\n");
    assert!(has(&f, "\\é", SyntaxRole::Escape), "{f:?}");
}

#[test]
fn comments_directives_and_strings_do_not_mix() {
    let source = "a = 1 // trailing\nb = \"http://x\" // real\n//@version=6 // note\n";
    let f = found(source);
    let comments: Vec<&str> = f
        .iter()
        .filter(|(_, r)| *r == SyntaxRole::Comment)
        .map(|(t, _)| t.as_str())
        .collect();
    assert_eq!(comments, ["// trailing", "// real", " // note"]);
    assert!(has(&f, "\"http://x\"", SyntaxRole::String));
    assert!(has(&f, "//@version=6", SyntaxRole::Directive));
}

#[test]
fn multibyte_text_keeps_offsets_aligned() {
    let source = "// 日本語 ✓\nlabel = \"héllo\"\nlabel2 = 1\nplot(1)\n";
    let f = found(source);
    assert!(has(&f, "// 日本語 ✓", SyntaxRole::Comment), "{f:?}");
    assert!(has(&f, "\"héllo\"", SyntaxRole::String));
    assert!(has(&f, "label2", SyntaxRole::UserVariable));
    assert!(has(&f, "plot", SyntaxRole::BuiltinFunction));
    // Names after astral characters are still found at the right bytes.
    let f = found("a = \"😀😀\"\nbb = 2\nplot(bb)\n");
    assert_eq!(
        roles_of_text(&f, "bb"),
        [SyntaxRole::UserVariable, SyntaxRole::UserVariable],
        "{f:?}"
    );
}

#[test]
fn crlf_sources_classify_like_lf() {
    let lf = "x = 1\nplot(x)\n// c\n";
    let crlf = lf.replace('\n', "\r\n");
    // A line comment runs to the line feed, so it keeps the carriage return before it.
    let roles = |f: Found| {
        f.into_iter()
            .map(|(text, role)| (text.trim_end_matches('\r').to_owned(), role))
            .collect::<Found>()
    };
    assert_eq!(roles(found(lf)), roles(found(&crlf)));
}

#[test]
fn half_typed_code_still_classifies() {
    for source in [
        "x = ta.sma(close,",
        "f(a, b =>\n    a",
        "if\n",
        "x = \"unterminated\nplot(x)",
        "type\n",
    ] {
        let spans = classify(source, Some(&symbols::analyze(source)));
        let mut end = 0;
        for span in &spans {
            assert!(span.range.start >= end && span.range.end <= source.len());
            assert!(source.is_char_boundary(span.range.start));
            assert!(source.is_char_boundary(span.range.end));
            end = span.range.end;
        }
    }
}

#[test]
fn utf16_offsets_map_to_bytes_and_character_indexes() {
    let source = "a😀b";
    assert_eq!(utf16_to_byte_offsets(source), [0, 1, 1, 5, 6]);
    assert_eq!(utf16_to_char_index(source, 3), 2);
    assert_eq!(utf16_to_char_index(source, 99), 3);
}

#[test]
fn the_layout_covers_the_whole_source_in_order() {
    let source = "plot(1) // note\nlen = 14\n";
    let appearance = Appearance::of(crate::model::editor_theme::BuiltInTheme::Light);
    let job = layout_job(
        source,
        &spans(source),
        FontId::monospace(13.0),
        &appearance,
        appearance.role(SyntaxRole::Default),
    );
    assert_eq!(job.text, source);
    assert_eq!(
        layout_job(
            "",
            &[],
            FontId::monospace(13.0),
            &appearance,
            Color32::WHITE
        )
        .text,
        ""
    );
}

#[test]
fn runs_of_one_colour_are_one_section() {
    // Dark gives user names, parameters, punctuation and plain text one colour: the whole line
    // is a single section however many spans it has.
    let _restore = RestoreDefaultTheme;
    let ctx = egui::Context::default();
    theme::apply(&ctx, AppTheme::Dark);
    let appearance = Appearance::of(crate::model::editor_theme::BuiltInTheme::Dark);
    let source = "total = (alpha, beta)\n";
    let spans = spans(source);
    assert!(spans.len() > 3);
    let job = layout_job(
        source,
        &spans,
        FontId::monospace(13.0),
        &appearance,
        appearance.role(SyntaxRole::Default),
    );
    // `=` is an operator, so the line is: default, operator, default.
    assert_eq!(job.sections.len(), 3, "{:?}", job.sections);
    let covered: usize = job
        .sections
        .iter()
        .map(|s| s.byte_range.end.0 - s.byte_range.start.0)
        .sum();
    assert_eq!(covered, source.len());
}

#[test]
fn a_colour_change_classifies_nothing_again() {
    // Spans carry roles, not colours: two themes lay out the same spans.
    let source = "x = ta.sma(close, 14) // note\n";
    let spans = spans(source);
    let font = FontId::monospace(13.0);
    let a = Appearance::of(crate::model::editor_theme::BuiltInTheme::Dark);
    let b = Appearance::of(crate::model::editor_theme::BuiltInTheme::Light);
    let one = layout_job(
        source,
        &spans,
        font.clone(),
        &a,
        a.role(SyntaxRole::Default),
    );
    let two = layout_job(source, &spans, font, &b, b.role(SyntaxRole::Default));
    assert_eq!(one.text, two.text);
    assert_ne!(one.sections[0].format.color, two.sections[0].format.color);
}

#[test]
fn highlighting_does_not_touch_the_source() {
    let source = String::from("x = 1 // a\r\ny = \"é\"\r\n");
    let before = source.clone();
    let appearance = Appearance::of(crate::model::editor_theme::BuiltInTheme::Light);
    let job = layout_job(
        &source,
        &spans(&source),
        FontId::monospace(13.0),
        &appearance,
        Color32::BLACK,
    );
    assert_eq!(source, before);
    assert_eq!(job.text, before);
}

/// A realistic script, repeated: strings, calls, user functions, comments and control flow.
fn large_script(blocks: usize) -> String {
    let mut source = String::from("//@version=6\nindicator(\"Perf\", overlay=true)\n");
    for block in 0..blocks {
        source.push_str(&format!(
            "// block {block}\nlength{block} = input.int(14, \"Period {block}\")\n\
             average{block}(src, period) =>\n    ta.sma(src, period) + math.max(src, 0)\n\
             value{block} = average{block}(close, length{block})\n\
             if value{block} > close and barstate.isconfirmed\n    label.new(bar_index, high, \"x\\n\")\n\
             plot(value{block}, color=color.new(color.blue, 20), linewidth=2)\n"
        ));
    }
    source
}

#[test]
#[ignore = "timings are machine-specific: cargo test --release --lib syntax::tests::timings -- --ignored --nocapture"]
fn timings() {
    use std::time::Instant;
    let font = FontId::monospace(13.0);
    let appearance = Appearance::of(crate::model::editor_theme::BuiltInTheme::Dark);
    for blocks in [10, 100, 600] {
        let source = large_script(blocks);
        let lines = source.lines().count();
        let time = |what: &str, run: &mut dyn FnMut()| {
            run();
            let started = Instant::now();
            let rounds = 20;
            for _ in 0..rounds {
                run();
            }
            let each = started.elapsed() / rounds;
            eprintln!("{lines:>6} lines  {what:<28} {each:?}");
        };
        time("lexical classify", &mut || {
            std::hint::black_box(classify(&source, None));
        });
        time("analyze", &mut || {
            std::hint::black_box(symbols::analyze(&source));
        });
        time("analyze + classify (a keystroke)", &mut || {
            std::hint::black_box(spans(&source));
        });
        let table = symbols::analyze(&source);
        time("classify with a cached table", &mut || {
            std::hint::black_box(classify(&source, Some(&table)));
        });
        let spans = spans(&source);
        time("layout_job (a frame, cached spans)", &mut || {
            std::hint::black_box(layout_job(
                &source,
                &spans,
                font.clone(),
                &appearance,
                appearance.role(SyntaxRole::Default),
            ));
        });
    }
}

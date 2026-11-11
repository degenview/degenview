use super::*;

fn chars(text: &str) -> Vec<char> {
    text.chars().collect()
}

/// The source text of every site.
fn found(source: &str) -> Vec<String> {
    let all = chars(source);
    scan(source)
        .iter()
        .map(|site| all[site.range.clone()].iter().collect())
        .collect()
}

fn only(source: &str) -> ColorSite {
    let mut sites = scan(source);
    assert_eq!(sites.len(), 1, "{source:?} gave {sites:?}");
    sites.remove(0)
}

/// Rewrites the only site in `source` and returns the whole new document.
fn edited(source: &str, picked: Picked) -> String {
    let site = only(source);
    let all = chars(source);
    let text = rewrite(&site, &all[site.range.clone()], picked);
    let mut out: String = all[..site.range.start].iter().collect();
    out.push_str(&text);
    out.extend(&all[site.range.end..]);
    out
}

fn rgb(rgb: [u8; 3]) -> Picked {
    Picked {
        rgb: Some(rgb),
        transparency: None,
    }
}

fn transparency(percent: u8) -> Picked {
    Picked {
        rgb: None,
        transparency: Some(percent),
    }
}

#[test]
fn every_named_colour_is_a_site_with_its_value() {
    for name in builtins::COLOR_NAMES {
        let site = only(&format!("c = {name}"));
        assert_eq!(site.rgba, builtins::color(name).unwrap(), "{name}");
        assert_eq!(site.range, 4..4 + name.len());
        assert_eq!(site.shape, Shape::Named);
    }
    assert_eq!(builtins::COLOR_NAMES.len(), 17);
}

#[test]
fn hex_literals_with_and_without_alpha() {
    let site = only("c = #8000C0");
    assert_eq!(site.rgba, 0x8000_C0FF);
    assert_eq!(site.range, 4..11);
    let site = only("c = #FF000080");
    assert_eq!(site.rgba, 0xFF00_0080);
    assert_eq!(site.range, 4..13);
    assert_eq!(only("c = #ff33cc").rgba, 0xFF33_CCFF);
}

#[test]
fn malformed_hex_is_not_a_site() {
    assert!(scan("c = #80").is_empty());
    assert!(scan("c = #8000C").is_empty());
    assert!(scan("c = #1234567").is_empty());
}

#[test]
fn rgb_constructors() {
    assert_eq!(only("c = color.rgb(255, 0, 0)").rgba, 0xFF00_00FF);
    assert_eq!(only("c = color.rgb(128, 0, 192)").rgba, 0x8000_C0FF);
    let site = only("c = color.rgb(255, 51, 204, 40)");
    assert_eq!(site.rgba, builtins::rgb(255.0, 51.0, 204.0, 40.0));
    assert_eq!(site.transparency_percent(), 40);
    assert_eq!(found("c = color.rgb(1,2,3)"), ["color.rgb(1,2,3)"]);
}

#[test]
fn rgb_arguments_are_folded_like_the_runtime() {
    // Channels truncate and clamp, as `builtins::rgb` does.
    assert_eq!(only("c = color.rgb(255 / 2, 300, -5)").rgba, 0x7FFF_00FF);
    assert_eq!(only("c = color.rgb((1 + 2) * 10, 0, 0)").rgba, 0x1E00_00FF);
}

#[test]
fn new_wraps_names_hex_and_rgb() {
    let site = only("c = color.new(color.red, 40)");
    assert_eq!(site.rgb(), split(0xF236_45FF));
    assert_eq!(site.transparency_percent(), 40);
    assert_eq!(
        only("c = color.new(#8000C0, 40)").rgba & 0xFFFF_FF00,
        0x8000_C000
    );
    let nested = only("c = color.new(color.rgb(128, 0, 192), 40)");
    assert_eq!(nested.rgba, builtins::with_transparency(0x8000_C0FF, 40.0));
    assert_eq!(nested.range, 4..41);
    assert_eq!(
        only("c = color.new(color.white, 75)").transparency_percent(),
        75
    );
}

#[test]
fn nested_expression_gets_one_swatch_not_three() {
    assert_eq!(
        found("c = color.new(color.rgb(1, 2, 3), 40)"),
        ["color.new(color.rgb(1, 2, 3), 40)"]
    );
}

#[test]
fn multiline_input_color_call() {
    let source = "ratioColor = input.color(\n    color.new(color.rgb(128, 0, 192), 40),\n    \"Ratio Color\"\n)\n";
    assert_eq!(found(source), ["color.new(color.rgb(128, 0, 192), 40)"]);
    let split_call = "c = color.new(\n    color.rgb(128,\n 0, 192),\n    40\n)";
    assert_eq!(found(split_call).len(), 1);
}

#[test]
fn input_color_is_not_a_colour() {
    assert!(scan("c = input.color(1)").is_empty());
    assert!(scan("color c = na").is_empty());
}

#[test]
fn whitespace_variations() {
    assert_eq!(found("c = color . red").len(), 1);
    assert_eq!(
        found("c = color.new( color.red ,40 )"),
        ["color.new( color.red ,40 )"]
    );
    assert_eq!(found("c = color.rgb( 1 , 2 , 3 )").len(), 1);
}

#[test]
fn comments_and_strings_are_ignored() {
    assert!(scan("// color.red and #FF0000\n").is_empty());
    assert!(scan("x = \"color.red #FF0000\"").is_empty());
    assert_eq!(found("c = color.red // color.blue"), ["color.red"]);
}

#[test]
fn incomplete_expressions_are_not_sites_and_never_panic() {
    for source in [
        "c = color.rgb(255, 0,",
        "c = color.rgb(255, 0, 0",
        "c = color.rgb(255, 0)",
        "c = color.rgb(1, 2, 3, 4, 5)",
        "c = color.rgb(",
        "c = color.",
        "c = color",
        "c = color.nonsense",
        "c = color.rgb(1, 2, 3,)",
        "c = color.rgb(1, 2, 1/0)",
        "(((((((((((((((((((((((((((((((((((((((((",
    ] {
        assert!(scan(source).is_empty(), "{source:?}");
    }
    // An unfinished `color.new` is not a site, but the constant inside it still is.
    for source in [
        "c = color.new(color.red",
        "c = color.new(color.red,",
        "c = color.new(color.red, )",
    ] {
        assert_eq!(found(source), ["color.red"], "{source:?}");
    }
    let deep = format!(
        "c = color.rgb({}1{}, 0, 0)",
        "(".repeat(5000),
        ")".repeat(5000)
    );
    assert!(scan(&deep).is_empty());
}

#[test]
fn dynamic_expressions_are_not_one_fixed_colour() {
    // The two constants may be previewed; the ternary is not a site.
    assert_eq!(
        found("c = close > open ? color.green : color.red"),
        ["color.green", "color.red"]
    );
    assert!(scan("c = color.rgb(r, g, b)").is_empty());
    assert!(scan("c = color.rgb(255, g, 0)").is_empty());
    assert!(scan("c = color.new(myColor, 40)").is_empty());
    // An unknown transparency shows the base alone, never an invented alpha.
    assert_eq!(found("c = color.new(color.red, t)"), ["color.red"]);
    assert_eq!(found("c = color.new(#8000C0, close)"), ["#8000C0"]);
}

#[test]
fn several_on_one_line_and_in_one_call() {
    assert_eq!(
        found("plot(close, color = color.red, linewidth = 1, bgcolor = #00FF00)"),
        ["color.red", "#00FF00"]
    );
    assert_eq!(
        found("f(color.red, color.new(color.blue, 20), color.rgb(1, 2, 3))"),
        [
            "color.red",
            "color.new(color.blue, 20)",
            "color.rgb(1, 2, 3)"
        ]
    );
}

#[test]
fn ranges_count_characters_not_bytes_or_utf16() {
    // Two astral characters (4 bytes, 2 UTF-16 units each) before the colour.
    let source = "// 😀😀\nc = color.red";
    let site = only(source);
    assert_eq!(site.range, 10..19);
    assert_eq!(
        chars(source)[site.range.clone()].iter().collect::<String>(),
        "color.red"
    );
}

#[test]
fn crlf_documents_keep_their_ranges() {
    let source = "a = 1\r\nc = color.red\r\nd = #FF0000\r\n";
    let all = chars(source);
    let texts: Vec<String> = scan(source)
        .iter()
        .map(|site| all[site.range.clone()].iter().collect())
        .collect();
    assert_eq!(texts, ["color.red", "#FF0000"]);
}

#[test]
fn transparency_conversions_round_trip() {
    for percent in 0..=100u8 {
        let alpha = transparency_to_alpha(percent);
        assert_eq!(
            alpha_to_transparency(alpha).round() as u8,
            percent,
            "{percent}"
        );
    }
    assert_eq!(transparency_to_alpha(0), 255);
    assert_eq!(transparency_to_alpha(100), 0);
    assert_eq!(transparency_to_alpha(40), 153);
    // Same rounding as `color.new`.
    assert_eq!(
        u32::from(transparency_to_alpha(40)),
        builtins::with_transparency(0, 40.0) & 0xff
    );
}

// --- rewriting ---

#[test]
fn nothing_picked_changes_nothing() {
    for source in [
        "c = color.red",
        "c = #8000c0",
        "c = color.rgb(255 / 2, 0, 0)",
        "c = color.new(color.rgb(1, 2, 3), 40.5)",
    ] {
        assert_eq!(edited(source, Picked::default()), source);
    }
}

#[test]
fn named_to_rgb_or_back_to_a_name() {
    assert_eq!(
        edited("c = color.red", rgb([120, 40, 220])),
        "c = color.rgb(120, 40, 220)"
    );
    // The exact colour of another constant is written as that constant.
    assert_eq!(edited("c = color.red", rgb([0, 0, 0])), "c = color.black");
    assert_eq!(
        edited("c = color.red", rgb([0xF2, 0x36, 0x45])),
        "c = color.red"
    );
}

#[test]
fn named_with_transparency_only_keeps_the_name() {
    assert_eq!(
        edited("c = color.red", transparency(40)),
        "c = color.new(color.red, 40)"
    );
    assert_eq!(edited("c = color.red", transparency(0)), "c = color.red");
    assert_eq!(
        edited(
            "c = color.red",
            Picked {
                rgb: Some([1, 2, 3]),
                transparency: Some(30)
            }
        ),
        "c = color.new(color.rgb(1, 2, 3), 30)"
    );
}

#[test]
fn hex_stays_hex() {
    assert_eq!(edited("c = #8000C0", rgb([255, 80, 120])), "c = #FF5078");
    assert_eq!(edited("c = #8000c0", rgb([255, 80, 120])), "c = #ff5078");
    assert_eq!(
        edited("c = #8000C0", rgb([0xAB, 0xCD, 0xEF])),
        "c = #ABCDEF"
    );
}

#[test]
fn hex_alpha_appears_only_when_needed() {
    assert_eq!(edited("c = #8000C0", transparency(40)), "c = #8000C099");
    assert_eq!(edited("c = #8000C0", transparency(0)), "c = #8000C0");
    // An eight-digit literal stays eight digits and keeps its alpha when only the colour moves.
    assert_eq!(edited("c = #FF000080", rgb([1, 2, 3])), "c = #01020380");
    assert_eq!(edited("c = #FF000080", transparency(0)), "c = #FF0000FF");
}

#[test]
fn rgb_rewrites_only_the_changed_arguments() {
    assert_eq!(
        edited("c = color.rgb(128, 0, 192)", rgb([128, 10, 192])),
        "c = color.rgb(128, 10, 192)"
    );
    assert_eq!(
        edited("c = color.rgb(255 / 2, 0, 192)", rgb([127, 5, 192])),
        "c = color.rgb(255 / 2, 5, 192)"
    );
    assert_eq!(
        edited("c = color.rgb(1,2,3)", rgb([9, 8, 7])),
        "c = color.rgb(9,8,7)"
    );
}

#[test]
fn rgb_transparency_is_added_replaced_or_left() {
    assert_eq!(
        edited("c = color.rgb(1, 2, 3)", transparency(40)),
        "c = color.rgb(1, 2, 3, 40)"
    );
    assert_eq!(
        edited("c = color.rgb(1, 2, 3)", transparency(0)),
        "c = color.rgb(1, 2, 3)"
    );
    assert_eq!(
        edited("c = color.rgb(1, 2, 3, 40)", transparency(65)),
        "c = color.rgb(1, 2, 3, 65)"
    );
}

#[test]
fn new_changes_only_the_transparency_when_only_that_moves() {
    assert_eq!(
        edited("c = color.new(#8000C0, 40)", transparency(65)),
        "c = color.new(#8000C0, 65)"
    );
    assert_eq!(
        edited("c = color.new(color.rgb(128, 0, 192), 40)", transparency(0)),
        "c = color.new(color.rgb(128, 0, 192), 0)"
    );
}

#[test]
fn new_changes_only_the_base_when_only_the_colour_moves() {
    assert_eq!(
        edited(
            "c = color.new(color.rgb(128, 0, 192), 40)",
            rgb([255, 80, 120])
        ),
        "c = color.new(color.rgb(255, 80, 120), 40)"
    );
    assert_eq!(
        edited("c = color.new(#8000C0, 40)", rgb([255, 80, 120])),
        "c = color.new(#FF5078, 40)"
    );
    // A hex base keeps its own alpha: `color.new` overrides it anyway.
    assert_eq!(
        edited("c = color.new(#8000C0FF, 40)", rgb([1, 2, 3])),
        "c = color.new(#010203FF, 40)"
    );
    assert_eq!(
        edited("c = color.new(color.white, 75)", rgb([1, 2, 3])),
        "c = color.new(color.rgb(1, 2, 3), 75)"
    );
}

#[test]
fn new_with_both_changes() {
    assert_eq!(
        edited(
            "c = color.new(color.rgb(1, 2, 3), 40)",
            Picked {
                rgb: Some([4, 2, 3]),
                transparency: Some(41)
            }
        ),
        "c = color.new(color.rgb(4, 2, 3), 41)"
    );
}

#[test]
fn surroundings_survive_multiline_comments_and_unicode() {
    let source = "// ünïcode 😀\nratio = input.color(\n    color.new(color.rgb(128, 0, 192), 40), // keep me\n    \"Ratio Color\"\n)\nplot(close)\n";
    let out = edited(source, rgb([255, 80, 120]));
    assert_eq!(out, source.replace("rgb(128, 0, 192)", "rgb(255, 80, 120)"));
}

#[test]
fn identical_colours_are_edited_one_at_a_time() {
    let source = "a = color.red\nb = color.red\nc = color.red\n";
    let all = chars(source);
    let sites = scan(source);
    assert_eq!(sites.len(), 3);
    let middle = &sites[1];
    let text = rewrite(middle, &all[middle.range.clone()], rgb([1, 2, 3]));
    let mut out: String = all[..middle.range.start].iter().collect();
    out.push_str(&text);
    out.extend(&all[middle.range.end..]);
    assert_eq!(
        out,
        "a = color.red\nb = color.rgb(1, 2, 3)\nc = color.red\n"
    );
}

#[test]
fn edits_are_valid_pine_that_still_scan() {
    for (source, picked) in [
        ("c = color.red", rgb([9, 9, 9])),
        ("c = #123456", transparency(10)),
        ("c = color.rgb(1, 2, 3)", transparency(10)),
        ("c = color.new(color.red, 40)", transparency(10)),
    ] {
        let out = edited(source, picked);
        assert_eq!(scan(&out).len(), 1, "{out}");
    }
}

#[test]
fn repeated_open_and_close_does_not_drift() {
    // Reading a site and writing it back untouched is the identity, however often it happens.
    let mut source = "c = color.new(color.rgb(128, 0, 192), 40.5)".to_string();
    for _ in 0..100 {
        let site = only(&source);
        let all = chars(&source);
        let text = rewrite(&site, &all[site.range.clone()], Picked::default());
        let mut out: String = all[..site.range.start].iter().collect();
        out.push_str(&text);
        out.extend(&all[site.range.end..]);
        source = out;
    }
    assert_eq!(source, "c = color.new(color.rgb(128, 0, 192), 40.5)");
    // And picking the transparency the slider already shows (40.5 reads as 41) is a real change.
    assert_eq!(
        edited(&source, transparency(41)),
        "c = color.new(color.rgb(128, 0, 192), 41)"
    );
}

#[test]
fn the_map_places_every_site_in_bytes_lines_and_columns() {
    let source = "a = 1\n// é😀\nplot(close, color = color.red, bgcolor = #00FF00)\n";
    let map = map(source);
    assert_eq!(map.sites.len(), 2);
    assert_eq!(map.lines, [2, 2]);
    let line = "plot(close, color = color.red, bgcolor = #00FF00)";
    assert_eq!(
        map.columns,
        [line.find("color.red").unwrap(), line.find('#').unwrap()]
    );
    for (site, byte) in map.sites.iter().zip(&map.bytes) {
        let start: usize = source
            .chars()
            .take(site.range.start)
            .map(char::len_utf8)
            .sum();
        assert_eq!(*byte, start);
    }
    assert_eq!(map.at(map.sites[1].range.start), Some(1));
    assert_eq!(map.at(1), None);
    assert_eq!(map.on_lines(2, 3), 0..2);
    assert_eq!(map.on_lines(0, 2), 0..0);
}

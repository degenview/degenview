//! The built-in fonts: that the file is the one the licence notes describe, that the licence texts and
//! READMEs that the SIL OFL requires to travel with it are there and say the right things, that they are
//! shipped in every kind of download, and that the font draws as the editor needs.

use crate::model::editor_theme::{BUNDLED_FONT, SYSTEM_FONT};
use crate::ui::editor_appearance::{EDITOR_FAMILY, EditorFont};
use crate::ui::theme;
use eframe::egui;
use sha2::{Digest, Sha256};

const FONT: &[u8] = include_bytes!("../../assets/fonts/JetBrainsMonoNL-Regular.ttf");
const NOTO: &[u8] = include_bytes!("../../assets/fonts/NotoSansSymbols2-subset.ttf");
const JETBRAINS_LICENCE: &str = include_str!("../../assets/fonts/OFL-JetBrainsMono.txt");
const NOTO_LICENCE: &str = include_str!("../../assets/fonts/OFL-NotoSansSymbols2.txt");
const FONTS_README: &str = include_str!("../../assets/fonts/README.md");
const ROOT_README: &str = include_str!("../../README.md");
const BUNDLE_SCRIPT: &str = include_str!("../../tools/bundle.sh");
const FLATPAK: &str = include_str!("../../packaging/flatpak/es.nico.degenview.yml");
const RELEASE: &str = include_str!("../../.github/workflows/release.yml");

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[test]
fn the_font_is_the_file_the_readme_describes() {
    let line = FONTS_README
        .lines()
        .find(|line| line.starts_with("sha256 "))
        .expect("the README states the file's hash");
    assert!(line.contains("JetBrainsMonoNL-Regular.ttf"), "{line}");
    assert!(
        line.contains(&sha256(FONT)),
        "the committed font is not the one the README hashes: {} vs {line}",
        sha256(FONT)
    );
    assert!(FONTS_README.contains("208,576 bytes"));
    assert_eq!(FONT.len(), 208_576);
    // A TrueType font, and not one with a layout (GSUB) table: no ligatures to switch.
    assert_eq!(&FONT[..4], &[0, 1, 0, 0]);
    assert!(
        !FONT.windows(4).any(|tag| tag == b"GSUB"),
        "this is the NL build: it has no substitution table"
    );
}

#[test]
fn both_licence_texts_are_the_ofl_with_the_right_copyright() {
    for (text, holder) in [
        (
            JETBRAINS_LICENCE,
            "Copyright 2020 The JetBrains Mono Project Authors",
        ),
        (NOTO_LICENCE, "Copyright 2022 The Noto Project Authors"),
    ] {
        let first = text.lines().next().unwrap();
        assert!(first.starts_with(holder), "{first}");
        assert!(
            !first.contains("Reserved Font Name"),
            "the READMEs say there is none: {first}"
        );
        assert!(text.contains("SIL Open Font License, Version 1.1"));
        assert!(text.contains("SIL OPEN FONT LICENSE Version 1.1 - 26 February 2007"));
        assert!(text.contains("PERMISSION & CONDITIONS"));
        assert!(text.contains("DISCLAIMER"));
    }
    assert!(JETBRAINS_LICENCE.contains("https://github.com/JetBrains/JetBrainsMono"));
}

#[test]
fn the_readmes_name_both_fonts_their_licence_and_their_files() {
    for readme in [FONTS_README, ROOT_README] {
        for needed in [
            "JetBrains Mono",
            "Noto Sans Symbols 2",
            "SIL Open Font License",
            "OFL-JetBrainsMono.txt",
            "OFL-NotoSansSymbols2.txt",
            "The JetBrains Mono Project Authors",
            "The Noto Project Authors",
        ] {
            assert!(readme.contains(needed), "a README does not say {needed:?}");
        }
    }
    for needed in [
        "2.304",
        "NL",
        "ligatures",
        "unmodified",
        "Reserved Font Name",
    ] {
        assert!(FONTS_README.contains(needed), "fonts README: {needed:?}");
    }
    // And the licence stays GPL for the program itself.
    assert!(ROOT_README.contains("GPL-3.0-only"));
}

#[test]
fn every_file_the_readmes_point_at_exists() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for file in [
        "assets/fonts/JetBrainsMonoNL-Regular.ttf",
        "assets/fonts/NotoSansSymbols2-subset.ttf",
        "assets/fonts/OFL-JetBrainsMono.txt",
        "assets/fonts/OFL-NotoSansSymbols2.txt",
        "assets/fonts/README.md",
    ] {
        assert!(root.join(file).is_file(), "{file}");
    }
    assert!(
        !root.join("assets/fonts/OFL.txt").exists(),
        "renamed, not duplicated"
    );
    assert!(NOTO.len() > 50_000);
}

#[test]
fn the_licence_texts_ship_with_every_kind_of_download() {
    for (what, text) in [
        ("the macOS bundle script", BUNDLE_SCRIPT),
        ("the Flatpak manifest", FLATPAK),
        ("the release workflow", RELEASE),
    ] {
        assert!(
            text.contains("OFL-JetBrainsMono.txt"),
            "{what}: JetBrains Mono's licence"
        );
        assert!(
            text.contains("OFL-NotoSansSymbols2.txt"),
            "{what}: Noto's licence"
        );
    }
    assert!(BUNDLE_SCRIPT.contains("Contents/Resources/Licenses"));
    assert!(FLATPAK.contains("/app/share/licenses/es.nico.degenview"));
    // Linux and Windows archives both.
    assert_eq!(RELEASE.matches("OFL-JetBrainsMono.txt").count(), 2);
}

/// A context with the app's fonts, and one frame run so that they exist.
fn context() -> egui::Context {
    let ctx = egui::Context::default();
    theme::install_offscreen(&ctx);
    ctx.run_ui(egui::RawInput::default(), |_| {})
        .textures_delta
        .clear();
    ctx
}

#[test]
fn the_default_editor_font_is_the_bundled_one_in_a_fresh_context() {
    let ctx = context();
    let font = EditorFont::default().code(&ctx);
    assert_eq!(
        font.family,
        egui::FontFamily::Name(EDITOR_FAMILY.into()),
        "the editor draws in its own family, which holds JetBrains Mono"
    );
    assert_eq!(BUNDLED_FONT, "JetBrains Mono");
    assert_ne!(SYSTEM_FONT, BUNDLED_FONT);
}

#[test]
fn the_bundled_font_is_monospaced_and_draws_no_ligatures() {
    let ctx = context();
    let size = 20.0;
    let font = EditorFont::default().at(&ctx, size);
    ctx.run_ui(egui::RawInput::default(), |ui| {
        let advance = |c: char| ui.fonts_mut(|fonts| fonts.glyph_width(&font, c));
        let widths: Vec<f32> = ['i', 'W', '0', ' ', 'm', '.', '=', '>']
            .map(advance)
            .to_vec();
        for width in &widths {
            assert!(
                (width - widths[0]).abs() < 0.01,
                "every character has one width: {widths:?}"
            );
        }
        // JetBrains Mono is 600 units per 1000: 12 points at 20.
        assert!((widths[0] - 12.0).abs() < 0.2, "{widths:?}");
        // "=>" is two characters wide: nothing substituted a ligature glyph for them.
        let job = egui::text::LayoutJob::simple_singleline(
            "=> == != <= >= ->".to_owned(),
            font.clone(),
            egui::Color32::WHITE,
        );
        let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
        let count = "=> == != <= >= ->".chars().count() as f32;
        assert!(
            (galley.size().x - count * widths[0]).abs() < 0.5,
            "{} vs {}",
            galley.size().x,
            count * widths[0]
        );
    })
    .textures_delta
    .clear();
}

#[test]
fn what_the_bundled_font_lacks_falls_back_to_the_rest_of_the_chain() {
    let ctx = context();
    let font = EditorFont::default().at(&ctx, 14.0);
    ctx.run_ui(egui::RawInput::default(), |ui| {
        // These draw (from this font or from one later in the chain), never as tofu squares.
        let tofu = ui.fonts_mut(|fonts| fonts.glyph_width(&font, '\u{10FFFF}'));
        for symbol in ['▲', '★', '✔', '⚠'] {
            let width = ui.fonts_mut(|fonts| fonts.glyph_width(&font, symbol));
            assert!(width > 0.0 && (width - tofu).abs() > 0.01, "{symbol} draws");
        }
    })
    .textures_delta
    .clear();
}

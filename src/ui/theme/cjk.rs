//! The fonts that draw Han characters.
//!
//! egui draws from the fonts it is given and has no system fallback, and neither Inter nor egui's own
//! fonts have a Han character. Two things need one, whatever the other does:
//!
//! * A Chinese interface. Its font is a file of the system, never bundled (a font with the
//!   characters a catalog needs is megabytes), and is read only when the language in use needs it:
//!   the files are 20 MB and more.
//! * The language picker, which lists every language by its own name in every language. A bundled
//!   subset holding just the Han characters of those names serves it ([`language_names_font`]).
//!
//! `zh` is Simplified Chinese (CLDR's default script for the language; Traditional is `zh-Hant`).
//!
//! One Han character is drawn differently in each region that writes it, and a font collection holds
//! one face per region, so a language names the face it wants, not only the file.

use crate::i18n::Locale;
use eframe::egui;
use std::path::PathBuf;

/// A face of a font file: a font collection (`.ttc`) holds several, one per regional form.
struct Face {
    path: PathBuf,
    index: u32,
}

/// Simplified Chinese fonts macOS ships: Hiragino Sans GB, then STHeiti.
const SIMPLIFIED_MACOS: &[(&str, u32)] = &[
    ("/System/Library/Fonts/Hiragino Sans GB.ttc", 0),
    ("/System/Library/Fonts/STHeiti Light.ttc", 0),
];

/// Microsoft YaHei, Windows's Simplified Chinese UI font, and the older ones, in the system's font
/// folder.
const SIMPLIFIED_WINDOWS: &[(&str, u32)] = &[("msyh.ttc", 0), ("simsun.ttc", 0), ("simhei.ttf", 0)];

/// The faces `locale` can be drawn with, best first. Empty for a language Inter covers.
fn faces(locale: Locale) -> Vec<Face> {
    match locale.tag() {
        "zh" => {
            let windows = std::env::var_os("WINDIR")
                .map(|root| PathBuf::from(root).join("Fonts"))
                .into_iter()
                .flat_map(|folder| {
                    SIMPLIFIED_WINDOWS.iter().map(move |(name, index)| Face {
                        path: folder.join(name),
                        index: *index,
                    })
                });
            let macos = SIMPLIFIED_MACOS.iter().map(|(path, index)| Face {
                path: PathBuf::from(path),
                index: *index,
            });
            fontconfig(&["zh-cn", "zh-sg", "cmn", "zh-tw", "zh-hk"])
                .into_iter()
                .chain(macos)
                .chain(windows)
                .collect()
        }
        _ => Vec::new(),
    }
}

/// The font the system's fontconfig ranks first among those that cover `languages` (`fc-match -s`),
/// on the platforms that have it. Linux distributions each put Noto Sans CJK, WenQuanYi or Droid in
/// another directory and file name (and the Flatpak sandbox shows the host's under
/// `/run/host/fonts`); fontconfig is what knows which is installed, so no path is guessed.
///
/// The first match for a language is not always a font with its characters: with Droid Sans
/// Fallback alone, `:lang=zh-cn` ranks DejaVu Sans first, because Droid declares `zh-tw` and not
/// `zh-cn`. So the ranked list is read and the first font that declares the first language of
/// `languages` is taken; if none does, the first that declares any of the others.
fn fontconfig(languages: &[&str]) -> Option<Face> {
    if !cfg!(all(unix, not(target_os = "macos"))) {
        return None;
    }
    let output = std::process::Command::new("fc-match")
        .args([
            "-s",
            "--format",
            "%{file}\t%{index}\t%{lang}\n",
            &format!(":lang={}", languages.first()?),
        ])
        .output()
        .ok()?;
    parse_fontconfig_ranking(&String::from_utf8(output.stdout).ok()?, languages)
}

/// The first font of fontconfig's ranking, lines of `file`, `index` and `lang` (the languages it
/// declares, joined by `|`), that declares `languages[0]`, else the first that declares any of the
/// rest. The index of a variable font's named instance carries the instance in its high 16 bits
/// and the face in the low ones.
fn parse_fontconfig_ranking(output: &str, languages: &[&str]) -> Option<Face> {
    let fonts: Vec<(&str, &str, Vec<&str>)> = output
        .lines()
        .filter_map(|line| {
            let mut fields = line.split('\t');
            Some((
                fields.next()?,
                fields.next()?,
                fields.next()?.split('|').collect(),
            ))
        })
        .collect();
    let declares = |wanted: &[&str]| {
        fonts
            .iter()
            .find(|(_, _, langs)| wanted.iter().any(|lang| langs.contains(lang)))
    };
    let (path, index, _) =
        declares(languages.get(..1)?).or_else(|| declares(languages.get(1..)?))?;
    Some(Face {
        path: PathBuf::from(path),
        index: index.trim().parse::<u32>().ok()? & 0xFFFF,
    })
}

/// The first of `locale`'s fonts the machine has, `None` when the language needs none or no file
/// is there (its Han characters then show as the missing-glyph box). `reference` is the font the
/// interface's text is set in (Inter): the Han font is shifted to share its baseline.
pub(super) fn system_font(locale: Locale, reference: &[u8]) -> Option<egui::FontData> {
    faces(locale).into_iter().find_map(|face| {
        let bytes = std::fs::read(&face.path).ok()?;
        let offset = baseline_offset(&bytes, face.index, reference)?;
        let mut data = shifted(egui::FontData::from_owned(bytes), offset);
        data.index = face.index;
        Some(data)
    })
}

/// Noto Sans SC cut down to the Han characters of the languages' own names (`assets/fonts`, SIL OFL
/// 1.1; how to regenerate it is in `assets/fonts/README.md`). It follows the system's font in the
/// chain, so a Chinese interface draws all its text in one face.
const LANGUAGE_NAMES: &[u8] = include_bytes!("../../../assets/fonts/NotoSansSC-language-names.otf");

/// The bundled font for the languages' own names, on the baseline of `reference` like
/// [`system_font`]'s.
pub(super) fn language_names_font(reference: &[u8]) -> egui::FontData {
    let offset = baseline_offset(LANGUAGE_NAMES, 0, reference).unwrap_or_default();
    shifted(egui::FontData::from_static(LANGUAGE_NAMES), offset)
}

fn shifted(data: egui::FontData, y_offset_factor: f32) -> egui::FontData {
    data.tweak(egui::FontTweak {
        y_offset_factor,
        ..Default::default()
    })
}

/// How far, in em, a glyph of the font at `index` of `bytes` must move down to sit on the baseline
/// of `reference`'s glyphs when the two share a line.
///
/// egui centres each font's own line box (ascent, descent and line gap) in the line and sets its
/// glyphs from the top of that box, so two fonts agree on the baseline only when
/// `ascent - height / 2` is the same. A Chinese font's line gap is large (Hiragino Sans GB: half an
/// em), which puts its characters about 0.23 em above Inter's.
fn baseline_offset(bytes: &[u8], index: u32, reference: &[u8]) -> Option<f32> {
    Some(centred_baseline(reference, 0)? - centred_baseline(bytes, index)?)
}

/// `ascent - line height / 2` in em, from the vertical metrics egui reads: the typographic ones when
/// the font asks for them (`USE_TYPO_METRICS`), the `hhea` ones otherwise.
fn centred_baseline(bytes: &[u8], index: u32) -> Option<f32> {
    let face = sfnt_offset(bytes, index)?;
    let table = |tag: &[u8; 4]| table_offset(bytes, face, tag);
    let units_per_em = f32::from(be_u16(bytes, table(b"head")? + 18)?);
    let hhea = table(b"hhea")?;
    let (mut ascent, mut descent, mut gap) = (
        be_i16(bytes, hhea + 4)?,
        be_i16(bytes, hhea + 6)?,
        be_i16(bytes, hhea + 8)?,
    );
    if let Some(os2) = table(b"OS/2") {
        const USE_TYPO_METRICS: u16 = 1 << 7;
        if be_u16(bytes, os2 + 62)? & USE_TYPO_METRICS != 0 {
            (ascent, descent, gap) = (
                be_i16(bytes, os2 + 68)?,
                be_i16(bytes, os2 + 70)?,
                be_i16(bytes, os2 + 72)?,
            );
        }
    }
    let (ascent, descent, gap) = (
        f32::from(ascent) / units_per_em,
        f32::from(descent) / units_per_em,
        f32::from(gap) / units_per_em,
    );
    Some(ascent - (ascent - descent + gap) / 2.0)
}

/// Where face `index` starts: 0 in a single font file, the collection's table of offsets in a `.ttc`.
fn sfnt_offset(bytes: &[u8], index: u32) -> Option<usize> {
    if bytes.get(..4)? == b"ttcf" {
        be_u32(bytes, 12 + 4 * index as usize).map(|at| at as usize)
    } else {
        (index == 0).then_some(0)
    }
}

fn table_offset(bytes: &[u8], face: usize, tag: &[u8; 4]) -> Option<usize> {
    let count = usize::from(be_u16(bytes, face + 4)?);
    (0..count).find_map(|n| {
        let record = face + 12 + 16 * n;
        (bytes.get(record..record + 4)? == tag)
            .then(|| be_u32(bytes, record + 8).map(|at| at as usize))?
    })
}

fn be_u16(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn be_i16(bytes: &[u8], at: usize) -> Option<i16> {
    be_u16(bytes, at).map(|value| value as i16)
}

fn be_u32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    const INTER: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");
    const JETBRAINS: &[u8] = include_bytes!("../../../assets/fonts/JetBrainsMonoNL-Regular.ttf");

    #[test]
    fn only_a_language_with_han_characters_asks_for_a_font() {
        assert!(faces(Locale::ENGLISH).is_empty());
        assert!(faces(Locale::PSEUDO).is_empty());
        assert!(system_font(Locale::ENGLISH, INTER).is_none());
    }

    #[test]
    fn simplified_chinese_tries_every_platform_and_each_path_is_absolute() {
        let tried = faces(Locale::from_tag("zh").expect("Chinese is offered"));
        assert!(tried.iter().all(|face| face.path.is_absolute()));
        assert!(
            tried
                .iter()
                .any(|face| face.path.ends_with("Hiragino Sans GB.ttc")),
            "macOS has a font to try"
        );
    }

    const HAN: &[&str] = &["zh-cn", "zh-sg", "cmn", "zh-tw", "zh-hk"];

    #[test]
    fn fontconfig_names_the_file_and_the_face_of_the_first_font_with_the_language() {
        let ranking = "/fonts/DejaVuSans.ttf\t0\ten|es|fr\n\
                       /fonts/google-noto-sans-cjk-vf-fonts/NotoSansCJK-VF.ttc\t262146\tja|zh-cn|zh-tw\n\
                       /fonts/Other.ttf\t0\tzh-cn\n";
        let face = parse_fontconfig_ranking(ranking, HAN).expect("a font with zh-cn");
        assert_eq!(
            face.path,
            PathBuf::from("/fonts/google-noto-sans-cjk-vf-fonts/NotoSansCJK-VF.ttc")
        );
        // A named instance of a variable font: the face is in the low 16 bits.
        assert_eq!(face.index, 2);
    }

    #[test]
    fn a_font_that_declares_another_chinese_language_serves_when_none_declares_the_first() {
        // Droid Sans Fallback declares `zh-tw` but not `zh-cn`, and ranks below DejaVu Sans.
        let ranking = "/fonts/DejaVuSans.ttf\t0\ten|es\n\
                       /fonts/DroidSansFallbackFull.ttf\t0\tja|zh-tw|cmn\n";
        let face = parse_fontconfig_ranking(ranking, HAN).expect("Droid covers Chinese");
        assert_eq!(face.path, PathBuf::from("/fonts/DroidSansFallbackFull.ttf"));
        assert_eq!(face.index, 0);
    }

    #[test]
    fn a_ranking_with_no_chinese_font_gives_none() {
        assert!(parse_fontconfig_ranking("/fonts/DejaVuSans.ttf\t0\ten|es\n", HAN).is_none());
        assert!(parse_fontconfig_ranking("", HAN).is_none());
        assert!(parse_fontconfig_ranking("/fonts/x.ttf\tnot a number\tzh-cn\n", HAN).is_none());
        assert!(parse_fontconfig_ranking("/fonts/x.ttf\t0\n", HAN).is_none());
    }

    /// The numbers are the fonts' own `hhea`/`OS/2` values (Inter 1984, -494 over 2048;
    /// JetBrains Mono 1020, -300, gap 0 over 1000).
    #[test]
    fn the_centred_baseline_is_read_from_the_font_tables() {
        let inter = centred_baseline(INTER, 0).expect("Inter has its tables");
        assert!((inter - 0.363_77).abs() < 1e-4, "{inter}");
        let mono = centred_baseline(JETBRAINS, 0).expect("JetBrains Mono has its tables");
        assert!((mono - 0.36).abs() < 1e-4, "{mono}");
        assert_eq!(baseline_offset(INTER, 0, INTER), Some(0.0));
        let shift = baseline_offset(JETBRAINS, 0, INTER).expect("both parse");
        assert!((shift - 0.003_77).abs() < 1e-4, "{shift}");
    }

    #[test]
    fn the_language_names_font_has_the_tables_its_baseline_is_read_from() {
        assert!(baseline_offset(LANGUAGE_NAMES, 0, INTER).is_some());
    }

    #[test]
    fn a_face_that_is_not_there_has_no_baseline() {
        assert_eq!(centred_baseline(INTER, 1), None);
        assert_eq!(centred_baseline(b"not a font", 0), None);
        assert_eq!(centred_baseline(&[], 0), None);
    }

    /// The Hiragino collection on a Mac: its half-em line gap puts the characters 0.23 em too high
    /// until they are moved.
    #[cfg(target_os = "macos")]
    #[test]
    fn hiragino_sits_lower_than_its_own_metrics_place_it() {
        let Ok(bytes) = std::fs::read("/System/Library/Fonts/Hiragino Sans GB.ttc") else {
            return;
        };
        let shift = baseline_offset(&bytes, 0, INTER).expect("the collection has face 0");
        assert!((0.2..0.27).contains(&shift), "{shift}");
    }
}

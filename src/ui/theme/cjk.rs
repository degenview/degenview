//! The Han font a Chinese interface is drawn with.
//!
//! egui draws from the fonts it is given and has no system fallback, and neither Inter nor egui's own
//! fonts have a Han character. A Chinese font is a file of the system, never bundled (a font with the
//! characters a catalog needs is megabytes), and is read only when the language in use needs it: the
//! files are 20 MB and more.
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
            fontconfig("zh-cn")
                .into_iter()
                .chain(macos)
                .chain(windows)
                .collect()
        }
        _ => Vec::new(),
    }
}

/// The font the system's fontconfig picks for `lang` (`fc-match`), on the platforms that have it.
/// Linux distributions each put Noto Sans CJK, WenQuanYi or Droid in another directory and file
/// name (and the Flatpak sandbox shows the host's under `/run/host/fonts`); fontconfig is what
/// knows which is installed, so no path is guessed.
fn fontconfig(lang: &str) -> Option<Face> {
    if !cfg!(all(unix, not(target_os = "macos"))) {
        return None;
    }
    let output = std::process::Command::new("fc-match")
        .args(["--format", "%{file}\n%{index}", &format!(":lang={lang}")])
        .output()
        .ok()?;
    parse_fontconfig_match(&String::from_utf8(output.stdout).ok()?)
}

/// `file` and `index` as fontconfig prints them. For a variable font's named instance the index
/// carries the instance in its high 16 bits, and the face in the low ones.
fn parse_fontconfig_match(output: &str) -> Option<Face> {
    let (path, index) = output.trim().rsplit_once('\n')?;
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
        let mut data = egui::FontData::from_owned(bytes).tweak(egui::FontTweak {
            y_offset_factor: offset,
            ..Default::default()
        });
        data.index = face.index;
        Some(data)
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

    #[test]
    fn fontconfig_names_the_file_and_the_face() {
        let face = parse_fontconfig_match(
            "/usr/share/fonts/google-noto-sans-cjk-vf-fonts/NotoSansCJK-VF.ttc\n2\n",
        )
        .expect("a file and an index");
        assert_eq!(
            face.path,
            PathBuf::from("/usr/share/fonts/google-noto-sans-cjk-vf-fonts/NotoSansCJK-VF.ttc")
        );
        assert_eq!(face.index, 2);
        // A named instance of a variable font: instance 1, face 2.
        let instance = parse_fontconfig_match("/fonts/Noto.ttc\n65538").expect("parses");
        assert_eq!(instance.index, 2);
        assert!(parse_fontconfig_match("").is_none());
        assert!(parse_fontconfig_match("/fonts/Noto.ttc").is_none());
        assert!(parse_fontconfig_match("/fonts/Noto.ttc\nnot a number").is_none());
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

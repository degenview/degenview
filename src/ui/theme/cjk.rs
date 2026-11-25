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

/// Where Simplified Chinese fonts are on macOS, Windows and Linux, in the order tried. Linux
/// distributions put Noto Sans CJK in different directories (the Flatpak sandbox shows the host's
/// under `/run/host/fonts`), and its collection holds Japanese, Korean, Simplified, Traditional and
/// Hong Kong faces, in that order.
const SIMPLIFIED: &[(&str, u32)] = &[
    ("/System/Library/Fonts/Hiragino Sans GB.ttc", 0),
    ("/System/Library/Fonts/STHeiti Light.ttc", 0),
    ("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc", 2),
    ("/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc", 2),
    (
        "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
        2,
    ),
    (
        "/usr/share/fonts/google-noto-sans-cjk-fonts/NotoSansCJK-Regular.ttc",
        2,
    ),
    (
        "/usr/share/fonts/opentype/noto/NotoSansCJKsc-Regular.otf",
        0,
    ),
    ("/usr/share/fonts/truetype/wqy/wqy-microhei.ttc", 0),
    (
        "/usr/share/fonts/wenquanyi/wqy-microhei/wqy-microhei.ttc",
        0,
    ),
    ("/run/host/fonts/opentype/noto/NotoSansCJK-Regular.ttc", 2),
    ("/run/host/fonts/noto-cjk/NotoSansCJK-Regular.ttc", 2),
    (
        "/run/host/fonts/google-noto-sans-cjk-fonts/NotoSansCJK-Regular.ttc",
        2,
    ),
];

/// Microsoft YaHei, Windows's Simplified Chinese UI font, and the older ones, in the system's
/// font folder.
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
            let others = SIMPLIFIED.iter().map(|(path, index)| Face {
                path: PathBuf::from(path),
                index: *index,
            });
            others.chain(windows).collect()
        }
        _ => Vec::new(),
    }
}

/// The first of `locale`'s fonts the machine has, `None` when the language needs none or no file
/// is there (its Han characters then show as the missing-glyph box).
pub(super) fn system_font(locale: Locale) -> Option<egui::FontData> {
    faces(locale).into_iter().find_map(|face| {
        let bytes = std::fs::read(&face.path).ok()?;
        let mut data = egui::FontData::from_owned(bytes);
        data.index = face.index;
        Some(data)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_language_with_han_characters_asks_for_a_font() {
        assert!(faces(Locale::ENGLISH).is_empty());
        assert!(faces(Locale::PSEUDO).is_empty());
        assert!(system_font(Locale::ENGLISH).is_none());
    }

    #[test]
    fn simplified_chinese_tries_every_platform_and_each_path_is_absolute() {
        let tried = faces(Locale::from_tag("zh").expect("Chinese is offered"));
        assert!(tried.len() >= SIMPLIFIED.len());
        assert!(tried.iter().all(|face| face.path.is_absolute()));
        assert!(
            tried
                .iter()
                .any(|face| face.path.ends_with("Hiragino Sans GB.ttc")),
            "macOS has a font to try"
        );
    }
}

//! Which characters the installed fonts can draw, for tests that guard against empty boxes.

use eframe::egui;

/// Whether some font of `family` has every character of `text`, so none of it draws as a box.
///
/// Asked of the fonts' own character maps. epaint's `has_glyph` cannot answer this: it also says
/// "no" for everything in the face that supplies its replacement box, which without a system
/// monospace font (as on Linux) is the first monospace font, Latin letters included.
pub(crate) fn covers(ctx: &egui::Context, family: &egui::FontFamily, text: &str) -> bool {
    ctx.fonts_mut(|fonts| {
        let mut font = fonts.fonts.font(family);
        let characters = font.characters();
        text.chars().all(|c| characters.contains_key(&c))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// egui's own fonts alone, with nothing from the system: the chain a Linux build starts from.
    #[test]
    fn the_first_monospace_font_s_letters_are_covered_and_chinese_is_not() {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(Default::default(), |_| {});
        output.textures_delta.clear();
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            assert!(covers(&ctx, &family, "English"), "{family:?}");
            assert!(!covers(&ctx, &family, "简体中文"), "{family:?}");
        }
    }
}

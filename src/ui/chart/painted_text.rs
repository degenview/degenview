//! What a frame of egui wrote, for the tests that draw a screen in the pseudo-locale.

use eframe::egui::{FullOutput, Shape};

/// Every piece of text the frame painted.
pub(super) fn of(output: &FullOutput) -> Vec<String> {
    fn walk(shape: &Shape, out: &mut Vec<String>) {
        match shape {
            Shape::Text(text) => out.push(text.galley.text().to_owned()),
            Shape::Vec(shapes) => shapes.iter().for_each(|shape| walk(shape, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for clipped in &output.shapes {
        walk(&clipped.shape, &mut out);
    }
    out
}

/// The painted words that never went through `tr!`: text with a run of three letters that the
/// pseudo-locale did not wrap in brackets. Prices, symbols and provider data show up here too, so
/// this is read by a person, not asserted.
pub(super) fn unbracketed(texts: &[String]) -> Vec<&str> {
    texts
        .iter()
        .map(String::as_str)
        .filter(|text| !text.starts_with('['))
        .filter(|text| {
            let mut run = 0;
            text.chars().any(|ch| {
                run = if ch.is_alphabetic() { run + 1 } else { 0 };
                run >= 3
            })
        })
        .collect()
}

/// Panics unless the frame painted `message` as the pseudo-locale writes it.
pub(super) fn assert_painted(texts: &[String], message: &'static str) {
    let wanted = crate::i18n::with_locale(crate::i18n::Locale::Pseudo, || {
        crate::i18n::translate("", message)
    });
    assert!(
        texts.iter().any(|text| text.contains(wanted)),
        "{wanted:?} was not painted: {texts:#?}"
    );
}

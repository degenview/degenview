//! What a screen painted in the pseudo-locale (`en-XA`), for the tests that check a screen was put
//! through `tr!`: every translated message is wrapped in brackets and accented, so a text with
//! words and no bracket is a literal that was never converted.

use eframe::egui::{self, Shape};

/// Every text `output` paints.
pub(crate) fn painted(output: &egui::FullOutput) -> Vec<String> {
    fn walk(shape: &Shape, into: &mut Vec<String>) {
        match shape {
            Shape::Text(text) => into.push(text.galley.text().to_owned()),
            Shape::Vec(shapes) => shapes.iter().for_each(|shape| walk(shape, into)),
            _ => {}
        }
    }
    let mut texts = Vec::new();
    for clipped in &output.shapes {
        walk(&clipped.shape, &mut texts);
    }
    texts
}

/// The painted texts that have a word in them and no bracket, except those that contain one of
/// `data` (a ticker, a name, a currency: text that is data, not interface).
pub(crate) fn untranslated(output: &egui::FullOutput, data: &[&str]) -> Vec<String> {
    let has_word = |text: &str| {
        text.chars()
            .collect::<Vec<_>>()
            .windows(3)
            .any(|run| run.iter().all(|ch| ch.is_ascii_alphabetic()))
    };
    painted(output)
        .into_iter()
        .filter(|text| has_word(text) && !text.contains('[') && !text.contains(']'))
        .filter(|text| !data.iter().any(|known| text.contains(known)))
        .collect()
}

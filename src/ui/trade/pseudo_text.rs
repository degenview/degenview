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

/// Every text `output` paints, with the box it covers.
pub(crate) fn painted_boxes(output: &egui::FullOutput) -> Vec<(String, egui::Rect)> {
    fn walk(shape: &Shape, into: &mut Vec<(String, egui::Rect)>) {
        match shape {
            // An empty text has no box.
            Shape::Text(text) if !text.galley.text().is_empty() => {
                into.push((text.galley.text().to_owned(), text.visual_bounding_rect()))
            }
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

/// The pairs of painted texts that cover each other: a control laid over the words beside it, which
/// is what a text longer than the English one it replaced does to a width tuned for English.
pub(crate) fn overlapping(output: &egui::FullOutput) -> Vec<(String, String)> {
    let boxes = painted_boxes(output);
    let mut pairs = Vec::new();
    for (index, (first, a)) in boxes.iter().enumerate() {
        for (second, b) in &boxes[index + 1..] {
            // A point of give: glyph boxes of lines that touch are not an overlap.
            if a.shrink(1.0).intersects(b.shrink(1.0)) {
                pairs.push((first.clone(), second.clone()));
            }
        }
    }
    pairs
}

/// The painted texts that have a word in them and no bracket, except those that contain one of
/// `data` (a ticker, a name, a currency: text that is data, not interface). A month in a date is
/// not a word: `i18n::datetime` writes dates in the pseudo-locale as English, on purpose.
pub(crate) fn untranslated(output: &egui::FullOutput, data: &[&str]) -> Vec<String> {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let has_word = |text: &str| {
        text.split(|ch: char| !ch.is_ascii_alphabetic())
            .any(|word| word.len() >= 3 && !MONTHS.contains(&word))
    };
    painted(output)
        .into_iter()
        .filter(|text| has_word(text) && !text.contains('[') && !text.contains(']'))
        .filter(|text| !data.iter().any(|known| text.contains(known)))
        .collect()
}

//! What a test can read off a frame: the words it drew, for the tests that draw a screen in the
//! pseudo-locale and check that every one of them went through `tr!`.

use eframe::egui::{FullOutput, epaint::Shape};

/// Every piece of text painted in `output`, in paint order.
pub(crate) fn texts(output: &FullOutput) -> Vec<String> {
    fn collect(shape: &Shape, found: &mut Vec<String>) {
        match shape {
            Shape::Text(text) => found.push(text.galley.job.text.clone()),
            Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect(shape, found)),
            _ => {}
        }
    }
    let mut found = Vec::new();
    for clipped in &output.shapes {
        collect(&clipped.shape, &mut found);
    }
    found
}

/// The words of `texts` that read as English interface text but carry no pseudo-locale brackets:
/// a literal nobody put through `tr!`. Text with no lowercase run of three letters (a glyph, a
/// number, a ticker) and the `known` data (a symbol, a layout name) are not counted.
pub(crate) fn untranslated<'a>(texts: &'a [String], known: &[&str]) -> Vec<&'a str> {
    texts
        .iter()
        .map(String::as_str)
        .filter(|text| !text.trim().is_empty())
        .filter(|text| !text.contains('[') && !text.contains(']'))
        .filter(|text| !known.iter().any(|known| text.contains(known)))
        .filter(|text| {
            let mut run = 0;
            text.chars().any(|c| {
                run = if c.is_ascii_lowercase() { run + 1 } else { 0 };
                run >= 3
            })
        })
        .collect()
}

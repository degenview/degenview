//! The Image tool's data: which stored picture a drawing shows, and how.
//!
//! A drawing names its picture by the hash of the picture's bytes and never carries them (see
//! `store::asset_store`), so a layout, an undo step and a synced drawing record stay small. The
//! picture sits between two chart points like a rectangle, so it moves and scales with the chart
//! the way TradingView's image does.

use super::drawings::Anchor;
use super::link_card::LinkCard;
use serde::{Deserialize, Serialize};

/// The longest name an image may be given, in characters.
pub const MAX_NAME_CHARS: usize = 200;
/// The range of opacity, so a picture can never be made invisible and lost.
pub const OPACITY_RANGE: (f32, f32) = (0.05, 1.0);
/// The largest side a picture may declare, in pixels.
pub const MAX_SIDE: u32 = 4_096;

fn full() -> f32 {
    1.0
}

fn keeps_aspect() -> bool {
    true
}

fn is_full(value: &f32) -> bool {
    *value == 1.0
}

fn is_true(value: &bool) -> bool {
    *value
}

/// A picture on the chart, between two opposite corners.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ImageData {
    pub a: Anchor,
    pub b: Anchor,
    /// The hash of the picture's bytes, in lower-case hex: its name in the asset store.
    pub asset: String,
    /// What the picture is called: the file it came from, until the user renames it.
    pub name: String,
    /// The picture's own size in pixels, which fixes the shape it keeps when resized.
    pub pixels: [u32; 2],
    #[serde(default = "full", skip_serializing_if = "is_full")]
    pub opacity: f32,
    /// Resizing keeps the picture's shape.
    #[serde(default = "keeps_aspect", skip_serializing_if = "is_true")]
    pub keep_aspect: bool,
}

/// Whether `text` is the name of a stored picture: 64 lower-case hex digits.
pub fn is_asset_id(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

impl ImageData {
    pub fn new(a: Anchor, b: Anchor, content: &ImageContent) -> Self {
        Self {
            a,
            b,
            asset: content.asset.clone(),
            name: content.name.clone(),
            pixels: content.pixels,
            opacity: 1.0,
            keep_aspect: true,
        }
    }

    /// The picture's width over its height.
    pub fn aspect(&self) -> f32 {
        self.pixels[0].max(1) as f32 / self.pixels[1].max(1) as f32
    }

    /// The opacity held to its range.
    pub fn clamped_opacity(&self) -> f32 {
        if self.opacity.is_finite() {
            self.opacity.clamp(OPACITY_RANGE.0, OPACITY_RANGE.1)
        } else {
            1.0
        }
    }

    /// Whether this is an image a tool could have made.
    pub fn is_valid(&self) -> bool {
        is_asset_id(&self.asset)
            && self.name.chars().count() <= MAX_NAME_CHARS
            && self.pixels.iter().all(|side| (1..=MAX_SIDE).contains(side))
            && self.opacity.is_finite()
            && (OPACITY_RANGE.0..=OPACITY_RANGE.1).contains(&self.opacity)
            && self.a.is_finite()
            && self.b.is_finite()
    }
}

/// A picture ready to be placed: what a drawing needs besides where it goes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImageContent {
    pub asset: String,
    pub name: String,
    pub pixels: [u32; 2],
}

/// What is waiting to be placed by a click while a content tool is armed.
#[derive(Clone, Debug, PartialEq)]
pub enum Content {
    Image(ImageContent),
    /// A link to an X post, as a card.
    Link(LinkCard),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> ImageData {
        ImageData::new(
            Anchor {
                time_ms: 1,
                price: 2.0,
            },
            Anchor {
                time_ms: 5,
                price: 1.0,
            },
            &ImageContent {
                asset: "a".repeat(64),
                name: "chart.png".into(),
                pixels: [200, 100],
            },
        )
    }

    #[test]
    fn a_new_image_is_valid_opaque_and_keeps_its_shape() {
        let image = data();
        assert!(image.is_valid());
        assert_eq!(image.opacity, 1.0);
        assert!(image.keep_aspect);
        assert_eq!(image.aspect(), 2.0);
    }

    #[test]
    fn what_is_not_a_stored_picture_or_is_out_of_range_is_not_valid() {
        let mut bad = data();
        bad.asset = "A".repeat(64);
        assert!(!bad.is_valid(), "ids are lower case");
        let mut bad = data();
        bad.asset = "a".repeat(63);
        assert!(!bad.is_valid());
        let mut bad = data();
        bad.name = "x".repeat(MAX_NAME_CHARS + 1);
        assert!(!bad.is_valid());
        let mut bad = data();
        bad.pixels = [0, 10];
        assert!(!bad.is_valid());
        let mut bad = data();
        bad.pixels = [10, MAX_SIDE + 1];
        assert!(!bad.is_valid());
        for opacity in [0.0, 0.01, 1.5, f32::NAN] {
            let mut bad = data();
            bad.opacity = opacity;
            assert!(!bad.is_valid(), "{opacity}");
        }
        let mut bad = data();
        bad.a.price = f64::NAN;
        assert!(!bad.is_valid());
    }

    #[test]
    fn opacity_is_held_to_its_range_where_it_is_read() {
        let mut image = data();
        image.opacity = 0.0;
        assert_eq!(image.clamped_opacity(), OPACITY_RANGE.0);
        image.opacity = f32::NAN;
        assert_eq!(image.clamped_opacity(), 1.0);
    }

    #[test]
    fn defaults_are_not_stored_and_a_row_without_them_reads_back() {
        let image = data();
        let json = serde_json::to_value(&image).unwrap();
        assert!(json.get("opacity").is_none() && json.get("keep_aspect").is_none());
        assert_eq!(serde_json::from_value::<ImageData>(json).unwrap(), image);
        let mut custom = data();
        custom.opacity = 0.4;
        custom.keep_aspect = false;
        let json = serde_json::to_value(&custom).unwrap();
        assert_eq!(serde_json::from_value::<ImageData>(json).unwrap(), custom);
    }
}

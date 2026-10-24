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
/// The range a picture's size on screen may have, per side, in logical pixels.
pub const SIZE_RANGE: (f32, f32) = (8.0, 8_192.0);
/// The size a picture is first shown at is at most this wide and this tall, in logical pixels.
const FIRST_SIZE: [f32; 2] = [480.0, 360.0];
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

/// A picture on the chart: its top-left corner is at a chart point and it has a size of its own on
/// screen, so it moves with the chart but is never scaled by it (zooming or dragging an axis
/// leaves its dimensions alone).
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ImageData {
    pub at: Anchor,
    /// Its width and height on screen, in logical pixels.
    pub size: [f32; 2],
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
    pub fn new(at: Anchor, content: &ImageContent) -> Self {
        Self {
            at,
            size: content.size,
            asset: content.asset.clone(),
            name: content.name.clone(),
            pixels: content.pixels,
            opacity: 1.0,
            keep_aspect: true,
        }
    }

    /// Shows `content`'s picture in place of this one: the same corner, the same width, and (when
    /// the picture keeps its shape) the height its own shape gives. The name is the caller's.
    pub fn swap_picture(&mut self, content: &ImageContent) {
        self.asset = content.asset.clone();
        self.pixels = content.pixels;
        if self.keep_aspect {
            self.size[1] = (self.size[0] / self.aspect()).clamp(SIZE_RANGE.0, SIZE_RANGE.1);
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
            && self.at.is_finite()
            && self
                .size
                .iter()
                .all(|side| side.is_finite() && (SIZE_RANGE.0..=SIZE_RANGE.1).contains(side))
    }
}

/// A picture ready to be placed: what a drawing needs besides where it goes.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageContent {
    pub asset: String,
    pub name: String,
    pub pixels: [u32; 2],
    /// The size it is first shown at on screen: its own size, brought down to fit
    /// [`FIRST_SIZE`]. Where it is placed, the plot may bring it down further.
    pub size: [f32; 2],
}

impl ImageContent {
    pub fn new(asset: impl Into<String>, name: impl Into<String>, pixels: [u32; 2]) -> Self {
        let (width, height) = (pixels[0].max(1) as f32, pixels[1].max(1) as f32);
        let scale = (FIRST_SIZE[0] / width).min(FIRST_SIZE[1] / height).min(1.0);
        Self {
            asset: asset.into(),
            name: name.into(),
            pixels,
            size: [
                (width * scale).max(SIZE_RANGE.0),
                (height * scale).max(SIZE_RANGE.0),
            ],
        }
    }
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
            &ImageContent::new("a".repeat(64), "chart.png", [200, 100]),
        )
    }

    #[test]
    fn a_picture_first_shows_at_its_own_size_brought_down_to_fit() {
        assert_eq!(
            ImageContent::new("a".repeat(64), "x", [200, 100]).size,
            [200.0, 100.0]
        );
        // Too wide: scaled to the width, keeping its shape.
        assert_eq!(
            ImageContent::new("a".repeat(64), "x", [960, 480]).size,
            [480.0, 240.0]
        );
        // Too tall.
        assert_eq!(
            ImageContent::new("a".repeat(64), "x", [400, 720]).size,
            [200.0, 360.0]
        );
        // A sliver is held to the least a picture may be.
        let sliver = ImageContent::new("a".repeat(64), "x", [2_000, 1]).size;
        assert_eq!(sliver[1], SIZE_RANGE.0);
    }

    #[test]
    fn its_size_is_in_pixels_and_never_in_chart_units() {
        let image = data();
        let json = serde_json::to_value(&image).unwrap();
        assert_eq!(json["size"], serde_json::json!([200.0, 100.0]));
        assert!(
            json.get("b").is_none(),
            "there is no second chart point to scale it by"
        );
    }

    #[test]
    fn swapping_the_picture_keeps_the_corner_and_width_and_refits_the_height() {
        let mut image = data();
        let square = ImageContent::new("b".repeat(64), "other", [64, 64]);
        image.swap_picture(&square);
        assert_eq!(image.asset, square.asset);
        assert_eq!(image.pixels, [64, 64]);
        assert_eq!(image.size, [200.0, 200.0]);
        assert_eq!(image.at, data().at);
        // Told not to keep its shape, it keeps the size it was given.
        let mut loose = data();
        loose.keep_aspect = false;
        loose.swap_picture(&square);
        assert_eq!(loose.size, [200.0, 100.0]);
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
        bad.at.price = f64::NAN;
        assert!(!bad.is_valid());
        for size in [
            [0.0, 10.0],
            [10.0, SIZE_RANGE.1 + 1.0],
            [f32::NAN, 10.0],
            [10.0, f32::INFINITY],
        ] {
            let mut bad = data();
            bad.size = size;
            assert!(!bad.is_valid(), "{size:?}");
        }
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

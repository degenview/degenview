//! Turning a picture a user gave us (a file, a drop, the clipboard) into the bytes an Image
//! drawing keeps, or refusing it.
//!
//! Nothing the user gives is trusted. The size of the input is limited before anything reads it,
//! the format is checked by its signature, the dimensions are read from the header and checked
//! before a pixel is decoded (so a small file that claims to be enormous is refused instead of
//! being unpacked), and the picture is decoded with allocation limits on top. What is kept is not
//! the input but a fresh encoding of the pixels: no metadata, no trailing data, no odd chunks, and
//! small enough to travel in one synced record. The result is named by the hash of its bytes, so
//! the same picture is one asset however many drawings show it.

use image::{
    DynamicImage, ImageEncoder, ImageFormat, ImageReader, Limits, RgbaImage, imageops::FilterType,
};
use sha2::{Digest, Sha256};
use std::io::Cursor;

/// The most a picture may be as a file, in bytes: far above what a chart annotation needs.
pub const MAX_INPUT_BYTES: usize = 10 * 1024 * 1024;
/// The largest side, and the most pixels, an input may declare.
pub const MAX_INPUT_SIDE: u32 = 8_192;
pub const MAX_INPUT_PIXELS: u64 = 32 * 1024 * 1024;
/// The side a picture is scaled down to when it is larger (TradingView's own limit).
pub const MAX_SIDE: u32 = 2_000;
/// The most a kept picture may be, in bytes. Base64 in a synced record makes it a third larger,
/// which must still fit the 900 kB a record may carry.
pub const MAX_STORED_BYTES: usize = 600_000;
/// The smallest a picture is squeezed to before it is given up on.
const MIN_SIDE: u32 = 48;
/// What decoding may allocate, in bytes.
const MAX_DECODE_ALLOC: u64 = 512 * 1024 * 1024;

/// A picture ready to be kept.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Ingested {
    /// The hash of `bytes`, in lower-case hex: the picture's name everywhere.
    pub id: String,
    pub mime: &'static str,
    pub width: u32,
    pub height: u32,
    pub bytes: Vec<u8>,
}

/// Why a picture was refused, in words for the person who gave it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IngestError {
    Empty,
    /// More bytes than `MAX_INPUT_BYTES`.
    TooLarge,
    /// Not a PNG or a JPEG.
    Unsupported,
    /// Damaged, or not decodable.
    Corrupt,
    /// Declares more pixels than `MAX_INPUT_PIXELS`, or a side past `MAX_INPUT_SIDE`.
    TooManyPixels,
    /// Could not be made small enough to keep.
    CannotFit,
}

impl IngestError {
    pub const fn message(self) -> &'static str {
        match self {
            Self::Empty => "That file is empty.",
            Self::TooLarge => "That picture is over 10 MB. Choose a smaller one.",
            Self::Unsupported => "Only PNG and JPEG pictures can be added.",
            Self::Corrupt => "That picture could not be read.",
            Self::TooManyPixels => "That picture is too large (over 8192 pixels a side).",
            Self::CannotFit => "That picture could not be made small enough to keep.",
        }
    }
}

/// The hash a kept picture is named by.
pub fn hash(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The format a picture's first bytes say it is, if it is one this accepts.
pub fn sniff(bytes: &[u8]) -> Option<ImageFormat> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(ImageFormat::Png)
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(ImageFormat::Jpeg)
    } else {
        None
    }
}

fn limits() -> Limits {
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_INPUT_SIDE);
    limits.max_image_height = Some(MAX_INPUT_SIDE);
    limits.max_alloc = Some(MAX_DECODE_ALLOC);
    limits
}

fn reader(bytes: &[u8], format: ImageFormat) -> ImageReader<Cursor<&[u8]>> {
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(limits());
    reader
}

/// Checks `bytes` and decodes them, or says why not.
fn decode(bytes: &[u8]) -> Result<DynamicImage, IngestError> {
    if bytes.is_empty() {
        return Err(IngestError::Empty);
    }
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(IngestError::TooLarge);
    }
    let format = sniff(bytes).ok_or(IngestError::Unsupported)?;
    // The header alone says how big it claims to be: nothing is allocated for it yet.
    let (width, height) = reader(bytes, format).into_dimensions().map_err(refused)?;
    if width == 0 || height == 0 {
        return Err(IngestError::Corrupt);
    }
    if width > MAX_INPUT_SIDE
        || height > MAX_INPUT_SIDE
        || u64::from(width) * u64::from(height) > MAX_INPUT_PIXELS
    {
        return Err(IngestError::TooManyPixels);
    }
    reader(bytes, format).decode().map_err(refused)
}

/// Why a decoder gave up: a limit it was given (a side or an allocation too large) is a picture
/// too large; anything else is a picture that cannot be read.
fn refused(error: image::ImageError) -> IngestError {
    match error {
        image::ImageError::Limits(_) => IngestError::TooManyPixels,
        _ => IngestError::Corrupt,
    }
}

fn encode_png(image: &RgbaImage) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new_with_quality(
        &mut out,
        image::codecs::png::CompressionType::Best,
        image::codecs::png::FilterType::Adaptive,
    )
    .write_image(
        image.as_raw(),
        image.width(),
        image.height(),
        image::ExtendedColorType::Rgba8,
    )
    .ok()?;
    Some(out)
}

fn encode_jpeg(image: &DynamicImage, quality: u8) -> Option<Vec<u8>> {
    let rgb = image.to_rgb8();
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality)
        .write_image(
            rgb.as_raw(),
            rgb.width(),
            rgb.height(),
            image::ExtendedColorType::Rgb8,
        )
        .ok()?;
    Some(out)
}

/// The picture scaled so its longer side is `side`, or as it is when that is already so.
fn scaled(image: &DynamicImage, side: u32) -> DynamicImage {
    let longer = image.width().max(image.height());
    if longer <= side {
        return image.clone();
    }
    let factor = f64::from(side) / f64::from(longer);
    let width = ((f64::from(image.width()) * factor).round() as u32).max(1);
    let height = ((f64::from(image.height()) * factor).round() as u32).max(1);
    image.resize_exact(width, height, FilterType::Lanczos3)
}

/// Whether any pixel of the picture is not fully opaque.
fn has_transparency(image: &DynamicImage) -> bool {
    image.color().has_alpha() && image.to_rgba8().pixels().any(|pixel| pixel.0[3] < 255)
}

/// Checks a picture, scales it to at most [`MAX_SIDE`] and keeps its pixels as a PNG (when any are
/// transparent) or a JPEG, squeezed until it fits [`MAX_STORED_BYTES`].
pub fn ingest(input: &[u8]) -> Result<Ingested, IngestError> {
    let decoded = decode(input)?;
    let transparent = has_transparency(&decoded);
    let mut side = decoded.width().max(decoded.height()).min(MAX_SIDE);
    loop {
        let candidate = scaled(&decoded, side);
        let attempts: &[u8] = if transparent {
            &[0]
        } else {
            &[85, 75, 65, 55, 45]
        };
        for quality in attempts {
            let bytes = if transparent {
                encode_png(&candidate.to_rgba8())
            } else {
                encode_jpeg(&candidate, *quality)
            }
            .ok_or(IngestError::Corrupt)?;
            if bytes.len() <= MAX_STORED_BYTES {
                return Ok(Ingested {
                    id: hash(&bytes),
                    mime: if transparent {
                        "image/png"
                    } else {
                        "image/jpeg"
                    },
                    width: candidate.width(),
                    height: candidate.height(),
                    bytes,
                });
            }
        }
        // Still too big: a smaller picture, until there is nothing left worth keeping.
        let next = side * 4 / 5;
        if next < MIN_SIDE {
            return Err(IngestError::CannotFit);
        }
        side = next;
    }
}

/// Reads a picture file, refusing one over the byte limit before reading more than that of it.
pub fn read_file(path: &std::path::Path) -> Result<Vec<u8>, IngestError> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|_| IngestError::Corrupt)?;
    let mut bytes = Vec::new();
    file.take(MAX_INPUT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| IngestError::Corrupt)?;
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(IngestError::TooLarge);
    }
    Ok(bytes)
}

/// The pixels of a kept picture, for painting. `None` for bytes that are not one.
pub fn pixels(bytes: &[u8]) -> Option<eframe::egui::ColorImage> {
    let image = decode(bytes).ok()?.to_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    Some(eframe::egui::ColorImage::from_rgba_unmultiplied(
        size,
        image.as_raw(),
    ))
}

/// What a kept picture is, read from its bytes: its type and its size. `None` when the bytes are
/// not a PNG or a JPEG.
pub fn describe(bytes: &[u8]) -> Option<(&'static str, u32, u32)> {
    let format = sniff(bytes)?;
    let (width, height) = reader(bytes, format).into_dimensions().ok()?;
    let mime = match format {
        ImageFormat::Png => "image/png",
        _ => "image/jpeg",
    };
    Some((mime, width, height))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32, alpha: u8) -> Vec<u8> {
        let image = RgbaImage::from_fn(width, height, |x, y| {
            image::Rgba([
                (x % 256) as u8,
                (y % 256) as u8,
                ((x + y) % 256) as u8,
                alpha,
            ])
        });
        encode_png(&image).unwrap()
    }

    fn jpeg(width: u32, height: u32) -> Vec<u8> {
        let image = DynamicImage::ImageRgba8(RgbaImage::from_fn(width, height, |x, y| {
            image::Rgba([(x % 256) as u8, (y % 256) as u8, 90, 255])
        }));
        encode_jpeg(&image, 90).unwrap()
    }

    #[test]
    fn a_png_and_a_jpeg_are_kept_with_their_size_and_a_name_from_their_bytes() {
        let kept = ingest(&png(120, 80, 255)).unwrap();
        // Fully opaque: kept as a JPEG, which is far smaller for a photograph.
        assert_eq!((kept.width, kept.height), (120, 80));
        assert_eq!(kept.mime, "image/jpeg");
        assert_eq!(sniff(&kept.bytes), Some(ImageFormat::Jpeg));
        assert_eq!(kept.id, hash(&kept.bytes));
        assert_eq!(kept.id.len(), 64);
        let jpeg = ingest(&jpeg(64, 64)).unwrap();
        assert_eq!(jpeg.mime, "image/jpeg");
        assert_eq!(describe(&jpeg.bytes), Some(("image/jpeg", 64, 64)));
    }

    #[test]
    fn a_picture_with_transparency_stays_a_png_with_its_alpha() {
        let kept = ingest(&png(40, 30, 100)).unwrap();
        assert_eq!(kept.mime, "image/png");
        assert_eq!(sniff(&kept.bytes), Some(ImageFormat::Png));
        let pixels = pixels(&kept.bytes).unwrap();
        assert_eq!(pixels.size, [40, 30]);
        assert!(pixels.pixels.iter().all(|pixel| pixel.a() < 255));
        // A PNG with an alpha channel that is all opaque is not transparent.
        let opaque = ingest(&png(40, 30, 255)).unwrap();
        assert_eq!(opaque.mime, "image/jpeg");
    }

    #[test]
    fn the_same_picture_is_the_same_asset_and_a_different_one_is_not() {
        let a = ingest(&png(50, 50, 255)).unwrap();
        let b = ingest(&png(50, 50, 255)).unwrap();
        assert_eq!(a.id, b.id);
        assert_ne!(a.id, ingest(&png(51, 50, 255)).unwrap().id);
    }

    #[test]
    fn a_large_picture_is_scaled_to_the_side_limit_keeping_its_shape() {
        let kept = ingest(&jpeg(4_000, 1_000)).unwrap();
        assert_eq!((kept.width, kept.height), (MAX_SIDE, 500));
        let tall = ingest(&jpeg(600, 3_000)).unwrap();
        assert_eq!((tall.width, tall.height), (400, MAX_SIDE));
        assert!(kept.bytes.len() <= MAX_STORED_BYTES);
    }

    #[test]
    fn what_is_not_a_png_or_a_jpeg_or_is_damaged_is_refused() {
        assert_eq!(ingest(&[]), Err(IngestError::Empty));
        assert_eq!(ingest(b"GIF89a....."), Err(IngestError::Unsupported));
        assert_eq!(ingest(b"just some text"), Err(IngestError::Unsupported));
        assert_eq!(ingest(b"<svg></svg>"), Err(IngestError::Unsupported));
        let good = png(60, 60, 255);
        // A file cut short, and one with its middle gone.
        assert_eq!(ingest(&good[..good.len() / 2]), Err(IngestError::Corrupt));
        let mut broken = good.clone();
        for byte in &mut broken[40..80] {
            *byte = 0;
        }
        assert_eq!(ingest(&broken), Err(IngestError::Corrupt));
        assert_eq!(ingest(&good[..8]), Err(IngestError::Corrupt));
    }

    #[test]
    fn an_input_over_the_byte_limit_is_refused_before_it_is_read() {
        let mut huge = b"\x89PNG\r\n\x1a\n".to_vec();
        huge.resize(MAX_INPUT_BYTES + 1, 0);
        assert_eq!(ingest(&huge), Err(IngestError::TooLarge));
    }

    #[test]
    fn a_picture_declaring_too_many_pixels_is_refused_by_its_header_alone() {
        // 8200 wide: past the side limit.
        let wide = png(8_200, 4, 255);
        assert_eq!(ingest(&wide), Err(IngestError::TooManyPixels));
        // 8000 × 5000 = 40 Mpx: each side is allowed, the product is not.
        let image = image::GrayImage::new(8_000, 5_000);
        let mut bytes = Vec::new();
        image::codecs::png::PngEncoder::new(&mut bytes)
            .write_image(image.as_raw(), 8_000, 5_000, image::ExtendedColorType::L8)
            .unwrap();
        assert!(
            bytes.len() < MAX_INPUT_BYTES,
            "small on disk, large when unpacked"
        );
        assert_eq!(ingest(&bytes), Err(IngestError::TooManyPixels));
    }

    #[test]
    fn metadata_and_trailing_bytes_do_not_survive() {
        let mut with_tail = jpeg(32, 32);
        with_tail.extend_from_slice(b"SECRET-TRAILING-DATA");
        let kept = ingest(&with_tail).unwrap();
        assert!(!String::from_utf8_lossy(&kept.bytes).contains("SECRET"));
        // The kept bytes are a fresh encoding, not the input.
        assert_ne!(kept.bytes, with_tail);
    }

    #[test]
    fn a_noisy_picture_is_squeezed_until_it_fits_what_a_record_can_carry() {
        // Noise does not compress: a big noisy picture needs a smaller size to fit.
        let mut state = 0x2545F4914F6CDD1Du64;
        let image = RgbaImage::from_fn(1_300, 1_300, |_, _| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            image::Rgba([state as u8, (state >> 8) as u8, (state >> 16) as u8, 255])
        });
        let input = encode_png(&image).unwrap();
        assert!(
            input.len() <= MAX_INPUT_BYTES,
            "the test input is a legal size"
        );
        let kept = ingest(&input).unwrap();
        assert!(kept.bytes.len() <= MAX_STORED_BYTES);
        assert!(
            kept.width < 1_800 || kept.height < 1_800,
            "it had to shrink"
        );
        // Transparent noise cannot be squeezed by quality, only by size: it still fits.
        let noisy = RgbaImage::from_fn(900, 900, |x, y| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            image::Rgba([
                state as u8,
                (state >> 8) as u8,
                (state >> 16) as u8,
                (100 + (x + y) % 100) as u8,
            ])
        });
        let kept = ingest(&encode_png(&noisy).unwrap()).unwrap();
        assert!(kept.bytes.len() <= MAX_STORED_BYTES);
        assert_eq!(kept.mime, "image/png");
    }

    #[test]
    fn kept_bytes_are_read_back_for_painting_and_unreadable_ones_say_so() {
        let kept = ingest(&jpeg(70, 50)).unwrap();
        let pixels = pixels(&kept.bytes).unwrap();
        assert_eq!(pixels.size, [70, 50]);
        assert!(super::pixels(b"nope").is_none());
        assert!(describe(b"nope").is_none());
    }

    #[test]
    fn every_error_has_words() {
        for error in [
            IngestError::Empty,
            IngestError::TooLarge,
            IngestError::Unsupported,
            IngestError::Corrupt,
            IngestError::TooManyPixels,
            IngestError::CannotFit,
        ] {
            assert!(!error.message().is_empty());
        }
    }
}

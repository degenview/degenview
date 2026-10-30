/// A colour as it is written in a theme: straight (not premultiplied) 8-bit channels.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// An opaque colour from `0xRRGGBB`.
    pub const fn hex(rgb: u32) -> Self {
        Self::new((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8, 255)
    }

    /// `rgb` at `alpha` (0-255).
    pub const fn with_alpha(rgb: u32, alpha: u8) -> Self {
        let opaque = Self::hex(rgb);
        Self::new(opaque.r, opaque.g, opaque.b, alpha)
    }

    pub const CLEAR: Self = Self::new(0, 0, 0, 0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_unpacks_channels() {
        assert_eq!(Rgba::hex(0x282A36), Rgba::new(0x28, 0x2A, 0x36, 255));
        assert_eq!(Rgba::with_alpha(0xFF5555, 7), Rgba::new(255, 0x55, 0x55, 7));
    }
}

/// A colour as it is written in a theme: straight (not premultiplied) 8-bit channels.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const CLEAR: Self = Self::new(0, 0, 0, 0);

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

    /// `#RRGGBB`, or `#RRGGBBAA` with its alpha when it is not opaque. Upper case.
    pub fn to_hex(self) -> String {
        if self.a == 255 {
            format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
        } else {
            format!("#{:02X}{:02X}{:02X}{:02X}", self.r, self.g, self.b, self.a)
        }
    }

    /// `#RRGGBB` or `#RRGGBBAA`, in either case. Nothing else is a colour: no shorthand, no names,
    /// no missing `#`, so a file means one thing to every reader.
    pub fn parse(text: &str) -> Option<Self> {
        let digits = text.strip_prefix('#')?;
        if !digits.bytes().all(|b| b.is_ascii_hexdigit()) || !matches!(digits.len(), 6 | 8) {
            return None;
        }
        let channel = |at: usize| u8::from_str_radix(&digits[at..at + 2], 16).ok();
        Some(Self::new(
            channel(0)?,
            channel(2)?,
            channel(4)?,
            if digits.len() == 8 { channel(6)? } else { 255 },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_unpacks_channels() {
        assert_eq!(Rgba::hex(0x282A36), Rgba::new(0x28, 0x2A, 0x36, 255));
        assert_eq!(Rgba::with_alpha(0xFF5555, 7), Rgba::new(255, 0x55, 0x55, 7));
    }

    #[test]
    fn text_round_trips() {
        for color in [
            Rgba::hex(0x282A36),
            Rgba::with_alpha(0xBD93F9, 64),
            Rgba::CLEAR,
            Rgba::new(1, 2, 3, 254),
        ] {
            assert_eq!(Rgba::parse(&color.to_hex()), Some(color), "{color:?}");
        }
        assert_eq!(Rgba::hex(0x0A84FF).to_hex(), "#0A84FF");
        assert_eq!(Rgba::with_alpha(0x0A84FF, 0x40).to_hex(), "#0A84FF40");
    }

    #[test]
    fn only_full_hex_is_a_colour() {
        assert_eq!(Rgba::parse("#ff79c6"), Some(Rgba::hex(0xFF79C6)));
        for bad in [
            "",
            "#",
            "ff79c6",
            "#fff",
            "#ff79c",
            "#ff79c6f",
            "#gg79c6",
            "red",
            "#ff79c6 ",
            " #ff79c6",
            "#ff79c6ffaa",
            "#é79c6",
            "rgb(1,2,3)",
            "#+f79c6",
        ] {
            assert_eq!(Rgba::parse(bad), None, "{bad:?}");
        }
    }
}

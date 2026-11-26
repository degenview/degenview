//! TradingView writes colours as `#rrggbb`, `#rrggbbaa`, `rgb(…)` or `rgba(…)`, and sometimes an
//! opacity apart, as a `transparency` percentage.

use crate::model::chart::kind::Rgba;

pub fn parse(text: &str) -> Option<Rgba> {
    let text = text.trim();
    if let Some(hex) = text.strip_prefix('#') {
        let byte = |at: usize| u8::from_str_radix(hex.get(at..at + 2)?, 16).ok();
        return match hex.len() {
            6 => Some([byte(0)?, byte(2)?, byte(4)?, 255]),
            8 => Some([byte(0)?, byte(2)?, byte(4)?, byte(6)?]),
            3 => {
                let mut digits = hex.chars().map(|c| c.to_digit(16));
                let mut next = || Some(digits.next()?? as u8 * 17);
                Some([next()?, next()?, next()?, 255])
            }
            _ => None,
        };
    }
    let inner = text
        .strip_prefix("rgba(")
        .or_else(|| text.strip_prefix("rgb("))?
        .strip_suffix(')')?;
    let mut parts = inner.split(',').map(str::trim);
    let channel = |part: &str| {
        part.parse::<f32>()
            .ok()
            .map(|v| v.round().clamp(0.0, 255.0) as u8)
    };
    let red = channel(parts.next()?)?;
    let green = channel(parts.next()?)?;
    let blue = channel(parts.next()?)?;
    let alpha = match parts.next() {
        Some(part) => (part.parse::<f32>().ok()?.clamp(0.0, 1.0) * 255.0).round() as u8,
        None => 255,
    };
    Some([red, green, blue, alpha])
}

/// `color` with the opacity a `transparency` percentage leaves (0 is opaque).
pub fn with_transparency(color: Rgba, transparency: f64) -> Rgba {
    let alpha = ((1.0 - transparency.clamp(0.0, 100.0) / 100.0) * 255.0).round() as u8;
    [color[0], color[1], color[2], alpha]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_written_form_reads() {
        assert_eq!(parse("#2962FF"), Some([41, 98, 255, 255]));
        assert_eq!(parse("#2962ff80"), Some([41, 98, 255, 128]));
        assert_eq!(parse("#fff"), Some([255, 255, 255, 255]));
        assert_eq!(parse("rgb(1, 2, 3)"), Some([1, 2, 3, 255]));
        assert_eq!(parse("rgba(242, 54, 69, 0.2)"), Some([242, 54, 69, 51]));
        assert_eq!(parse("blue"), None);
        assert_eq!(parse("#12"), None);
    }

    #[test]
    fn transparency_is_the_opposite_of_opacity() {
        assert_eq!(with_transparency([1, 2, 3, 9], 80.0)[3], 51);
        assert_eq!(with_transparency([1, 2, 3, 9], 0.0)[3], 255);
    }
}

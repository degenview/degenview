//! Number formatting for prices.

/// `text` (a plain decimal number) with commas between thousands: `1234567.5` is `1,234,567.5`.
pub fn group_thousands(text: &str) -> String {
    let (integer, fraction) = text.split_once('.').unwrap_or((text, ""));
    let (sign, digits) = integer
        .strip_prefix('-')
        .map_or(("", integer), |remaining| ("-", remaining));
    let mut output = String::new();
    for (index, character) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            output.push(',');
        }
        output.push(character);
    }
    if fraction.is_empty() {
        format!("{sign}{output}")
    } else {
        format!("{sign}{output}.{fraction}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thousands_are_grouped_and_the_fraction_and_sign_are_kept() {
        assert_eq!(group_thousands("1234567.5"), "1,234,567.5");
        assert_eq!(group_thousands("-1234"), "-1,234");
        assert_eq!(group_thousands("999"), "999");
        assert_eq!(group_thousands("1000"), "1,000");
        assert_eq!(group_thousands("0.123456"), "0.123456");
    }
}

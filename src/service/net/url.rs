//! Pieces of a URL built from text a person typed.

/// `text` as one value of a query string or one path segment: every byte but the unreserved
/// ones (letters, digits and `-_.~`) becomes `%XX`, so a space, an `&`, a `#` or a character
/// outside ASCII (a Chinese name searched for) reaches the server as what was typed.
pub fn encode_component(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 2);
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unreserved_characters_stay_and_everything_else_is_escaped() {
        assert_eq!(encode_component("btc-usd_1.5~x"), "btc-usd_1.5~x");
        assert_eq!(encode_component("a b&c=d#e%f"), "a%20b%26c%3Dd%23e%25f");
    }

    #[test]
    fn text_outside_ascii_is_escaped_byte_by_byte() {
        assert_eq!(encode_component("比特币"), "%E6%AF%94%E7%89%B9%E5%B8%81");
        assert_eq!(encode_component("café"), "caf%C3%A9");
        assert_eq!(encode_component(""), "");
    }
}

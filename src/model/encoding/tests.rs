use super::*;

#[test]
fn base64_round_trips_every_length_and_byte() {
    for len in 0..40 {
        let data: Vec<u8> = (0..len).map(|n| (n * 37 + 11) as u8).collect();
        assert_eq!(base64_decode(&base64_encode(&data)).unwrap(), data, "{len}");
    }
    let all: Vec<u8> = (0..=255).collect();
    assert_eq!(base64_decode(&base64_encode(&all)).unwrap(), all);
}

#[test]
fn base64_matches_the_rfc_vectors() {
    for (input, expected) in [
        ("", ""),
        ("f", "Zg=="),
        ("fo", "Zm8="),
        ("foo", "Zm9v"),
        ("foob", "Zm9vYg=="),
        ("fooba", "Zm9vYmE="),
        ("foobar", "Zm9vYmFy"),
        ("user:pass", "dXNlcjpwYXNz"),
    ] {
        assert_eq!(base64_encode(input.as_bytes()), expected, "{input:?}");
    }
    assert_eq!(base64_encode(&[0xFF, 0xEE, 0xDD]), "/+7d");
    assert_eq!(base64_decode("Zm9vYmE=").unwrap(), b"fooba");
    assert!(base64_decode("not base64!").is_none());
}

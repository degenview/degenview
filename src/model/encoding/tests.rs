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

#[test]
fn hex_is_lower_case_and_reads_back_at_its_length_only() {
    let key = [0xAB; 32];
    assert_eq!(hex(&[0x00, 0x0f, 0xa0, 0xff]), "000fa0ff");
    assert_eq!(hex_decode_array::<32>(&hex(&key)), Some(key));
    assert_eq!(
        hex_decode_array::<32>(&format!("  {}\n", hex(&key))),
        Some(key)
    );
    assert_eq!(hex_decode_array::<2>("ABcd"), Some([0xab, 0xcd]));
    assert_eq!(hex_decode_array::<32>(""), None);
    assert_eq!(hex_decode_array::<32>(&"zz".repeat(32)), None);
    assert_eq!(hex_decode_array::<32>(&"ab".repeat(31)), None);
    assert_eq!(hex_decode_array::<32>(&"é".repeat(32)), None);
}

#[test]
fn sha256_hex_matches_the_known_digest() {
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

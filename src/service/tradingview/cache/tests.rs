use super::testing::{self, Fake, Scratch};
use super::*;

const WANTED: &str = "https://charts-storage.tradingview.com/wanted?jwt=secret";

fn entries() -> Vec<Fake<'static>> {
    vec![
        Fake::gzipped(WANTED, r#"{"a":1}"#),
        Fake::ok("https://www.tradingview.com/not-asked-for", "private"),
        Fake {
            status: 403,
            ..Fake::ok("https://charts-storage.tradingview.com/wanted/denied", "no")
        },
        Fake {
            coding: Some("compress"),
            ..Fake::ok("https://charts-storage.tradingview.com/wanted/odd", "??")
        },
    ]
}

fn wanted(url: &str) -> bool {
    url.contains("/wanted")
}

#[test]
fn both_formats_give_the_successful_wanted_responses_decompressed() {
    for (format, write) in [
        ("simple", testing::simple as fn(&Path, &[Fake])),
        ("blockfile", testing::blockfile),
    ] {
        let temp = Scratch::new();
        write(temp.path(), &entries());
        assert!(is_cache(temp.path()), "{format}");
        let found = read(temp.path(), &wanted).unwrap();
        assert_eq!(found.len(), 1, "{format}: {found:?}");
        assert_eq!(found[0].url, WANTED, "{format}");
        assert_eq!(found[0].body, br#"{"a":1}"#, "{format}");
        assert!(found[0].at_ms > 1_700_000_000_000, "{format}");
    }
}

#[test]
fn brotli_and_zstd_bodies_decompress() {
    // `echo -n hello | brotli` and `| zstd`.
    let brotli = [0x0b, 0x02, 0x80, b'h', b'e', b'l', b'l', b'o', 0x03];
    let zstd = [
        0x28, 0xb5, 0x2f, 0xfd, 0x20, 0x05, 0x29, 0x00, 0x00, b'h', b'e', b'l', b'l', b'o',
    ];
    assert_eq!(encoding::decode(Some("br"), &brotli).unwrap(), b"hello");
    assert_eq!(encoding::decode(Some("zstd"), &zstd).unwrap(), b"hello");
    // Two frames with a skippable one between them.
    let skipped = [0x50, 0x2a, 0x4d, 0x18, 0x02, 0x00, 0x00, 0x00, 0xaa, 0xbb];
    let several = [&zstd[..], &skipped, &zstd].concat();
    assert_eq!(
        encoding::decode(Some("zstd"), &several).unwrap(),
        b"hellohello"
    );
}

#[test]
fn damage_never_panics() {
    let temp = Scratch::new();
    testing::simple(temp.path(), &[Fake::ok(WANTED, "body")]);
    let path = temp.path().join(format!("{:016x}_0", 0));
    let whole = std::fs::read(&path).unwrap();
    for cut in 0..whole.len() {
        std::fs::write(&path, &whole[..cut]).unwrap();
        assert!(read(temp.path(), &wanted).unwrap().is_empty(), "cut {cut}");
    }

    let temp = Scratch::new();
    testing::blockfile(temp.path(), &[Fake::ok(WANTED, "body")]);
    for name in ["data_1", "index", "f_000001", "f_000002"] {
        let path = temp.path().join(name);
        let whole = std::fs::read(&path).unwrap();
        for cut in [0, 1, whole.len() / 2, whole.len().saturating_sub(1)] {
            std::fs::write(&path, &whole[..cut]).unwrap();
            let _ = read(temp.path(), &wanted);
        }
        let mut garbage = whole.clone();
        garbage
            .iter_mut()
            .skip(8)
            .step_by(3)
            .for_each(|b| *b = 0xff);
        std::fs::write(&path, garbage).unwrap();
        let _ = read(temp.path(), &wanted);
        std::fs::write(&path, whole).unwrap();
    }
}

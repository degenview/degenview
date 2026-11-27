use super::*;

#[test]
fn a_name_is_one_clean_line() {
    assert_eq!(
        sanitize("  Pepe\n\tthe  Frog "),
        Some("Pepe the Frog".into())
    );
    assert_eq!(sanitize("Bit\u{200B}co\u{202E}in"), Some("Bitcoin".into()));
    assert_eq!(sanitize(" \u{0} "), None);
    let long = sanitize(&"a".repeat(200)).expect("a name");
    assert_eq!(long.chars().count(), 64);
    assert!(long.ends_with('…'));
}

#[test]
fn a_company_loses_the_security_but_keeps_its_class() {
    for (listing, name) in [
        ("Apple Inc. Common Stock", "Apple Inc."),
        (
            "Alphabet Inc. Class A Common Stock",
            "Alphabet Inc. Class A",
        ),
        (
            "Berkshire Hathaway Inc. Class B",
            "Berkshire Hathaway Inc. Class B",
        ),
        (
            "Alibaba Group Holding Limited American Depositary Shares, each representing eight Ordinary share",
            "Alibaba Group Holding Limited",
        ),
        (
            "Accenture plc Class A Ordinary Shares",
            "Accenture plc Class A",
        ),
        (
            "Microsoft Corporation, Common Stock, $0.00000625 par value",
            "Microsoft Corporation",
        ),
        ("SPDR S&P 500 ETF Trust", "SPDR S&P 500 ETF Trust"),
        ("Common Stock", "Common Stock"),
    ] {
        assert_eq!(company_name(listing), name, "{listing}");
    }
}

#[test]
fn currencies_and_stablecoins_have_names() {
    assert_eq!(currency_name("usd"), Some("US Dollar"));
    assert_eq!(currency_name("USDT"), Some("Tether"));
    assert_eq!(currency_name("BTC"), None);
}

#[test]
fn old_records_without_names_still_load() {
    let names: AssetNames = serde_json::from_str("{}").expect("loads");
    assert_eq!(names, AssetNames::default());
}

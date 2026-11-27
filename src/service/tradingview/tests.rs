use super::cache::testing::{self, Fake, Scratch};
use super::*;
use crate::model::tradingview::fixtures;

#[test]
fn only_layouts_drawings_watchlists_and_own_scripts_are_read() {
    for url in [
        "https://www.tradingview.com/chart/AbCd1234/",
        "https://www.tradingview.com/chart/",
        "https://de.tradingview.com/chart/AbCd1234/?symbol=A",
        "https://charts-storage.tradingview.com/charts-storage/get/layout/AbCd1234/sources?jwt=x",
        "https://www.tradingview.com/api/v1/symbols_list/all/",
        "https://www.tradingview.com/api/v1/symbols_list/custom/42",
        "https://pine-facade.tradingview.com/pine-facade/get/USER%3Babc/2.0",
        "https://pine-facade.tradingview.com/pine-facade/list/?filter=saved",
    ] {
        assert!(route(url).is_some(), "{url}");
    }
    for url in [
        "https://www.tradingview.com/api/v1/user/profile/me/",
        "https://www.tradingview.com/accounts/signin/",
        "https://www.tradingview.com/chart/-/XyZ12345/json/",
        "https://www.tradingview.com/api/v1/symbols_list/colored/",
        "https://pine-facade.tradingview.com/pine-facade/get/STD%3BSMA/last",
        "https://pine-facade.tradingview.com/pine-facade/list/?filter=standard",
        "https://evil.example/chart/AbCd1234/",
        "https://tradingview.com.evil.example/chart/AbCd1234/",
        "https://charts-storage.tradingview.com/charts-storage/get/user/sources",
    ] {
        assert_eq!(route(url), None, "{url}");
    }
}

#[test]
fn a_chosen_folder_is_scanned_into_a_library() {
    let temp = Scratch::new();
    let sources = fixtures::sources("Ab", 1, &[fixtures::trend_line("a")]);
    let Document::Sources { json, .. } = sources.document else {
        unreachable!();
    };
    testing::simple(
        &temp.path().join("Cache/Cache_Data"),
        &[Fake::gzipped(
            "https://charts-storage.tradingview.com/charts-storage/get/layout/Ab/sources?jwt=t",
            &json,
        )],
    );
    let scan = scan(Some(temp.path())).unwrap();
    assert_eq!(scan.folder, temp.path());
    assert_eq!(scan.library.layouts[0].drawings.len(), 1);
}

#[test]
fn a_folder_without_a_cache_and_a_cache_without_charts_say_so() {
    let temp = Scratch::new();
    assert!(matches!(scan(Some(temp.path())), Err(ScanError::NotACache)));
    testing::simple(temp.path(), &[Fake::ok("https://www.tradingview.com/", "")]);
    assert!(matches!(
        scan(Some(temp.path())),
        Err(ScanError::NothingFound)
    ));
}

#[test]
fn a_sandbox_that_finds_nothing_says_to_point_at_the_folder() {
    assert!(matches!(nowhere(false, true), ScanError::Sandboxed));
    assert!(matches!(nowhere(false, false), ScanError::NotInstalled));
    assert!(matches!(nowhere(true, true), ScanError::NotACache));
    assert!(ScanError::Sandboxed.hint().contains(".config/TradingView"));
}

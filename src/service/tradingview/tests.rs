use super::cache::testing::{self, Fake, Scratch};
use super::*;
use crate::model::tradingview::fixtures;

#[test]
fn only_layouts_drawings_watchlists_and_own_scripts_are_read() {
    for url in [
        "https://www.tradingview.com/chart/24Y3zHQt/",
        "https://www.tradingview.com/chart/",
        "https://de.tradingview.com/chart/24Y3zHQt/?symbol=A",
        "https://charts-storage.tradingview.com/charts-storage/get/layout/24Y3zHQt/sources?jwt=x",
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
        "https://www.tradingview.com/chart/-/bzIRuGXC/json/",
        "https://www.tradingview.com/api/v1/symbols_list/colored/",
        "https://pine-facade.tradingview.com/pine-facade/get/STD%3BSMA/last",
        "https://pine-facade.tradingview.com/pine-facade/list/?filter=standard",
        "https://evil.example/chart/24Y3zHQt/",
        "https://tradingview.com.evil.example/chart/24Y3zHQt/",
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
#[ignore]
fn zz_probe_real() {
    let found = match scan(
        std::env::var_os("TV_DIR")
            .map(std::path::PathBuf::from)
            .as_deref(),
    ) {
        Ok(scan) => scan,
        Err(error) => return println!("ERR {error}"),
    };
    for l in &found.library.layouts {
        println!(
            "layout {} {:?} grid {:?} drawings {} charts {:?}",
            l.id,
            l.name,
            l.grid,
            l.drawings.len(),
            l.charts
                .iter()
                .map(|c| format!(
                    "{} {} shown {} tick {:?}",
                    c.symbol, c.interval, c.shown, c.tick
                ))
                .collect::<Vec<_>>()
        );
    }
}

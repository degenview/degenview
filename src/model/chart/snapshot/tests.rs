use super::*;

const NOON: i64 = 1_792_413_720_000; // 2026-10-19 12:42:00 UTC

#[test]
fn the_menu_has_five_rows_and_three_need_a_host() {
    assert_eq!(SnapshotAction::ALL.len(), 5);
    let hosted: Vec<_> = SnapshotAction::ALL
        .into_iter()
        .filter(|action| action.is_hosted())
        .map(SnapshotAction::label)
        .collect();
    assert_eq!(hosted, ["Copy link", "Open in new tab", "Tweet image"]);
}

#[test]
fn a_snapshot_is_drawn_again_at_the_scale_not_stretched() {
    let size = SnapshotDimensions::fit(760.0, 520.0, SnapshotScale::X2).expect("fits");
    assert_eq!((size.width, size.height), (1520, 1040));
    assert_eq!(size.pixels_per_point, 2.0);
    let size = SnapshotDimensions::fit(760.0, 520.0, SnapshotScale::X1).expect("fits");
    assert_eq!((size.width, size.height), (760, 520));
    // The aspect ratio holds when the size is not a whole number of points.
    let size = SnapshotDimensions::fit(333.4, 200.2, SnapshotScale::X2).expect("fits");
    assert_eq!((size.width, size.height), (667, 400));
}

#[test]
fn a_huge_chart_is_drawn_at_a_smaller_scale_within_the_limits() {
    let size = SnapshotDimensions::fit(6000.0, 4000.0, SnapshotScale::X2).expect("fits");
    assert!(size.pixels_per_point < 2.0);
    assert!(size.width <= MAX_SIDE_PX && size.height <= MAX_SIDE_PX);
    assert!(u64::from(size.width) * u64::from(size.height) <= MAX_PIXELS);
    // A side past the limit at scale 1 is drawn smaller rather than refused.
    let wide = SnapshotDimensions::fit(10_000.0, 100.0, SnapshotScale::X1).expect("fits");
    assert!(
        (8_000..=MAX_SIDE_PX).contains(&wide.width),
        "{}",
        wide.width
    );
    assert!(wide.pixels_per_point < 1.0);
    // Too large for even the smallest scale.
    assert_eq!(
        SnapshotDimensions::fit(40_000.0, 40_000.0, SnapshotScale::X1),
        None
    );
}

#[test]
fn a_chart_without_a_size_has_no_snapshot() {
    for (w, h) in [
        (0.0, 100.0),
        (100.0, 0.5),
        (f32::NAN, 100.0),
        (100.0, f32::INFINITY),
    ] {
        assert_eq!(
            SnapshotDimensions::fit(w, h, SnapshotScale::X2),
            None,
            "{w}x{h}"
        );
    }
}

#[test]
fn the_file_is_named_for_the_chart_and_the_local_time() {
    assert_eq!(
        file_name("BTCUSD", "1H", NOON, 0),
        "DegenView_BTCUSD_1H_2026-10-19_1242.png"
    );
    // Two hours east of UTC: the local clock and, across midnight, the local date.
    assert_eq!(
        file_name("BTCUSD", "1H", NOON, 2 * 3_600),
        "DegenView_BTCUSD_1H_2026-10-19_1442.png"
    );
    assert_eq!(
        file_name("BTCUSD", "1H", NOON + 12 * 3_600_000, 0),
        "DegenView_BTCUSD_1H_2026-10-20_0042.png"
    );
    assert_eq!(
        file_name("BTCUSD", "1H", NOON, -13 * 3_600),
        "DegenView_BTCUSD_1H_2026-10-18_2342.png"
    );
}

#[test]
fn a_name_keeps_to_what_every_file_system_takes() {
    assert_eq!(
        file_name("BTC/USDT", "1M", NOON, 0),
        "DegenView_BTCUSDT_1M_2026-10-19_1242.png"
    );
    assert_eq!(
        file_name("a:b*c?d\"e<f>g|h\\i j", "1D", NOON, 0),
        "DegenView_abcdefghij_1D_2026-10-19_1242.png"
    );
    assert_eq!(
        file_name("../../etc/passwd\0", "1D", NOON, 0),
        "DegenView_etcpasswd_1D_2026-10-19_1242.png"
    );
    assert_eq!(
        file_name("日本語", "", NOON, 0),
        "DegenView_Chart_Chart_2026-10-19_1242.png"
    );
    let long = "X".repeat(200);
    let name = file_name(&long, "1D", NOON, 0);
    assert_eq!(name.matches('X').count(), MAX_NAME_PART);
}

#[test]
fn a_name_gets_its_extension_and_a_free_number() {
    assert_eq!(with_png_extension("chart"), "chart.png");
    assert_eq!(with_png_extension("chart.PNG"), "chart.PNG");
    assert_eq!(unused_name("a.png", |_| false), "a.png");
    assert_eq!(unused_name("a.png", |n| n == "a.png"), "a (2).png");
    assert_eq!(
        unused_name("a.png", |n| n == "a.png" || n == "a (2).png"),
        "a (3).png"
    );
}

#[test]
fn the_post_names_the_chart_and_carries_the_link() {
    assert_eq!(share_text("BTC/USD", "1H"), "BTC/USD 1H chart — DegenView");
    assert_eq!(
        x_compose_url(
            "BTC/USD 1H chart — DegenView",
            "https://s.example/a?b=1&c=2"
        ),
        "https://x.com/intent/post?text=BTC%2FUSD%201H%20chart%20%E2%80%94%20DegenView\
         &url=https%3A%2F%2Fs.example%2Fa%3Fb%3D1%26c%3D2"
    );
}

#[test]
fn a_host_is_https_or_this_computer() {
    let host = SnapshotHost::parse(" https://snap.example.com/v1/snapshots ").expect("valid");
    assert_eq!(host.endpoint(), "https://snap.example.com/v1/snapshots");
    assert_eq!(host.authority(), "snap.example.com");
    assert_eq!(
        SnapshotHost::parse("https://snap.example.com:8443/up")
            .expect("valid")
            .authority(),
        "snap.example.com:8443"
    );
    assert!(SnapshotHost::parse("http://127.0.0.1:9000/up").is_ok());
    assert!(SnapshotHost::parse("http://localhost/up").is_ok());
    for (text, error) in [
        ("", HostError::Empty),
        ("   ", HostError::Empty),
        ("snap.example.com", HostError::NotAUrl),
        ("https://", HostError::NotAUrl),
        ("http://snap.example.com/up", HostError::NotHttps),
        ("ftp://snap.example.com/up", HostError::NotHttps),
        (
            "https://me:secret@snap.example.com/up",
            HostError::Credentials,
        ),
        ("https://snap.example.com/up#top", HostError::Fragment),
        ("https://snap.example.com/u p", HostError::NotAUrl),
    ] {
        assert_eq!(SnapshotHost::parse(text), Err(error), "{text:?}");
    }
}

fn host() -> SnapshotHost {
    SnapshotHost::parse("https://snap.example.com/up").expect("valid")
}

#[test]
fn a_host_answers_with_a_link() {
    let shared = parse_upload_response(
        br#"{"url":"https://cdn.example.com/s/AbC123.png","delete_url":"https://snap.example.com/d/xyz","expires_at":"2027-01-01T00:00:00Z"}"#,
        &host(),
    )
    .expect("a snapshot");
    assert_eq!(shared.url, "https://cdn.example.com/s/AbC123.png");
    assert_eq!(
        shared.delete_url.as_deref(),
        Some("https://snap.example.com/d/xyz")
    );
    assert_eq!(shared.expires_at.as_deref(), Some("2027-01-01T00:00:00Z"));
    let plain = parse_upload_response(br#"{"url":"https://a.example/x"}"#, &host())
        .expect("only the link is needed");
    assert_eq!((plain.delete_url, plain.expires_at), (None, None));
}

#[test]
fn anything_but_a_safe_link_is_no_snapshot() {
    for body in [
        &b""[..],
        b"not json",
        b"[]",
        b"{}",
        br#"{"url":null}"#,
        br#"{"url":""}"#,
        br#"{"url":"http://a.example/x"}"#,
        br#"{"url":"file:///etc/passwd"}"#,
        br#"{"url":"javascript:alert(1)"}"#,
        br#"{"url":"https://user:pw@a.example/x"}"#,
        br#"{"url":"https://a.example/with space"}"#,
        br#"{"url":42}"#,
    ] {
        assert_eq!(
            parse_upload_response(body, &host()),
            Err(UploadError::BadResponse),
            "{}",
            String::from_utf8_lossy(body)
        );
    }
    let long = format!(
        r#"{{"url":"https://a.example/{}"}}"#,
        "x".repeat(MAX_LINK_CHARS)
    );
    assert_eq!(
        parse_upload_response(long.as_bytes(), &host()),
        Err(UploadError::BadResponse)
    );
    // A bad delete link is dropped, not trusted and not fatal.
    let shared = parse_upload_response(
        br#"{"url":"https://a.example/x","delete_url":"javascript:1"}"#,
        &host(),
    )
    .expect("the link is fine");
    assert_eq!(shared.delete_url, None);
}

#[test]
fn a_host_on_this_computer_may_answer_with_plain_http() {
    let local = SnapshotHost::parse("http://127.0.0.1:9000/up").expect("valid");
    assert!(parse_upload_response(br#"{"url":"http://127.0.0.1:9000/s/1"}"#, &local).is_ok());
    assert!(parse_upload_response(br#"{"url":"http://127.0.0.1:9000/s/1"}"#, &host()).is_err());
}

#[test]
fn only_a_failure_that_can_pass_is_worth_a_retry() {
    for error in [
        UploadError::Offline,
        UploadError::Timeout,
        UploadError::RateLimited {
            retry_after: Some(3),
        },
        UploadError::Server(503),
    ] {
        assert!(error.retryable(), "{error:?}");
    }
    for error in [
        UploadError::TooLarge,
        UploadError::Unauthorized,
        UploadError::RejectedAsTooLarge,
        UploadError::BadResponse,
    ] {
        assert!(!error.retryable(), "{error:?}");
    }
    assert_eq!(
        UploadError::RateLimited {
            retry_after: Some(7)
        }
        .message("ImgBB"),
        "Too many uploads. Try again in 7 s."
    );
}

#[test]
fn what_went_wrong_names_where_the_image_was_going() {
    assert_eq!(
        UploadError::Unauthorized.message("ImgBB"),
        "ImgBB refused the credentials. Check Settings ▸ Snapshots."
    );
    assert_eq!(
        UploadError::Offline.message("snap.example.com"),
        "Could not reach snap.example.com. Check your connection."
    );
    assert_eq!(
        UploadError::Server(502).message("ImgBB"),
        "ImgBB failed (HTTP 502)."
    );
    assert_eq!(
        UploadError::BadResponse.message("ImgBB"),
        "ImgBB did not return a link."
    );
}

fn read(json: &str) -> SnapshotPrefs {
    serde_json::from_str(json).expect("prefs")
}

#[test]
fn the_defaults_share_nothing_and_include_no_trading() {
    let prefs = SnapshotPrefs::default();
    assert_eq!(prefs.provider, None);
    assert!(prefs.providers.is_empty());
    assert!(!prefs.include_trading);
    assert_eq!(prefs.scale, SnapshotScale::X2);
    // Nothing saved reads as the defaults.
    assert_eq!(read("{}"), prefs);
}

#[test]
fn a_save_from_before_providers_keeps_its_host_and_keeps_sharing_to_it() {
    let prefs = read(
        r#"{"host":"https://s.example/up","include_trading":true,"scale":"X1","consented":["s.example"]}"#,
    );
    assert_eq!(prefs.provider.as_deref(), Some(SELF_HOSTED));
    assert_eq!(
        prefs
            .settings_of(SELF_HOSTED)
            .get("host")
            .map(String::as_str),
        Some("https://s.example/up")
    );
    assert!(prefs.include_trading);
    assert_eq!(prefs.scale, SnapshotScale::X1);
    assert!(prefs.has_consented("s.example"));
    // An address the page would not have accepted is not carried over, and not synced.
    for bad in [
        "https://u:p@s.example/up",
        "http://remote.example/up",
        "s.example",
    ] {
        let prefs = read(&format!(r#"{{"host":"{bad}"}}"#));
        assert_eq!(prefs.provider, None, "{bad}");
        assert!(prefs.providers.is_empty(), "{bad}");
    }
    // An older save that never set a host shares to nothing.
    assert_eq!(read(r#"{"host":null,"scale":"X1"}"#).provider, None);
    assert_eq!(read(r#"{"host":"  "}"#).provider, None);
}

#[test]
fn once_the_provider_is_written_a_stray_host_is_ignored() {
    // Turned off on purpose: the `null` is a choice, not a missing key.
    let off = read(r#"{"provider":null,"host":"https://s.example/up"}"#);
    assert_eq!(off.provider, None);
    assert!(off.providers.is_empty());
    // Another provider is on: an older Mac's host does not replace it.
    let other = read(r#"{"provider":"imgbb","host":"https://s.example/up"}"#);
    assert_eq!(other.provider.as_deref(), Some("imgbb"));
    assert!(other.providers.is_empty());
}

#[test]
fn a_migrated_save_is_written_in_the_new_shape_and_reads_back_the_same() {
    let migrated = read(r#"{"host":"https://s.example/up"}"#);
    let json = serde_json::to_value(&migrated).unwrap();
    assert!(json.get("host").is_none(), "{json}");
    assert_eq!(json["provider"], SELF_HOSTED);
    assert_eq!(
        serde_json::from_value::<SnapshotPrefs>(json).unwrap(),
        migrated
    );
    // Off is written as an explicit null so it is not migrated back on.
    let off = SnapshotPrefs::default();
    let json = serde_json::to_value(&off).unwrap();
    assert!(json["provider"].is_null());
    assert!(json.as_object().unwrap().contains_key("provider"));
}

#[test]
fn a_provider_this_build_does_not_know_is_kept() {
    let prefs = read(r#"{"provider":"cloudinary","providers":{"cloudinary":{"cloud":"x"}}}"#);
    assert_eq!(prefs.provider.as_deref(), Some("cloudinary"));
    let back = serde_json::to_value(&prefs).unwrap();
    assert_eq!(back["providers"]["cloudinary"]["cloud"], "x");
}

#[test]
fn turning_a_provider_on_turns_the_other_off_and_keeps_what_it_was_given() {
    let mut prefs = SnapshotPrefs::default();
    prefs.set_field("imgbb", "expiry", "7d");
    prefs.activate(Some("imgbb"));
    prefs.activate(Some(SELF_HOSTED));
    assert_eq!(prefs.provider.as_deref(), Some(SELF_HOSTED));
    assert_eq!(
        prefs.settings_of("imgbb").get("expiry").map(String::as_str),
        Some("7d"),
        "the one switched off keeps its settings"
    );
    prefs.activate(None);
    assert_eq!(prefs.provider, None);
}

#[test]
fn a_field_is_trimmed_and_an_empty_one_clears_it() {
    let mut prefs = SnapshotPrefs::default();
    prefs.set_field(SELF_HOSTED, "host", "  https://s.example/up ");
    assert_eq!(
        prefs
            .settings_of(SELF_HOSTED)
            .get("host")
            .map(String::as_str),
        Some("https://s.example/up")
    );
    prefs.set_field(SELF_HOSTED, "host", "   ");
    assert!(prefs.providers.is_empty(), "nothing left, nothing kept");
    prefs.set_field("never-set", "x", "");
    assert!(prefs.providers.is_empty());
}

#[test]
fn consent_is_per_destination() {
    let mut prefs = SnapshotPrefs::default();
    assert!(!prefs.has_consented("a.example"));
    prefs.consent("a.example");
    prefs.consent("a.example");
    assert!(prefs.has_consented("a.example") && !prefs.has_consented("b.example"));
    assert_eq!(prefs.consented.len(), 1);
}

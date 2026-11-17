use super::*;
use crate::ui::settings::window::{SIDEBAR_WIDTH, SettingsSection, WINDOW_SIZE, page_shell};
use eframe::egui::Vec2;

fn manual(host: &str, port: &str) -> NetworkState {
    let mut state = NetworkState::default();
    state.load(&ProxyConfig::default());
    state.draft.mode = ProxyMode::Manual;
    state.draft.host = host.to_owned();
    state.port_text = port.to_owned();
    state
}

fn active(source: Source, server: &str) -> Status {
    Status {
        mode: ProxyMode::System,
        active: Some(Active {
            source,
            server: server.to_owned(),
        }),
        problem: None,
    }
}

#[test]
fn the_draft_follows_the_saved_setting_when_it_changes_elsewhere() {
    let mut state = NetworkState::default();
    let saved = ProxyConfig {
        mode: ProxyMode::Manual,
        host: "p".to_owned(),
        port: Some(8080),
        ..ProxyConfig::default()
    };
    state.load(&saved);
    assert_eq!(state.draft, saved);
    assert_eq!(state.port_text, "8080");
    state.draft.host = "typing".to_owned();
    state.load(&saved);
    assert_eq!(
        state.draft.host, "typing",
        "an unchanged setting leaves the draft alone"
    );
    let other = ProxyConfig {
        mode: ProxyMode::Off,
        ..saved
    };
    state.load(&other);
    assert_eq!(state.draft, other);
}

#[test]
fn a_manual_proxy_with_nothing_typed_waits_without_an_error() {
    let state = manual("", "");
    assert_eq!(state.problem(), None);
    assert!(!state.can_save());
}

#[test]
fn what_is_wrong_is_said_in_words() {
    assert!(manual("", "8080").problem().unwrap().contains("host"));
    assert!(manual("proxy", "").problem().unwrap().contains("port"));
    assert!(
        manual("proxy", "99999")
            .problem()
            .unwrap()
            .contains("1 to 65535")
    );
    assert!(
        manual("proxy", "0")
            .problem()
            .unwrap()
            .contains("1 to 65535")
    );
    assert!(
        manual("proxy", "http")
            .problem()
            .unwrap()
            .contains("1 to 65535")
    );
    assert!(
        manual("http://proxy", "80")
            .problem()
            .unwrap()
            .contains("http://")
    );
    assert_eq!(manual("proxy", "8080").problem(), None);
}

/// The port field is only on the page in Manual mode, so what is typed in it cannot hold up the
/// other two: switching to System or Off after a typo must save.
#[test]
fn a_bad_port_only_matters_while_the_proxy_is_manual() {
    let mut state = manual("proxy", "abc");
    assert!(state.problem().is_some());
    assert!(state.save().is_none());
    for mode in [ProxyMode::System, ProxyMode::Off] {
        state.draft.mode = mode;
        assert_eq!(state.problem(), None, "{mode:?}");
        let Some(NetworkAction::Save { config, .. }) = state.save() else {
            panic!("{mode:?} should save with a bad port left behind");
        };
        assert_eq!(config.mode, mode);
        assert_eq!(config.port, None);
    }
}

#[test]
fn the_no_proxy_list_is_checked_where_its_field_is_shown() {
    let mut state = NetworkState::default();
    state.load(&ProxyConfig::default());
    state.draft.no_proxy = "10.0.0.0/8".to_owned();
    for mode in [ProxyMode::System, ProxyMode::Manual] {
        state.draft.mode = mode;
        state.draft.host = "proxy".to_owned();
        state.port_text = "8080".to_owned();
        assert!(state.problem().unwrap().contains("10.0.0.0/8"), "{mode:?}");
        assert!(state.save().is_none(), "{mode:?}");
    }
    // Off hides the list, so it cannot block turning the proxy off.
    state.draft.mode = ProxyMode::Off;
    assert_eq!(state.problem(), None);
    assert!(state.save().is_some());
}

#[test]
fn system_and_off_save_straight_away() {
    let mut state = NetworkState::default();
    state.load(&ProxyConfig::default());
    for mode in [ProxyMode::System, ProxyMode::Off] {
        state.draft.mode = mode;
        let Some(NetworkAction::Save { config, password }) = state.save() else {
            panic!("{mode:?} should save");
        };
        assert_eq!(config.mode, mode);
        assert_eq!(password, PasswordChange::Keep);
    }
}

#[test]
fn a_manual_save_trims_and_carries_the_port() {
    let mut state = manual("  proxy.example ", " 8080 ");
    state.draft.username = " me ".to_owned();
    let Some(NetworkAction::Save { config, .. }) = state.save() else {
        panic!("should save");
    };
    assert_eq!(config.host, "proxy.example");
    assert_eq!(config.port, Some(8080));
    assert_eq!(config.username, "me");
}

#[test]
fn a_password_is_sent_once_and_then_forgotten_by_the_page() {
    let mut state = manual("proxy", "8080");
    state.password_input = "s3cret".to_owned();
    let Some(NetworkAction::Save { password, .. }) = state.save() else {
        panic!("should save");
    };
    assert_eq!(password, PasswordChange::Set("s3cret".to_owned()));
    assert!(state.password_input.is_empty());
    let Some(NetworkAction::Save { password, .. }) = state.save() else {
        panic!("should save");
    };
    assert_eq!(password, PasswordChange::Keep);
}

#[test]
fn removing_the_password_is_asked_once() {
    let mut state = manual("proxy", "8080");
    state.removing_password = true;
    let Some(NetworkAction::Save { password, .. }) = state.save() else {
        panic!("should save");
    };
    assert_eq!(password, PasswordChange::Remove);
    assert!(!state.removing_password);
}

#[test]
fn the_badge_says_what_is_in_use() {
    let (text, tone) = badge(&active(Source::System, "127.0.0.1:7890"));
    assert_eq!(text, "Using 127.0.0.1:7890 from the system");
    assert_eq!(tone, Tone::Good);
    assert_eq!(
        badge(&active(Source::Environment, "p:1")).0,
        "Using p:1 from the environment"
    );
    for (mode, text) in [
        (ProxyMode::System, "Connecting directly"),
        (ProxyMode::Off, "Proxy off"),
        (ProxyMode::Manual, "No proxy server set"),
    ] {
        let (shown, tone) = badge(&Status {
            mode,
            active: None,
            problem: None,
        });
        assert_eq!(shown, text);
        assert_eq!(tone, Tone::Neutral);
    }
}

#[test]
fn every_mode_has_a_hint() {
    for mode in [ProxyMode::System, ProxyMode::Manual, ProxyMode::Off] {
        assert!(!mode_hint(mode).is_empty());
    }
}

/// The page in the Settings window's size, in both themes; with `DEGENVIEW_GALLERY_DIR` set the
/// pictures are written out to look at.
#[test]
fn the_network_page_draws_in_every_mode_and_both_themes() {
    use crate::ui::render_tests::{Screen, gallery_dir};
    use crate::ui::theme::AppTheme;
    let _theme = crate::ui::theme::RestoreDefaultTheme;
    let _ = ProxyPasswordStore::shared();
    let cases = [
        (
            "system",
            ProxyConfig::default(),
            active(Source::System, "127.0.0.1:7890"),
        ),
        (
            "manual",
            ProxyConfig {
                mode: ProxyMode::Manual,
                protocol: Protocol::Socks5,
                host: "127.0.0.1".to_owned(),
                port: Some(1080),
                username: "me".to_owned(),
                no_proxy: "example.com, *.corp.test".to_owned(),
            },
            Status {
                mode: ProxyMode::Manual,
                active: Some(Active {
                    source: Source::Manual,
                    server: "127.0.0.1:1080 (SOCKS5)".to_owned(),
                }),
                problem: None,
            },
        ),
        (
            "off",
            ProxyConfig {
                mode: ProxyMode::Off,
                ..ProxyConfig::default()
            },
            Status {
                mode: ProxyMode::Off,
                active: None,
                problem: None,
            },
        ),
    ];
    for (theme_name, app_theme) in [("dark", AppTheme::Dark), ("light", AppTheme::Light)] {
        for (case, config, status) in &cases {
            let mut screen = Screen::new(WINDOW_SIZE - Vec2::new(SIDEBAR_WIDTH, 0.0));
            theme::apply(&screen.ctx, app_theme);
            let mut state = NetworkState::default();
            let mut actions = Vec::new();
            let mut last = None;
            for _ in 0..3 {
                last = Some(screen.run(Vec::new(), |ui| {
                    let mut page = ui.new_child(egui::UiBuilder::new().max_rect(ui.max_rect()));
                    page_shell(
                        &mut page,
                        SettingsSection::Network,
                        "Route DegenView's connections through a proxy.",
                        |ui| {
                            body(
                                ui,
                                &mut state,
                                &NetworkInput { config, status },
                                &mut actions,
                            )
                        },
                    );
                }));
            }
            assert!(actions.is_empty(), "{case}: drawing alone saved something");
            if gallery_dir().is_some() {
                screen.save(
                    &format!("settings-network-{case}-{theme_name}"),
                    last.expect("a frame"),
                );
            }
        }
    }
}

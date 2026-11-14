use super::clients::client_row;
use super::controls::{action_card, field_with_rect};
use super::security::maximum_row;
use super::security::{detect_local_address, parse_interface};
use super::*;
use crate::mcp::state::McpStatus;
use crate::model::mcp::{
    BindMode, Cidr, McpAction, McpClient, McpSettings, Permission, PermissionSet,
};
use crate::ui::icons::Icon;
use crate::ui::kit;
use crate::ui::kit::Tone;
use eframe::egui::{self, Id, Pos2, Rect, Ui, vec2};

#[test]
fn connection_json_names_the_endpoint_and_never_invents_a_token() {
    let with_token = connection_json("http://127.0.0.1:8765/mcp", Some("dvmcp_abc"));
    let parsed: serde_json::Value = serde_json::from_str(&with_token).unwrap();
    let server = &parsed["mcpServers"]["degenview"];
    assert_eq!(server["url"], "http://127.0.0.1:8765/mcp");
    assert_eq!(server["headers"]["Authorization"], "Bearer dvmcp_abc");

    let placeholder = connection_json("http://127.0.0.1:8765/mcp", None);
    assert!(placeholder.contains("<YOUR_TOKEN>"));
    assert!(!placeholder.contains("dvmcp_"));
}

#[test]
fn statuses_have_distinct_tones() {
    assert_eq!(status_tone(&McpStatus::Running { port: 1 }), Tone::Good);
    assert_eq!(status_tone(&McpStatus::Failed("x".into())), Tone::Bad);
    assert_eq!(status_tone(&McpStatus::Disabled), Tone::Neutral);
}

fn overview_of(set: &PermissionSet) -> (&'static str, String, bool) {
    let o = access_overview(set);
    (o.headline, o.detail, o.writes)
}

#[test]
fn a_set_of_permissions_reads_as_how_much_and_where_not_as_a_capsule_per_area() {
    assert_eq!(
        overview_of(&PermissionSet::none()),
        ("No access", String::new(), false)
    );
    let total = Permission::GROUPS.len();
    assert_eq!(
        overview_of(&PermissionSet::read_only()),
        ("Read only", format!("All {total} areas"), false)
    );
    // A few areas are named; a longer list is counted.
    let some = PermissionSet::of([Permission::ChartsRead, Permission::AlertsRead]);
    assert_eq!(
        overview_of(&some),
        ("Read only", "Charts, Price Alerts".to_owned(), false)
    );
    let many = PermissionSet::of([
        Permission::ChartsRead,
        Permission::DrawingsRead,
        Permission::AlertsRead,
        Permission::MarketsRead,
        Permission::PortfolioRead,
    ]);
    assert_eq!(
        overview_of(&many),
        (
            "Read only",
            "Charts, Drawings, Price Alerts +2 more".to_owned(),
            false
        )
    );
    // Anything that changes DegenView says so, names what it changes, and counts the rest.
    let mixed = PermissionSet::of([
        Permission::ChartsRead,
        Permission::ChartsCreate,
        Permission::AlertsDelete,
        Permission::MarketsRead,
    ]);
    assert_eq!(
        overview_of(&mixed),
        (
            "Read & write",
            "Changes Charts, Price Alerts · reads 1 more area".to_owned(),
            true
        )
    );
    let only_writes = PermissionSet::of([Permission::AlertsDelete]);
    assert_eq!(
        overview_of(&only_writes),
        ("Read & write", "Changes Price Alerts".to_owned(), true)
    );
}

#[test]
fn initials_are_one_clean_letter() {
    assert_eq!(initial("Claude Desktop"), "C");
    assert_eq!(initial("  local automation"), "L");
    assert_eq!(initial("🙂 bot"), "B");
    assert_eq!(initial("   "), "?");
}

fn client(id: &str, name: &str, permissions: PermissionSet) -> McpClient {
    McpClient {
        id: id.into(),
        name: name.into(),
        verifier: String::new(),
        permissions,
        created_at_ms: 1_000,
        last_used_ms: Some(5_000),
        remote_allowed: false,
        allowed_cidrs: Vec::new(),
    }
}

/// Runs `body` for one headless frame; returns the actions and the response rects it recorded.
fn frame(state: &mut McpSettingsState, input: &McpSettingsInput, width: f32) -> Vec<McpAction> {
    let ctx = egui::Context::default();
    let mut actions = Vec::new();
    let raw = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(width, 900.0))),
        ..Default::default()
    };
    // Two frames: the first lays widgets out, the second runs on settled state.
    for _ in 0..2 {
        let mut output = ctx.run_ui(raw.clone(), |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                body(ui, state, input, &mut actions);
            });
        });
        output.textures_delta.clear();
    }
    actions
}

fn sample<'a>(
    status: &'a McpStatus,
    settings: &'a McpSettings,
    clients: &'a [McpClient],
    global: &'a PermissionSet,
) -> McpSettingsInput<'a> {
    McpSettingsInput {
        status,
        settings,
        clients,
        global,
        fresh_token: None,
        load_error: None,
        can_reset: false,
        notice: None,
        activity: &[],
        activity_unreadable: 0,
        now_ms: 10_000,
        utc_offset_secs: 0,
        tls_fingerprint: None,
    }
}

/// Runs frames over a page with a pending "Turn off approvals?" alert, pressing `keys`.
fn alert_after(state: McpSettingsState, keys: &[egui::Key]) -> (Vec<McpAction>, McpSettingsState) {
    let status = McpStatus::Disabled;
    let settings = McpSettings::default();
    let global = PermissionSet::read_only();
    let input = sample(&status, &settings, &[], &global);
    let mut state = state;
    let ctx = egui::Context::default();
    let mut actions = Vec::new();
    let run = |ctx: &egui::Context,
               state: &mut McpSettingsState,
               actions: &mut Vec<McpAction>,
               events: Vec<egui::Event>,
               time: f64| {
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(600.0, 900.0))),
            time: Some(time),
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(raw, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| body(ui, state, &input, actions));
        });
        output.textures_delta.clear();
    };
    run(&ctx, &mut state, &mut actions, Vec::new(), 0.0);
    run(&ctx, &mut state, &mut actions, Vec::new(), 0.1);
    for (index, key) in keys.iter().enumerate() {
        let press = |pressed| egui::Event::Key {
            key: *key,
            physical_key: Some(*key),
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let time = 0.2 + index as f64 * 0.2;
        run(&ctx, &mut state, &mut actions, vec![press(true)], time);
        run(
            &ctx,
            &mut state,
            &mut actions,
            vec![press(false)],
            time + 0.1,
        );
    }
    (actions, state)
}

fn unprotect_alert_after(keys: &[egui::Key]) -> (Vec<McpAction>, bool) {
    let (actions, state) = alert_after(
        McpSettingsState {
            confirm_unprotect: true,
            ..Default::default()
        },
        keys,
    );
    (actions, state.confirm_unprotect)
}

#[test]
fn turning_off_approvals_asks_first_and_only_a_chosen_button_does_it() {
    let (actions, open) = unprotect_alert_after(&[]);
    assert!(actions.is_empty() && open);
    // The dialog starts on Cancel: Return alone cancels, changing nothing, and so does Escape.
    let (actions, open) = unprotect_alert_after(&[egui::Key::Enter]);
    assert!(actions.is_empty() && !open);
    let (actions, open) = unprotect_alert_after(&[egui::Key::Escape]);
    assert!(actions.is_empty() && !open);
    let (actions, open) = unprotect_alert_after(&[egui::Key::Tab, egui::Key::Enter]);
    assert_eq!(actions, [McpAction::SetRequireConfirmation(false)]);
    assert!(!open);
}

#[test]
fn the_page_and_both_sheets_lay_out_without_acting_on_their_own() {
    let status = McpStatus::Running { port: 8765 };
    let settings = McpSettings {
        enabled: true,
        port: 8765,
        ..McpSettings::default()
    };
    let clients = [
        client("a", "Claude Desktop", PermissionSet::read_only()),
        client("b", "Automation", PermissionSet::everything()),
    ];
    let global = PermissionSet::read_only();
    let input = sample(&status, &settings, &clients, &global);

    for width in [360.0, 560.0, 900.0] {
        let mut state = McpSettingsState::default();
        assert!(
            frame(&mut state, &input, width).is_empty(),
            "page at {width}"
        );
        for editing in [Editing::Maximum, Editing::Client("b".into())] {
            state.sheet = Some(editing);
            assert!(
                frame(&mut state, &input, width).is_empty(),
                "sheet at {width}"
            );
        }
        state.sheet = None;
        state.activity.open();
        assert!(
            frame(&mut state, &input, width).is_empty(),
            "activity sheet at {width}"
        );
    }
}

#[test]
fn an_interface_is_one_address_of_this_mac_never_loopback_or_all_of_them() {
    assert_eq!(
        parse_interface(" 192.168.1.20 "),
        Ok(BindMode::Interface("192.168.1.20".parse().unwrap()))
    );
    assert_eq!(
        parse_interface("[fd00::20]"),
        Ok(BindMode::Interface("fd00::20".parse().unwrap()))
    );
    for bad in [
        "",
        "   ",
        "lan",
        "192.168.1",
        "0.0.0.0",
        "::",
        "224.0.0.1",
        "fe80::1",
    ] {
        assert!(
            parse_interface(bad).is_err(),
            "{bad:?} must not be accepted"
        );
    }
    let own = parse_interface("127.0.0.1").unwrap_err();
    assert!(own.contains("own address"), "{own}");
    assert!(
        parse_interface("0.0.0.0")
            .unwrap_err()
            .contains("one interface")
    );
}

#[test]
fn a_detected_address_is_never_loopback_or_unspecified() {
    if let Some(address) = detect_local_address() {
        assert!(
            !address.is_loopback() && !address.is_unspecified(),
            "{address}"
        );
    }
}

#[test]
fn remote_access_lays_out_in_every_state_without_acting_on_its_own() {
    let status = McpStatus::Running { port: 8765 };
    let lan = BindMode::interface("192.168.1.20".parse().unwrap()).unwrap();
    let on = McpSettings {
        enabled: true,
        bind: lan,
        remote_ceiling: PermissionSet::of([Permission::ChartsRead, Permission::AlertsRead]),
        ..McpSettings::default()
    };
    let off = McpSettings {
        enabled: true,
        ..McpSettings::default()
    };
    let mut remote = client("r", "Laptop", PermissionSet::read_only());
    remote.remote_allowed = true;
    remote.allowed_cidrs = vec![
        Cidr::parse("192.168.1.0/24").unwrap(),
        Cidr::parse("100.64.0.0/10").unwrap(),
        Cidr::parse("fd00::/8").unwrap(),
    ];
    let clients = [remote, client("l", "Local", PermissionSet::read_only())];
    let global = PermissionSet::read_only();
    let fingerprint = "AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89";

    for width in [360.0, 560.0, 900.0] {
        // Off, then switched on and waiting for an address, then on with a certificate.
        let input = sample(&status, &off, &clients, &global);
        let mut state = McpSettingsState::default();
        assert!(frame(&mut state, &input, width).is_empty());
        state.remote_editing = true;
        assert!(
            frame(&mut state, &input, width).is_empty(),
            "editing at {width}"
        );
        state.bind_error = Some("`lan` is not an IP address".into());
        assert!(frame(&mut state, &input, width).is_empty());

        let mut input = sample(&status, &on, &clients, &global);
        input.tls_fingerprint = Some(fingerprint);
        let mut state = McpSettingsState::default();
        assert!(frame(&mut state, &input, width).is_empty(), "on at {width}");
        assert_eq!(
            state.bind_text, "192.168.1.20",
            "the field shows the saved address"
        );
        for editing in [Editing::RemoteCeiling, Editing::Client("r".into())] {
            state.sheet = Some(editing);
            assert!(
                frame(&mut state, &input, width).is_empty(),
                "sheet at {width}"
            );
        }
        state.range_error = Some("`x` is not an address range".into());
        assert!(frame(&mut state, &input, width).is_empty());
    }
}

/// Every text one frame of the page paints.
fn painted(input: &McpSettingsInput) -> Vec<String> {
    fn walk(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(text) => out.push(text.galley.text().to_owned()),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| walk(shape, out)),
            _ => {}
        }
    }
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(600.0, 6000.0))),
        ..Default::default()
    };
    let mut state = McpSettingsState::default();
    let mut actions = Vec::new();
    let mut out = Vec::new();
    for _ in 0..2 {
        out.clear();
        let mut output = ctx.run_ui(raw.clone(), |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| body(ui, &mut state, input, &mut actions));
        });
        output.textures_delta.clear();
        for clipped in &output.shapes {
            walk(&clipped.shape, &mut out);
        }
    }
    out
}

#[test]
fn a_remote_credential_is_marked_on_its_row_and_a_local_one_is_not() {
    let status = McpStatus::Disabled;
    let settings = McpSettings::default();
    let global = PermissionSet::read_only();
    let local = [client("l", "Local", PermissionSet::read_only())];
    let input = sample(&status, &settings, &local, &global);
    assert!(!painted(&input).iter().any(|text| text == "Remote"));

    let mut remote = client("r", "Laptop", PermissionSet::read_only());
    remote.remote_allowed = true;
    let clients = [remote, local[0].clone()];
    let input = sample(&status, &settings, &clients, &global);
    let marks = painted(&input)
        .iter()
        .filter(|text| *text == "Remote")
        .count();
    assert_eq!(marks, 1, "only the remote credential carries the mark");
}

#[test]
fn a_row_says_which_limit_holds_a_granted_permission_back() {
    let status = McpStatus::Disabled;
    let settings = McpSettings::default();
    // Granted: read and create. The maximum allows both; the remote ceiling reads only.
    let everything = PermissionSet::everything();
    let mut remote = client(
        "r",
        "Laptop",
        PermissionSet::of([Permission::ChartsRead, Permission::ChartsCreate]),
    );
    remote.remote_allowed = true;
    let mut local = remote.clone();
    local.id = "l".into();
    local.name = "Local".into();
    local.remote_allowed = false;

    let texts = painted(&sample(&status, &settings, &[remote.clone()], &everything));
    assert!(
        texts
            .iter()
            .any(|t| t == "1 granted permission held back by the remote ceiling")
    );
    assert!(!texts.iter().any(|t| t.contains("held back by the maximum")));
    let texts = painted(&sample(&status, &settings, &[local], &everything));
    assert!(
        !texts.iter().any(|t| t.contains("held back")),
        "a local credential has no ceiling"
    );

    // Outside the maximum is the maximum's doing, not the ceiling's.
    let capped = PermissionSet::of([Permission::ChartsRead]);
    let texts = painted(&sample(&status, &settings, &[remote], &capped));
    assert!(
        texts
            .iter()
            .any(|t| t == "1 granted permission held back by the maximum")
    );
    assert!(!texts.iter().any(|t| t.contains("remote ceiling")));
}

#[test]
fn the_page_tells_the_truth_about_where_it_listens() {
    let status = McpStatus::Disabled;
    let global = PermissionSet::read_only();
    let local = McpSettings::default();
    let texts = painted(&sample(&status, &local, &[], &global));
    assert!(texts.iter().any(|t| t == "127.0.0.1"));
    assert!(texts.iter().any(|t| t == "Remote connections"));
    assert!(
        !texts.iter().any(|t| t == "Interface"),
        "nothing to set while it is off"
    );
    assert!(texts.iter().any(|t| t == "http://127.0.0.1:8765/mcp"));
    assert!(
        !texts.iter().any(|t| t == "Certificate"),
        "no certificate on loopback"
    );

    let remote = McpSettings {
        bind: BindMode::interface("192.168.1.20".parse().unwrap()).unwrap(),
        ..McpSettings::default()
    };
    let mut input = sample(&status, &remote, &[], &global);
    input.tls_fingerprint = Some("AB:CD");
    let texts = painted(&input);
    assert!(texts.iter().any(|t| t == "192.168.1.20"));
    assert!(texts.iter().any(|t| t == "Interface"));
    assert!(texts.iter().any(|t| t == "https://192.168.1.20:8765/mcp"));
    assert!(texts.iter().any(|t| t == "Certificate"));
    assert!(
        texts.iter().any(|t| t == "AB:CD"),
        "the fingerprint is shown to be pinned"
    );
}

#[test]
fn a_sheet_for_a_client_that_no_longer_exists_closes_itself() {
    let status = McpStatus::Disabled;
    let settings = McpSettings::default();
    let global = PermissionSet::read_only();
    let input = sample(&status, &settings, &[], &global);
    let mut state = McpSettingsState {
        sheet: Some(Editing::Client("gone".into())),
        confirm_revoke: true,
        ..Default::default()
    };
    assert!(frame(&mut state, &input, 600.0).is_empty());
    assert!(state.sheet.is_none() && !state.confirm_revoke);
}

#[test]
fn an_action_card_is_only_as_tall_as_its_text_and_one_control_row() {
    for width in [360.0, 560.0, 900.0] {
        let ctx = egui::Context::default();
        let mut height = 0.0;
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(width, 1400.0))),
            ..Default::default()
        };
        let mut output = ctx.run_ui(raw, |ui| {
            height = ui
                .scope(|ui| {
                    action_card(ui, Icon::Key, "Title", "A hint of some length.", |ui| {
                        let _ = kit::sheet_button(ui, "Go", true, true);
                    })
                })
                .response
                .rect
                .height();
        });
        output.textures_delta.clear();
        assert!(
            height < 130.0,
            "a card at {width}pt wide is {height}pt tall: it must not stretch to the page"
        );
    }
}

/// Runs `add` in a headless frame `width` wide and returns what it returned with the right
/// edge the frame offered.
fn laid_out<R>(width: f32, add: impl FnOnce(&mut Ui) -> R) -> (R, f32) {
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(width, 1600.0))),
        ..Default::default()
    };
    let (mut out, mut right) = (None, 0.0);
    let mut add = Some(add);
    let mut output = ctx.run_ui(raw, |ui| {
        right = ui.max_rect().right();
        if let Some(add) = add.take() {
            out = Some(add(ui));
        }
    });
    output.textures_delta.clear();
    (out.expect("ran"), right)
}

#[test]
fn the_edit_button_stays_on_the_title_line_however_much_the_maximum_holds() {
    let status = McpStatus::Running { port: 8765 };
    let settings = McpSettings::default();
    let sets = [
        PermissionSet::none(),
        PermissionSet::of([Permission::ChartsRead]),
        PermissionSet::read_only(),
        PermissionSet::everything(),
    ];
    for width in [300.0, 420.0, 560.0, 900.0] {
        let mut button_tops = Vec::new();
        for set in &sets {
            let input = sample(&status, &settings, &[], set);
            let mut state = McpSettingsState::default();
            let (row, right) = laid_out(width, |ui| maximum_row(ui, &mut state, &input));
            let what = format!("{} at {width}pt", access_overview(set).headline);
            assert!(
                (row.edit.center().y - row.title.center().y).abs() < 1.0,
                "Edit is not level with the title, {what}: {:?} vs {:?}",
                row.edit,
                row.title
            );
            assert!(
                row.summary.top() >= row.edit.bottom() - 0.5,
                "the summary starts above the button's line, {what}"
            );
            assert!(
                row.edit.right() > right - 2.0 && row.edit.right() <= right + 0.5,
                "Edit is not flush right, {what}: {} of {right}",
                row.edit.right()
            );
            assert!(
                row.title.right() <= row.edit.left(),
                "title runs under Edit, {what}"
            );
            assert!(
                row.summary.right() <= right + 0.5,
                "the summary overflows the row, {what}: {} of {right}",
                row.summary.right()
            );
            assert!(
                !row.summary.intersects(row.edit),
                "the summary covers Edit, {what}"
            );
            button_tops.push(row.edit.top());
        }
        assert!(
            button_tops
                .windows(2)
                .all(|pair| (pair[0] - pair[1]).abs() < 0.5),
            "the button moves with the size of the maximum at {width}pt: {button_tops:?}"
        );
    }
}

#[test]
fn a_long_summary_wraps_under_its_capsule_and_never_runs_past_its_line() {
    let status = McpStatus::Disabled;
    let settings = McpSettings::default();
    let summary = |set: &PermissionSet, width: f32| {
        let input = sample(&status, &settings, &[], set);
        let mut state = McpSettingsState::default();
        laid_out(width, |ui| maximum_row(ui, &mut state, &input))
            .0
            .summary
    };
    let mixed = PermissionSet::of([
        Permission::ChartsRead,
        Permission::ChartsCreate,
        Permission::AlertsDelete,
        Permission::MarketsRead,
    ]);
    for width in [240.0, 300.0, 560.0] {
        let rect = summary(&mixed, width);
        assert!(rect.width() <= width + 1.0, "{rect:?} at {width}pt");
        assert!(rect.height() < 80.0, "a summary is a line or two: {rect:?}");
    }
    assert!(
        summary(&mixed, 240.0).height() >= summary(&mixed, 560.0).height(),
        "a narrow line wraps, a wide one does not need to"
    );
}

#[test]
fn a_client_with_every_group_keeps_its_capsules_clear_of_the_chevron() {
    let status = McpStatus::Running { port: 8765 };
    let settings = McpSettings::default();
    let everything = PermissionSet::everything();
    let clients = [
        client("a", "Claude Desktop", PermissionSet::everything()),
        client("b", "Read only", PermissionSet::read_only()),
        client("c", "Nothing", PermissionSet::none()),
    ];
    let input = sample(&status, &settings, &clients, &everything);
    for width in [300.0, 420.0, 560.0, 900.0] {
        let (rects, right) = laid_out(width, |ui| {
            clients
                .iter()
                .map(|c| ui.scope(|ui| client_row(ui, c, &input)).response.rect)
                .collect::<Vec<_>>()
        });
        for (rect, c) in rects.iter().zip(&clients) {
            assert!(
                rect.right() <= right + 0.5,
                "{} overflows at {width}pt",
                c.name
            );
            assert!(
                rect.height() < 220.0,
                "{} is {}pt tall at {width}pt",
                c.name,
                rect.height()
            );
        }
        assert!(
            rects[0].height() >= rects[2].height(),
            "more groups should not make a card shorter"
        );
    }
}

#[test]
fn controls_beside_each_other_are_the_same_height() {
    let ctx = egui::Context::default();
    let (mut field_height, mut button_height) = (0.0, 0.0);
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        let mut text = String::from("8765");
        field_height = field_with_rect(ui, Id::new("t"), &mut text, "", 72.0)
            .1
            .height();
        button_height = kit::sheet_button(ui, "Apply", true, true).rect.height();
    });
    output.textures_delta.clear();
    assert_eq!(CONTROL_HEIGHT, 28.0);
    assert_eq!(button_height, CONTROL_HEIGHT);
    assert!(
        (field_height - CONTROL_HEIGHT).abs() < 0.5,
        "the text field ({field_height}) must match the button ({button_height})"
    );
}

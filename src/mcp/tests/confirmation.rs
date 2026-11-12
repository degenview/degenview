//! The confirmation step over real loopback requests: a high-impact call is held with a typed
//! question, runs once after the user approves it, and is refused after a no. The harness's stub
//! app runs the app's own gate, so everything between the socket and the user's answer is real.

use super::{ALL, Harness, call, error_code};
use crate::mcp::confirm::PENDING_TTL;
use crate::model::mcp::{McpAuditKind, McpAuditOutcome};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

const REMOVE: &str = "degenview_remove_chart";

fn remove(url: &str, token: &str) -> Value {
    call(
        url,
        token,
        REMOVE,
        json!({"workspace_id": "1", "chart_id": "2"}),
    )
}

fn question_id(result: &Value) -> u64 {
    result["structuredContent"]["confirmation_id"]
        .as_str()
        .expect("a confirmation_id")
        .parse()
        .expect("a number")
}

fn removed(harness: &Harness) -> usize {
    harness
        .draw_calls()
        .iter()
        .filter(|call| call.as_str() == "remove_chart")
        .count()
}

fn harness_asking() -> Harness {
    let harness = Harness::start(&[("c1", "tok", ALL), ("c2", "other", ALL)]);
    harness.require_confirmation(true);
    harness
}

#[test]
fn a_high_impact_call_is_held_with_a_typed_question_and_changes_nothing() {
    let harness = harness_asking();
    let result = remove(&harness.url(), "tok");
    assert_eq!(error_code(&result), "CONFIRMATION_REQUIRED");
    let content = &result["structuredContent"];
    assert!(content["confirmation_id"].is_string());
    assert!(
        content["expires_in_seconds"]
            .as_u64()
            .is_some_and(|s| s > 100 && s <= 120)
    );
    assert!(content["operation"].as_str().unwrap().contains(REMOVE));
    assert!(
        content["next_step"]
            .as_str()
            .unwrap()
            .contains("repeat this exact call")
    );
    assert!(content["message"].as_str().unwrap().contains("approval"));
    assert_eq!(removed(&harness), 0, "nothing ran");
    // The user sees one question, tied to the client that asked.
    let open = harness.confirmations().pending(Instant::now());
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].client_name, "client c1");
}

#[test]
fn asking_again_before_an_answer_gets_the_same_question_not_a_second_one() {
    let harness = harness_asking();
    let url = harness.url();
    let first = question_id(&remove(&url, "tok"));
    let second = question_id(&remove(&url, "tok"));
    assert_eq!(first, second);
    assert_eq!(harness.confirmations().pending(Instant::now()).len(), 1);
}

#[test]
fn approving_lets_the_same_call_run_once_and_the_next_one_asks_again() {
    let harness = harness_asking();
    let url = harness.url();
    let id = question_id(&remove(&url, "tok"));
    harness.confirmations().approve(Instant::now(), id);

    let done = remove(&url, "tok");
    assert_eq!(done["isError"], json!(false), "{done}");
    assert_eq!(removed(&harness), 1);
    // The approval was used up by that call.
    let again = remove(&url, "tok");
    assert_eq!(error_code(&again), "CONFIRMATION_REQUIRED");
    assert_ne!(question_id(&again), id, "a new question, not the old one");
    assert_eq!(removed(&harness), 1);
}

#[test]
fn a_no_is_final_for_that_call_and_says_so() {
    let harness = harness_asking();
    let url = harness.url();
    let id = question_id(&remove(&url, "tok"));
    harness.confirmations().deny(Instant::now(), id);
    let refused = remove(&url, "tok");
    assert_eq!(error_code(&refused), "CONFIRMATION_DENIED");
    let content = &refused["structuredContent"];
    assert!(
        content["next_step"]
            .as_str()
            .unwrap()
            .contains("Do not repeat")
    );
    assert!(content["confirmation_id"].is_null());
    assert_eq!(removed(&harness), 0);
    assert!(harness.confirmations().pending(Instant::now()).is_empty());
}

#[test]
fn an_approval_covers_only_the_exact_call_and_only_its_client() {
    let harness = harness_asking();
    let url = harness.url();
    let id = question_id(&remove(&url, "tok"));
    harness.confirmations().approve(Instant::now(), id);

    // Another client making the same call is a different question.
    let other = remove(&url, "other");
    assert_eq!(error_code(&other), "CONFIRMATION_REQUIRED");
    assert_ne!(question_id(&other), id);
    // The same client with a different argument is a different operation too.
    let changed = call(
        &url,
        "tok",
        REMOVE,
        json!({"workspace_id": "9", "chart_id": "2"}),
    );
    assert_eq!(error_code(&changed), "CONFIRMATION_REQUIRED");
    assert_eq!(removed(&harness), 0, "neither ran");
    // The approved call is still there for its owner.
    assert_eq!(remove(&url, "tok")["isError"], json!(false));
    assert_eq!(removed(&harness), 1);
}

#[test]
fn a_question_nobody_answers_runs_out_and_the_next_try_asks_afresh() {
    let harness = harness_asking();
    let url = harness.url();
    let id = question_id(&remove(&url, "tok"));
    let late = Instant::now() + PENDING_TTL + Duration::from_secs(1);
    harness.confirmations().tick(late);
    assert!(harness.confirmations().pending(late).is_empty());
    // Answering it now does nothing: it is gone.
    harness.confirmations().approve(late, id);
    let retry = remove(&url, "tok");
    assert_eq!(error_code(&retry), "CONFIRMATION_REQUIRED");
    assert_ne!(question_id(&retry), id);
    assert_eq!(removed(&harness), 0);
}

#[test]
fn only_high_impact_calls_are_asked_about() {
    let harness = harness_asking();
    let url = harness.url();
    for (tool, arguments) in [
        ("degenview_list_workspaces", json!({})),
        ("degenview_list_charts", json!({"workspace_id": "1"})),
        (
            "degenview_add_chart",
            json!({"workspace_id": "1", "source": "binance", "symbol": "ETHUSDT"}),
        ),
    ] {
        let result = call(&url, "tok", tool, arguments);
        assert_eq!(result["isError"], json!(false), "{tool}: {result}");
    }
    assert!(harness.confirmations().pending(Instant::now()).is_empty());
}

#[test]
fn switching_the_step_off_lets_the_call_straight_through() {
    let harness = harness_asking();
    let url = harness.url();
    harness.require_confirmation(false);
    assert_eq!(remove(&url, "tok")["isError"], json!(false));
    assert_eq!(removed(&harness), 1);
    assert!(harness.confirmations().pending(Instant::now()).is_empty());
}

#[test]
fn a_client_cannot_bury_the_user_in_questions() {
    let harness = harness_asking();
    let url = harness.url();
    for chart in 0..crate::mcp::confirm::MAX_PENDING_PER_CLIENT {
        let result = call(
            &url,
            "tok",
            REMOVE,
            json!({"workspace_id": "1", "chart_id": chart.to_string()}),
        );
        assert_eq!(error_code(&result), "CONFIRMATION_REQUIRED");
    }
    let over = call(
        &url,
        "tok",
        REMOVE,
        json!({"workspace_id": "1", "chart_id": "99"}),
    );
    assert_eq!(error_code(&over), "RESOURCE_LIMIT");
    // Another client is not held back by it.
    assert_eq!(
        error_code(&call(
            &url,
            "other",
            REMOVE,
            json!({"workspace_id": "1", "chart_id": "99"})
        )),
        "CONFIRMATION_REQUIRED"
    );
}

#[test]
fn the_activity_log_shows_the_call_that_was_held_the_answer_and_the_call_that_ran() {
    let harness = harness_asking();
    let url = harness.url();
    let id = question_id(&remove(&url, "tok"));
    harness.confirmations().approve(Instant::now(), id);
    assert_eq!(remove(&url, "tok")["isError"], json!(false));

    let deadline = Instant::now() + Duration::from_secs(3);
    let mut rows = Vec::new();
    while rows.len() < 3 && Instant::now() < deadline {
        rows.extend(harness.shared.audit().take(0, false));
        std::thread::sleep(Duration::from_millis(20));
    }
    let summary: Vec<_> = rows
        .iter()
        .map(|row| (row.kind, row.outcome, row.code.as_str(), row.tool.as_str()))
        .collect();
    assert_eq!(
        summary,
        [
            (
                McpAuditKind::Call,
                McpAuditOutcome::Held,
                "CONFIRMATION_REQUIRED",
                REMOVE
            ),
            (
                McpAuditKind::Confirmation,
                McpAuditOutcome::Approved,
                "",
                REMOVE
            ),
            (McpAuditKind::Call, McpAuditOutcome::Succeeded, "", REMOVE),
        ]
    );
}

#[test]
fn every_tool_that_can_ask_says_so_in_its_description() {
    use crate::mcp::confirm::GATED_TOOLS;
    let harness = Harness::start(&[("c1", "tok", ALL)]);
    let reply = super::rpc(&harness.url(), Some("tok"), "tools/list", json!({}));
    let tools = reply.json()["result"]["tools"].clone();
    for name in GATED_TOOLS {
        let tool = tools
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["name"] == name)
            .unwrap_or_else(|| panic!("{name} is not listed"));
        let description = tool["description"].as_str().unwrap();
        assert!(
            description.contains("CONFIRMATION_REQUIRED")
                && description.contains("CONFIRMATION_DENIED"),
            "{name}: {description}"
        );
        assert_eq!(
            tool["annotations"]["destructiveHint"],
            json!(true),
            "{name} should warn the client it is destructive"
        );
    }
    // And no other tool claims to ask.
    for tool in tools.as_array().unwrap() {
        let name = tool["name"].as_str().unwrap();
        if !GATED_TOOLS.contains(&name) {
            let description = tool["description"].as_str().unwrap();
            assert!(
                !description.contains("CONFIRMATION_REQUIRED"),
                "{name} does not ask"
            );
        }
    }
}

#[test]
fn a_dry_run_is_never_asked_about() {
    use crate::mcp::bridge::McpCommand;
    use crate::mcp::confirm::{Gate, gate};
    use crate::model::paper::{PaperInstrument, PaperOrderRequest};
    let instrument: PaperInstrument = serde_json::from_value(json!({
        "key": "BINANCE:BTCUSDT",
        "source": serde_json::to_value(crate::model::DataSource::Binance).unwrap(),
    }))
    .unwrap();
    let request = Box::new(PaperOrderRequest {
        account_id: 1,
        instrument,
        side: crate::model::paper::PaperOrderSide::Buy,
        kind: crate::model::paper::PaperOrderType::Market,
        quantity: 1.0,
        limit_price: None,
        stop_price: None,
        time_in_force: Default::default(),
        take_profit: None,
        stop_loss: None,
    });
    let order = |dry_run| McpCommand::PlacePaperOrder {
        request: request.clone(),
        dry_run,
        request_id: Some("r".into()),
    };
    assert_eq!(gate(&order(true)), Gate::Free);
    assert_eq!(gate(&order(false)), Gate::Ask);
}

mod summaries {
    //! What the user is shown, built from the app's own data about the target.

    use super::super::Harness;
    use crate::host::mcp_confirmation as describe;
    use crate::mcp::bridge::McpError;
    use crate::model::DataSource;
    use crate::model::paper::{
        PaperAccountSettings, PaperCurrency, PaperInstrument, PaperOrderChanges, PaperOrderRequest,
        PaperOrderSide, PaperOrderType, PaperQuote, PaperTimeInForce,
    };
    use crate::model::watchlist::now_ms;

    fn instrument() -> PaperInstrument {
        PaperInstrument::chart("BTCUSDT", "Bitcoin", DataSource::Binance)
    }

    fn request(account_id: u64, kind: PaperOrderType, limit: Option<f64>) -> PaperOrderRequest {
        PaperOrderRequest {
            account_id,
            instrument: instrument(),
            side: PaperOrderSide::Buy,
            kind,
            quantity: 0.5,
            limit_price: limit,
            stop_price: None,
            time_in_force: PaperTimeInForce::GoodTilCanceled,
            take_profit: None,
            stop_loss: None,
        }
    }

    /// An account with an open long and one working limit order.
    fn seeded(harness: &Harness) -> (u64, u64) {
        let mut host = harness.paper();
        let account = host
            .create_account(
                "Main",
                PaperCurrency::Usd,
                100_000.0,
                PaperAccountSettings::default(),
            )
            .unwrap();
        host.process_quote(PaperQuote {
            instrument_key: instrument().key,
            bid: Some(99.0),
            ask: Some(100.0),
            last: Some(99.5),
            timestamp: now_ms(),
            is_market_open: true,
        })
        .unwrap();
        host.submit(request(account, PaperOrderType::Market, None))
            .unwrap();
        let working = host
            .submit(request(account, PaperOrderType::Limit, Some(50.0)))
            .unwrap();
        (account, working)
    }

    #[test]
    fn paper_orders_and_positions_are_described_in_words() {
        let harness = Harness::start(&[("c1", "tok", super::ALL)]);
        let (account, working) = seeded(&harness);
        let host = harness.paper();

        let placed =
            describe::place_order(&host, &request(account, PaperOrderType::Limit, Some(48.5)))
                .unwrap();
        assert_eq!(placed.action, "Place a paper order");
        assert!(placed.target.starts_with("Buy 0.5 "), "{}", placed.target);
        assert!(placed.target.contains("limit 48.5") && placed.target.contains("“Main”"));
        assert_eq!(placed.note.as_deref(), Some("Simulated money."));

        let changed = describe::modify_order(
            &host,
            working,
            &PaperOrderChanges {
                quantity: Some(2.0),
                limit_price: Some(55.0),
                stop_price: None,
            },
        )
        .unwrap();
        assert!(
            changed.target.ends_with("quantity 2, limit 55"),
            "{}",
            changed.target
        );

        let all = describe::cancel_orders(&host, account, None).unwrap();
        assert!(
            all.target.starts_with("all 1 working order on"),
            "{}",
            all.target
        );
        let named = describe::cancel_orders(&host, account, Some(&[working, working + 1])).unwrap();
        assert!(
            named.target.starts_with("2 named orders on"),
            "{}",
            named.target
        );

        let every = describe::close_positions(&host, account, None).unwrap();
        assert!(
            every.target.starts_with("all 1 open position on"),
            "{}",
            every.target
        );
        let one = describe::close_positions(&host, account, Some(&instrument().key)).unwrap();
        assert!(one.target.contains("on “Main”"));
    }

    #[test]
    fn a_target_that_does_not_exist_is_refused_without_a_question() {
        let harness = Harness::start(&[("c1", "tok", super::ALL)]);
        let (account, _) = seeded(&harness);
        let host = harness.paper();
        assert!(matches!(
            describe::place_order(&host, &request(account + 99, PaperOrderType::Market, None)),
            Err(McpError::NotFound(_))
        ));
        assert!(matches!(
            describe::modify_order(&host, 987_654, &PaperOrderChanges::default()),
            Err(McpError::NotFound(_))
        ));
        assert!(matches!(
            describe::close_positions(&host, account, Some("BINANCE:NOPE")),
            Err(McpError::NotFound(_))
        ));
        assert!(matches!(
            describe::cancel_orders(&host, account + 99, None),
            Err(McpError::NotFound(_))
        ));
    }

    #[test]
    fn a_watchlist_is_described_and_favorites_is_refused_unasked() {
        let shared = crate::store::SharedStore::new(std::sync::Mutex::new(
            crate::store::Store::open_in_memory().unwrap(),
        ));
        let mut store = crate::store::watchlist::WatchlistStore::new(shared);
        let id = store.create_list("Swing trades", None).unwrap();
        let summary = describe::delete_watchlist(&store, id).unwrap();
        assert_eq!(summary.action, "Delete a watchlist");
        assert_eq!(summary.target, "“Swing trades” with 0 markets");
        assert!(matches!(
            describe::delete_watchlist(&store, id + 1),
            Err(McpError::NotFound(_))
        ));
        if let Some(favorites) = store.lists().iter().find(|list| list.is_favorites) {
            assert!(matches!(
                describe::delete_watchlist(&store, favorites.id),
                Err(McpError::Conflict(_))
            ));
        }
    }

    #[test]
    fn a_script_summary_names_the_script_and_what_saving_does() {
        let summary = describe::save_script("Apex\nAlgo");
        assert_eq!(
            summary.target, "“Apex Algo”",
            "control characters become spaces"
        );
        assert!(summary.note.unwrap().contains("overwritten"));
    }
}

#[test]
fn a_credential_is_told_the_action_it_asked_for_never_what_the_app_found_out() {
    // It may delete alerts but not read them: the question names the tool, not the alert.
    let harness = Harness::start(&[("w", "tok", &[crate::model::mcp::Permission::AlertsDelete])]);
    harness.require_confirmation(true);
    let result = call(
        &harness.url(),
        "tok",
        "degenview_delete_price_alert",
        json!({"alert_id": "1"}),
    );
    assert_eq!(error_code(&result), "CONFIRMATION_REQUIRED");
    let text = result.to_string();
    assert_eq!(
        result["structuredContent"]["operation"],
        json!("Run degenview_delete_price_alert")
    );
    // The stand-in app describes the target as "the stub's target"; that is the user's to read.
    assert!(!text.contains("stub's target"), "{text}");
    let open = harness.confirmations().pending(Instant::now());
    assert!(open[0].summary.line().contains("the stub's target"));
    // A no is reported the same way.
    harness.confirmations().deny(Instant::now(), open[0].id);
    let refused = call(
        &harness.url(),
        "tok",
        "degenview_delete_price_alert",
        json!({"alert_id": "1"}),
    );
    assert_eq!(error_code(&refused), "CONFIRMATION_DENIED");
    assert!(!refused.to_string().contains("stub's target"));
}

#[test]
fn a_yes_does_not_survive_the_server_being_stopped_and_started() {
    let harness = harness_asking();
    let url = harness.url();
    let id = question_id(&remove(&url, "tok"));
    harness.confirmations().approve(Instant::now(), id);
    // What stopping and starting the server does to the shared state, and the app's next frame.
    harness.shared.bump_generation();
    harness.confirmations().tick(Instant::now());
    let after = remove(&url, "tok");
    assert_eq!(error_code(&after), "CONFIRMATION_REQUIRED", "{after}");
    assert_eq!(removed(&harness), 0, "the old yes did not run it");
    assert_ne!(question_id(&after), id);
}

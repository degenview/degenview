//! End-to-end tests over a real loopback socket, with a stub standing in for the app thread.

use crate::mcp::McpServer;
use crate::mcp::auth::verifier_hex;
use crate::mcp::bridge::{self, McpCommand, McpError, McpReply};
use crate::mcp::state::McpShared;
use crate::mcp::types::{
    AddChartResult, AddIndicatorResult, AlertChangeResult, AlertInfo, AlertsResult,
    AppliedPineDetail, AppliedPineInfo, AppliedPineList, ChartsResult, CreateAlertResult,
    IndicatorsInfo, PineAlertsResult, PineReportResult, PineStylesResult, UpdateAlertResult,
    WorkspaceInfo,
};
use crate::model::InstrumentId;
use crate::model::mcp::{McpClient, Permission, PermissionSet};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// `(workspace, chart, indicator id, EMA period)` as the stub app received it.
type IndicatorCall = (u64, u64, String, Option<usize>);

/// What the stub app holds and records, shared with the tests.
#[derive(Default)]
struct Stub {
    alerts: Mutex<Vec<AlertInfo>>,
    /// Alert commands that reached the app: `create`, `pause`, `resume`, `delete`.
    alert_calls: Mutex<Vec<String>>,
    /// Pine commands that reached the app, by kind.
    pine_calls: Mutex<Vec<String>>,
    /// Scripts the app was asked to apply to a chart.
    applied: Mutex<Vec<crate::mcp::pine::ScriptToApply>>,
    /// `(inputs, visible)` the app was asked to change on an applied script.
    instance_updates: Mutex<Vec<(Option<crate::mcp::pine::InputArgs>, Option<bool>)>>,
}

pub struct Harness {
    /// Where this harness's scripts live; removed on drop.
    pub scripts_root: std::path::PathBuf,
    pub shared: Arc<McpShared>,
    pub port: u16,
    pub added: Arc<Mutex<Vec<(u64, InstrumentId)>>>,
    pub indicators: Arc<Mutex<Vec<IndicatorCall>>>,
    stub: Arc<Stub>,
    server: Option<McpServer>,
    stop_app: Arc<AtomicBool>,
    app: Option<JoinHandle<()>>,
}

fn client(id: &str, token: &str, permissions: &[Permission]) -> McpClient {
    McpClient {
        id: id.into(),
        name: format!("client {id}"),
        verifier: verifier_hex(token),
        permissions: PermissionSet::of(permissions.iter().copied()),
        created_at_ms: 0,
        last_used_ms: None,
    }
}

impl Harness {
    /// `clients`: `(id, token, grants)`. The global cap allows everything unless changed later.
    pub fn start(clients: &[(&str, &str, &[Permission])]) -> Self {
        let shared = McpShared::new();
        shared.replace(
            clients
                .iter()
                .map(|(id, token, perms)| client(id, token, perms))
                .collect(),
            PermissionSet::of(Permission::ALL),
        );
        let scripts_root = std::env::temp_dir().join(format!(
            "degenview-mcp-e2e-{}",
            crate::model::watchlist::new_id()
        ));
        let locator_root = scripts_root.clone();
        shared.set_script_locator(Box::new(move || Some(script_store_at(&locator_root))));
        let (bridge, inbox) = bridge::channel(shared.clone(), Arc::new(|| {}));
        let server = McpServer::start(shared.clone(), bridge, 0).expect("server starts");
        let port = match shared.status() {
            crate::mcp::state::McpStatus::Running { port } => port,
            other => panic!("not running: {other:?}"),
        };
        let stub = Arc::new(Stub::default());
        let added = Arc::new(Mutex::new(Vec::new()));
        let indicators = Arc::new(Mutex::new(Vec::new()));
        let stop_app = Arc::new(AtomicBool::new(false));
        let app = {
            let shared = shared.clone();
            let added = added.clone();
            let indicators = indicators.clone();
            let stub = stub.clone();
            let root = scripts_root.clone();
            let stop = stop_app.clone();
            std::thread::spawn(move || {
                while !stop.load(Ordering::Acquire) {
                    for request in inbox.take(8) {
                        let added = added.clone();
                        let indicators = indicators.clone();
                        let stub = stub.clone();
                        let root = root.clone();
                        request.run(&shared, move |command| {
                            stub_app(command, &added, &indicators, &stub, &root)
                        });
                    }
                    std::thread::sleep(Duration::from_millis(2));
                }
            })
        };
        Self {
            scripts_root,
            shared,
            port,
            added,
            indicators,
            stub,
            server: Some(server),
            stop_app,
            app: Some(app),
        }
    }

    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/mcp", self.port)
    }

    pub fn stop_server(&mut self) {
        if let Some(mut server) = self.server.take() {
            server.stop();
        }
    }
}

/// The script library a harness serves, so tests never read the user's own scripts.
pub fn script_store_at(root: &std::path::Path) -> crate::script_manager::store::ScriptStore {
    crate::script_manager::store::ScriptStore::new(
        root.join("Scripts"),
        root.join("ScriptMetadata"),
    )
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.scripts_root);
        self.stop_server();
        self.stop_app.store(true, Ordering::Release);
        if let Some(app) = self.app.take() {
            let _ = app.join();
        }
    }
}

fn instance_changed(status: &str, instance_id: &str) -> McpReply {
    McpReply::PineInstanceChanged(Box::new(crate::mcp::types::PineInstanceChangeResult {
        status: status.into(),
        instance_id: instance_id.into(),
        chart: AppliedPineList {
            workspace_id: "1".into(),
            chart_id: "2".into(),
            instrument: "BINANCE:BTCUSDT".into(),
            timeframe: "1D".into(),
            instances: Vec::new(),
        },
    }))
}

fn stub_app(
    command: McpCommand,
    added: &Mutex<Vec<(u64, InstrumentId)>>,
    indicators: &Mutex<Vec<IndicatorCall>>,
    stub: &Stub,
    scripts_root: &std::path::Path,
) -> Result<McpReply, McpError> {
    match command {
        McpCommand::ListWorkspaces => Ok(McpReply::Workspaces(vec![WorkspaceInfo {
            workspace_id: "1".into(),
            title: "Unnamed".into(),
            chart_count: 0,
            active: true,
            timeframe: "1D".into(),
        }])),
        McpCommand::ListCharts { workspace_id: 1 } => Ok(McpReply::Charts(ChartsResult {
            workspace_id: "1".into(),
            charts: Vec::new(),
        })),
        McpCommand::ListCharts { .. } => Err(McpError::NotFound("No such workspace".into())),
        McpCommand::ListAlerts { only } => {
            let alerts = stub.alerts.lock().unwrap();
            let matching: Vec<_> = alerts
                .iter()
                .filter(|alert| only.is_none_or(|id| alert.alert_id == id.to_string()))
                .cloned()
                .collect();
            Ok(McpReply::Alerts(AlertsResult {
                total: matching.len(),
                truncated: false,
                alerts: matching,
            }))
        }
        McpCommand::CreateAlert(request) => {
            stub.alert_calls.lock().unwrap().push("create".into());
            let mut alerts = stub.alerts.lock().unwrap();
            let condition = if request.crosses_above {
                "crosses_above"
            } else {
                "crosses_below"
            };
            if let Some(existing) = alerts.iter().find(|alert| {
                alert.instrument == request.instrument.key()
                    && alert.condition == condition
                    && alert.target_price.as_deref() == Some(request.target_text.as_str())
            }) {
                return Ok(McpReply::AlertCreated(CreateAlertResult {
                    status: "already_exists".into(),
                    alert: existing.clone(),
                }));
            }
            let alert = AlertInfo {
                alert_id: (1000 + alerts.len()).to_string(),
                instrument: request.instrument.key(),
                source: "binance".into(),
                symbol: request.instrument.symbol.clone(),
                condition: condition.into(),
                target_price: Some(request.target_text.clone()),
                percent: None,
                reference_price: None,
                currency: request.currency.code().into(),
                frequency: "once".into(),
                status: "active".into(),
                note: request.note.clone(),
                sends_to_webhooks: false,
                created_at: "2026-10-12T10:00:00Z".into(),
                updated_at: "2026-10-12T10:00:00Z".into(),
                last_triggered_at: None,
            };
            alerts.push(alert.clone());
            Ok(McpReply::AlertCreated(CreateAlertResult {
                status: "queued".into(),
                alert,
            }))
        }
        McpCommand::ListChartPine { .. }
        | McpCommand::GetChartPine { .. }
        | McpCommand::GetPineStyles { .. }
        | McpCommand::GetPineReport { .. } => {
            let (workspace_id, chart_id, instance_id, kind) = match &command {
                McpCommand::ListChartPine {
                    workspace_id,
                    chart_id,
                } => (*workspace_id, *chart_id, None, "list"),
                McpCommand::GetChartPine {
                    workspace_id,
                    chart_id,
                    instance_id,
                } => (*workspace_id, *chart_id, Some(*instance_id), "detail"),
                McpCommand::GetPineStyles {
                    workspace_id,
                    chart_id,
                    instance_id,
                } => (*workspace_id, *chart_id, Some(*instance_id), "styles"),
                McpCommand::GetPineReport {
                    workspace_id,
                    chart_id,
                    instance_id,
                    ..
                } => (*workspace_id, *chart_id, Some(*instance_id), "report"),
                _ => unreachable!(),
            };
            stub.pine_calls.lock().unwrap().push(kind.into());
            if workspace_id != 1 || chart_id != 2 || instance_id.is_some_and(|id| id != 1) {
                return Err(McpError::NotFound("No such chart or instance".into()));
            }
            let info = AppliedPineInfo {
                instance_id: "1".into(),
                script_id: None,
                script_name: "Strat".into(),
                visible: true,
                source_hash: "h".repeat(64),
                run_status: "ok".into(),
                error: None,
                is_strategy: Some(true),
            };
            Ok(match command {
                McpCommand::ListChartPine { .. } => McpReply::ChartPine(AppliedPineList {
                    workspace_id: "1".into(),
                    chart_id: "2".into(),
                    instrument: "BINANCE:BTCUSDT".into(),
                    timeframe: "1D".into(),
                    instances: vec![info],
                }),
                McpCommand::GetChartPine { .. } => McpReply::ChartPineDetail(AppliedPineDetail {
                    workspace_id: "1".into(),
                    chart_id: "2".into(),
                    instrument: "BINANCE:BTCUSDT".into(),
                    timeframe: "1D".into(),
                    instance: info,
                    inputs: Default::default(),
                    style_overrides: Default::default(),
                    bar_count: Some(500),
                    script_status: String::new(),
                }),
                McpCommand::GetPineStyles { .. } => McpReply::PineStyles(PineStylesResult {
                    workspace_id: "1".into(),
                    chart_id: "2".into(),
                    instance_id: "1".into(),
                    available: false,
                    reason: Some("not run".into()),
                    rows: Vec::new(),
                }),
                McpCommand::GetPineReport { offset, .. } => {
                    McpReply::PineReport(Box::new(PineReportResult {
                        workspace_id: "1".into(),
                        chart_id: "2".into(),
                        instance_id: "1".into(),
                        available: false,
                        reason: Some("No report yet".into()),
                        context: None,
                        metrics: None,
                        settings: None,
                        trades: Vec::new(),
                        trades_total: 0,
                        trades_offset: offset,
                        open_trades: Vec::new(),
                        equity_curve: Vec::new(),
                        equity_points_total: 0,
                    }))
                }
                _ => unreachable!(),
            })
        }
        McpCommand::EditPineScript {
            script_id,
            expected_revision,
            source,
        } => {
            stub.pine_calls.lock().unwrap().push("edit".into());
            let mut store = script_store_at(scripts_root);
            // The same functions the app runs, with no Script Manager editor open.
            let result = match source {
                Some(source) => crate::mcp::pine::update_draft(
                    &mut store,
                    &script_id,
                    &expected_revision,
                    &source,
                    None,
                ),
                None => {
                    crate::mcp::pine::save_script(&mut store, &script_id, &expected_revision, None)
                }
            }?;
            Ok(McpReply::PineEdit(Box::new(result)))
        }
        McpCommand::AddPineInstance { script, .. } => {
            stub.pine_calls.lock().unwrap().push("add".into());
            stub.applied.lock().unwrap().push(*script);
            Ok(instance_changed("added", "7"))
        }
        McpCommand::UpdatePineInstance {
            instance_id,
            inputs,
            visible,
            ..
        } => {
            stub.pine_calls.lock().unwrap().push("update".into());
            stub.instance_updates
                .lock()
                .unwrap()
                .push((inputs, visible));
            Ok(instance_changed("updated", &instance_id.to_string()))
        }
        McpCommand::RemovePineInstance { instance_id, .. } => {
            stub.pine_calls.lock().unwrap().push("remove".into());
            Ok(instance_changed("removed", &instance_id.to_string()))
        }
        McpCommand::ListPineAlerts => {
            stub.pine_calls.lock().unwrap().push("alerts".into());
            Ok(McpReply::PineAlerts(PineAlertsResult {
                subscriptions: Vec::new(),
                total: 0,
                event_history_available: false,
                recent_events: Vec::new(),
            }))
        }
        McpCommand::UpdateAlert(request) => {
            let mut alerts = stub.alerts.lock().unwrap();
            let alert = alerts
                .iter_mut()
                .find(|alert| alert.alert_id == request.alert_id.to_string())
                .ok_or_else(|| McpError::NotFound("No such alert".into()))?;
            let before = alert.clone();
            if let Some(above) = request.crosses_above {
                alert.condition = if above {
                    "crosses_above"
                } else {
                    "crosses_below"
                }
                .into();
            }
            if let Some(target) = &request.target_text {
                alert.target_price = Some(target.clone());
            }
            if let Some(currency) = request.currency {
                alert.currency = currency.code().into();
            }
            if let Some(frequency) = request.frequency {
                alert.frequency = match frequency {
                    crate::model::alert::AlertFrequency::Once => "once",
                    crate::model::alert::AlertFrequency::EveryTime => "every_time",
                }
                .into();
            }
            if let Some(note) = &request.note {
                alert.note = note.clone();
            }
            let status = if *alert == before {
                "unchanged"
            } else {
                stub.alert_calls.lock().unwrap().push("update".into());
                "queued"
            };
            Ok(McpReply::AlertUpdated(UpdateAlertResult {
                status: status.into(),
                alert: alert.clone(),
            }))
        }
        McpCommand::SetAlertPaused { alert_id, paused } => {
            let mut alerts = stub.alerts.lock().unwrap();
            let alert = alerts
                .iter_mut()
                .find(|alert| alert.alert_id == alert_id.to_string())
                .ok_or_else(|| McpError::NotFound("No such alert".into()))?;
            let (wanted, already) = if paused {
                ("paused", "already_paused")
            } else {
                ("active", "already_active")
            };
            let status = if alert.status == wanted {
                already
            } else {
                stub.alert_calls
                    .lock()
                    .unwrap()
                    .push(if paused { "pause" } else { "resume" }.into());
                alert.status = wanted.into();
                "queued"
            };
            Ok(McpReply::AlertChanged(AlertChangeResult {
                alert_id: alert_id.to_string(),
                status: status.into(),
            }))
        }
        McpCommand::DeleteAlert { alert_id } => {
            let mut alerts = stub.alerts.lock().unwrap();
            let before = alerts.len();
            alerts.retain(|alert| alert.alert_id != alert_id.to_string());
            if alerts.len() == before {
                return Err(McpError::NotFound("No such alert".into()));
            }
            stub.alert_calls.lock().unwrap().push("delete".into());
            Ok(McpReply::AlertChanged(AlertChangeResult {
                alert_id: alert_id.to_string(),
                status: "queued".into(),
            }))
        }
        McpCommand::ApplyIndicator {
            workspace_id,
            chart_id,
            indicator,
            ema_period,
        } => {
            if workspace_id != 1 || chart_id != 2 {
                return Err(McpError::NotFound("No such chart".into()));
            }
            indicators.lock().unwrap().push((
                workspace_id,
                chart_id,
                indicator.id().to_owned(),
                ema_period,
            ));
            Ok(McpReply::IndicatorApplied(AddIndicatorResult {
                workspace_id: "1".into(),
                chart_id: "2".into(),
                indicator: indicator.id().to_owned(),
                status: "added".into(),
                indicators: IndicatorsInfo {
                    volume: false,
                    rsi: false,
                    ema: true,
                    ema_period: ema_period.unwrap_or(20),
                    bollinger: false,
                    trend_flips: false,
                },
            }))
        }
        McpCommand::AddChart {
            workspace_id,
            instrument,
        } => {
            if workspace_id != 1 {
                return Err(McpError::NotFound("No such workspace".into()));
            }
            added
                .lock()
                .unwrap()
                .push((workspace_id, instrument.clone()));
            Ok(McpReply::ChartAdded(AddChartResult {
                workspace_id: "1".into(),
                chart_id: "2".into(),
                instrument: instrument.key(),
                source: "binance".into(),
                symbol: instrument.symbol.clone(),
                display_symbol: instrument.symbol.clone(),
                timeframe: "1D".into(),
                status: "created".into(),
            }))
        }
    }
}

pub struct Reply {
    pub status: u16,
    pub body: String,
}

impl Reply {
    pub fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or_else(|_| {
            // An SSE-framed answer carries the JSON on its `data:` line.
            self.body
                .lines()
                .find_map(|line| line.strip_prefix("data:"))
                .and_then(|data| serde_json::from_str(data.trim()).ok())
                .unwrap_or(Value::Null)
        })
    }
}

pub fn post(url: &str, token: Option<&str>, headers: &[(&str, &str)], body: &str) -> Reply {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(10)))
        .build()
        .into();
    let mut request = agent
        .post(url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream");
    if let Some(token) = token {
        request = request.header("Authorization", &format!("Bearer {token}"));
    }
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    match request.send(body) {
        Ok(mut response) => Reply {
            status: response.status().as_u16(),
            body: response.body_mut().read_to_string().unwrap_or_default(),
        },
        Err(error) => Reply {
            status: 0,
            body: error.to_string(),
        },
    }
}

const READ: &[Permission] = &[Permission::ChartsRead];
const BOTH: &[Permission] = &[Permission::ChartsRead, Permission::ChartsCreate];
const ALL: &[Permission] = &Permission::ALL;
const PROTOCOL: (&str, &str) = ("MCP-Protocol-Version", "2025-06-18");

fn rpc(url: &str, token: Option<&str>, method: &str, params: Value) -> Reply {
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
    post(url, token, &[PROTOCOL], &body.to_string())
}

fn call(url: &str, token: &str, name: &str, arguments: Value) -> Value {
    let reply = rpc(
        url,
        Some(token),
        "tools/call",
        json!({"name": name, "arguments": arguments}),
    );
    assert_eq!(reply.status, 200, "{}", reply.body);
    reply.json()["result"].clone()
}

fn tool_names(url: &str, token: &str) -> Vec<String> {
    let reply = rpc(url, Some(token), "tools/list", json!({}));
    assert_eq!(reply.status, 200, "{}", reply.body);
    let mut names: Vec<String> = reply.json()["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_owned())
        .collect();
    names.sort();
    names
}

fn error_code(result: &Value) -> String {
    assert_eq!(result["isError"], json!(true), "{result}");
    result["structuredContent"]["error"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

fn raw_request(port: u16, head: &str, body: &str) -> String {
    use std::io::{Read, Write};
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let request = format!(
        "POST /mcp HTTP/1.1\r\n{head}Content-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).unwrap();
    let mut response = String::new();
    let _ = stream.read_to_string(&mut response);
    response
}

fn status_line(response: &str) -> &str {
    response.lines().next().unwrap_or_default()
}

const INITIALIZE: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#;

#[test]
fn every_request_needs_a_valid_token() {
    let harness = Harness::start(&[("c1", "good-token", BOTH)]);
    let url = harness.url();
    let init = json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1"}});

    for token in [
        None,
        Some(""),
        Some("wrong"),
        Some("good-toke"),
        Some("good-token2"),
    ] {
        let reply = rpc(&url, token, "initialize", init.clone());
        assert_eq!(reply.status, 401, "token {token:?}");
        assert!(!reply.body.contains("good-token"));
    }
    // Wrong scheme, and a token in the URL instead of the header, are not accepted.
    let body = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":init}).to_string();
    let basic = post(&url, None, &[("Authorization", "Basic good-token")], &body);
    assert_eq!(basic.status, 401);
    let query = post(
        &format!("{url}?token=good-token&access_token=good-token"),
        None,
        &[],
        &body,
    );
    assert_eq!(query.status, 401);
    // Not even an unknown path answers without a token.
    let other = post(
        &format!("http://127.0.0.1:{}/other", harness.port),
        None,
        &[],
        &body,
    );
    assert_eq!(other.status, 401);
    let reply = post(
        &format!("http://127.0.0.1:{}/other", harness.port),
        Some("good-token"),
        &[],
        &body,
    );
    assert_eq!(reply.status, 404);

    assert_eq!(
        rpc(&url, Some("good-token"), "initialize", init).status,
        200
    );
}

#[test]
fn discovery_and_calls_work_for_a_permitted_client() {
    let harness = Harness::start(&[("c1", "tok", BOTH)]);
    let url = harness.url();
    assert_eq!(
        tool_names(&url, "tok"),
        [
            "degenview_add_chart",
            "degenview_list_charts",
            "degenview_list_indicators",
            "degenview_list_workspaces"
        ]
    );

    let workspaces = call(&url, "tok", "degenview_list_workspaces", json!({}));
    assert_eq!(workspaces["isError"], json!(false));
    assert_eq!(
        workspaces["structuredContent"]["workspaces"][0]["workspace_id"],
        "1"
    );

    let charts = call(
        &url,
        "tok",
        "degenview_list_charts",
        json!({"workspace_id": "1"}),
    );
    assert_eq!(charts["structuredContent"]["workspace_id"], "1");
    let missing = call(
        &url,
        "tok",
        "degenview_list_charts",
        json!({"workspace_id": "99"}),
    );
    assert_eq!(error_code(&missing), "NOT_FOUND");

    let added = call(
        &url,
        "tok",
        "degenview_add_chart",
        json!({"workspace_id": "1", "source": "binance", "symbol": " btcusdt "}),
    );
    assert_eq!(added["isError"], json!(false), "{added}");
    assert_eq!(added["structuredContent"]["chart_id"], "2");
    // The app side received the validated, normalised market.
    let seen = harness.added.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].0, 1);
    assert_eq!(seen[0].1.key(), "BINANCE:BTCUSDT");
}

#[test]
fn bad_arguments_never_reach_the_app() {
    let harness = Harness::start(&[("c1", "tok", BOTH)]);
    let url = harness.url();
    let cases = [
        json!({"workspace_id": "1", "source": "binance", "symbol": "BTC-USD"}),
        json!({"workspace_id": "1", "source": "binance", "symbol": ""}),
        json!({"workspace_id": "1", "source": "binance", "symbol": "BTCUSDT|title"}),
        json!({"workspace_id": "one", "source": "binance", "symbol": "BTCUSDT"}),
        json!({"workspace_id": "-1", "source": "binance", "symbol": "BTCUSDT"}),
        json!({"workspace_id": "1", "source": "polymarket", "symbol": "0xabc"}),
        json!({"workspace_id": "1", "source": "binance", "symbol": "BTCUSDT", "timeframe": "1h"}),
        json!({"workspace_id": "1", "symbol": "BTCUSDT"}),
        json!({"workspace_id": "1", "source": "binance", "symbol": "B".repeat(500)}),
    ];
    for arguments in cases {
        let reply = rpc(
            &url,
            Some("tok"),
            "tools/call",
            json!({"name": "degenview_add_chart", "arguments": arguments}),
        );
        let json = reply.json();
        let rejected = json.get("error").is_some() || json["result"]["isError"] == json!(true);
        assert!(rejected, "accepted {arguments}: {}", reply.body);
    }
    assert!(harness.added.lock().unwrap().is_empty());
}

#[test]
fn permissions_are_enforced_on_every_call_not_just_discovery() {
    let harness = Harness::start(&[
        ("reader", "read-tok", READ),
        ("writer", "write-tok", BOTH),
        ("none", "none-tok", &[]),
    ]);
    let url = harness.url();

    assert_eq!(
        tool_names(&url, "read-tok"),
        [
            "degenview_list_charts",
            "degenview_list_indicators",
            "degenview_list_workspaces"
        ]
    );
    assert!(tool_names(&url, "none-tok").is_empty());

    // Guessing the name of a hidden tool is denied, and nothing runs.
    let add = json!({"workspace_id": "1", "source": "binance", "symbol": "BTCUSDT"});
    let denied = call(&url, "read-tok", "degenview_add_chart", add.clone());
    assert_eq!(error_code(&denied), "PERMISSION_DENIED");
    let denied = call(&url, "none-tok", "degenview_list_workspaces", json!({}));
    assert_eq!(error_code(&denied), "PERMISSION_DENIED");
    assert!(harness.added.lock().unwrap().is_empty());

    // Two clients with different rights, at the same time.
    let handles: Vec<_> = [
        ("read-tok", false),
        ("write-tok", true),
        ("read-tok", false),
        ("write-tok", true),
    ]
    .into_iter()
    .map(|(token, may_add)| {
        let url = url.clone();
        let add = add.clone();
        std::thread::spawn(move || {
            let result = call(&url, token, "degenview_add_chart", add);
            (may_add, result["isError"] == json!(false))
        })
    })
    .collect();
    for handle in handles {
        let (may_add, added) = handle.join().unwrap();
        assert_eq!(may_add, added);
    }

    // The global cap overrides a client's own grant, immediately.
    harness.shared.replace(
        harness.shared.clients(),
        PermissionSet::of([Permission::ChartsRead]),
    );
    let denied = call(&url, "write-tok", "degenview_add_chart", add.clone());
    assert_eq!(error_code(&denied), "PERMISSION_DENIED");
    assert_eq!(
        tool_names(&url, "write-tok"),
        [
            "degenview_list_charts",
            "degenview_list_indicators",
            "degenview_list_workspaces"
        ]
    );
}

#[test]
fn revoking_a_credential_takes_effect_on_the_next_request() {
    let harness = Harness::start(&[("c1", "tok", BOTH), ("c2", "other", BOTH)]);
    let url = harness.url();
    assert_eq!(rpc(&url, Some("tok"), "tools/list", json!({})).status, 200);

    let remaining: Vec<_> = harness
        .shared
        .clients()
        .into_iter()
        .filter(|c| c.id != "c1")
        .collect();
    harness.shared.replace(remaining, harness.shared.global());
    assert_eq!(rpc(&url, Some("tok"), "tools/list", json!({})).status, 401);
    assert_eq!(
        rpc(&url, Some("other"), "tools/list", json!({})).status,
        200
    );
}

#[test]
fn foreign_hosts_and_browser_origins_are_rejected() {
    let harness = Harness::start(&[("c1", "tok", BOTH)]);
    let port = harness.port;
    let auth = "Authorization: Bearer tok\r\n";
    let good_host = format!("Host: 127.0.0.1:{port}\r\n");

    let ok = raw_request(port, &format!("{auth}{good_host}"), INITIALIZE);
    assert!(status_line(&ok).contains("200"), "{ok}");
    let localhost = raw_request(
        port,
        &format!("{auth}Host: localhost:{port}\r\n"),
        INITIALIZE,
    );
    assert!(status_line(&localhost).contains("200"), "{localhost}");

    // DNS rebinding: the attacker's name resolves to 127.0.0.1 but the Host header gives it away.
    for host in [
        "Host: evil.example\r\n".to_owned(),
        format!("Host: evil.example:{port}\r\n"),
        "Host: 127.0.0.1:1\r\n".to_owned(),
    ] {
        let response = raw_request(port, &format!("{auth}{host}"), INITIALIZE);
        assert!(
            !status_line(&response).contains("200"),
            "{host}: {response}"
        );
    }
    // A request from a web page carries an Origin; a native MCP client does not.
    for origin in [
        "https://evil.example",
        "http://127.0.0.1",
        &format!("http://localhost:{port}"),
        "null",
    ] {
        let response = raw_request(
            port,
            &format!("{auth}{good_host}Origin: {origin}\r\n"),
            INITIALIZE,
        );
        assert!(
            !status_line(&response).contains("200"),
            "{origin}: {response}"
        );
    }
}

#[test]
fn oversized_bodies_are_refused() {
    let harness = Harness::start(&[("c1", "tok", BOTH)]);
    let padding = "x".repeat(crate::mcp::transport::MAX_BODY_BYTES + 1024);
    let body = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"degenview_list_workspaces","arguments":{"pad": padding}}});
    let reply = post(&harness.url(), Some("tok"), &[PROTOCOL], &body.to_string());
    assert_eq!(reply.status, 413, "{}", reply.body);
    assert_eq!(
        rpc(&harness.url(), Some("tok"), "tools/list", json!({})).status,
        200
    );
}

#[test]
fn listens_on_loopback_only() {
    let harness = Harness::start(&[("c1", "tok", BOTH)]);
    // Binding the same port on a second address collides only if ours holds a wildcard. Which
    // second address works depends on the OS: Linux lets 127.0.0.2 sit beside a 127.0.0.1
    // listener but refuses 0.0.0.0 whenever anything holds the port; macOS has no 127.0.0.2 by
    // default but lets 0.0.0.0 sit beside a loopback listener.
    match std::net::TcpListener::bind(("127.0.0.2", harness.port)) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {
            panic!("the server must not hold the wildcard address: {error}")
        }
        Err(_) => {
            let wildcard = std::net::TcpListener::bind(("0.0.0.0", harness.port));
            assert!(
                wildcard.is_ok(),
                "the server must not hold the wildcard address"
            );
        }
    }
    // And a non-loopback address of this machine is not served.
    let lan = std::net::UdpSocket::bind("0.0.0.0:0")
        .and_then(|socket| socket.connect("192.0.2.1:9").map(|()| socket))
        .and_then(|socket| socket.local_addr());
    if let Ok(address) = lan
        && !address.ip().is_loopback()
        && !address.ip().is_unspecified()
    {
        let refused = std::net::TcpStream::connect_timeout(
            &std::net::SocketAddr::new(address.ip(), harness.port),
            Duration::from_secs(2),
        );
        assert!(refused.is_err(), "reachable on {address}");
    }
}

#[test]
fn stopping_closes_the_port_at_once_and_it_can_be_reused() {
    let mut harness = Harness::start(&[("c1", "tok", BOTH)]);
    let port = harness.port;
    assert_eq!(
        rpc(&harness.url(), Some("tok"), "tools/list", json!({})).status,
        200
    );
    harness.stop_server();
    assert_eq!(
        harness.shared.status(),
        crate::mcp::state::McpStatus::Disabled
    );
    assert_eq!(
        rpc(
            &format!("http://127.0.0.1:{port}/mcp"),
            Some("tok"),
            "tools/list",
            json!({})
        )
        .status,
        0
    );
    assert!(std::net::TcpListener::bind(("127.0.0.1", port)).is_ok());
}

#[test]
fn a_port_conflict_is_reported_not_swallowed() {
    let blocker = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = blocker.local_addr().unwrap().port();
    let shared = McpShared::new();
    let (bridge, _inbox) = bridge::channel(shared.clone(), Arc::new(|| {}));
    let error = McpServer::start(shared.clone(), bridge, port)
        .err()
        .expect("must fail");
    assert!(error.contains("already in use"), "{error}");
    assert!(
        matches!(shared.status(), crate::mcp::state::McpStatus::Failed(message) if message.contains("already in use"))
    );
}

#[test]
fn indicator_catalog_is_readable_with_charts_read() {
    let harness = Harness::start(&[("c1", "tok", READ)]);
    let result = call(
        &harness.url(),
        "tok",
        "degenview_list_indicators",
        json!({}),
    );
    assert_eq!(result["isError"], json!(false), "{result}");
    let indicators = result["structuredContent"]["indicators"]
        .as_array()
        .unwrap();
    let ids: Vec<_> = indicators
        .iter()
        .map(|i| i["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["volume", "rsi", "ema", "bollinger", "trend_flips"]);
    let ema = indicators.iter().find(|i| i["id"] == "ema").unwrap();
    assert_eq!(
        ema["parameters"][0]["allowed_values"],
        json!([9, 20, 50, 100, 200])
    );
}

#[test]
fn adding_an_indicator_needs_its_own_permission() {
    let harness = Harness::start(&[("rw", "rw-tok", BOTH), ("all", "all-tok", ALL)]);
    let url = harness.url();
    let args =
        json!({"workspace_id": "1", "chart_id": "2", "indicator": "ema", "inputs": {"period": 50}});

    // charts.create is not enough, and the tool is not even offered.
    assert!(!tool_names(&url, "rw-tok").contains(&"degenview_add_indicator".to_owned()));
    let denied = call(&url, "rw-tok", "degenview_add_indicator", args.clone());
    assert_eq!(error_code(&denied), "PERMISSION_DENIED");
    assert!(harness.indicators.lock().unwrap().is_empty());

    assert!(tool_names(&url, "all-tok").contains(&"degenview_add_indicator".to_owned()));
    let result = call(&url, "all-tok", "degenview_add_indicator", args);
    assert_eq!(result["isError"], json!(false), "{result}");
    assert_eq!(result["structuredContent"]["indicators"]["ema_period"], 50);
    assert_eq!(
        harness.indicators.lock().unwrap().as_slice(),
        [(1, 2, "ema".to_owned(), Some(50))]
    );

    // The global cap wins over the client's own grant.
    harness.shared.replace(
        harness.shared.clients(),
        PermissionSet::of([Permission::ChartsRead]),
    );
    let denied = call(
        &url,
        "all-tok",
        "degenview_add_indicator",
        json!({"workspace_id": "1", "chart_id": "2", "indicator": "rsi"}),
    );
    assert_eq!(error_code(&denied), "PERMISSION_DENIED");
    assert_eq!(harness.indicators.lock().unwrap().len(), 1);
}

#[test]
fn bad_indicator_arguments_never_reach_the_app() {
    let harness = Harness::start(&[("c1", "tok", ALL)]);
    let url = harness.url();
    let cases = [
        json!({"workspace_id": "1", "chart_id": "2", "indicator": "ema", "inputs": {"period": 21}}),
        json!({"workspace_id": "1", "chart_id": "2", "indicator": "ema", "inputs": {"period": 0}}),
        json!({"workspace_id": "1", "chart_id": "2", "indicator": "ema", "inputs": {"period": -5}}),
        json!({"workspace_id": "1", "chart_id": "2", "indicator": "rsi", "inputs": {"period": 14}}),
        json!({"workspace_id": "1", "chart_id": "2", "indicator": "ema", "inputs": {"length": 20}}),
        json!({"workspace_id": "1", "chart_id": "2", "indicator": "macd"}),
        json!({"workspace_id": "1", "chart_id": "abc", "indicator": "rsi"}),
        json!({"workspace_id": "x", "chart_id": "2", "indicator": "rsi"}),
        json!({"workspace_id": "1", "indicator": "rsi"}),
        json!({"workspace_id": "1", "chart_id": "2", "indicator": "rsi", "extra": true}),
    ];
    for arguments in cases {
        let reply = rpc(
            &url,
            Some("tok"),
            "tools/call",
            json!({"name": "degenview_add_indicator", "arguments": arguments}),
        );
        let json = reply.json();
        let rejected = json.get("error").is_some() || json["result"]["isError"] == json!(true);
        assert!(rejected, "accepted {arguments}: {}", reply.body);
    }
    assert!(harness.indicators.lock().unwrap().is_empty());

    // A well-formed request for a chart that is not there is the app's NOT_FOUND.
    let missing = call(
        &url,
        "tok",
        "degenview_add_indicator",
        json!({"workspace_id": "1", "chart_id": "9", "indicator": "rsi"}),
    );
    assert_eq!(error_code(&missing), "NOT_FOUND");
}

const ALERT_ALL: &[Permission] = &[
    Permission::AlertsRead,
    Permission::AlertsCreate,
    Permission::AlertsEdit,
    Permission::AlertsPause,
    Permission::AlertsDelete,
];

fn create_args(target: &str) -> Value {
    json!({"source": "binance", "symbol": "btcusdt", "condition": "crosses_above", "target_price": target})
}

#[test]
fn alert_lifecycle_create_pause_resume_delete() {
    let harness = Harness::start(&[("c1", "tok", ALERT_ALL)]);
    let url = harness.url();

    let listed = call(&url, "tok", "degenview_list_price_alerts", json!({}));
    assert_eq!(listed["structuredContent"]["alerts"], json!([]));

    let created = call(
        &url,
        "tok",
        "degenview_create_price_alert",
        create_args("70000"),
    );
    assert_eq!(created["isError"], json!(false), "{created}");
    let alert = &created["structuredContent"]["alert"];
    assert_eq!(created["structuredContent"]["status"], "created");
    assert_eq!(alert["instrument"], "BINANCE:BTCUSDT");
    assert_eq!(alert["target_price"], "70000");
    assert_eq!(alert["status"], "active");
    let id = alert["alert_id"].as_str().unwrap().to_owned();

    // Retrying the same request returns the same alert, not a second one.
    let again = call(
        &url,
        "tok",
        "degenview_create_price_alert",
        create_args("70000"),
    );
    assert_eq!(again["structuredContent"]["status"], "already_exists");
    assert_eq!(again["structuredContent"]["alert"]["alert_id"], id.as_str());

    let by_id = json!({"alert_id": id});
    let paused = call(&url, "tok", "degenview_pause_price_alert", by_id.clone());
    assert_eq!(paused["structuredContent"]["status"], "paused");
    let again = call(&url, "tok", "degenview_pause_price_alert", by_id.clone());
    assert_eq!(again["structuredContent"]["status"], "already_paused");
    let resumed = call(&url, "tok", "degenview_resume_price_alert", by_id.clone());
    assert_eq!(resumed["structuredContent"]["status"], "active");
    let again = call(&url, "tok", "degenview_resume_price_alert", by_id.clone());
    assert_eq!(again["structuredContent"]["status"], "already_active");

    let deleted = call(&url, "tok", "degenview_delete_price_alert", by_id.clone());
    assert_eq!(deleted["structuredContent"]["status"], "deleted");
    let gone = call(&url, "tok", "degenview_delete_price_alert", by_id.clone());
    assert_eq!(error_code(&gone), "NOT_FOUND");
    let missing = call(&url, "tok", "degenview_pause_price_alert", by_id);
    assert_eq!(error_code(&missing), "NOT_FOUND");

    // The no-op retries never reached the runtime as changes.
    assert_eq!(
        harness.stub.alert_calls.lock().unwrap().as_slice(),
        ["create", "create", "pause", "resume", "delete"]
    );
}

#[test]
fn each_alert_permission_gates_only_its_own_tools() {
    let harness = Harness::start(&[
        ("read", "read-tok", &[Permission::AlertsRead]),
        ("create", "create-tok", &[Permission::AlertsCreate]),
        ("edit", "edit-tok", &[Permission::AlertsEdit]),
        ("pause", "pause-tok", &[Permission::AlertsPause]),
        ("delete", "delete-tok", &[Permission::AlertsDelete]),
        ("charts", "charts-tok", BOTH),
    ]);
    let url = harness.url();
    // Seed one alert through a client allowed to create.
    let created = call(
        &url,
        "create-tok",
        "degenview_create_price_alert",
        create_args("70000"),
    );
    assert_eq!(created["isError"], json!(false), "{created}");
    let id = created["structuredContent"]["alert"]["alert_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let by_id = json!({"alert_id": id});

    let tools = [
        ("degenview_list_price_alerts", json!({}), "read-tok"),
        (
            "degenview_create_price_alert",
            create_args("71000"),
            "create-tok",
        ),
        (
            "degenview_update_price_alert",
            json!({"alert_id": id, "note": "edited"}),
            "edit-tok",
        ),
        ("degenview_pause_price_alert", by_id.clone(), "pause-tok"),
        ("degenview_resume_price_alert", by_id.clone(), "pause-tok"),
        ("degenview_delete_price_alert", by_id.clone(), "delete-tok"),
    ];
    for (tool, args, owner) in &tools {
        // Every other client is refused, however much else it may do.
        for token in [
            "read-tok",
            "create-tok",
            "edit-tok",
            "pause-tok",
            "delete-tok",
            "charts-tok",
        ] {
            if token == *owner {
                continue;
            }
            let result = call(&url, token, tool, args.clone());
            assert_eq!(
                error_code(&result),
                "PERMISSION_DENIED",
                "{tool} as {token}"
            );
        }
    }
    // Nothing the denied calls attempted reached the app.
    assert_eq!(
        harness.stub.alert_calls.lock().unwrap().as_slice(),
        ["create"]
    );

    // Discovery shows each client exactly its own tools.
    assert_eq!(
        tool_names(&url, "read-tok"),
        ["degenview_list_price_alerts"]
    );
    assert_eq!(
        tool_names(&url, "pause-tok"),
        [
            "degenview_pause_price_alert",
            "degenview_resume_price_alert"
        ]
    );
    assert_eq!(
        tool_names(&url, "delete-tok"),
        ["degenview_delete_price_alert"]
    );

    // The owners can do their one thing.
    for (tool, args, owner) in &tools {
        let result = call(&url, owner, tool, args.clone());
        assert_eq!(result["isError"], json!(false), "{tool}: {result}");
    }
}

#[test]
fn global_cap_blocks_alert_deletion_even_if_the_client_was_granted_it() {
    let harness = Harness::start(&[("c1", "tok", ALERT_ALL)]);
    let url = harness.url();
    let created = call(
        &url,
        "tok",
        "degenview_create_price_alert",
        create_args("70000"),
    );
    let id = created["structuredContent"]["alert"]["alert_id"]
        .as_str()
        .unwrap()
        .to_owned();

    harness.shared.replace(
        harness.shared.clients(),
        PermissionSet::of([Permission::AlertsRead, Permission::AlertsPause]),
    );
    let denied = call(
        &url,
        "tok",
        "degenview_delete_price_alert",
        json!({"alert_id": id}),
    );
    assert_eq!(error_code(&denied), "PERMISSION_DENIED");
    let listed = call(&url, "tok", "degenview_list_price_alerts", json!({}));
    assert_eq!(
        listed["structuredContent"]["alerts"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn bad_alert_arguments_never_reach_the_app() {
    let harness = Harness::start(&[("c1", "tok", ALERT_ALL)]);
    let url = harness.url();
    let bad_creates = [
        create_args("-5"),
        create_args("0"),
        create_args("abc"),
        create_args("1e3"),
        create_args("inf"),
        create_args("NaN"),
        create_args(".5"),
        create_args("5."),
        create_args("1.2.3"),
        create_args(""),
        create_args(&"9".repeat(40)),
        json!({"source": "binance", "symbol": "BTC-USD", "condition": "crosses_above", "target_price": "1"}),
        json!({"source": "polymarket", "symbol": "x", "condition": "crosses_above", "target_price": "1"}),
        json!({"source": "binance", "symbol": "BTCUSDT", "condition": "rises_by", "target_price": "1"}),
        json!({"source": "binance", "symbol": "BTCUSDT", "condition": "crosses_above", "target_price": "1", "currency": "XYZ"}),
        json!({"source": "binance", "symbol": "BTCUSDT", "condition": "crosses_above", "target_price": "1", "frequency": "daily"}),
        json!({"source": "binance", "symbol": "BTCUSDT", "condition": "crosses_above", "target_price": "1", "note": "a\nb"}),
        json!({"source": "binance", "symbol": "BTCUSDT", "condition": "crosses_above", "target_price": "1", "webhook": "x"}),
        json!({"source": "binance", "symbol": "BTCUSDT", "condition": "crosses_above"}),
    ];
    for arguments in bad_creates {
        let reply = rpc(
            &url,
            Some("tok"),
            "tools/call",
            json!({"name": "degenview_create_price_alert", "arguments": arguments}),
        );
        let json = reply.json();
        let rejected = json.get("error").is_some() || json["result"]["isError"] == json!(true);
        assert!(rejected, "accepted {arguments}: {}", reply.body);
    }
    for tool in [
        "degenview_pause_price_alert",
        "degenview_resume_price_alert",
        "degenview_delete_price_alert",
    ] {
        for id in ["abc", "-1", "1.5", "", "99999999999999999999999"] {
            let result = call(&url, "tok", tool, json!({"alert_id": id}));
            assert_eq!(result["isError"], json!(true), "{tool} {id:?}");
        }
    }
    assert!(harness.stub.alert_calls.lock().unwrap().is_empty());
}

#[test]
fn clean_price_text_accepts_only_plain_positive_decimals() {
    use crate::mcp::tools::clean_price_text;
    for good in ["70000", " 0.00000278 ", "1", "123456.789"] {
        assert_eq!(
            clean_price_text(good).as_deref(),
            Some(good.trim()),
            "{good}"
        );
    }
    for bad in [
        "", " ", "0", "0.0", "-1", "+1", "1e5", "1E5", "inf", "NaN", ".5", "5.", "1.2.3", "1,000",
        "$5", "0x10",
    ] {
        assert_eq!(clean_price_text(bad), None, "{bad}");
    }
}

#[test]
fn alert_editing_changes_only_what_is_given() {
    let harness = Harness::start(&[("c1", "tok", ALERT_ALL)]);
    let url = harness.url();
    let created = call(
        &url,
        "tok",
        "degenview_create_price_alert",
        create_args("70000"),
    );
    let id = created["structuredContent"]["alert"]["alert_id"]
        .as_str()
        .unwrap()
        .to_owned();

    let edited = call(
        &url,
        "tok",
        "degenview_update_price_alert",
        json!({"alert_id": id, "target_price": "72500.5", "note": " breakout "}),
    );
    assert_eq!(edited["isError"], json!(false), "{edited}");
    let alert = &edited["structuredContent"]["alert"];
    assert_eq!(edited["structuredContent"]["status"], "updated");
    assert_eq!(alert["target_price"], "72500.5");
    assert_eq!(alert["note"], "breakout");
    // Untouched settings stay.
    assert_eq!(alert["condition"], "crosses_above");
    assert_eq!(alert["currency"], "USD");
    assert_eq!(alert["alert_id"], id.as_str());

    let flipped = call(
        &url,
        "tok",
        "degenview_update_price_alert",
        json!({"alert_id": id, "condition": "crosses_below", "currency": "EUR", "frequency": "every_time"}),
    );
    let alert = &flipped["structuredContent"]["alert"];
    assert_eq!(
        (
            alert["condition"].as_str(),
            alert["currency"].as_str(),
            alert["frequency"].as_str(),
            alert["target_price"].as_str()
        ),
        (
            Some("crosses_below"),
            Some("EUR"),
            Some("every_time"),
            Some("72500.5")
        )
    );

    // Saying what is already true queues nothing; an empty note clears it.
    let same = call(
        &url,
        "tok",
        "degenview_update_price_alert",
        json!({"alert_id": id, "frequency": "every_time"}),
    );
    assert_eq!(same["structuredContent"]["status"], "unchanged");
    let cleared = call(
        &url,
        "tok",
        "degenview_update_price_alert",
        json!({"alert_id": id, "note": ""}),
    );
    assert_eq!(cleared["structuredContent"]["alert"]["note"], "");
    assert_eq!(
        harness.stub.alert_calls.lock().unwrap().as_slice(),
        ["create", "update", "update", "update"]
    );

    let missing = call(
        &url,
        "tok",
        "degenview_update_price_alert",
        json!({"alert_id": "123", "note": "x"}),
    );
    assert_eq!(error_code(&missing), "NOT_FOUND");
}

#[test]
fn bad_alert_edits_never_reach_the_app() {
    let harness = Harness::start(&[("c1", "tok", ALERT_ALL)]);
    let url = harness.url();
    let cases = [
        json!({"alert_id": "1"}),
        json!({"alert_id": "abc", "note": "x"}),
        json!({"alert_id": "1", "target_price": "-1"}),
        json!({"alert_id": "1", "target_price": "1e3"}),
        json!({"alert_id": "1", "target_price": ""}),
        json!({"alert_id": "1", "condition": "rises_by"}),
        json!({"alert_id": "1", "currency": "XYZ"}),
        json!({"alert_id": "1", "frequency": "hourly"}),
        json!({"alert_id": "1", "note": "a\u{7}b"}),
        json!({"alert_id": "1", "symbol": "ETHUSDT"}),
        json!({"alert_id": "1", "webhook_endpoint_ids": [1]}),
    ];
    for arguments in cases {
        let reply = rpc(
            &url,
            Some("tok"),
            "tools/call",
            json!({"name": "degenview_update_price_alert", "arguments": arguments}),
        );
        let json = reply.json();
        let rejected = json.get("error").is_some() || json["result"]["isError"] == json!(true);
        assert!(rejected, "accepted {arguments}: {}", reply.body);
    }
    assert!(harness.stub.alert_calls.lock().unwrap().is_empty());
}

#[test]
fn editing_is_not_implied_by_create_pause_or_delete() {
    let harness = Harness::start(&[(
        "c1",
        "tok",
        &[
            Permission::AlertsRead,
            Permission::AlertsCreate,
            Permission::AlertsPause,
            Permission::AlertsDelete,
        ],
    )]);
    let url = harness.url();
    let created = call(
        &url,
        "tok",
        "degenview_create_price_alert",
        create_args("70000"),
    );
    let id = created["structuredContent"]["alert"]["alert_id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(!tool_names(&url, "tok").contains(&"degenview_update_price_alert".to_owned()));
    let denied = call(
        &url,
        "tok",
        "degenview_update_price_alert",
        json!({"alert_id": id, "note": "x"}),
    );
    assert_eq!(error_code(&denied), "PERMISSION_DENIED");
    assert_eq!(
        harness.stub.alert_calls.lock().unwrap().as_slice(),
        ["create"]
    );
}

const PINE_ALL: &[Permission] = &[
    Permission::PineScriptsList,
    Permission::PineScriptsRead,
    Permission::PineDiagnosticsRead,
    Permission::PineInputsRead,
];

const GOOD_SCRIPT: &str = "//@version=6\nindicator(\"Good\", overlay=true)\nlen = input.int(20, \"Length\", minval=1)\nplot(ta.sma(close, len))\n";

fn seed_script(harness: &Harness, name: &str, source: &str) -> String {
    use crate::script_manager::model::ScriptType;
    let mut store = script_store_at(&harness.scripts_root);
    store
        .create(name, ScriptType::Indicator, Some(source), true)
        .unwrap()
        .id
}

#[test]
fn pine_scripts_can_be_listed_read_compiled_and_their_inputs_read() {
    let harness = Harness::start(&[("c1", "tok", PINE_ALL)]);
    let url = harness.url();
    let id = seed_script(&harness, "Good", GOOD_SCRIPT);

    let listed = call(&url, "tok", "degenview_list_pine_scripts", json!({}));
    assert_eq!(listed["isError"], json!(false), "{listed}");
    let scripts = listed["structuredContent"]["scripts"].as_array().unwrap();
    assert_eq!(scripts.len(), 1);
    assert_eq!(
        (
            scripts[0]["script_id"].as_str(),
            scripts[0]["script_type"].as_str()
        ),
        (Some(id.as_str()), Some("indicator"))
    );
    assert_eq!(scripts[0]["has_draft"], json!(false));

    let source = call(
        &url,
        "tok",
        "degenview_get_pine_script",
        json!({"script_id": id, "version": "saved"}),
    );
    assert_eq!(source["structuredContent"]["source"], GOOD_SCRIPT);
    assert_eq!(source["structuredContent"]["version"], "saved");
    assert_eq!(
        source["structuredContent"]["source_hash"]
            .as_str()
            .unwrap()
            .len(),
        64
    );
    let no_draft = call(
        &url,
        "tok",
        "degenview_get_pine_script",
        json!({"script_id": id, "version": "draft"}),
    );
    assert_eq!(error_code(&no_draft), "NOT_FOUND");

    let compiled = call(
        &url,
        "tok",
        "degenview_compile_pine_script",
        json!({"script_id": id}),
    );
    assert_eq!(
        compiled["structuredContent"]["success"],
        json!(true),
        "{compiled}"
    );
    let broken = call(
        &url,
        "tok",
        "degenview_compile_pine_script",
        json!({"source": "//@version=6\nindicator(\"B\")\nplot(\n"}),
    );
    assert_eq!(broken["structuredContent"]["success"], json!(false));
    assert!(
        broken["structuredContent"]["diagnostics"][0]["line"]
            .as_u64()
            .unwrap()
            >= 1
    );

    let inputs = call(
        &url,
        "tok",
        "degenview_get_pine_inputs",
        json!({"script_id": id}),
    );
    let first = &inputs["structuredContent"]["inputs"][0];
    assert_eq!(
        (
            first["id"].as_str(),
            first["value_type"].as_str(),
            first["min"].as_f64()
        ),
        (Some("len"), Some("integer"), Some(1.0))
    );
    assert_eq!(first["default"], 20);
}

#[test]
fn each_pine_permission_gates_only_its_own_tool() {
    let harness = Harness::start(&[
        ("list", "list-tok", &[Permission::PineScriptsList]),
        ("read", "read-tok", &[Permission::PineScriptsRead]),
        ("diag", "diag-tok", &[Permission::PineDiagnosticsRead]),
        ("inputs", "inputs-tok", &[Permission::PineInputsRead]),
        ("charts", "charts-tok", BOTH),
    ]);
    let url = harness.url();
    let id = seed_script(&harness, "Good", GOOD_SCRIPT);
    let tools = [
        ("degenview_list_pine_scripts", json!({}), "list-tok"),
        (
            "degenview_get_pine_script",
            json!({"script_id": id, "version": "saved"}),
            "read-tok",
        ),
        (
            "degenview_compile_pine_script",
            json!({"script_id": id}),
            "diag-tok",
        ),
        (
            "degenview_get_pine_inputs",
            json!({"script_id": id}),
            "inputs-tok",
        ),
    ];
    for (tool, args, owner) in &tools {
        for token in [
            "list-tok",
            "read-tok",
            "diag-tok",
            "inputs-tok",
            "charts-tok",
        ] {
            let result = call(&url, token, tool, args.clone());
            if token == *owner {
                assert_eq!(result["isError"], json!(false), "{tool}: {result}");
            } else {
                assert_eq!(
                    error_code(&result),
                    "PERMISSION_DENIED",
                    "{tool} as {token}"
                );
            }
        }
    }
    assert_eq!(tool_names(&url, "read-tok"), ["degenview_get_pine_script"]);

    // Source is private: the global cap switches it off whatever the client was granted.
    harness.shared.replace(
        harness.shared.clients(),
        PermissionSet::of([Permission::PineScriptsList]),
    );
    let denied = call(
        &url,
        "read-tok",
        "degenview_get_pine_script",
        json!({"script_id": id, "version": "saved"}),
    );
    assert_eq!(error_code(&denied), "PERMISSION_DENIED");
}

#[test]
fn bad_pine_arguments_are_refused_and_paths_cannot_be_reached() {
    let harness = Harness::start(&[("c1", "tok", PINE_ALL)]);
    let url = harness.url();
    let id = seed_script(&harness, "Good", GOOD_SCRIPT);
    let cases = [
        ("degenview_get_pine_script", json!({"script_id": id})),
        (
            "degenview_get_pine_script",
            json!({"script_id": id, "version": "latest"}),
        ),
        (
            "degenview_get_pine_script",
            json!({"script_id": "../etc/passwd", "version": "saved"}),
        ),
        (
            "degenview_get_pine_script",
            json!({"script_id": "a/b", "version": "saved"}),
        ),
        (
            "degenview_get_pine_script",
            json!({"script_id": "", "version": "saved"}),
        ),
        (
            "degenview_get_pine_script",
            json!({"script_id": "x".repeat(65), "version": "saved"}),
        ),
        (
            "degenview_get_pine_script",
            json!({"script_id": id, "version": "saved", "path": "/etc/hosts"}),
        ),
        ("degenview_compile_pine_script", json!({})),
        (
            "degenview_compile_pine_script",
            json!({"script_id": id, "source": "plot(close)"}),
        ),
        (
            "degenview_compile_pine_script",
            json!({"source": "plot(close)", "version": "draft"}),
        ),
        (
            "degenview_compile_pine_script",
            json!({"source": "x".repeat(200_001)}),
        ),
        (
            "degenview_get_pine_inputs",
            json!({"script_id": id, "version": "other"}),
        ),
        (
            "degenview_get_pine_inputs",
            json!({"script_id": "..", "version": "saved"}),
        ),
    ];
    for (tool, arguments) in cases {
        let reply = rpc(
            &url,
            Some("tok"),
            "tools/call",
            json!({"name": tool, "arguments": arguments}),
        );
        let json = reply.json();
        let rejected = json.get("error").is_some() || json["result"]["isError"] == json!(true);
        assert!(rejected, "accepted {tool} {arguments}: {}", reply.body);
    }
    let missing = call(
        &url,
        "tok",
        "degenview_get_pine_script",
        json!({"script_id": "deadbeef", "version": "saved"}),
    );
    assert_eq!(error_code(&missing), "NOT_FOUND");
}

#[test]
fn script_errors_do_not_leak_file_system_paths() {
    let harness = Harness::start(&[("c1", "tok", PINE_ALL)]);
    let url = harness.url();
    let root = harness.scripts_root.to_string_lossy().into_owned();
    for (tool, args) in [
        (
            "degenview_get_pine_script",
            json!({"script_id": "nope", "version": "saved"}),
        ),
        ("degenview_list_pine_scripts", json!({})),
        (
            "degenview_compile_pine_script",
            json!({"script_id": "nope"}),
        ),
    ] {
        let reply = rpc(
            &url,
            Some("tok"),
            "tools/call",
            json!({"name": tool, "arguments": args}),
        );
        assert!(
            !reply.body.contains(&root),
            "{tool} leaked a path: {}",
            reply.body
        );
    }
}

const PINE_INSTANCE_ALL: &[Permission] = &[
    Permission::PineScriptsList,
    Permission::PineInputsRead,
    Permission::PineStylesRead,
    Permission::PineReportsRead,
    Permission::PineAlertsRead,
];

fn instance_args() -> Value {
    json!({"workspace_id": "1", "chart_id": "2", "instance_id": "1"})
}

#[test]
fn applied_pine_tools_reach_the_app_and_return_typed_results() {
    let harness = Harness::start(&[("c1", "tok", PINE_INSTANCE_ALL)]);
    let url = harness.url();
    let list = call(
        &url,
        "tok",
        "degenview_list_chart_pine_instances",
        json!({"workspace_id": "1", "chart_id": "2"}),
    );
    assert_eq!(list["isError"], json!(false), "{list}");
    assert_eq!(
        list["structuredContent"]["instances"][0]["script_name"],
        "Strat"
    );

    let detail = call(&url, "tok", "degenview_get_pine_instance", instance_args());
    assert_eq!(detail["structuredContent"]["bar_count"], 500);
    // With no script behind it, the instance is simply unlinked (filled in on the worker side).
    assert_eq!(detail["structuredContent"]["script_status"], "unlinked");

    let styles = call(&url, "tok", "degenview_get_pine_styles", instance_args());
    assert_eq!(styles["structuredContent"]["available"], json!(false));

    let mut report_args = instance_args();
    report_args["trades_limit"] = json!(10);
    report_args["include_equity"] = json!(true);
    let report = call(&url, "tok", "degenview_get_pine_report", report_args);
    assert_eq!(report["isError"], json!(false), "{report}");
    assert_eq!(report["structuredContent"]["available"], json!(false));
    assert_eq!(report["structuredContent"]["reason"], "No report yet");

    let alerts = call(&url, "tok", "degenview_list_pine_alerts", json!({}));
    assert_eq!(
        alerts["structuredContent"]["event_history_available"],
        json!(false)
    );

    let missing = call(
        &url,
        "tok",
        "degenview_get_pine_report",
        json!({"workspace_id": "1", "chart_id": "2", "instance_id": "9"}),
    );
    assert_eq!(error_code(&missing), "NOT_FOUND");
}

#[test]
fn each_applied_pine_permission_gates_only_its_own_tool() {
    let harness = Harness::start(&[
        ("list", "list-tok", &[Permission::PineScriptsList]),
        ("inputs", "inputs-tok", &[Permission::PineInputsRead]),
        ("styles", "styles-tok", &[Permission::PineStylesRead]),
        ("reports", "reports-tok", &[Permission::PineReportsRead]),
        ("alerts", "alerts-tok", &[Permission::PineAlertsRead]),
    ]);
    let url = harness.url();
    let tools = [
        (
            "degenview_list_chart_pine_instances",
            json!({"workspace_id": "1", "chart_id": "2"}),
            "list-tok",
        ),
        ("degenview_get_pine_instance", instance_args(), "inputs-tok"),
        ("degenview_get_pine_styles", instance_args(), "styles-tok"),
        ("degenview_get_pine_report", instance_args(), "reports-tok"),
        ("degenview_list_pine_alerts", json!({}), "alerts-tok"),
    ];
    for (tool, args, owner) in &tools {
        for token in [
            "list-tok",
            "inputs-tok",
            "styles-tok",
            "reports-tok",
            "alerts-tok",
        ] {
            let result = call(&url, token, tool, args.clone());
            if token == *owner {
                assert_eq!(result["isError"], json!(false), "{tool}: {result}");
            } else {
                assert_eq!(
                    error_code(&result),
                    "PERMISSION_DENIED",
                    "{tool} as {token}"
                );
            }
        }
    }
    // Each denied call stopped before the app: only the five owners' calls arrived.
    assert_eq!(harness.stub.pine_calls.lock().unwrap().len(), 5);
}

#[test]
fn bad_applied_pine_arguments_never_reach_the_app() {
    let harness = Harness::start(&[("c1", "tok", PINE_INSTANCE_ALL)]);
    let url = harness.url();
    let mut cases = vec![
        (
            "degenview_get_pine_instance",
            json!({"workspace_id": "1", "chart_id": "2"}),
        ),
        (
            "degenview_get_pine_instance",
            json!({"workspace_id": "1", "chart_id": "x", "instance_id": "1"}),
        ),
        (
            "degenview_get_pine_instance",
            json!({"workspace_id": "1", "chart_id": "2", "instance_id": "-1"}),
        ),
        (
            "degenview_get_pine_styles",
            json!({"workspace_id": "w", "chart_id": "2", "instance_id": "1"}),
        ),
        (
            "degenview_list_chart_pine_instances",
            json!({"workspace_id": "1"}),
        ),
        (
            "degenview_list_chart_pine_instances",
            json!({"workspace_id": "1", "chart_id": "2", "extra": 1}),
        ),
    ];
    for bad in [
        json!({"trades_limit": 0}),
        json!({"trades_limit": 5000}),
        json!({"trades_offset": -1}),
        json!({"include_equity": "yes"}),
        json!({"source": "x"}),
    ] {
        let mut args = instance_args();
        for (key, value) in bad.as_object().unwrap() {
            args[key] = value.clone();
        }
        cases.push(("degenview_get_pine_report", args));
    }
    for (tool, arguments) in cases {
        let reply = rpc(
            &url,
            Some("tok"),
            "tools/call",
            json!({"name": tool, "arguments": arguments}),
        );
        let json = reply.json();
        let rejected = json.get("error").is_some() || json["result"]["isError"] == json!(true);
        assert!(rejected, "accepted {tool} {arguments}: {}", reply.body);
    }
    assert!(harness.stub.pine_calls.lock().unwrap().is_empty());
}

const EDIT_V1: &str = "//@version=6\nindicator(\"E\")\nplot(close)\n";
const EDIT_V2: &str = "//@version=6\nindicator(\"E\")\nplot(open)\n";
const EDIT_V3: &str = "//@version=6\nindicator(\"E\")\nplot(high)\n";

fn sha(text: &str) -> String {
    crate::script_manager::store::hash_source(text)
}

#[test]
fn a_script_is_edited_as_a_draft_and_only_then_saved() {
    let harness = Harness::start(&[(
        "c1",
        "tok",
        &[
            Permission::PineScriptsRead,
            Permission::PineScriptsWrite,
            Permission::PineScriptsSave,
        ],
    )]);
    let url = harness.url();
    let id = seed_script(&harness, "Edit me", EDIT_V1);

    let draft = call(
        &url,
        "tok",
        "degenview_update_pine_script",
        json!({"script_id": id, "source": EDIT_V2, "expected_revision": sha(EDIT_V1)}),
    );
    assert_eq!(draft["isError"], json!(false), "{draft}");
    let result = &draft["structuredContent"];
    assert_eq!(
        (result["action"].as_str(), result["status"].as_str()),
        (Some("update_draft"), Some("updated"))
    );
    assert_eq!(result["previous_revision"], sha(EDIT_V1));
    assert_eq!(result["revision"], sha(EDIT_V2));
    assert_eq!(result["has_draft"], json!(true));
    // The draft's own diagnostics come back with it.
    assert_eq!(result["compile"]["success"], json!(true));
    assert_eq!(result["compile"]["version"], "draft");

    // Saved and draft are now different texts, and only the draft moved.
    let saved = call(
        &url,
        "tok",
        "degenview_get_pine_script",
        json!({"script_id": id, "version": "saved"}),
    );
    let draft_read = call(
        &url,
        "tok",
        "degenview_get_pine_script",
        json!({"script_id": id, "version": "draft"}),
    );
    assert_eq!(saved["structuredContent"]["source"], EDIT_V1);
    assert_eq!(draft_read["structuredContent"]["source"], EDIT_V2);

    // Saving needs the reviewed draft's revision.
    let wrong = call(
        &url,
        "tok",
        "degenview_save_pine_script",
        json!({"script_id": id, "expected_revision": sha(EDIT_V1)}),
    );
    assert_eq!(error_code(&wrong), "CONFLICT");
    let done = call(
        &url,
        "tok",
        "degenview_save_pine_script",
        json!({"script_id": id, "expected_revision": sha(EDIT_V2)}),
    );
    assert_eq!(done["isError"], json!(false), "{done}");
    assert_eq!(done["structuredContent"]["status"], "saved");
    assert!(done["structuredContent"]["saved_revision_id"].is_string());
    assert_eq!(done["structuredContent"]["compile"]["success"], json!(true));
    let saved = call(
        &url,
        "tok",
        "degenview_get_pine_script",
        json!({"script_id": id, "version": "saved"}),
    );
    assert_eq!(saved["structuredContent"]["source"], EDIT_V2);
    let gone = call(
        &url,
        "tok",
        "degenview_get_pine_script",
        json!({"script_id": id, "version": "draft"}),
    );
    assert_eq!(error_code(&gone), "NOT_FOUND");
}

#[test]
fn a_stale_edit_is_a_conflict_and_the_newer_text_survives() {
    let harness = Harness::start(&[(
        "c1",
        "tok",
        &[Permission::PineScriptsWrite, Permission::PineScriptsRead],
    )]);
    let url = harness.url();
    let id = seed_script(&harness, "Race", EDIT_V1);
    let edit = |source: &str, revision: &str| {
        call(
            &url,
            "tok",
            "degenview_update_pine_script",
            json!({"script_id": id, "source": source, "expected_revision": revision}),
        )
    };
    // Two clients read V1; the first wins, the second is told so.
    assert_eq!(edit(EDIT_V2, &sha(EDIT_V1))["isError"], json!(false));
    let lost = edit(EDIT_V3, &sha(EDIT_V1));
    assert_eq!(error_code(&lost), "CONFLICT");
    assert!(
        lost["structuredContent"]["message"]
            .as_str()
            .unwrap()
            .contains(&sha(EDIT_V2))
    );
    let draft = call(
        &url,
        "tok",
        "degenview_get_pine_script",
        json!({"script_id": id, "version": "draft"}),
    );
    assert_eq!(draft["structuredContent"]["source"], EDIT_V2);
    // Re-reading and retrying works.
    assert_eq!(edit(EDIT_V3, &sha(EDIT_V2))["isError"], json!(false));
}

#[test]
fn saving_a_draft_with_errors_is_refused_and_the_draft_is_kept() {
    let harness = Harness::start(&[(
        "c1",
        "tok",
        &[
            Permission::PineScriptsWrite,
            Permission::PineScriptsSave,
            Permission::PineScriptsRead,
        ],
    )]);
    let url = harness.url();
    let id = seed_script(&harness, "Broken", EDIT_V1);
    let broken = "//@version=6\nindicator(\"E\")\nplot(\n";
    let draft = call(
        &url,
        "tok",
        "degenview_update_pine_script",
        json!({"script_id": id, "source": broken, "expected_revision": sha(EDIT_V1)}),
    );
    // A broken draft is allowed (it is only a draft) and its problems are reported.
    assert_eq!(draft["isError"], json!(false));
    assert_eq!(
        draft["structuredContent"]["compile"]["success"],
        json!(false)
    );
    assert!(
        draft["structuredContent"]["compile"]["error_count"]
            .as_u64()
            .unwrap()
            > 0
    );

    let refused = call(
        &url,
        "tok",
        "degenview_save_pine_script",
        json!({"script_id": id, "expected_revision": sha(broken)}),
    );
    assert_eq!(error_code(&refused), "COMPILATION_FAILED");
    let saved = call(
        &url,
        "tok",
        "degenview_get_pine_script",
        json!({"script_id": id, "version": "saved"}),
    );
    assert_eq!(saved["structuredContent"]["source"], EDIT_V1);
    let kept = call(
        &url,
        "tok",
        "degenview_get_pine_script",
        json!({"script_id": id, "version": "draft"}),
    );
    assert_eq!(kept["structuredContent"]["source"], broken);
}

#[test]
fn editing_and_saving_are_separate_permissions_and_neither_is_implied_by_reading() {
    let harness = Harness::start(&[
        (
            "reader",
            "read-tok",
            &[
                Permission::PineScriptsRead,
                Permission::PineScriptsList,
                Permission::PineDiagnosticsRead,
            ],
        ),
        ("writer", "write-tok", &[Permission::PineScriptsWrite]),
        ("saver", "save-tok", &[Permission::PineScriptsSave]),
    ]);
    let url = harness.url();
    let id = seed_script(&harness, "Gate", EDIT_V1);
    let update = json!({"script_id": id, "source": EDIT_V2, "expected_revision": sha(EDIT_V1)});
    let save = json!({"script_id": id, "expected_revision": sha(EDIT_V1)});

    for token in ["read-tok", "save-tok"] {
        assert_eq!(
            error_code(&call(
                &url,
                token,
                "degenview_update_pine_script",
                update.clone()
            )),
            "PERMISSION_DENIED",
            "{token}"
        );
    }
    for token in ["read-tok", "write-tok"] {
        assert_eq!(
            error_code(&call(
                &url,
                token,
                "degenview_save_pine_script",
                save.clone()
            )),
            "PERMISSION_DENIED",
            "{token}"
        );
    }
    assert!(
        harness.stub.pine_calls.lock().unwrap().is_empty(),
        "no denied call reached the app"
    );
    assert!(!tool_names(&url, "read-tok").contains(&"degenview_update_pine_script".to_owned()));
    assert_eq!(
        tool_names(&url, "write-tok"),
        ["degenview_update_pine_script"]
    );
    assert_eq!(tool_names(&url, "save-tok"), ["degenview_save_pine_script"]);

    // The global cap overrides a grant: with writes capped off, even the writer is refused.
    harness.shared.replace(
        harness.shared.clients(),
        PermissionSet::of([Permission::PineScriptsRead]),
    );
    assert_eq!(
        error_code(&call(
            &url,
            "write-tok",
            "degenview_update_pine_script",
            update
        )),
        "PERMISSION_DENIED"
    );
    let read = call(
        &url,
        "read-tok",
        "degenview_get_pine_script",
        json!({"script_id": id, "version": "saved"}),
    );
    assert_eq!(
        read["structuredContent"]["source"], EDIT_V1,
        "nothing was written"
    );
}

#[test]
fn bad_edit_arguments_never_reach_the_app() {
    let harness = Harness::start(&[(
        "c1",
        "tok",
        &[Permission::PineScriptsWrite, Permission::PineScriptsSave],
    )]);
    let url = harness.url();
    let id = seed_script(&harness, "Args", EDIT_V1);
    let good = sha(EDIT_V1);
    let cases = [
        (
            "degenview_update_pine_script",
            json!({"script_id": id, "source": EDIT_V2}),
        ),
        (
            "degenview_update_pine_script",
            json!({"script_id": id, "source": EDIT_V2, "expected_revision": "abc"}),
        ),
        (
            "degenview_update_pine_script",
            json!({"script_id": id, "source": EDIT_V2, "expected_revision": good.to_uppercase()}),
        ),
        (
            "degenview_update_pine_script",
            json!({"script_id": id, "source": EDIT_V2, "expected_revision": "z".repeat(64)}),
        ),
        (
            "degenview_update_pine_script",
            json!({"script_id": "../x", "source": EDIT_V2, "expected_revision": good}),
        ),
        (
            "degenview_update_pine_script",
            json!({"script_id": id, "source": "x".repeat(200_001), "expected_revision": good}),
        ),
        (
            "degenview_update_pine_script",
            json!({"script_id": id, "source": EDIT_V2, "expected_revision": good, "path": "/tmp/x"}),
        ),
        (
            "degenview_update_pine_script",
            json!({"script_id": id, "source": 5, "expected_revision": good}),
        ),
        ("degenview_save_pine_script", json!({"script_id": id})),
        (
            "degenview_save_pine_script",
            json!({"script_id": id, "expected_revision": "nope"}),
        ),
        (
            "degenview_save_pine_script",
            json!({"script_id": id, "expected_revision": good, "source": EDIT_V2}),
        ),
    ];
    for (tool, arguments) in cases {
        let reply = rpc(
            &url,
            Some("tok"),
            "tools/call",
            json!({"name": tool, "arguments": arguments}),
        );
        let json = reply.json();
        let rejected = json.get("error").is_some() || json["result"]["isError"] == json!(true);
        assert!(rejected, "accepted {tool} {arguments}: {}", reply.body);
    }
    assert!(harness.stub.pine_calls.lock().unwrap().is_empty());
}

// MARK: Applying scripts to charts

const APPLY_ONLY: &[Permission] = &[Permission::ChartsPineInstancesWrite];

fn apply_args(script_id: &str) -> Value {
    json!({"workspace_id": "1", "chart_id": "2", "script_id": script_id})
}

#[test]
fn applying_scripts_has_its_own_permission() {
    let harness = Harness::start(&[
        ("apply", "apply-tok", APPLY_ONLY),
        (
            "indicators",
            "ind-tok",
            &[Permission::ChartsIndicatorsWrite],
        ),
        (
            "scripts",
            "scripts-tok",
            &[
                Permission::PineScriptsWrite,
                Permission::PineScriptsSave,
                Permission::PineScriptsList,
            ],
        ),
    ]);
    let url = harness.url();
    let id = seed_script(&harness, "Good", GOOD_SCRIPT);
    let tools = [
        ("degenview_add_pine_script_to_chart", apply_args(&id)),
        (
            "degenview_update_pine_instance",
            json!({"workspace_id": "1", "chart_id": "2", "instance_id": "1", "visible": false}),
        ),
        ("degenview_remove_pine_instance", instance_args()),
    ];
    for (tool, args) in &tools {
        let allowed = call(&url, "apply-tok", tool, args.clone());
        assert_eq!(allowed["isError"], json!(false), "{tool}: {allowed}");
        for token in ["ind-tok", "scripts-tok"] {
            let denied = call(&url, token, tool, args.clone());
            assert_eq!(
                error_code(&denied),
                "PERMISSION_DENIED",
                "{tool} as {token}"
            );
        }
    }
    assert_eq!(
        tool_names(&url, "apply-tok"),
        [
            "degenview_add_pine_script_to_chart",
            "degenview_remove_pine_instance",
            "degenview_update_pine_instance",
        ]
    );
    assert!(
        !tool_names(&url, "ind-tok").contains(&"degenview_add_pine_script_to_chart".to_owned())
    );

    harness
        .shared
        .replace(harness.shared.clients(), PermissionSet::none());
    let denied = call(&url, "apply-tok", tools[0].0, tools[0].1.clone());
    assert_eq!(error_code(&denied), "PERMISSION_DENIED");
}

#[test]
fn a_saved_script_reaches_the_app_with_checked_inputs() {
    use crate::pine::program::InputValue;
    let harness = Harness::start(&[("c1", "tok", APPLY_ONLY)]);
    let url = harness.url();
    let id = seed_script(&harness, "Good", GOOD_SCRIPT);

    let mut args = apply_args(&id);
    args["inputs"] = json!({"len": 50});
    args["visible"] = json!(false);
    let result = call(&url, "tok", "degenview_add_pine_script_to_chart", args);
    assert_eq!(result["isError"], json!(false), "{result}");
    assert_eq!(result["structuredContent"]["status"], json!("added"));
    assert_eq!(result["structuredContent"]["instance_id"], json!("7"));

    // A value equal to the input's default is no override, as in the Inputs card.
    let mut at_default = apply_args(&id);
    at_default["inputs"] = json!({"len": 20.0});
    let result = call(
        &url,
        "tok",
        "degenview_add_pine_script_to_chart",
        at_default,
    );
    assert_eq!(result["isError"], json!(false), "{result}");

    let applied = harness.stub.applied.lock().unwrap();
    assert_eq!(applied.len(), 2);
    assert_eq!(applied[0].script_id, id);
    assert_eq!(applied[0].name, "Good");
    assert_eq!(applied[0].source, GOOD_SCRIPT);
    assert_eq!(applied[0].inputs.get("len"), Some(&InputValue::Int(50)));
    assert!(applied[1].inputs.is_empty());
}

#[test]
fn an_unusable_script_or_input_never_reaches_the_app() {
    use crate::script_manager::model::ScriptType;
    let harness = Harness::start(&[("c1", "tok", APPLY_ONLY)]);
    let url = harness.url();
    let good = seed_script(&harness, "Good", GOOD_SCRIPT);
    let broken = seed_script(
        &harness,
        "Broken",
        "//@version=6\nindicator(\"B\")\nplot(close\n",
    );
    let library = {
        let mut store = script_store_at(&harness.scripts_root);
        store
            .create("Lib", ScriptType::Library, None, true)
            .unwrap()
            .id
    };
    let with_inputs = |inputs: Value| {
        let mut args = apply_args(&good);
        args["inputs"] = inputs;
        args
    };
    let cases = [
        ("unknown script", apply_args("nope")),
        ("not an id", apply_args("../x")),
        ("library", apply_args(&library)),
        ("does not compile", apply_args(&broken)),
        ("unknown input", with_inputs(json!({"width": 3}))),
        ("wrong type", with_inputs(json!({"len": "ten"}))),
        ("below minimum", with_inputs(json!({"len": 0}))),
        ("fraction for int", with_inputs(json!({"len": 2.5}))),
        ("flag for number", with_inputs(json!({"len": true}))),
        (
            "null chart",
            json!({"workspace_id": "1", "chart_id": "x", "script_id": good}),
        ),
        ("extra field", {
            let mut args = apply_args(&good);
            args["color"] = json!("red");
            args
        }),
        ("visible not a flag", {
            let mut args = apply_args(&good);
            args["visible"] = json!("yes");
            args
        }),
    ];
    for (why, arguments) in cases {
        let reply = rpc(
            &url,
            Some("tok"),
            "tools/call",
            json!({"name": "degenview_add_pine_script_to_chart", "arguments": arguments}),
        );
        let json = reply.json();
        let rejected = json.get("error").is_some() || json["result"]["isError"] == json!(true);
        assert!(rejected, "accepted ({why}) {arguments}: {}", reply.body);
    }
    // Updates need something to change, and ids that are ids.
    for arguments in [
        json!({"workspace_id": "1", "chart_id": "2", "instance_id": "1"}),
        json!({"workspace_id": "1", "chart_id": "2", "instance_id": "1", "inputs": {}}),
        json!({"workspace_id": "1", "chart_id": "2", "instance_id": "x", "visible": true}),
    ] {
        let result = call(
            &url,
            "tok",
            "degenview_update_pine_instance",
            arguments.clone(),
        );
        assert_eq!(error_code(&result), "INVALID_ARGUMENT", "{arguments}");
    }
    assert!(harness.stub.pine_calls.lock().unwrap().is_empty());
    assert!(harness.stub.applied.lock().unwrap().is_empty());
}

#[test]
fn an_applied_script_can_be_changed_and_removed() {
    let harness = Harness::start(&[("c1", "tok", APPLY_ONLY)]);
    let url = harness.url();
    let result = call(
        &url,
        "tok",
        "degenview_update_pine_instance",
        json!({
            "workspace_id": "1", "chart_id": "2", "instance_id": "1",
            "inputs": {"len": 30, "unset": null}, "visible": false
        }),
    );
    assert_eq!(result["isError"], json!(false), "{result}");
    let result = call(
        &url,
        "tok",
        "degenview_remove_pine_instance",
        instance_args(),
    );
    assert_eq!(result["structuredContent"]["status"], json!("removed"));

    let updates = harness.stub.instance_updates.lock().unwrap();
    assert_eq!(updates.len(), 1);
    let (inputs, visible) = &updates[0];
    assert_eq!(*visible, Some(false));
    let inputs = inputs.as_ref().unwrap();
    assert_eq!(inputs.len(), 2);
    assert_eq!(inputs["unset"], None);
    assert!(inputs["len"].is_some());
    assert_eq!(
        *harness.stub.pine_calls.lock().unwrap(),
        ["update", "remove"]
    );
}

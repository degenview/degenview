use super::DegenViewApp;
use super::alert::commit_price_alert_checked;
use super::now_ms;
use super::tabs::TabContent;
use super::window::settings_viewport_id;
use crate::host::mcp;
use crate::host::mcp::confirmation::{self, ConfirmationHost};
use crate::mcp::bridge::{McpCommand, McpError, McpReply};
use crate::mcp::types::{AlertChangeResult, CreateAlertResult, UpdateAlertResult};
use crate::model::alert::{AlertPrecondition, AlertRuntimeCommand, AlertRuntimeCommandPayload};
use crate::service::alert::macos_notifications;
use crate::workspace::ChartWorkspace;
use eframe::egui;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// The question an MCP client's request raises, if one is open. Drawn by the focused window only,
/// so one question shows however many windows are open; the answer goes to the host.
pub(super) fn show_mcp_confirmation(ctx: &egui::Context, host: &Mutex<ConfirmationHost>) {
    let now = Instant::now();
    let pending = host
        .lock()
        .map(|host| host.pending(now))
        .unwrap_or_default();
    let answer = crate::ui::mcp::confirmation::show(ctx, &pending);
    let Some(answer) = answer else {
        return;
    };
    if let Ok(mut host) = host.lock() {
        match answer {
            crate::ui::mcp::confirmation::Answer::Allow(id) => host.approve(now, id),
            crate::ui::mcp::confirmation::Answer::Deny(id) => host.deny(now, id),
        }
    }
}

fn alerts_unavailable() -> McpError {
    McpError::AppUnavailable("Price alerts are unavailable right now".into())
}

fn snapshot_unavailable() -> McpError {
    McpError::AppUnavailable("Chart pictures are unavailable right now".into())
}

fn portfolio_unavailable() -> McpError {
    McpError::AppUnavailable("Portfolios are unavailable right now".into())
}

fn paper_unavailable() -> McpError {
    McpError::AppUnavailable("Paper trading is unavailable right now".into())
}

fn watchlists_unavailable() -> McpError {
    McpError::AppUnavailable("Watchlists are unavailable right now".into())
}

impl DegenViewApp {
    /// Applies queued MCP Settings changes and runs the tool requests the server queued. Every
    /// request is checked again just before it runs (see `mcp::bridge::Request::run`).
    pub(super) fn tick_mcp(&mut self, ctx: &egui::Context) {
        /// Requests run per frame; the rest wait for the next one.
        const REQUESTS_PER_FRAME: usize = 8;
        let (requests, shared) = {
            let Ok(mut host) = self.mcp_host.lock() else {
                return;
            };
            host.tick();
            (host.take_requests(REQUESTS_PER_FRAME), host.shared())
        };
        if requests.len() == REQUESTS_PER_FRAME {
            ctx.request_repaint();
        }
        // Pictures an agent never collected are dropped on time, not at its next snapshot call.
        if let Ok(mut snapshots) = self.mcp_snapshots.lock() {
            snapshots.expire();
        }
        for request in requests {
            let client_id = request.client_id.clone();
            request.run(&shared, |command, origin| {
                self.run_gated(ctx, &shared, &client_id, origin.remote, command)
            });
        }
        // Questions nobody answered run out, and a new one is announced where the user can see it.
        let fresh = match self.mcp_confirmations.lock() {
            Ok(mut confirmations) => {
                confirmations.tick(Instant::now());
                confirmations.take_fresh()
            }
            Err(_) => Vec::new(),
        };
        if !fresh.is_empty() {
            self.announce_questions(ctx, &fresh);
        }
    }

    /// Runs `command`, but a high-impact one only once the user has approved this exact request
    /// (see `mcp::confirm`). Until then the caller is told the question to put to the user.
    fn run_gated(
        &self,
        ctx: &egui::Context,
        shared: &crate::mcp::state::McpShared,
        client_id: &str,
        remote_origin: bool,
        command: McpCommand,
    ) -> Result<McpReply, McpError> {
        if crate::mcp::confirm::needs_approval(shared, client_id, remote_origin, &command) {
            let name = shared
                .clients()
                .into_iter()
                .find(|client| client.id == client_id)
                .map(|client| client.name)
                .unwrap_or_default();
            confirmation::gate_command(
                &self.mcp_confirmations,
                Instant::now(),
                client_id,
                &name,
                remote_origin,
                &command,
                || self.confirmation_summary(&command),
            )?;
        }
        self.run_mcp_command(ctx, command)
    }

    /// What `command` would do, in words, from what the app knows about its target. A target that
    /// does not exist is refused with the error the command itself would give, unasked. A gated
    /// command with no description here is refused rather than run unasked.
    fn confirmation_summary(
        &self,
        command: &McpCommand,
    ) -> Result<crate::mcp::confirm::Summary, McpError> {
        match command {
            McpCommand::DeleteAlert { alert_id } => {
                let host = self.alert_host.lock().map_err(|_| alerts_unavailable())?;
                confirmation::delete_alert(&host, *alert_id)
            }
            McpCommand::DeleteWatchlist { watchlist_id } => {
                let store = self
                    .watchlist
                    .lock()
                    .map_err(|_| watchlists_unavailable())?;
                confirmation::delete_watchlist(&store, *watchlist_id)
            }
            McpCommand::RemoveChart {
                workspace_id,
                chart_id,
            } => self.with_mcp_workspace(*workspace_id, |workspace| {
                confirmation::remove_chart(workspace, *chart_id)
            }),
            McpCommand::RemoveDrawings {
                workspace_id,
                chart_id,
                drawing_ids,
            } => self.with_mcp_workspace(*workspace_id, |workspace| {
                confirmation::remove_drawings(workspace, *chart_id, drawing_ids)
            }),
            McpCommand::RemovePineInstance {
                workspace_id,
                chart_id,
                instance_id,
            } => {
                let alerts = self.pine_subscriptions_for_mcp()?;
                self.with_mcp_workspace(*workspace_id, |workspace| {
                    confirmation::remove_pine_instance(workspace, *chart_id, *instance_id, &alerts)
                })
            }
            McpCommand::EditPineScript { script_id, .. } => {
                let shared = self
                    .mcp_host
                    .lock()
                    .map_err(|_| McpError::AppUnavailable("The MCP host is unavailable".into()))?
                    .shared();
                let mut store = shared.script_store().ok_or_else(|| {
                    McpError::AppUnavailable("The script library is unavailable".into())
                })?;
                let script = store
                    .script(script_id)
                    .map_err(|_| {
                        McpError::AppUnavailable("The script library is unavailable".into())
                    })?
                    .ok_or_else(|| McpError::NotFound(format!("No script {script_id}")))?;
                Ok(confirmation::save_script(&script.name))
            }
            McpCommand::PlacePaperOrder { request, .. } => {
                let host = self.paper.lock().map_err(|_| paper_unavailable())?;
                confirmation::place_order(&host, request)
            }
            McpCommand::ModifyPaperOrder { order_id, changes } => {
                let host = self.paper.lock().map_err(|_| paper_unavailable())?;
                confirmation::modify_order(&host, *order_id, changes)
            }
            McpCommand::CancelPaperOrders {
                account_id,
                order_ids,
            } => {
                let host = self.paper.lock().map_err(|_| paper_unavailable())?;
                confirmation::cancel_orders(&host, *account_id, order_ids.as_deref())
            }
            McpCommand::ClosePaperPositions {
                account_id,
                instrument_key,
            } => {
                let host = self.paper.lock().map_err(|_| paper_unavailable())?;
                confirmation::close_positions(&host, *account_id, instrument_key.as_deref())
            }
            // The rest are asked about only of a credential with remote access, and described from
            // the command alone. A command with no description is refused rather than run unasked.
            _ => crate::mcp::confirm::remote_summary(command).ok_or_else(|| {
                McpError::AppUnavailable(
                    "This request cannot be described, so it was not run".into(),
                )
            }),
        }
    }

    /// A new question is up: wake every window so the focused one draws it, and tell the user
    /// outside the app too, if they have notifications on.
    fn announce_questions(&self, ctx: &egui::Context, fresh: &[crate::mcp::confirm::PendingView]) {
        ctx.request_repaint_of(egui::ViewportId::ROOT);
        ctx.request_repaint_of(settings_viewport_id());
        for (window, _) in self.tab_host.snapshot() {
            ctx.request_repaint_of(Self::viewport_id(window.id));
        }
        let enabled = self.alert_host.lock().is_ok_and(|host| {
            crate::model::alert::notification::notifications_enabled(host.settings())
        });
        if !enabled {
            return;
        }
        for view in fresh {
            macos_notifications::notify_async(
                Arc::clone(&self.notifier),
                crate::model::alert::notification::NotificationContent {
                    title: format!("“{}” is asking permission", view.client_name),
                    body: view.summary.line(),
                    identifier: format!("mcp-confirmation-{}", view.id),
                    sound: false,
                    user_info: Vec::new(),
                },
                |_, _| {},
            );
        }
    }

    fn run_mcp_command(
        &self,
        ctx: &egui::Context,
        command: McpCommand,
    ) -> Result<McpReply, McpError> {
        match command {
            McpCommand::ListWorkspaces => {
                let active = self.focused_tab.load(Ordering::Relaxed);
                let mut workspaces = Vec::new();
                for tab in &self.tabs {
                    let Ok(content) = tab.content.lock() else {
                        continue;
                    };
                    if let TabContent::Chart(workspace) = &*content {
                        workspaces.push(mcp::chart::workspace_info(
                            workspace,
                            workspace.id() == active,
                        ));
                    }
                }
                Ok(McpReply::Workspaces(workspaces))
            }
            McpCommand::ListCharts { workspace_id } => self
                .with_mcp_workspace(workspace_id, |workspace| {
                    Ok(McpReply::Charts(mcp::chart::charts_result(workspace)))
                }),
            McpCommand::ListAlerts { only } => {
                let host = self.alert_host.lock().map_err(|_| alerts_unavailable())?;
                Ok(McpReply::Alerts(mcp::alert::list_alerts(&host, only)))
            }
            McpCommand::CreateAlert(request) => {
                let now = now_ms();
                let plan = {
                    let host = self.alert_host.lock().map_err(|_| alerts_unavailable())?;
                    mcp::alert::plan_create_alert(
                        &host,
                        &self.quote_book,
                        &self.alert_fx_rates,
                        &request,
                        now,
                    )?
                };
                match plan {
                    mcp::alert::CreatePlan::Existing(alert) => {
                        Ok(McpReply::AlertCreated(CreateAlertResult {
                            status: "already_exists".into(),
                            alert: *alert,
                            command_id: None,
                        }))
                    }
                    mcp::alert::CreatePlan::New { alert, baseline } => {
                        let info = mcp::alert::alert_info(&alert, None);
                        let store = self.store.lock().map_err(|_| alerts_unavailable())?;
                        // The same commit the price-alert sheet's Save does, but a save that
                        // never reached the queue is an error here, not a "queued".
                        let command_id = commit_price_alert_checked(
                            &store,
                            &self.notifier,
                            &self.background_service,
                            &self.alert_runtime_control,
                            alert,
                            baseline,
                            // Other requests in this frame planned against the same snapshot:
                            // the runtime decides again (duplicate, cap) when it applies.
                            Some(AlertPrecondition::Create {
                                max_alerts: mcp::alert::MAX_ALERTS,
                            }),
                            now,
                        )
                        .map_err(|_| {
                            McpError::AppUnavailable("Could not queue the alert".into())
                        })?;
                        ctx.request_repaint();
                        Ok(McpReply::AlertCreated(CreateAlertResult {
                            status: "queued".into(),
                            alert: info,
                            command_id: Some(command_id),
                        }))
                    }
                }
            }
            McpCommand::AlertCommandOutcome { command_id } => {
                let host = self.alert_host.lock().map_err(|_| alerts_unavailable())?;
                Ok(McpReply::AlertCommand(mcp::alert::alert_command_outcome(
                    &host, command_id,
                )))
            }
            McpCommand::UpdateAlert(request) => {
                let now = now_ms();
                let plan = {
                    let host = self.alert_host.lock().map_err(|_| alerts_unavailable())?;
                    mcp::alert::plan_update_alert(
                        &host,
                        &self.quote_book,
                        &self.alert_fx_rates,
                        &request,
                        now,
                    )?
                };
                match plan {
                    mcp::alert::UpdatePlan::Unchanged(alert) => {
                        Ok(McpReply::AlertUpdated(UpdateAlertResult {
                            status: "unchanged".into(),
                            alert: *alert,
                            command_id: None,
                        }))
                    }
                    mcp::alert::UpdatePlan::Change {
                        alert,
                        base,
                        baseline,
                    } => {
                        let info = mcp::alert::alert_info(&alert, None);
                        // Planned from a snapshot other requests may also have planned from:
                        // the runtime applies it only to the alert it was planned from.
                        let command_id = self.queue_alert_command(
                            ctx,
                            AlertRuntimeCommandPayload::Save { alert, baseline },
                            Some(AlertPrecondition::Unchanged(base)),
                        )?;
                        Ok(McpReply::AlertUpdated(UpdateAlertResult {
                            status: "queued".into(),
                            alert: info,
                            command_id: Some(command_id),
                        }))
                    }
                }
            }
            McpCommand::SetAlertPaused { alert_id, paused } => {
                let plan = {
                    let host = self.alert_host.lock().map_err(|_| alerts_unavailable())?;
                    mcp::alert::plan_pause_alert(&host, alert_id, paused)?
                };
                let status = match plan {
                    mcp::alert::StatePlan::Already if paused => "already_paused",
                    mcp::alert::StatePlan::Already => "already_active",
                    mcp::alert::StatePlan::Change { id, state } => {
                        self.queue_alert_command(
                            ctx,
                            AlertRuntimeCommandPayload::SetState {
                                id,
                                state,
                                baseline: None,
                            },
                            None,
                        )?;
                        "queued"
                    }
                };
                Ok(McpReply::AlertChanged(AlertChangeResult {
                    alert_id: alert_id.to_string(),
                    status: status.into(),
                }))
            }
            McpCommand::DeleteAlert { alert_id } => {
                let id = {
                    let host = self.alert_host.lock().map_err(|_| alerts_unavailable())?;
                    mcp::alert::plan_delete_alert(&host, alert_id)?
                };
                self.queue_alert_command(ctx, AlertRuntimeCommandPayload::Delete(id), None)?;
                Ok(McpReply::AlertChanged(AlertChangeResult {
                    alert_id: id.to_string(),
                    status: "queued".into(),
                }))
            }
            McpCommand::ListChartPine {
                workspace_id,
                chart_id,
            } => self.with_mcp_workspace(workspace_id, |workspace| {
                mcp::pine::list_chart_pine(workspace, chart_id).map(McpReply::ChartPine)
            }),
            McpCommand::GetChartPine {
                workspace_id,
                chart_id,
                instance_id,
            } => self.with_mcp_workspace(workspace_id, |workspace| {
                mcp::pine::chart_pine_detail(workspace, chart_id, instance_id)
                    .map(McpReply::ChartPineDetail)
            }),
            McpCommand::GetPineStyles {
                workspace_id,
                chart_id,
                instance_id,
            } => self.with_mcp_workspace(workspace_id, |workspace| {
                mcp::pine::pine_styles(workspace, chart_id, instance_id).map(McpReply::PineStyles)
            }),
            McpCommand::GetPineReport {
                workspace_id,
                chart_id,
                instance_id,
                offset,
                limit,
                include_equity,
            } => self.with_mcp_workspace(workspace_id, |workspace| {
                mcp::pine::pine_report(
                    workspace,
                    chart_id,
                    instance_id,
                    offset,
                    limit,
                    include_equity,
                )
                .map(|report| McpReply::PineReport(Box::new(report)))
            }),
            McpCommand::ListPaperAccounts => {
                let host = self.paper.lock().map_err(|_| paper_unavailable())?;
                mcp::paper::list_paper_accounts(&host).map(McpReply::PaperAccounts)
            }
            McpCommand::GetPaperAccount { account_id } => {
                let host = self.paper.lock().map_err(|_| paper_unavailable())?;
                mcp::paper::get_paper_account(&host, account_id)
                    .map(|detail| McpReply::PaperAccount(Box::new(detail)))
            }
            McpCommand::ListPaperHistory {
                account_id,
                closed_trades,
                offset,
                limit,
            } => {
                let host = self.paper.lock().map_err(|_| paper_unavailable())?;
                if closed_trades {
                    mcp::paper::paper_trades(&host, account_id, offset, limit)
                        .map(|result| McpReply::PaperTrades(Box::new(result)))
                } else {
                    mcp::paper::paper_fills(&host, account_id, offset, limit)
                        .map(|result| McpReply::PaperFills(Box::new(result)))
                }
            }
            McpCommand::PlacePaperOrder {
                request,
                dry_run,
                request_id,
            } => {
                let mut host = self.paper.lock().map_err(|_| paper_unavailable())?;
                let mut memory = self.mcp_orders.lock().map_err(|_| paper_unavailable())?;
                let result = mcp::paper::place_paper_order(
                    &mut host,
                    &mut memory,
                    *request,
                    dry_run,
                    request_id.as_deref(),
                )?;
                ctx.request_repaint();
                Ok(McpReply::PaperOrder(Box::new(result)))
            }
            McpCommand::ModifyPaperOrder { order_id, changes } => {
                let mut host = self.paper.lock().map_err(|_| paper_unavailable())?;
                let result = mcp::paper::modify_paper_order(&mut host, order_id, changes)?;
                ctx.request_repaint();
                Ok(McpReply::PaperChange(Box::new(result)))
            }
            McpCommand::CancelPaperOrders {
                account_id,
                order_ids,
            } => {
                let mut host = self.paper.lock().map_err(|_| paper_unavailable())?;
                let result =
                    mcp::paper::cancel_paper_orders(&mut host, account_id, order_ids.as_deref())?;
                ctx.request_repaint();
                Ok(McpReply::PaperChange(Box::new(result)))
            }
            McpCommand::ClosePaperPositions {
                account_id,
                instrument_key,
            } => {
                let mut host = self.paper.lock().map_err(|_| paper_unavailable())?;
                let result = mcp::paper::close_paper_positions(
                    &mut host,
                    account_id,
                    instrument_key.as_deref(),
                )?;
                ctx.request_repaint();
                Ok(McpReply::PaperChange(Box::new(result)))
            }
            McpCommand::ListPortfolios => {
                let host = self.portfolio.lock().map_err(|_| portfolio_unavailable())?;
                mcp::portfolio::list_portfolios(&host).map(McpReply::Portfolios)
            }
            McpCommand::GetPortfolioHoldings { portfolio_id } => {
                let host = self.portfolio.lock().map_err(|_| portfolio_unavailable())?;
                mcp::portfolio::portfolio_holdings_base(&host, portfolio_id, now_ms())
                    .map(|base| McpReply::PortfolioHoldings(Box::new(base)))
            }
            McpCommand::ListPortfolioTransactions {
                portfolio_id,
                asset,
                offset,
                limit,
            } => {
                let host = self.portfolio.lock().map_err(|_| portfolio_unavailable())?;
                mcp::portfolio::portfolio_transactions(
                    &host,
                    portfolio_id,
                    asset.as_deref(),
                    offset,
                    limit,
                )
                .map(|result| McpReply::PortfolioTransactions(Box::new(result)))
            }
            McpCommand::ListWatchlists => {
                let store = self
                    .watchlist
                    .lock()
                    .map_err(|_| watchlists_unavailable())?;
                mcp::watchlist::list_watchlists(&store).map(McpReply::Watchlists)
            }
            McpCommand::GetWatchlist {
                watchlist_id,
                offset,
                limit,
            } => {
                let store = self
                    .watchlist
                    .lock()
                    .map_err(|_| watchlists_unavailable())?;
                mcp::watchlist::get_watchlist(&store, watchlist_id, offset, limit)
                    .map(|page| McpReply::Watchlist(Box::new(page)))
            }
            McpCommand::CreateWatchlist { name, markets } => {
                let mut store = self
                    .watchlist
                    .lock()
                    .map_err(|_| watchlists_unavailable())?;
                let result = mcp::watchlist::create_watchlist(&mut store, &name, markets)?;
                ctx.request_repaint();
                Ok(McpReply::WatchlistChanged(Box::new(result)))
            }
            McpCommand::UpdateWatchlist {
                watchlist_id,
                changes,
            } => {
                let mut store = self
                    .watchlist
                    .lock()
                    .map_err(|_| watchlists_unavailable())?;
                let result = mcp::watchlist::update_watchlist(&mut store, watchlist_id, *changes)?;
                ctx.request_repaint();
                Ok(McpReply::WatchlistChanged(Box::new(result)))
            }
            McpCommand::DeleteWatchlist { watchlist_id } => {
                let mut store = self
                    .watchlist
                    .lock()
                    .map_err(|_| watchlists_unavailable())?;
                let result = mcp::watchlist::delete_watchlist(&mut store, watchlist_id)?;
                ctx.request_repaint();
                Ok(McpReply::WatchlistChanged(Box::new(result)))
            }
            McpCommand::EditPineScript {
                script_id,
                expected_revision,
                source,
            } => self
                .edit_pine_script(&script_id, &expected_revision, source.as_deref())
                .map(|result| McpReply::PineEdit(Box::new(result))),
            McpCommand::ListPineAlerts => {
                let store = self.store.lock().map_err(|_| alerts_unavailable())?;
                Ok(McpReply::PineAlerts(mcp::pine::pine_alerts(
                    &store.pine_subscriptions(),
                )))
            }
            McpCommand::GetCandles {
                workspace_id,
                chart_id,
                limit,
                before,
            } => self.with_mcp_workspace(workspace_id, |workspace| {
                mcp::chart::chart_candles(workspace, chart_id, limit, before)
                    .map(|result| McpReply::Candles(Box::new(result)))
            }),
            McpCommand::ListDrawings {
                workspace_id,
                chart_id,
            } => self.with_mcp_workspace(workspace_id, |workspace| {
                mcp::chart::chart_drawings(workspace, chart_id)
                    .map(|result| McpReply::Drawings(Box::new(result)))
            }),
            McpCommand::AddDrawings {
                workspace_id,
                chart_id,
                drawings,
            } => self.with_mcp_workspace(workspace_id, |workspace| {
                mcp::chart::add_drawings(workspace, chart_id, &drawings)
                    .map(|result| McpReply::DrawingsChanged(Box::new(result)))
            }),
            McpCommand::EditDrawing {
                workspace_id,
                chart_id,
                edit,
            } => self.with_mcp_workspace(workspace_id, |workspace| {
                mcp::chart::edit_drawing(workspace, chart_id, &edit)
                    .map(|result| McpReply::DrawingsChanged(Box::new(result)))
            }),
            McpCommand::RemoveDrawings {
                workspace_id,
                chart_id,
                drawing_ids,
            } => self.with_mcp_workspace(workspace_id, |workspace| {
                mcp::chart::remove_drawings(workspace, chart_id, &drawing_ids)
                    .map(|result| McpReply::DrawingsChanged(Box::new(result)))
            }),
            McpCommand::AddPineInstance {
                workspace_id,
                chart_id,
                script,
                visible,
            } => self.with_mcp_workspace(workspace_id, |workspace| {
                mcp::pine::add_pine_instance(workspace, chart_id, &script, visible)
                    .map(|result| McpReply::PineInstanceChanged(Box::new(result)))
            }),
            McpCommand::UpdatePineInstance {
                workspace_id,
                chart_id,
                instance_id,
                inputs,
                visible,
            } => {
                let alerts = self.pine_subscriptions_for_mcp()?;
                self.with_mcp_workspace(workspace_id, |workspace| {
                    mcp::pine::update_pine_instance(
                        workspace,
                        chart_id,
                        instance_id,
                        inputs.as_ref(),
                        visible,
                        &alerts,
                    )
                    .map(|result| McpReply::PineInstanceChanged(Box::new(result)))
                })
            }
            McpCommand::RemovePineInstance {
                workspace_id,
                chart_id,
                instance_id,
            } => {
                let alerts = self.pine_subscriptions_for_mcp()?;
                self.with_mcp_workspace(workspace_id, |workspace| {
                    mcp::pine::remove_pine_instance(workspace, chart_id, instance_id, &alerts)
                        .map(|result| McpReply::PineInstanceChanged(Box::new(result)))
                })
            }
            McpCommand::ApplyIndicator {
                workspace_id,
                chart_id,
                indicator,
                ema_period,
            } => self.with_mcp_workspace(workspace_id, |workspace| {
                mcp::chart::apply_indicator(workspace, chart_id, indicator, ema_period)
                    .map(McpReply::IndicatorApplied)
            }),
            McpCommand::SetTimeframe {
                workspace_id,
                timeframe,
            } => self.with_mcp_workspace(workspace_id, |workspace| {
                Ok(McpReply::Timeframe(mcp::chart::set_timeframe(
                    workspace, timeframe, ctx,
                )))
            }),
            McpCommand::SetChartType {
                workspace_id,
                chart_id,
                chart_type,
            } => self.with_mcp_workspace(workspace_id, |workspace| {
                mcp::chart::set_chart_type(workspace, chart_id, chart_type)
                    .map(|result| McpReply::ChartChanged(Box::new(result)))
            }),
            McpCommand::RemoveChart {
                workspace_id,
                chart_id,
            } => self.with_mcp_workspace(workspace_id, |workspace| {
                mcp::chart::remove_chart(workspace, chart_id)
                    .map(|result| McpReply::ChartChanged(Box::new(result)))
            }),
            McpCommand::BeginSnapshot {
                workspace_id,
                chart_id,
                scale,
            } => {
                let mut jobs = self
                    .mcp_snapshots
                    .lock()
                    .map_err(|_| snapshot_unavailable())?;
                let started = self.with_mcp_workspace(workspace_id, |workspace| {
                    jobs.begin(workspace, chart_id, scale, ctx)
                })?;
                Ok(McpReply::SnapshotStarted(started))
            }
            McpCommand::PollSnapshot { job_id } => {
                let mut jobs = self
                    .mcp_snapshots
                    .lock()
                    .map_err(|_| snapshot_unavailable())?;
                jobs.poll(job_id).map(McpReply::SnapshotProgress)
            }
            McpCommand::GetReplay { workspace_id } => self
                .with_mcp_workspace(workspace_id, |workspace| {
                    Ok(McpReply::Replay(mcp::replay::replay_state(workspace)))
                }),
            McpCommand::ControlReplay {
                workspace_id,
                command,
            } => self.with_mcp_workspace(workspace_id, |workspace| {
                mcp::replay::control_replay(workspace, command).map(McpReply::Replay)
            }),
            McpCommand::RemoveIndicator {
                workspace_id,
                chart_id,
                indicator,
            } => self.with_mcp_workspace(workspace_id, |workspace| {
                mcp::chart::remove_indicator(workspace, chart_id, indicator)
                    .map(McpReply::IndicatorApplied)
            }),
            McpCommand::GetIndicatorValues {
                workspace_id,
                chart_id,
                request,
            } => self.with_mcp_workspace(workspace_id, |workspace| {
                mcp::chart::indicator_readings(workspace, chart_id, &request)
                    .map(|result| McpReply::IndicatorValues(Box::new(result)))
            }),
            McpCommand::GetSessionStats {
                workspace_id,
                chart_id,
                query,
            } => self.with_mcp_workspace(workspace_id, |workspace| {
                mcp::chart::session_statistics(workspace, chart_id, &query)
                    .map(|result| McpReply::SessionStats(Box::new(result)))
            }),
            McpCommand::AddChart {
                workspace_id,
                instrument,
            } => self.with_mcp_workspace(workspace_id, |workspace| {
                Ok(McpReply::ChartAdded(mcp::chart::add_chart(
                    workspace,
                    &instrument,
                    ctx,
                )))
            }),
        }
    }

    /// The stored script alert subscriptions, for checks that must not disturb a live alert.
    fn pine_subscriptions_for_mcp(
        &self,
    ) -> Result<Vec<crate::model::alert::pine::PineAlertSubscription>, McpError> {
        let store = self.store.lock().map_err(|_| alerts_unavailable())?;
        Ok(store.pine_subscriptions())
    }

    /// Hands a change to the alert runtime through the command queue, as the Alerts window does.
    /// Returns the command's id, which [`McpCommand::AlertCommandOutcome`] can be asked about.
    fn queue_alert_command(
        &self,
        ctx: &egui::Context,
        payload: AlertRuntimeCommandPayload,
        precondition: Option<AlertPrecondition>,
    ) -> Result<u64, McpError> {
        let mut command = AlertRuntimeCommand::new(payload, now_ms());
        command.precondition = precondition;
        let store = self.store.lock().map_err(|_| alerts_unavailable())?;
        store
            .enqueue_alert_command(&command)
            .map_err(|_| McpError::AppUnavailable("Could not queue the alert change".into()))?;
        ctx.request_repaint();
        Ok(command.id)
    }

    /// Runs `action` on the chart workspace with this id. Portfolio and Pine Studio tabs share
    /// the id space but are not chart workspaces.
    fn with_mcp_workspace<R>(
        &self,
        workspace_id: u64,
        action: impl FnOnce(&mut ChartWorkspace) -> Result<R, McpError>,
    ) -> Result<R, McpError> {
        let not_found = || McpError::NotFound(format!("No workspace {workspace_id}"));
        for tab in &self.tabs {
            let mut content = tab
                .content
                .lock()
                .map_err(|_| McpError::AppUnavailable("The workspace is unavailable".into()))?;
            match &mut *content {
                TabContent::Chart(workspace) if workspace.id() == workspace_id => {
                    return action(workspace);
                }
                TabContent::Portfolio(tab) if tab.id() == workspace_id => {
                    return Err(McpError::InvalidArgument(format!(
                        "{workspace_id} is a portfolio tab, not a chart workspace"
                    )));
                }
                TabContent::Pine(manager) if manager.id() == workspace_id => {
                    return Err(McpError::InvalidArgument(format!(
                        "{workspace_id} is the Pine Studio, not a chart workspace"
                    )));
                }
                _ => {}
            }
        }
        Err(not_found())
    }
}

use super::AppAction;
use super::DegenViewApp;
use super::alert::commit_price_alert;
use super::now_ms;
use super::tabs::TabContent;
use super::window::alerts_viewport_id;
use super::window::raise_viewport;
use crate::host::settings::SettingsHost;
use crate::host::tab::window::TabRequest;
use crate::model::alert::pine::{PineAlertState, PineAlertSubscription};
use crate::model::alert::{AlertRuntimeCommand, AlertRuntimeCommandPayload};
use crate::model::watchlist::new_id;
use crate::native_bridge::{self, NativeCommand};
use crate::pine_studio::ExitReason;
use crate::pine_studio::editor::EditAction;
use crate::store::scripts::hash_source;
use crate::ui::window::tab_bar;
use eframe::egui;
use std::sync::atomic::Ordering;

impl DegenViewApp {
    pub(super) fn process_commands(&mut self, ctx: &egui::Context) {
        native_bridge::set_edit_menu(
            self.with_focused_pine_studio(|pine| pine.edit_menu())
                .unwrap_or_default(),
        );
        native_bridge::set_new_script_enabled(
            self.with_focused_pine_studio(|pine| pine.can_create_script())
                .unwrap_or(false),
        );
        for command in native_bridge::take_commands() {
            match command {
                NativeCommand::NewScript => {
                    self.with_focused_pine_studio(|pine| pine.new_script());
                    ctx.request_repaint();
                }
                NativeCommand::EditCut
                | NativeCommand::EditCopy
                | NativeCommand::EditPaste
                | NativeCommand::GoToDefinition
                | NativeCommand::RenameSymbol => {
                    let action = match command {
                        NativeCommand::EditCut => EditAction::Cut,
                        NativeCommand::EditCopy => EditAction::Copy,
                        NativeCommand::EditPaste => EditAction::Paste,
                        NativeCommand::GoToDefinition => EditAction::GoToDefinition,
                        _ => EditAction::Rename,
                    };
                    self.with_focused_pine_studio(|pine| pine.edit_command(action));
                    ctx.request_repaint();
                }
                NativeCommand::NewTab => {
                    let window = self.tabs.windows().focused_window();
                    self.add_tab(window);
                }
                NativeCommand::CloseTab => {
                    if let Some(tab) = self.tabs.windows().focused_tab() {
                        self.tabs.request_close(ctx, tab);
                    }
                }
                NativeCommand::MoveTabToNewWindow => self.tabs.windows().detach_active(),
                NativeCommand::MergeAllWindows => self.tabs.windows().merge_all(),
                NativeCommand::SelectNextTab => {
                    self.tabs.windows().cycle(true);
                }
                NativeCommand::SelectPreviousTab => {
                    self.tabs.windows().cycle(false);
                }
                NativeCommand::OpenSettings => self.open_settings(ctx),
                NativeCommand::OpenPineStudio => self.open_pine(ctx),
                NativeCommand::OpenPortfolio => self.open_portfolio(ctx),
                NativeCommand::OpenDocs => {
                    ctx.open_url(egui::OpenUrl::new_tab(tab_bar::DOCS_URL));
                }
                NativeCommand::OpenGitHub => {
                    ctx.open_url(egui::OpenUrl::new_tab(tab_bar::GITHUB_URL));
                }
                NativeCommand::OpenCoffee => {
                    ctx.open_url(egui::OpenUrl::new_tab(tab_bar::COFFEE_URL));
                }
                NativeCommand::Quit => self.request_quit(ctx),
                NativeCommand::SaveCurrentLayout => {
                    let focused = self.tabs.focused_tab();
                    // File ▸ Save (⌘S) belongs to the focused tab: a chart saves its layout,
                    // the Pine Studio saves the script open in its editor.
                    for tab in self.tabs.slots() {
                        let Ok(mut content) = tab.content.lock() else {
                            continue;
                        };
                        match &mut *content {
                            TabContent::Chart(workspace) if workspace.id() == focused => {
                                workspace.save_layout(ctx);
                                break;
                            }
                            TabContent::Pine(pine) if pine.id() == focused => {
                                pine.request_save();
                                ctx.request_repaint();
                                break;
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        let actions = self
            .actions
            .lock()
            .map(|mut actions| std::mem::take(&mut *actions))
            .unwrap_or_default();
        for action in actions {
            match action {
                AppAction::Quit => self.request_quit(ctx),
                AppAction::QuitDecided { proceed } => {
                    if proceed {
                        // The next script window with edits asks in turn; none left, and it quits.
                        self.request_quit(ctx);
                    } else {
                        self.call_off_quit();
                    }
                }
                AppAction::OpenPortfolio => self.open_portfolio(ctx),
                AppAction::OpenPine => self.open_pine(ctx),
                AppAction::OpenPineSource(target) => self.open_pine_source(ctx, &target),
                AppAction::OpenAlpacaSettings => {
                    self.open_settings_page(ctx, SettingsHost::open_alpaca);
                }
                AppAction::OpenBrokerSettings => {
                    self.open_settings_page(ctx, SettingsHost::open_brokers);
                }
                AppAction::OpenAiSettings => {
                    self.open_settings_page(ctx, SettingsHost::open_ai);
                }
                AppAction::OpenSnapshotSettings => {
                    self.open_settings_page(ctx, SettingsHost::open_snapshots);
                }
                AppAction::OpenAlerts => {
                    if self.alerts_open.swap(true, Ordering::AcqRel) {
                        raise_viewport(ctx, alerts_viewport_id());
                    }
                }
                AppAction::SavePriceAlert { alert, baseline } => {
                    if let Ok(store) = self.store.lock() {
                        commit_price_alert(
                            &store,
                            &self.notifier,
                            &self.background_service,
                            &self.alert_runtime_control,
                            alert,
                            baseline,
                            now_ms(),
                        );
                    }
                }
                AppAction::SetAlertState { id, state } => {
                    if let Ok(store) = self.store.lock() {
                        let _ = store.enqueue_alert_command(&AlertRuntimeCommand::new(
                            AlertRuntimeCommandPayload::SetState {
                                id,
                                state,
                                baseline: None,
                            },
                            now_ms(),
                        ));
                    }
                }
                AppAction::MoveAlert { alert, baseline } => {
                    if let Ok(store) = self.store.lock() {
                        let _ = store.enqueue_alert_command(&AlertRuntimeCommand::new(
                            AlertRuntimeCommandPayload::Save { alert, baseline },
                            now_ms(),
                        ));
                    }
                }
                AppAction::DeleteAlert(id) => {
                    if let Ok(store) = self.store.lock() {
                        let _ = store.enqueue_alert_command(&AlertRuntimeCommand::new(
                            AlertRuntimeCommandPayload::Delete(id),
                            now_ms(),
                        ));
                    }
                }
                AppAction::OpenWebhookSettings => {
                    if let Ok(mut host) = self.settings_host.lock() {
                        host.open_webhooks();
                    }
                }
                AppAction::OpenWatchlistInNewTab(item) => self.add_watchlist_tab(item, ctx),
                AppAction::AddPineScript {
                    script_id,
                    name,
                    source,
                } => {
                    for tab in self.tabs.slots() {
                        if let Ok(mut content) = tab.content.lock()
                            && let TabContent::Chart(workspace) = &mut *content
                        {
                            workspace.add_pine_on_focused(script_id.clone(), &name, &source);
                            break;
                        }
                    }
                }
                AppAction::CreatePineAlert {
                    chart_id,
                    instance_id,
                    script_id,
                    script_name,
                    source,
                    inputs,
                    symbol_key,
                    timeframe,
                    note,
                    webhook_endpoint_ids,
                } => {
                    let source_hash = hash_source(&source);
                    let subscription = PineAlertSubscription {
                        id: new_id(),
                        chart_id,
                        instance_id: Some(instance_id),
                        script_id,
                        script_name,
                        source,
                        inputs,
                        symbol_key,
                        timeframe,
                        source_hash,
                        note,
                        state: PineAlertState::Active,
                        created_at_ms: now_ms(),
                        webhook_endpoint_ids,
                    };
                    if let Ok(store) = self.store.lock() {
                        let mut subscriptions = store.pine_subscriptions();
                        subscriptions.push(subscription);
                        let _ = store.replace_pine_subscriptions(&subscriptions);
                        // The runtime host reads its own copy; without this a new alert
                        // would stay silent until the next launch.
                        if let Ok(mut host) = self.pine_host.lock() {
                            host.reload(&store);
                        }
                    }
                }
            }
        }
        for request in self.tabs.windows().take_requests() {
            match request {
                TabRequest::New(window) => self.add_tab(Some(window)),
                TabRequest::Close(tab) => self.tabs.request_close(ctx, tab),
                TabRequest::OpenSettings => self.open_settings(ctx),
                TabRequest::CloseWindow(window) => {
                    for tab in self.tabs.windows().tabs_of(window) {
                        self.tabs.request_close(ctx, tab);
                    }
                }
            }
        }
        self.tabs.drop_closed(ctx);
    }

    pub(super) fn request_quit(&mut self, ctx: &egui::Context) {
        let asking =
            self.tabs
                .slots()
                .iter()
                .find_map(|tab| match &mut *tab.content.lock().ok()? {
                    TabContent::Pine(pine) => pine.begin_exit(ExitReason::Quit).then(|| pine.id()),
                    _ => None,
                });
        match asking {
            Some(id) => self.tabs.show(ctx, id),
            None => ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Close),
        }
    }

    /// Cancel or a failed save: the quit stays off and no window keeps asking.
    fn call_off_quit(&mut self) {
        // A restart for an update was waiting on this quit; the app keeps running, so it must not
        // start the new version when it exits some other time.
        self.updates.disarm_relaunch();
        for tab in self.tabs.slots() {
            if let Ok(mut content) = tab.content.lock()
                && let TabContent::Pine(pine) = &mut *content
            {
                pine.cancel_exit();
            }
        }
    }
}

//! Settings ▸ Network: whether DegenView follows the system's proxy, uses one typed here, or
//! connects directly. The choices are the local `net.proxy` setting and save as they are typed
//! (`Autosave`); the proxy's password goes to the secure storage through the host. Every rule
//! (what may be saved, what the status line says) is a plain method tested without a window.

use crate::service::keychain::proxy_password::ProxyPasswordStore;
use crate::service::net::proxy::{Active, Protocol, ProxyConfig, ProxyMode, Source, Status};
use crate::ui::icons::Icon;
use crate::ui::kit::{Autosave, Tone, destructive_button, field};
use crate::ui::settings::card::StatusMessage;
use crate::ui::settings::window::{card, section, status_message};
use crate::ui::{kit, theme};
use eframe::egui;

/// What the proxy's password is to become.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PasswordChange {
    Keep,
    Set(String),
    Remove,
}

/// What the page asks the host to do.
#[derive(Clone, Debug, PartialEq)]
pub enum NetworkAction {
    Save {
        config: ProxyConfig,
        password: PasswordChange,
    },
}

pub struct NetworkInput<'a> {
    /// The saved setting.
    pub config: &'a ProxyConfig,
    /// The proxy now in force.
    pub status: &'a Status,
}

#[derive(Default)]
pub struct NetworkState {
    loaded: bool,
    /// The saved setting the draft was last read from; read again when it changes under it.
    seen: ProxyConfig,
    /// The setting being edited, saved as it is typed.
    draft: ProxyConfig,
    port_text: String,
    password_input: String,
    removing_password: bool,
    /// The last result worth a line: a password the secure storage refused, say.
    pub status: Option<StatusMessage>,
    autosave: Autosave,
}

impl NetworkState {
    fn load(&mut self, saved: &ProxyConfig) {
        if !self.loaded || self.seen != *saved {
            self.loaded = true;
            self.seen = saved.clone();
            self.draft = saved.clone();
            self.port_text = saved.port.map(|port| port.to_string()).unwrap_or_default();
        }
    }

    /// The port as typed; blank is none, and anything else must be a port.
    fn parse_port(text: &str) -> Result<Option<u16>, ()> {
        let text = text.trim();
        if text.is_empty() {
            return Ok(None);
        }
        text.parse::<u16>()
            .ok()
            .filter(|port| *port > 0)
            .map(Some)
            .ok_or(())
    }

    /// Why the draft is not saved, if it is not. A manual proxy with nothing typed yet is not
    /// an error, only not ready (`awaiting_server`).
    pub fn problem(&self) -> Option<String> {
        // The port field is on the page only for a manual proxy; elsewhere a leftover typo must not
        // hold up saving the mode.
        if self.draft.mode == ProxyMode::Manual && Self::parse_port(&self.port_text).is_err() {
            return Some("The port is a number from 1 to 65535.".to_owned());
        }
        if self.awaiting_server() {
            return None;
        }
        self.current()
            .validate()
            .err()
            .map(|error| error.to_string())
    }

    /// Manual, and neither host nor port typed: the fields are waiting, not wrong.
    fn awaiting_server(&self) -> bool {
        self.draft.mode == ProxyMode::Manual
            && self.draft.host.trim().is_empty()
            && self.port_text.trim().is_empty()
    }

    /// Whether the draft can be saved: valid, and not an unfinished manual proxy.
    fn can_save(&self) -> bool {
        !self.awaiting_server() && self.problem().is_none()
    }

    /// The draft with the typed port.
    fn current(&self) -> ProxyConfig {
        ProxyConfig {
            port: Self::parse_port(&self.port_text).unwrap_or(None),
            ..self.draft.clone()
        }
    }

    /// The save to ask for, or `None` while the draft cannot be saved.
    pub fn save(&mut self) -> Option<NetworkAction> {
        if !self.can_save() {
            return None;
        }
        let mut config = self.current();
        config.host = config.host.trim().to_owned();
        config.username = config.username.trim().to_owned();
        let password = if self.removing_password {
            PasswordChange::Remove
        } else if self.password_input.is_empty() {
            PasswordChange::Keep
        } else {
            PasswordChange::Set(self.password_input.clone())
        };
        self.password_input.clear();
        self.removing_password = false;
        Some(NetworkAction::Save { config, password })
    }
}

/// The line under the segmented control, for what the chosen mode does.
pub fn mode_hint(mode: ProxyMode) -> &'static str {
    match mode {
        ProxyMode::System if cfg!(windows) => {
            "Follows the HTTP_PROXY and HTTPS_PROXY variables, then the proxy in Windows Settings ▸ Network & internet ▸ Proxy."
        }
        ProxyMode::System if cfg!(target_os = "macos") => {
            "Follows the HTTP_PROXY and HTTPS_PROXY variables, then the proxy in System Settings ▸ Network."
        }
        ProxyMode::System => "Follows the HTTP_PROXY, HTTPS_PROXY and ALL_PROXY variables.",
        ProxyMode::Manual => "Uses the server below for prices, news, updates and live feeds.",
        ProxyMode::Off => "Connects directly, whatever the system or the environment says.",
    }
}

/// The badge at the head of the page: what the app is doing now.
pub fn badge(status: &Status) -> (String, Tone) {
    match (&status.active, status.mode) {
        (Some(Active { source, server }), _) => (
            format!("Using {server} from {}", source_name(*source)),
            Tone::Good,
        ),
        (None, ProxyMode::Off) => ("Proxy off".to_owned(), Tone::Neutral),
        (None, ProxyMode::Manual) => ("No proxy server set".to_owned(), Tone::Neutral),
        (None, ProxyMode::System) => ("Connecting directly".to_owned(), Tone::Neutral),
    }
}

fn source_name(source: Source) -> &'static str {
    match source {
        Source::Manual => "this page",
        Source::Environment => "the environment",
        Source::System => "the system",
    }
}

fn caption(ui: &mut egui::Ui, text: &str) {
    ui.add(
        egui::Label::new(kit::text(
            text,
            kit::size::CAPTION,
            kit::REGULAR,
            theme::text_secondary(),
        ))
        .wrap(),
    );
}

pub fn body(
    ui: &mut egui::Ui,
    state: &mut NetworkState,
    input: &NetworkInput,
    actions: &mut Vec<NetworkAction>,
) {
    state.load(input.config);
    let (text, tone) = badge(input.status);
    kit::status_badge(ui, &text, tone);
    ui.add_space(18.0);

    section(ui, "Proxy", None, |ui| {
        card(ui, |ui| {
            let options = [
                (ProxyMode::System, "System"),
                (ProxyMode::Manual, "Manual"),
                (ProxyMode::Off, "Off"),
            ];
            let mut mode = state.draft.mode;
            kit::segmented(ui, egui::Id::new("network-mode"), &options, &mut mode);
            if mode != state.draft.mode {
                state.draft.mode = mode;
                state.autosave.touch();
            }
            caption(ui, mode_hint(mode));
        });
    });
    ui.add_space(18.0);

    if state.draft.mode == ProxyMode::Manual {
        manual_server(ui, state);
        ui.add_space(18.0);
        sign_in(ui, state);
        ui.add_space(18.0);
    }

    if state.draft.mode != ProxyMode::Off {
        section(ui, "Skip the proxy", None, |ui| {
            card(ui, |ui| {
                let edit = ui
                    .vertical(|ui| {
                        field(
                            ui,
                            "Hosts that connect directly",
                            Some(
                                "Separated by commas. A name also covers its subdomains; use 192.168.1.* for a block of addresses, or <local> for every name without a dot. Local addresses (localhost, 10.*, 192.168.*, 172.16-31.*, .local) never use the proxy.",
                            ),
                            "example.com, *.corp.test",
                            &mut state.draft.no_proxy,
                            false,
                        )
                    })
                    .inner;
                state.autosave.watch(&edit, false);
            });
        });
        ui.add_space(18.0);
    }

    if let Some(problem) = state.problem() {
        status_message(ui, &problem, true);
        ui.add_space(8.0);
    } else if let Some(problem) = input.status.problem {
        status_message(ui, &problem.to_string(), true);
        ui.add_space(8.0);
    } else if let Some(status) = &state.status {
        status_message(ui, &status.text, status.is_error);
        ui.add_space(8.0);
    }
    if state.autosave.due(ui.ctx()) {
        match state.save() {
            Some(action) => actions.push(action),
            None => state.autosave.cancel(),
        }
    }

    kit::notice_card(
        ui,
        Icon::InfoCircle,
        theme::text_secondary(),
        "What goes through the proxy",
        Some(
            "Market data, live price feeds, updates, link previews, webhooks, AI requests and chart sharing. Connections to a broker for live trading connect directly for now.",
        ),
        None,
    );
}

fn manual_server(ui: &mut egui::Ui, state: &mut NetworkState) {
    section(ui, "Server", None, |ui| {
        card(ui, |ui| {
            let options = [(Protocol::Http, "HTTP"), (Protocol::Socks5, "SOCKS5")];
            let mut protocol = state.draft.protocol;
            kit::segmented(
                ui,
                egui::Id::new("network-protocol"),
                &options,
                &mut protocol,
            );
            if protocol != state.draft.protocol {
                state.draft.protocol = protocol;
                state.autosave.touch();
            }
            let hint = match protocol {
                Protocol::Http => "An HTTP proxy that allows CONNECT, as most do.",
                Protocol::Socks5 => "A SOCKS5 proxy; it looks up host names itself.",
            };
            caption(ui, hint);
            let host = ui
                .vertical(|ui| field(ui, "Host", None, "127.0.0.1", &mut state.draft.host, false))
                .inner;
            state.autosave.watch(&host, false);
            let port = ui
                .vertical(|ui| field(ui, "Port", None, "7890", &mut state.port_text, false))
                .inner;
            state.autosave.watch(&port, false);
            if state.awaiting_server() {
                caption(ui, "Enter the server's host and port to start using it.");
            }
        });
    });
}

fn sign_in(ui: &mut egui::Ui, state: &mut NetworkState) {
    let store = ProxyPasswordStore::shared();
    let saved = store.is_saved() && !state.removing_password;
    section(
        ui,
        "Sign in",
        Some("Only if the proxy asks for a user name and password."),
        |ui| {
            card(ui, |ui| {
                let user = ui
                    .vertical(|ui| {
                        field(ui, "User name", None, "", &mut state.draft.username, false)
                    })
                    .inner;
                state.autosave.watch(&user, false);
                let (title, hint) = if saved {
                    ("Replace password", crate::platform::credential_saved_hint())
                } else {
                    ("Password", crate::platform::credential_stored_hint())
                };
                let password = ui
                    .vertical(|ui| {
                        field(ui, title, Some(hint), "", &mut state.password_input, true)
                    })
                    .inner;
                state.autosave.watch(&password, true);
                if saved && destructive_button(ui, "Remove password").clicked() {
                    state.removing_password = true;
                    state.autosave.touch();
                }
            });
        },
    );
}

#[cfg(test)]
mod tests;

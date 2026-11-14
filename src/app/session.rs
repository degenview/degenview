use super::DegenViewApp;
use super::tabs::{TabContent, TabState};
use crate::store::SharedStore;
use crate::workspace::PersistedWorkspace;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum PersistedSpecialTabKind {
    Portfolio,
    Pine,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(super) struct PersistedSpecialTab {
    pub(super) id: u64,
    pub(super) kind: PersistedSpecialTabKind,
}

/// Untagged keeps every existing chart-tab row byte-compatible. Special tabs have a required
/// `kind`, so they cannot be mistaken for the older `PersistedWorkspace` shape.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(untagged)]
pub(super) enum PersistedTab {
    Chart(PersistedWorkspace),
    Special(PersistedSpecialTab),
}

impl PersistedTab {
    pub(super) fn id(&self) -> u64 {
        match self {
            Self::Chart(workspace) => workspace.id,
            Self::Special(tab) => tab.id,
        }
    }
}

impl crate::store::Document for PersistedTab {
    fn document_id(&self) -> String {
        self.id().to_string()
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(super) struct PersistedSession {
    pub(super) next_id: u64,
    pub(super) tabs: Vec<PersistedTab>,
}

/// Why the session could not be saved last time, if it could not; every tab window shows it.
/// Cheap to clone; each window callback holds one.
#[derive(Clone, Default)]
pub(super) struct SessionSaveError(Arc<Mutex<Option<String>>>);

impl SessionSaveError {
    pub(super) fn message(&self) -> Option<String> {
        self.0.lock().ok().and_then(|error| error.clone())
    }

    fn set(&self, message: Option<String>) {
        if let Ok(mut error) = self.0.lock() {
            *error = message;
        }
    }
}

/// The saved session: when it was last written, what was written (so an unchanged one is not
/// written again), and why the last write failed.
pub(super) struct SessionState {
    error: SessionSaveError,
    last_saved_at: Instant,
    last_saved: Option<PersistedSession>,
}

impl SessionState {
    pub(super) fn new() -> Self {
        Self {
            error: SessionSaveError::default(),
            last_saved_at: Instant::now(),
            last_saved: None,
        }
    }

    pub(super) fn error(&self) -> &SessionSaveError {
        &self.error
    }

    /// Ten seconds have passed since the last save.
    pub(super) fn is_due(&self) -> bool {
        self.last_saved_at.elapsed() >= Duration::from_secs(10)
    }

    /// Writes `session` unless it is the one written last, and remembers why that failed if it
    /// did. A session that could not be read from the tabs is such a failure too.
    fn save(&mut self, store: &SharedStore, session: Result<PersistedSession, String>) {
        let result = session.and_then(|session| {
            if self.last_saved.as_ref() == Some(&session) {
                return Ok(session);
            }
            store
                .lock()
                .map_err(|error| error.to_string())
                .and_then(|store| {
                    store
                        .replace_documents("tab", &session.tabs)
                        .map_err(|error| error.to_string())?;
                    store
                        .set_setting("session.next_id", &session.next_id)
                        .map_err(|error| error.to_string())
                })?;
            Ok(session)
        });
        self.error.set(result.as_ref().err().cloned());
        if let Ok(session) = result {
            self.last_saved = Some(session);
        }
        self.last_saved_at = Instant::now();
    }
}

/// What the open tabs save as: each tab's content in window order, then strip order, so a restore
/// lays them out in the order they were in.
fn persisted_session(tabs: &TabState) -> Result<PersistedSession, String> {
    let tabs_in_order = tabs
        .in_window_order()
        .into_iter()
        .map(|tab| {
            let content = tab.content.lock().map_err(|error| error.to_string())?;
            Ok(match &*content {
                TabContent::Chart(workspace) => PersistedTab::Chart(workspace.persisted()),
                TabContent::Portfolio(portfolio) => PersistedTab::Special(PersistedSpecialTab {
                    id: portfolio.id(),
                    kind: PersistedSpecialTabKind::Portfolio,
                }),
                TabContent::Pine(pine) => PersistedTab::Special(PersistedSpecialTab {
                    id: pine.id(),
                    kind: PersistedSpecialTabKind::Pine,
                }),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(PersistedSession {
        next_id: tabs.next_id(),
        tabs: tabs_in_order,
    })
}

impl DegenViewApp {
    pub(super) fn save_session(&mut self) {
        self.session
            .save(&self.store, persisted_session(&self.tabs));
    }
}

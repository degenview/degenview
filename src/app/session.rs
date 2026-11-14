use super::DegenViewApp;
use super::tabs::TabContent;
use crate::workspace::PersistedWorkspace;
use serde::{Deserialize, Serialize};
use std::time::Instant;

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

impl DegenViewApp {
    fn persisted_session(&self) -> Result<PersistedSession, String> {
        // Window order, then strip order: a restore lays the tabs out in the order they were in.
        let tabs = self
            .tabs
            .in_window_order()
            .into_iter()
            .map(|tab| {
                let content = tab.content.lock().map_err(|error| error.to_string())?;
                Ok(match &*content {
                    TabContent::Chart(workspace) => PersistedTab::Chart(workspace.persisted()),
                    TabContent::Portfolio(portfolio) => {
                        PersistedTab::Special(PersistedSpecialTab {
                            id: portfolio.id(),
                            kind: PersistedSpecialTabKind::Portfolio,
                        })
                    }
                    TabContent::Pine(pine) => PersistedTab::Special(PersistedSpecialTab {
                        id: pine.id(),
                        kind: PersistedSpecialTabKind::Pine,
                    }),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(PersistedSession {
            next_id: self.tabs.next_id(),
            tabs,
        })
    }

    pub(super) fn save_session(&mut self) {
        let result = self.persisted_session().and_then(|session| {
            if self.last_saved_session.as_ref() == Some(&session) {
                return Ok(session);
            }
            self.store
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
        if let Ok(mut error) = self.session_save_error.lock() {
            *error = result.as_ref().err().cloned();
        }
        if let Ok(session) = result {
            self.last_saved_session = Some(session);
        }
        self.session_last_saved_at = Instant::now();
    }
}

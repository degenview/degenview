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
    /// Last, so a row this version reads never lands here.
    Unknown(UnknownTab),
}

/// A row another (newer) DegenView version wrote that this one cannot read. Kept verbatim, so
/// that version still finds it, instead of failing every save.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(super) struct UnknownTab {
    pub(super) id: u64,
    #[serde(flatten)]
    pub(super) rest: serde_json::Map<String, serde_json::Value>,
}

impl PersistedTab {
    pub(super) fn id(&self) -> u64 {
        match self {
            Self::Chart(workspace) => workspace.id,
            Self::Special(tab) => tab.id,
            Self::Unknown(tab) => tab.id,
        }
    }
}

/// The id the next new tab gets: above the saved counter and above every kept row, so a new tab
/// never shares a document id with a row this version cannot read.
pub(super) fn next_tab_id(saved_next_id: u64, kept: &[PersistedTab]) -> u64 {
    kept.iter()
        .map(|tab| tab.id().saturating_add(1))
        .fold(saved_next_id.max(2), u64::max)
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

#[derive(Default)]
struct SaveFailure {
    message: Option<String>,
    dismissed: bool,
}

/// Why the session could not be saved last time, if it could not; the focused window shows it
/// until it is dismissed. A different failure shows again. Cheap to clone; each window callback
/// holds one.
#[derive(Clone, Default)]
pub(super) struct SessionSaveError(Arc<Mutex<SaveFailure>>);

impl SessionSaveError {
    /// The failure to show: none once dismissed.
    pub(super) fn visible_message(&self) -> Option<String> {
        let failure = self.0.lock().ok()?;
        failure.message.clone().filter(|_| !failure.dismissed)
    }

    pub(super) fn dismiss(&self) {
        if let Ok(mut failure) = self.0.lock() {
            failure.dismissed = true;
        }
    }

    fn set(&self, message: Option<String>) {
        if let Ok(mut failure) = self.0.lock() {
            if message != failure.message {
                failure.dismissed = false;
            }
            failure.message = message;
        }
    }
}

/// The saved session: when it was last written, what was written (so an unchanged one is not
/// written again), and why the last write failed.
pub(super) struct SessionState {
    error: SessionSaveError,
    last_saved_at: Instant,
    last_saved: Option<PersistedSession>,
    /// The rows found at launch that this version cannot read, written back after the open tabs.
    kept: Vec<PersistedTab>,
}

impl SessionState {
    pub(super) fn new(kept: Vec<PersistedTab>) -> Self {
        Self {
            error: SessionSaveError::default(),
            last_saved_at: Instant::now(),
            last_saved: None,
            kept,
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
        let kept = &self.kept;
        let session = session.map(|mut session| {
            session.tabs.extend(kept.iter().cloned());
            session
        });
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;
    use serde_json::json;

    fn decode(row: serde_json::Value) -> PersistedTab {
        serde_json::from_value(row).expect("a tab row")
    }

    fn unknown(row: serde_json::Value) -> PersistedTab {
        let tab = decode(row);
        assert!(matches!(tab, PersistedTab::Unknown(_)), "{tab:?}");
        tab
    }

    fn portfolio(id: u64) -> PersistedTab {
        PersistedTab::Special(PersistedSpecialTab {
            id,
            kind: PersistedSpecialTabKind::Portfolio,
        })
    }

    #[test]
    fn a_row_from_a_newer_version_is_kept_verbatim() {
        for row in [
            json!({ "id": 4, "kind": "journal" }),
            json!({ "id": 5, "kind": "journal", "page": "stats", "filters": { "tags": ["a"] } }),
        ] {
            let tab = unknown(row.clone());
            assert_eq!(serde_json::to_value(&tab).unwrap(), row);
        }
    }

    #[test]
    fn rows_this_version_reads_are_never_unknown() {
        let chart = decode(json!({
            "id": 2, "timeframe": "Hour", "watchlist_visible": true,
            "charts": [], "columns": [], "focused": null
        }));
        assert!(matches!(chart, PersistedTab::Chart(_)));
        assert_eq!(
            decode(json!({ "id": 9, "kind": "portfolio" })),
            portfolio(9)
        );
        assert_eq!(
            decode(json!({ "id": 3, "kind": "pine" })),
            PersistedTab::Special(PersistedSpecialTab {
                id: 3,
                kind: PersistedSpecialTabKind::Pine,
            })
        );
    }

    #[test]
    fn a_new_tab_never_takes_a_kept_rows_id() {
        let kept = [unknown(json!({ "id": 4, "kind": "journal" }))];
        assert_eq!(next_tab_id(3, &kept), 5);
        assert_eq!(next_tab_id(9, &kept), 9);
        assert_eq!(next_tab_id(0, &[]), 2);
    }

    #[test]
    fn kept_rows_are_written_back_after_the_open_tabs() {
        let journal = json!({ "id": 4, "kind": "journal", "page": "stats" });
        let kept = vec![unknown(journal.clone())];
        let store: SharedStore = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
        store
            .lock()
            .unwrap()
            .replace_documents("tab", &kept)
            .unwrap();

        let mut state = SessionState::new(kept);
        let session = PersistedSession {
            next_id: 6,
            tabs: vec![portfolio(2), portfolio(3)],
        };
        state.save(&store, Ok(session.clone()));
        assert_eq!(state.error().visible_message(), None);

        let rows = store
            .lock()
            .unwrap()
            .documents_strict::<serde_json::Value>("tab")
            .unwrap();
        assert_eq!(
            rows,
            vec![
                json!({ "id": 2, "kind": "portfolio" }),
                json!({ "id": 3, "kind": "portfolio" }),
                journal.clone(),
            ]
        );

        // An unchanged session is not written again, and still leaves the row in place.
        state.save(&store, Ok(session));
        assert_eq!(state.error().visible_message(), None);
        assert_eq!(
            store
                .lock()
                .unwrap()
                .documents_strict::<serde_json::Value>("tab")
                .unwrap()
                .last(),
            Some(&journal)
        );
    }

    #[test]
    fn a_save_error_shows_until_dismissed_and_again_when_it_changes() {
        let error = SessionSaveError::default();
        assert_eq!(error.visible_message(), None);

        error.set(Some("disk full".into()));
        assert_eq!(error.visible_message().as_deref(), Some("disk full"));

        error.dismiss();
        assert_eq!(error.visible_message(), None);
        // The retry fails the same way: still dismissed.
        error.set(Some("disk full".into()));
        assert_eq!(error.visible_message(), None);

        error.set(Some("locked".into()));
        assert_eq!(error.visible_message().as_deref(), Some("locked"));

        error.dismiss();
        error.set(None);
        assert_eq!(error.visible_message(), None);
        // Cleared means a later failure shows even if it matches the dismissed one.
        error.set(Some("locked".into()));
        assert_eq!(error.visible_message().as_deref(), Some("locked"));
    }
}

//! What an MCP client asks the app to do, for the question the user answers before it runs.
//!
//! The same operation is told two ways. [`Operation::wire`] is English in every language: it is
//! what the client is told while its question is open and what the activity log keeps. The
//! user's own words, [`Operation::label`] and [`Operation::question`], follow the language in use.
//! The target and the note of a summary (what it applies to, what follows from it) are the
//! user's alone and are written in the language by whoever builds the summary.

use crate::format::plural;
use crate::i18n::{tr, tr_n};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    DeleteAlert,
    DeleteWatchlist,
    RemoveChart,
    DeleteDrawings(usize),
    RemoveScriptFromChart,
    SaveScript,
    PlaceOrder,
    ChangeOrder,
    CancelOrders,
    ClosePositions,
    ControlReplay,
    CreateAlert,
    ChangeAlert,
    PauseAlert,
    ResumeAlert,
    AddChart,
    AddIndicator,
    RemoveIndicator,
    DrawOnChart,
    ChangeDrawing,
    ChangeTimeframe,
    ChangeChartType,
    ApplyScript,
    ChangeScript,
    WriteScriptDraft,
    CreateWatchlist,
    ChangeWatchlist,
}

impl Operation {
    /// The English line a client is told and the log keeps, whatever language the app speaks.
    pub fn wire(self) -> String {
        match self {
            Self::DeleteAlert => "Delete a price alert".to_owned(),
            Self::DeleteWatchlist => "Delete a watchlist".to_owned(),
            Self::RemoveChart => "Remove a chart".to_owned(),
            Self::DeleteDrawings(count) => {
                format!("Delete {}", plural(count, "drawing", "drawings"))
            }
            Self::RemoveScriptFromChart => "Remove a script from a chart".to_owned(),
            Self::SaveScript => "Save a script over its saved version".to_owned(),
            Self::PlaceOrder => "Place a paper order".to_owned(),
            Self::ChangeOrder => "Change a paper order".to_owned(),
            Self::CancelOrders => "Cancel paper orders".to_owned(),
            Self::ClosePositions => "Close paper positions".to_owned(),
            Self::ControlReplay => "Control the replay".to_owned(),
            Self::CreateAlert => "Create a price alert".to_owned(),
            Self::ChangeAlert => "Change a price alert".to_owned(),
            Self::PauseAlert => "Pause a price alert".to_owned(),
            Self::ResumeAlert => "Resume a price alert".to_owned(),
            Self::AddChart => "Add a chart".to_owned(),
            Self::AddIndicator => "Add an indicator to a chart".to_owned(),
            Self::RemoveIndicator => "Remove an indicator from a chart".to_owned(),
            Self::DrawOnChart => "Draw on a chart".to_owned(),
            Self::ChangeDrawing => "Change a drawing".to_owned(),
            Self::ChangeTimeframe => "Change the timeframe".to_owned(),
            Self::ChangeChartType => "Change the chart type".to_owned(),
            Self::ApplyScript => "Apply a script to a chart".to_owned(),
            Self::ChangeScript => "Change a script on a chart".to_owned(),
            Self::WriteScriptDraft => "Write a script draft".to_owned(),
            Self::CreateWatchlist => "Create a watchlist".to_owned(),
            Self::ChangeWatchlist => "Change a watchlist".to_owned(),
        }
    }

    /// The action as a heading, for a notification: "Delete a price alert".
    pub fn label(self) -> String {
        match self {
            Self::DeleteAlert => tr!("Delete a price alert").to_owned(),
            Self::DeleteWatchlist => tr!("Delete a watchlist").to_owned(),
            Self::RemoveChart => tr!("Remove a chart").to_owned(),
            Self::DeleteDrawings(count) => {
                tr_n!(count, "Delete {count} drawing", "Delete {count} drawings")
            }
            Self::RemoveScriptFromChart => tr!("Remove a script from a chart").to_owned(),
            Self::SaveScript => tr!("Save a script over its saved version").to_owned(),
            Self::PlaceOrder => tr!("Place a paper order").to_owned(),
            Self::ChangeOrder => tr!("Change a paper order").to_owned(),
            Self::CancelOrders => tr!("Cancel paper orders").to_owned(),
            Self::ClosePositions => tr!("Close paper positions").to_owned(),
            Self::ControlReplay => tr!("Control the replay").to_owned(),
            Self::CreateAlert => tr!("Create a price alert").to_owned(),
            Self::ChangeAlert => tr!("Change a price alert").to_owned(),
            Self::PauseAlert => tr!("Pause a price alert").to_owned(),
            Self::ResumeAlert => tr!("Resume a price alert").to_owned(),
            Self::AddChart => tr!("Add a chart").to_owned(),
            Self::AddIndicator => tr!("Add an indicator to a chart").to_owned(),
            Self::RemoveIndicator => tr!("Remove an indicator from a chart").to_owned(),
            Self::DrawOnChart => tr!("Draw on a chart").to_owned(),
            Self::ChangeDrawing => tr!("Change a drawing").to_owned(),
            Self::ChangeTimeframe => tr!("Change the timeframe").to_owned(),
            Self::ChangeChartType => tr!("Change the chart type").to_owned(),
            Self::ApplyScript => tr!("Apply a script to a chart").to_owned(),
            Self::ChangeScript => tr!("Change a script on a chart").to_owned(),
            Self::WriteScriptDraft => tr!("Write a script draft").to_owned(),
            Self::CreateWatchlist => tr!("Create a watchlist").to_owned(),
            Self::ChangeWatchlist => tr!("Change a watchlist").to_owned(),
        }
    }

    /// The title of the dialog: "Allow “Claude” to delete a price alert?". One message per
    /// operation, since a language may need the client's name and the action in another order.
    pub fn question(self, client: &str) -> String {
        match self {
            Self::DeleteAlert => tr!("Allow “{client}” to delete a price alert?", client = client),
            Self::DeleteWatchlist => {
                tr!("Allow “{client}” to delete a watchlist?", client = client)
            }
            Self::RemoveChart => tr!("Allow “{client}” to remove a chart?", client = client),
            Self::DeleteDrawings(count) => tr_n!(
                count,
                "Allow “{client}” to delete {count} drawing?",
                "Allow “{client}” to delete {count} drawings?",
                client = client
            ),
            Self::RemoveScriptFromChart => tr!(
                "Allow “{client}” to remove a script from a chart?",
                client = client
            ),
            Self::SaveScript => tr!(
                "Allow “{client}” to save a script over its saved version?",
                client = client
            ),
            Self::PlaceOrder => tr!("Allow “{client}” to place a paper order?", client = client),
            Self::ChangeOrder => tr!("Allow “{client}” to change a paper order?", client = client),
            Self::CancelOrders => tr!("Allow “{client}” to cancel paper orders?", client = client),
            Self::ClosePositions => {
                tr!(
                    "Allow “{client}” to close paper positions?",
                    client = client
                )
            }
            Self::ControlReplay => tr!("Allow “{client}” to control the replay?", client = client),
            Self::CreateAlert => tr!("Allow “{client}” to create a price alert?", client = client),
            Self::ChangeAlert => tr!("Allow “{client}” to change a price alert?", client = client),
            Self::PauseAlert => tr!("Allow “{client}” to pause a price alert?", client = client),
            Self::ResumeAlert => tr!("Allow “{client}” to resume a price alert?", client = client),
            Self::AddChart => tr!("Allow “{client}” to add a chart?", client = client),
            Self::AddIndicator => tr!(
                "Allow “{client}” to add an indicator to a chart?",
                client = client
            ),
            Self::RemoveIndicator => tr!(
                "Allow “{client}” to remove an indicator from a chart?",
                client = client
            ),
            Self::DrawOnChart => tr!("Allow “{client}” to draw on a chart?", client = client),
            Self::ChangeDrawing => tr!("Allow “{client}” to change a drawing?", client = client),
            Self::ChangeTimeframe => {
                tr!("Allow “{client}” to change the timeframe?", client = client)
            }
            Self::ChangeChartType => {
                tr!(
                    "Allow “{client}” to change the chart type?",
                    client = client
                )
            }
            Self::ApplyScript => tr!(
                "Allow “{client}” to apply a script to a chart?",
                client = client
            ),
            Self::ChangeScript => tr!(
                "Allow “{client}” to change a script on a chart?",
                client = client
            ),
            Self::WriteScriptDraft => {
                tr!("Allow “{client}” to write a script draft?", client = client)
            }
            Self::CreateWatchlist => {
                tr!("Allow “{client}” to create a watchlist?", client = client)
            }
            Self::ChangeWatchlist => {
                tr!("Allow “{client}” to change a watchlist?", client = client)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{Locale, with_locale};

    const ALL: [Operation; 27] = [
        Operation::DeleteAlert,
        Operation::DeleteWatchlist,
        Operation::RemoveChart,
        Operation::DeleteDrawings(2),
        Operation::RemoveScriptFromChart,
        Operation::SaveScript,
        Operation::PlaceOrder,
        Operation::ChangeOrder,
        Operation::CancelOrders,
        Operation::ClosePositions,
        Operation::ControlReplay,
        Operation::CreateAlert,
        Operation::ChangeAlert,
        Operation::PauseAlert,
        Operation::ResumeAlert,
        Operation::AddChart,
        Operation::AddIndicator,
        Operation::RemoveIndicator,
        Operation::DrawOnChart,
        Operation::ChangeDrawing,
        Operation::ChangeTimeframe,
        Operation::ChangeChartType,
        Operation::ApplyScript,
        Operation::ChangeScript,
        Operation::WriteScriptDraft,
        Operation::CreateWatchlist,
        Operation::ChangeWatchlist,
    ];

    #[test]
    fn english_reads_as_it_always_did() {
        assert_eq!(Operation::DeleteAlert.wire(), "Delete a price alert");
        assert_eq!(Operation::DeleteDrawings(1).wire(), "Delete 1 drawing");
        assert_eq!(Operation::DeleteDrawings(3).wire(), "Delete 3 drawings");
        assert_eq!(Operation::DeleteDrawings(3).label(), "Delete 3 drawings");
        assert_eq!(
            Operation::DeleteAlert.question("Claude Desktop"),
            "Allow “Claude Desktop” to delete a price alert?"
        );
        assert_eq!(
            Operation::DeleteDrawings(1).question("Claude"),
            "Allow “Claude” to delete 1 drawing?"
        );
        for operation in ALL {
            let wire = operation.wire();
            assert_eq!(operation.label(), wire, "{operation:?}");
            let mut chars = wire.chars();
            let first = chars.next().map(|c| c.to_lowercase().to_string());
            let sentence = first
                .into_iter()
                .chain(chars.map(String::from))
                .collect::<String>();
            assert_eq!(
                operation.question("C"),
                format!("Allow “C” to {sentence}?"),
                "{operation:?}"
            );
        }
    }

    #[test]
    fn the_users_words_follow_the_language_and_the_wire_does_not() {
        for operation in ALL {
            let english = operation.wire();
            let (wire, label, question) = with_locale(Locale::PSEUDO, || {
                (operation.wire(), operation.label(), operation.question("C"))
            });
            assert_eq!(wire, english, "{operation:?} is told to clients in English");
            assert_ne!(label, english, "{operation:?} label is not translated");
            assert!(
                question.starts_with('['),
                "{operation:?} question is not translated"
            );
            assert!(question.contains('C'), "{operation:?} lost the client");
        }
    }
}

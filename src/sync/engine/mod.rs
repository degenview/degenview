pub mod account;
pub mod apply;
pub mod core;
pub mod initial;
pub mod merge;
pub mod outbox;

use crate::store::sync::RawDataError;

/// Why a sync step could not read or change this Mac's data. The messages are what the iCloud
/// Sync page shows after "Could not …:".
#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error(transparent)]
    Store(#[from] rusqlite::Error),
    /// A synced table or setting that is not JSON, or a write that would leave a row this version
    /// cannot read.
    #[error(transparent)]
    RawData(#[from] RawDataError),
    /// An account was bound before this database had a sync identity (`account::enable`).
    #[error("this Mac has not been set up for iCloud Sync")]
    NoSyncState,
    /// A file beside the database, such as the backup made before an account change.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// The script library would not keep the losing side of a conflict as a copy.
    #[error("could not keep the other version: {0}")]
    ConflictCopy(String),
}

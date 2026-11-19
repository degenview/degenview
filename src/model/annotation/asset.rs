//! A picture as an Image drawing keeps it: stored once by the hash of its bytes, and sent as one
//! synced record. Only the shape lives here; making one from a user's file and checking one from
//! another Mac is `service::annotation::ingest`.

use serde::{Deserialize, Serialize};

/// A picture as it is stored and as it syncs.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StoredAsset {
    /// The hash of the picture's bytes, in lower-case hex.
    pub id: String,
    pub mime: String,
    pub width: u32,
    pub height: u32,
    /// The bytes, base64.
    pub data: String,
}

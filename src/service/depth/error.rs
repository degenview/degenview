//! Why a look-up a depth feed needs before it can start did not give an answer.

use crate::service::net::http;

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum DepthError {
    #[error("{0}")]
    Http(#[from] http::Error),
    #[error("unexpected reply from the exchange: {0}")]
    Parse(&'static str),
    #[error("the exchange does not list {0}")]
    UnknownSymbol(String),
}

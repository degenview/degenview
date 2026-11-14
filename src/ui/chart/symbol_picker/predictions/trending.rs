//! The busiest prediction markets, shown while the Predictions tab's box is empty.

use crate::model::DataSource;
use crate::service::work_queue::{self, Pending};
use crate::service::{ProviderError, SearchResult};
use eframe::egui;
use std::time::{Duration, Instant};

/// Events listed under "Trending", and how long that list is reused.
const LIMIT: usize = 8;
const TTL: Duration = Duration::from_secs(600);
/// How long a failed fetch waits before the empty box asks again.
const RETRY: Duration = Duration::from_secs(30);

/// The provider that ranks its markets by activity; Kalshi cannot.
pub const PROVIDER: DataSource = DataSource::Polymarket;

type Fetch = fn(DataSource, usize) -> Result<Vec<SearchResult>, ProviderError>;

/// The held list, and the fetch under way to renew it.
pub struct Trending {
    fetch: Fetch,
    results: Vec<SearchResult>,
    /// When the list stops being fresh; `None` before the first answer.
    expires: Option<Instant>,
    inbox: Option<Pending<Result<Vec<SearchResult>, ProviderError>>>,
}

impl Trending {
    pub fn new(fetch: Fetch) -> Self {
        Self {
            fetch,
            results: Vec::new(),
            expires: None,
            inbox: None,
        }
    }

    pub fn results(&self) -> &[SearchResult] {
        &self.results
    }

    /// A fetch is under way and nothing is held yet.
    pub fn is_loading(&self) -> bool {
        self.inbox.is_some() && self.results.is_empty()
    }

    /// Reads a finished fetch and starts one when the held list is stale. A failure is quiet:
    /// trending is an extra, not something the user asked for.
    pub fn refresh(&mut self, ctx: &egui::Context) {
        if let Some(inbox) = &mut self.inbox
            && let Some(answer) =
                inbox.poll_or(|| Err(ProviderError::other("The trending list stopped loading.")))
        {
            self.inbox = None;
            let lifetime = match answer {
                Ok(results) => {
                    self.results = results;
                    TTL
                }
                Err(_) => RETRY,
            };
            self.expires = Some(Instant::now() + lifetime);
        }
        let fresh = self.expires.is_some_and(|expires| Instant::now() < expires);
        if self.inbox.is_some() || fresh {
            return;
        }
        let fetch = self.fetch;
        self.inbox = Some(work_queue::start_search(PROVIDER, Some(ctx), move || {
            fetch(PROVIDER, LIMIT)
        }));
    }
}

//! Reads what the TradingView desktop app has on this computer. It is read-only, offline and
//! narrow: of the app's HTTP cache only the responses with layouts, drawings, watchlists and the
//! account's own scripts are opened. Cookies and sign-in data are never touched, and addresses
//! (which carry access tokens) go no further than this module. See
//! `docs/tradingview-import.md`.

pub mod cache;
pub mod locate;
pub mod matcher;

use crate::i18n::tr;
use crate::model::tradingview::Library;
use crate::model::tradingview::parse::{self, Document, Fetched};
use crate::model::tradingview::placement::{Spot, WINDOW};
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    #[error("{}", tr!("The TradingView app’s data wasn’t found on this computer."))]
    NotInstalled,
    #[error("{}", tr!("This Flatpak can’t look for TradingView’s data by itself."))]
    Sandboxed,
    #[error("{}", tr!("This folder doesn’t hold TradingView’s data."))]
    NotACache,
    #[error("{}", unreadable(.0))]
    Unreadable(#[from] std::io::Error),
    #[error("{}", tr!("No layouts, watchlists or scripts were found. Open them in TradingView first, then try again."))]
    NothingFound,
}

impl ScanError {
    /// What to do about it.
    pub fn hint(&self) -> &'static str {
        match self {
            Self::Sandboxed => tr!(
                "Choose TradingView’s data folder: usually snap/tradingview/current/.config/TradingView or .config/TradingView in your home folder. Press Ctrl+H to see hidden folders."
            ),
            Self::NotInstalled | Self::NotACache | Self::Unreadable(_) | Self::NothingFound => tr!(
                "Install the TradingView desktop app, sign in, and open the layouts and watchlists you want to bring over."
            ),
        }
    }
}

/// Why no data folder was there to read.
fn nowhere(chosen: bool, sandboxed: bool) -> ScanError {
    match (chosen, sandboxed) {
        (true, _) => ScanError::NotACache,
        (false, true) => ScanError::Sandboxed,
        (false, false) => ScanError::NotInstalled,
    }
}

// Written where it is shown, in the language in use: the extractor reads a `tr!` whose values are
// plain expressions.
fn unreadable(error: &std::io::Error) -> String {
    tr!(
        "TradingView’s data couldn’t be read: {error}",
        error = error
    )
}

/// How the candles around a spot are got; `None` when they could not be.
pub type Candles = dyn Fn(&Spot) -> Option<Vec<crate::model::Kline>> + Send + Sync;

/// The candles around `spot`, from its market's own provider.
pub fn candles_around(spot: &Spot) -> Option<Vec<crate::model::Kline>> {
    let (start_ms, end_ms) = spot.range_ms();
    crate::service::fetch_klines_range(
        spot.source,
        &spot.symbol,
        spot.timeframe,
        start_ms,
        end_ms,
        2 * WINDOW as usize + 1,
    )
    .ok()
}

/// What was found, and where.
#[derive(Clone, Debug, PartialEq)]
pub struct Scan {
    pub folder: PathBuf,
    pub library: Library,
}

/// Reads the app's data: `chosen`, or where the app keeps it.
pub fn scan(chosen: Option<&Path>) -> Result<Scan, ScanError> {
    let folders = match chosen {
        Some(folder) => vec![folder.to_owned()],
        None => locate::data_folders(&locate::Environment::current()),
    };
    let mut documents = Vec::new();
    let mut used = None;
    // One folder that cannot be read does not lose what the others hold.
    let mut failed = None;
    for (number, folder) in folders.into_iter().enumerate() {
        let Some(dir) = locate::cache_in(&folder) else {
            continue;
        };
        let responses = match cache::read(&dir, &|url| route(url).is_some()) {
            Ok(responses) => responses,
            Err(error) => {
                failed = Some(error);
                continue;
            }
        };
        documents.extend(responses.into_iter().filter_map(|response| {
            let document = document(&response.url, response.body)?;
            Some(Fetched {
                at_ms: response.at_ms,
                folder: number,
                document,
            })
        }));
        used.get_or_insert(folder);
    }
    let Some(folder) = used else {
        return Err(match failed {
            Some(error) => error.into(),
            None => nowhere(
                chosen.is_some(),
                crate::service::system::sandbox::is_flatpak(),
            ),
        });
    };
    let library = parse::library(documents);
    if library.is_empty() {
        return Err(ScanError::NothingFound);
    }
    Ok(Scan { folder, library })
}

/// What a response holds, going by its address.
#[derive(Debug, Eq, PartialEq)]
enum Route {
    LayoutPage,
    Sources(String),
    /// Watchlists; `true` for the address that gives all of them.
    Watchlists(bool),
    Script(String),
    SavedScripts,
}

/// The only addresses read.
fn route(url: &str) -> Option<Route> {
    let rest = url.strip_prefix("https://")?;
    let (host, rest) = rest.split_once('/')?;
    let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
    let parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
    let id = |text: &str| {
        (!text.is_empty() && text.chars().all(|c| c.is_ascii_alphanumeric()))
            .then(|| text.to_owned())
    };
    // `www.tradingview.com` and the language sites such as `de.tradingview.com`.
    let site = host == "tradingview.com"
        || host
            .strip_suffix(".tradingview.com")
            .is_some_and(|sub| sub == "www" || sub.len() == 2);
    match (host, parts.as_slice()) {
        // The desktop app on Windows opens `/chart/`, the others `/chart/<layout>/`.
        (_, ["chart"]) if site => Some(Route::LayoutPage),
        (_, ["chart", layout]) if site => id(layout).map(|_| Route::LayoutPage),
        (_, ["api", "v1", "symbols_list", rest @ ..])
            if site && matches!(rest, ["all" | "custom" | "active"] | ["custom", _]) =>
        {
            Some(Route::Watchlists(matches!(rest, ["all"])))
        }
        (
            "charts-storage.tradingview.com",
            ["charts-storage", "get", "layout", layout, "sources"],
        ) => id(layout).map(Route::Sources),
        ("pine-facade.tradingview.com", ["pine-facade", "get", script, _]) => {
            let script = script.replace("%3B", ";").replace("%3b", ";");
            script.starts_with("USER;").then_some(Route::Script(script))
        }
        ("pine-facade.tradingview.com", ["pine-facade", "list"])
            if query.split('&').any(|pair| pair == "filter=saved") =>
        {
            Some(Route::SavedScripts)
        }
        _ => None,
    }
}

fn document(url: &str, body: Vec<u8>) -> Option<Document> {
    let text = String::from_utf8(body).ok()?;
    Some(match route(url)? {
        Route::LayoutPage => Document::LayoutPage { html: text },
        Route::Sources(layout) => Document::Sources { layout, json: text },
        Route::Watchlists(complete) => Document::Watchlists {
            json: text,
            complete,
        },
        Route::Script(id) => Document::Script { id, json: text },
        Route::SavedScripts => Document::SavedScripts { json: text },
    })
}

#[cfg(test)]
mod tests;

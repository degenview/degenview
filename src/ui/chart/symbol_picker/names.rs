//! The names the picker's markets are described with, loaded in the background.

use super::search::Backend;
use crate::model::DataSource;
use crate::model::market::asset_name::AssetNames;
use crate::service::SearchResult;
use crate::service::market::coin_index::CoinIndex;
use crate::service::market::names::{Directory, Listed};
use crate::service::work_queue::{self, Pending};
use eframe::egui;
use std::sync::Arc;

/// The names directory and the two loads that fill it. Both are only read once per picker; until
/// they answer, markets whose provider lists no names have none.
#[derive(Default)]
pub struct Names {
    pub directory: Directory,
    coins: Option<Pending<Arc<CoinIndex>>>,
    listed: Option<Pending<Arc<Listed>>>,
    asked: bool,
}

impl Names {
    /// The same names for a freshly opened picker, which asks again so a failed load is retried.
    pub fn reopened(&self) -> Self {
        Self {
            directory: self.directory.clone(),
            ..Self::default()
        }
    }

    /// Starts the loads on the first call and takes in what has answered.
    pub fn poll(&mut self, ctx: &egui::Context, backend: Backend) {
        if !self.asked {
            self.asked = true;
            let coins = backend.coin_names;
            self.coins = Some(work_queue::start_for(
                DataSource::CoinGecko,
                Some(ctx),
                coins,
            ));
            let listed = backend.listed_names;
            self.listed = Some(work_queue::start(Some(ctx), listed));
        }
        if let Some(coins) = self.coins.as_mut().and_then(Pending::poll)
            && !coins.is_empty()
        {
            self.directory.coins = coins;
        }
        if let Some(listed) = self.listed.as_mut().and_then(Pending::poll)
            && !listed.is_empty()
        {
            self.directory.listed = listed;
        }
    }

    /// Whether a load may still change the names, for a test to wait on.
    #[cfg(test)]
    pub fn is_loading(&self) -> bool {
        self.coins.as_ref().is_some_and(Pending::is_pending)
            || self.listed.as_ref().is_some_and(Pending::is_pending)
    }

    pub fn of(&self, result: &SearchResult) -> AssetNames {
        self.directory.names(result)
    }
}

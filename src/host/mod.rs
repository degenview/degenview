pub mod alert;
pub mod broker;
pub mod depth;
pub mod editor_theme;
pub mod language;
pub mod lease_map;
pub mod liquidation;
pub mod mcp;
pub mod memo;
pub mod paper;
pub mod pine;
pub mod portfolio;
pub mod settings;
pub mod snapshot;
pub mod sync;
pub mod tab;
pub mod taker_flow;
pub mod trading;
pub mod tradingview;
pub mod update;

use crate::service::stream::Repaint;
use eframe::egui;
use std::sync::Arc;

/// What a feed calls when its record changed: asks `ctx` for another frame.
pub fn repaint_of(ctx: &egui::Context) -> Repaint {
    let ctx = ctx.clone();
    Arc::new(move || ctx.request_repaint())
}

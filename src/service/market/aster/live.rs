//! Aster's combined kline stream: the symbols and interval are in the URL, nothing is sent.

use super::{Aster, SHAPE};
use crate::model::{Kline, Timeframe};
use crate::service::market::live::CandleProtocol;
use crate::service::market::shape::binance::parse_kline;
use crate::service::net::ws::{Frame, Session};

impl CandleProtocol for Aster {
    fn sessions(&self, symbols: &[String], base: Timeframe) -> Option<Vec<Session>> {
        Some(SHAPE.kline_sessions(symbols, base))
    }

    fn parse(&self, frame: Frame<'_>, _base: Timeframe) -> Option<(String, Kline)> {
        parse_kline(frame.text()?)
    }
}

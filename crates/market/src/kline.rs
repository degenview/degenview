/// One OHLCV bar. Times are Unix milliseconds; `close_time` is the last millisecond of the bar.
#[derive(Clone, Debug, PartialEq)]
pub struct Kline {
    pub open_time: i64,
    pub close_time: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

impl Kline {
    pub fn is_up(&self) -> bool {
        self.close >= self.open
    }

    /// Takes in a later candle of the same bar: the range widens, the close moves to its close and
    /// the volumes add up.
    pub fn absorb(&mut self, later: &Kline) {
        self.high = self.high.max(later.high);
        self.low = self.low.min(later.low);
        self.close = later.close;
        self.volume += later.volume;
    }
}

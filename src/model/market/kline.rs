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
}

use crate::service::market::ports::clock::Clock;

pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        crate::clock::now_ms()
    }
}

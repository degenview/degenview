//! The wall clock, in the one unit the app stores and compares times in.

/// Milliseconds since the Unix epoch; 0 on a clock set before 1970.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as i64)
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_clock_is_past_2024() {
        assert!(super::now_ms() > 1_704_067_200_000);
    }
}

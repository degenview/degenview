//! Asking a provider again after a failure that may pass.
//!
//! The HTTP layer already retries each request once; this is for a job made of many requests
//! (a range of candles paged in parallel), where one page failing for a moment fails the whole.

use super::error::ProviderError;
use std::time::Duration;

/// `operation`, run again after each of `delays` while it fails with a transient error
/// ([`ProviderError::is_transient`]). Any other failure, and the last one, is returned as it came.
pub fn while_transient<T>(
    delays: &[Duration],
    mut operation: impl FnMut() -> Result<T, ProviderError>,
) -> Result<T, ProviderError> {
    let mut delays = delays.iter();
    loop {
        match operation() {
            Err(error) if error.is_transient() => match delays.next() {
                Some(delay) => std::thread::sleep(*delay),
                None => return Err(error),
            },
            result => return result,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DataSource, Timeframe};
    use crate::service::net::http;

    const NO_WAIT: [Duration; 2] = [Duration::ZERO; 2];

    fn busy() -> ProviderError {
        ProviderError::from(http::Error::Status(503))
    }

    #[test]
    fn a_transient_failure_is_asked_again_until_it_passes() {
        let mut calls = 0;
        let answer = while_transient(&NO_WAIT, || {
            calls += 1;
            if calls < 3 { Err(busy()) } else { Ok(calls) }
        });
        assert_eq!(answer, Ok(3));
    }

    #[test]
    fn it_gives_up_after_the_last_delay_with_the_last_error() {
        let mut calls = 0;
        let answer: Result<(), _> = while_transient(&NO_WAIT, || {
            calls += 1;
            Err(busy())
        });
        assert_eq!(answer, Err(busy()));
        assert_eq!(calls, 3, "once, and again after each delay");
    }

    #[test]
    fn a_failure_that_will_not_pass_is_not_asked_again() {
        let mut calls = 0;
        let answer: Result<(), _> = while_transient(&NO_WAIT, || {
            calls += 1;
            Err(ProviderError::unsupported(
                DataSource::Gate,
                Timeframe::SECOND_1,
            ))
        });
        assert!(answer.is_err_and(|error| error.is_unsupported_interval()));
        assert_eq!(calls, 1);
    }

    #[test]
    fn a_first_answer_is_returned_at_once() {
        let mut calls = 0;
        let answer = while_transient(&NO_WAIT, || {
            calls += 1;
            Ok::<_, ProviderError>("candles")
        });
        assert_eq!(answer, Ok("candles"));
        assert_eq!(calls, 1);
    }
}

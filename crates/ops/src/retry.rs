//! Exponential backoff with jitter for 5xx and network errors (SPEC 5.4).

use std::time::Duration;

pub const MAX_ATTEMPTS: u32 = 5;

/// Whether an HTTP status (or its absence, for a network-level error)
/// warrants a retry. 5xx and network errors are retried; everything else
/// (including 4xx, which indicates a request problem retrying won't fix) is
/// not.
pub fn should_retry(status: Option<u16>) -> bool {
    match status {
        None => true,
        Some(status) => (500..600).contains(&status),
    }
}

/// Exponential backoff, capped at `cap`, with full jitter: the caller
/// supplies `jitter` in `[0.0, 1.0)` (typically from an RNG at the call
/// site), and the returned delay is `jitter * min(cap, base * 2^attempt)`.
/// Keeping the randomness at the call site makes this function pure and
/// deterministically testable.
pub fn backoff_delay(attempt: u32, base: Duration, cap: Duration, jitter: f64) -> Duration {
    let multiplier = 2u32.checked_pow(attempt).unwrap_or(u32::MAX);
    let exponential = base.checked_mul(multiplier).unwrap_or(cap).min(cap);
    exponential.mul_f64(jitter.clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retries_5xx_and_network_errors_only() {
        assert!(should_retry(None));
        assert!(should_retry(Some(500)));
        assert!(should_retry(Some(503)));
        assert!(!should_retry(Some(404)));
        assert!(!should_retry(Some(429)));
        assert!(!should_retry(Some(200)));
    }

    #[test]
    fn backoff_grows_exponentially_before_the_cap() {
        let base = Duration::from_millis(100);
        let cap = Duration::from_secs(60);
        assert_eq!(backoff_delay(0, base, cap, 1.0), Duration::from_millis(100));
        assert_eq!(backoff_delay(1, base, cap, 1.0), Duration::from_millis(200));
        assert_eq!(backoff_delay(2, base, cap, 1.0), Duration::from_millis(400));
    }

    #[test]
    fn backoff_is_capped() {
        let base = Duration::from_millis(100);
        let cap = Duration::from_secs(1);
        assert_eq!(backoff_delay(10, base, cap, 1.0), cap);
    }

    #[test]
    fn jitter_scales_the_delay_down() {
        let base = Duration::from_millis(100);
        let cap = Duration::from_secs(60);
        assert_eq!(backoff_delay(0, base, cap, 0.0), Duration::ZERO);
        assert_eq!(backoff_delay(0, base, cap, 0.5), Duration::from_millis(50));
    }
}

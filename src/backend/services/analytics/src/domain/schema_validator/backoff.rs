//! Exponential backoff for the startup retry loop.
//!
//! DESIGN §3.2: "retry the affected row(s) with exponential backoff" when
//! ClickHouse is unreachable. Cap at 5 min so a long outage doesn't push the
//! next attempt past the operator's patience window.

use std::time::Duration;

#[derive(Debug, Clone, Copy)]
pub struct BackoffPolicy {
    pub initial: Duration,
    pub max: Duration,
    pub multiplier: u32,
}

impl BackoffPolicy {
    /// Default production policy: 1s → 2s → 4s → 8s → … capped at 5 min.
    #[must_use]
    pub fn default_for_validator() -> Self {
        Self {
            initial: Duration::from_secs(1),
            max: Duration::from_mins(5),
            multiplier: 2,
        }
    }

    /// Next delay for `attempt` (0-indexed). Capped at `max`.
    #[must_use]
    pub fn next_delay(&self, attempt: u32) -> Duration {
        // Saturating arithmetic so a long-running outage with `attempt` in the
        // tens doesn't overflow the underlying `u64` seconds count.
        let base_millis = u64::try_from(self.initial.as_millis()).unwrap_or(u64::MAX);
        let multiplied =
            base_millis.saturating_mul(u64::from(self.multiplier).saturating_pow(attempt));
        let max_millis = u64::try_from(self.max.as_millis()).unwrap_or(u64::MAX);
        Duration::from_millis(multiplied.min(max_millis))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_delay_is_initial() {
        let p = BackoffPolicy::default_for_validator();
        assert_eq!(p.next_delay(0), Duration::from_secs(1));
    }

    #[test]
    fn delays_double_each_attempt() {
        let p = BackoffPolicy::default_for_validator();
        assert_eq!(p.next_delay(1), Duration::from_secs(2));
        assert_eq!(p.next_delay(2), Duration::from_secs(4));
        assert_eq!(p.next_delay(3), Duration::from_secs(8));
        assert_eq!(p.next_delay(8), Duration::from_secs(256));
    }

    #[test]
    fn capped_at_max() {
        let p = BackoffPolicy::default_for_validator();
        // 2^10 = 1024s > 300s cap → must clamp to 300s.
        assert_eq!(p.next_delay(10), Duration::from_mins(5));
        // Far past saturation — still capped, no overflow.
        assert_eq!(p.next_delay(64), Duration::from_mins(5));
        assert_eq!(p.next_delay(u32::MAX), Duration::from_mins(5));
    }
}

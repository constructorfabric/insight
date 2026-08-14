//! Shared `?limit=` handling for the list endpoints.

/// Clamp `?limit=` to `[1, max]`; negatives → 1, absent → `default` (parity
/// with the .NET `int?` clamp — a nonsense value never 400s the request).
pub(crate) fn clamp_limit(limit: Option<i64>, default: u64, max: u64) -> u64 {
    limit.map_or(default, |l| u64::try_from(l).unwrap_or(1).clamp(1, max))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_negative_and_oversized_limits_all_land_in_range() {
        for (limit, expected) in [
            (None, 50),
            (Some(-3), 1),
            (Some(0), 1),
            (Some(7), 7),
            (Some(9_000), 500),
        ] {
            assert_eq!(clamp_limit(limit, 50, 500), expected, "limit: {limit:?}");
        }
    }
}

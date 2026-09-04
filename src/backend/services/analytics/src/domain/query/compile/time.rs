//! The time bucket and the window every scan is bounded by. UTC only.

use crate::domain::query::contract::dto::Grain;
use crate::domain::query::datasets::declaration::TimeField;

/// ClickHouse's Monday-first week mode.
const MONDAY_FIRST: u8 = 1;

// SAFETY: an absent event time compares NULL against both bounds, so the row
// falls outside every window and the scan never reads it.
pub fn event_day(field: &TimeField) -> String {
    format!("toDate({})", field.field)
}

// SAFETY: `assumeNotNull` is sound because the window bounds already excluded
// every row whose event time is absent.
pub fn bucket_expr(field: &TimeField, grain: Grain) -> String {
    let day = if field.nullable {
        format!("toDate(assumeNotNull({}))", field.field)
    } else {
        format!("toDate({})", field.field)
    };

    match grain {
        Grain::Day => day,
        Grain::Week => format!("toStartOfWeek({day}, {MONDAY_FIRST})"),
        Grain::Month => format!("toStartOfMonth({day})"),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn field(nullable: bool) -> TimeField {
        TimeField {
            field: "authored_at".to_owned(),
            nullable,
            default: true,
        }
    }

    #[test]
    fn each_grain_renders_its_own_bucket_boundary() {
        let cases = [
            (Grain::Day, "toDate(authored_at)"),
            (Grain::Week, "toStartOfWeek(toDate(authored_at), 1)"),
            (Grain::Month, "toStartOfMonth(toDate(authored_at))"),
        ];
        for (grain, expected) in cases {
            assert_eq!(bucket_expr(&field(false), grain), expected, "{grain:?}");
        }
    }

    #[test]
    fn a_nullable_event_time_buckets_the_value_the_window_already_admitted() {
        assert_eq!(
            bucket_expr(&field(true), Grain::Day),
            "toDate(assumeNotNull(authored_at))"
        );
    }

    #[test]
    fn the_window_bounds_compare_against_the_event_day() {
        assert_eq!(event_day(&field(true)), "toDate(authored_at)");
    }
}

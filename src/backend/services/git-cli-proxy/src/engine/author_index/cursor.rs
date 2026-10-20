use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};

/// Where the next page of one change-feed walk resumes.
///
/// The high-water mark is fixed by the walk's first page, so a change
/// recorded while the caller pages is read by its next walk, never skipped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangesCursor {
    #[serde(rename = "h")]
    pub high_water_us: i64,
    #[serde(rename = "s", default)]
    pub since_us: Option<i64>,
    #[serde(rename = "a")]
    pub after_us: i64,
    #[serde(rename = "e")]
    pub after_email: String,
}

#[derive(Debug, thiserror::Error)]
#[error("malformed change-feed cursor")]
pub struct MalformedCursor;

impl ChangesCursor {
    #[must_use]
    pub fn encode(&self) -> String {
        // Serialising four plain fields cannot fail.
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(self).unwrap_or_default())
    }

    /// # Errors
    ///
    /// [`MalformedCursor`] when `raw` is not a cursor this service issued.
    pub fn decode(raw: &str) -> Result<Self, MalformedCursor> {
        let bytes = URL_SAFE_NO_PAD.decode(raw).map_err(|_| MalformedCursor)?;
        serde_json::from_slice(&bytes).map_err(|_| MalformedCursor)
    }
}

/// Microseconds since the UNIX epoch; saturates at zero on a pre-epoch clock.
pub(super) fn now_us() -> i64 {
    chrono::Utc::now().timestamp_micros().max(0)
}

/// A change stamp as the feed renders it: UTC, microsecond precision.
#[must_use]
pub fn format_stamp(stamp_us: i64) -> String {
    chrono::DateTime::from_timestamp_micros(stamp_us).map_or_else(
        || stamp_us.to_string(),
        |at| at.format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string(),
    )
}

/// An RFC 3339 instant — or the colon-less offset Python's `%z` renders — as
/// microseconds since the UNIX epoch.
#[must_use]
pub fn parse_instant_us(raw: &str) -> Option<i64> {
    let raw = raw.trim();
    chrono::DateTime::parse_from_rfc3339(raw)
        .or_else(|_| chrono::DateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S%.f%z"))
        .ok()
        .map(|at| at.timestamp_micros())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cursor_round_trips_and_a_foreign_one_is_refused() {
        let cursor = ChangesCursor {
            high_water_us: 1_700_000_000_000_001,
            since_us: Some(1_600_000_000_000_000),
            after_us: 1_700_000_000_000_000,
            after_email: "dev@example.com".to_owned(),
        };
        assert_eq!(ChangesCursor::decode(&cursor.encode()).ok(), Some(cursor));

        for raw in ["", "!!!", "e30", "bm90IGpzb24"] {
            assert!(ChangesCursor::decode(raw).is_err(), "must refuse: {raw:?}");
        }
    }

    #[test]
    fn a_stamp_renders_and_parses_back_to_the_same_microsecond() {
        let stamp = 1_767_225_600_000_007;
        let rendered = format_stamp(stamp);
        assert_eq!(rendered, "2026-01-01T00:00:00.000007Z");
        assert_eq!(parse_instant_us(&rendered), Some(stamp));
    }

    #[test]
    fn both_offset_spellings_name_the_same_instant() {
        assert_eq!(
            parse_instant_us("2026-01-01T02:00:00.000001+02:00"),
            parse_instant_us("2026-01-01T02:00:00.000001+0200")
        );
        assert_eq!(parse_instant_us("yesterday"), None);
    }
}

//! Flexible date-time parsing for request DTOs.
//!
//! The .NET API accepts OpenAPI `format: date-time` (RFC-3339 with `Z` or a
//! numeric offset). `sea_orm::prelude::DateTime` is `chrono::NaiveDateTime`,
//! whose serde parser rejects `Z`/offset forms, so a normal client value like
//! `2026-07-23T10:00:00Z` would 400 before the handler. These helpers accept the
//! same forms the .NET binder does — RFC-3339 with `Z`/offset, a zone-less
//! datetime (treated as UTC), or a date-only value (midnight UTC) — and
//! normalise everything to naive-UTC (the DB column type).

use chrono::{DateTime as ChronoDateTime, NaiveDate, NaiveDateTime, Utc};
use serde::{Deserialize, Deserializer};

/// Parse the accepted date-time forms to naive-UTC. Returns `None` if the string
/// matches none of them.
#[must_use]
pub(crate) fn parse_flexible(raw: &str) -> Option<NaiveDateTime> {
    if let Ok(dt) = ChronoDateTime::parse_from_rfc3339(raw) {
        return Some(dt.with_timezone(&Utc).naive_utc());
    }
    if let Ok(ndt) = NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S%.f") {
        return Some(ndt);
    }
    if let Ok(ndt) = NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S") {
        return Some(ndt);
    }
    NaiveDate::parse_from_str(raw, "%Y-%m-%d")
        .ok()
        .and_then(|d| d.and_hms_opt(0, 0, 0))
}

/// serde `deserialize_with` for an optional timestamp: accepts the RFC-3339 /
/// zone-less / date-only forms and normalises to naive-UTC. Absent / empty →
/// `None`; an unrecognised string is a deserialization error (surfaces as 400).
///
/// # Errors
///
/// Returns a deserialization error if the value is present but not a recognised
/// date-time.
pub(crate) fn deserialize_opt<'de, D>(deserializer: D) -> Result<Option<NaiveDateTime>, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = Option::<String>::deserialize(deserializer)?;
    match raw {
        None => Ok(None),
        Some(s) if s.trim().is_empty() => Ok(None),
        Some(s) => parse_flexible(s.trim())
            .map(Some)
            .ok_or_else(|| serde::de::Error::custom(format!("invalid date-time: '{s}'"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ymd_hms(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> anyhow::Result<NaiveDateTime> {
        use anyhow::Context;
        NaiveDate::from_ymd_opt(y, mo, d)
            .and_then(|date| date.and_hms_opt(h, mi, s))
            .context("valid datetime")
    }

    #[test]
    fn accepts_rfc3339_and_zoneless_and_date_only() -> anyhow::Result<()> {
        let utc10 = ymd_hms(2026, 7, 23, 10, 0, 0)?;
        assert_eq!(parse_flexible("2026-07-23T10:00:00Z"), Some(utc10), "Z");
        assert_eq!(
            parse_flexible("2026-07-23T10:00:00"),
            Some(utc10),
            "zone-less"
        );
        // +03:00 → 07:00 UTC
        assert_eq!(
            parse_flexible("2026-07-23T10:00:00+03:00"),
            Some(ymd_hms(2026, 7, 23, 7, 0, 0)?),
            "offset"
        );
        assert_eq!(
            parse_flexible("2026-07-23"),
            Some(ymd_hms(2026, 7, 23, 0, 0, 0)?),
            "date-only"
        );
        assert_eq!(parse_flexible("not-a-date"), None);
        Ok(())
    }

    #[derive(Deserialize)]
    struct OptWrapper {
        #[serde(default, deserialize_with = "deserialize_opt")]
        at: Option<NaiveDateTime>,
    }

    #[test]
    fn deserialize_opt_boundary_cases() -> anyhow::Result<()> {
        let missing: OptWrapper = serde_json::from_value(serde_json::json!({}))?;
        assert_eq!(missing.at, None, "missing field");

        let null: OptWrapper = serde_json::from_value(serde_json::json!({ "at": null }))?;
        assert_eq!(null.at, None, "null");

        let empty: OptWrapper = serde_json::from_value(serde_json::json!({ "at": "" }))?;
        assert_eq!(empty.at, None, "empty string");

        let blank: OptWrapper = serde_json::from_value(serde_json::json!({ "at": "   " }))?;
        assert_eq!(blank.at, None, "blank string");

        let valid: OptWrapper = serde_json::from_value(serde_json::json!({
            "at": "2026-07-23T10:00:00Z"
        }))?;
        assert_eq!(
            valid.at,
            Some(ymd_hms(2026, 7, 23, 10, 0, 0)?),
            "valid RFC-3339"
        );

        let invalid: Result<OptWrapper, _> =
            serde_json::from_value(serde_json::json!({ "at": "not-a-date" }));
        assert!(
            invalid.is_err(),
            "invalid string is a deserialization error"
        );
        Ok(())
    }
}

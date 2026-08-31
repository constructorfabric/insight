use serde::Serialize;

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    serde::Deserialize,
    Serialize,
    utoipa::ToSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Bucket {
    Day,
    Week,
    Month,
}

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    serde::Deserialize,
    Serialize,
    utoipa::ToSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum MetricResultViewKind {
    Period,
    Timeseries,
    Peer,
    Breakdown,
    Rollup,
    Histogram,
}

#[cfg(test)]
mod tests {
    use super::Bucket;

    #[test]
    fn public_bucket_rejects_internal_report_granularities() {
        for value in [r#""quarter""#, r#""year""#] {
            assert!(
                serde_json::from_str::<Bucket>(value).is_err(),
                "should reject: {value}"
            );
        }
    }
}

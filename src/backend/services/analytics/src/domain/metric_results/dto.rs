use serde::{Deserialize, Serialize};

use super::view::{Bucket, MetricResultViewKind};
use crate::domain::metric_definitions::{MetricDirection, MetricFormat};
use crate::domain::metric_drilldown::MetricDrilldownCapability;

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct MetricResultsRequest {
    pub entity: MetricResultsEntity,
    pub period: MetricResultsPeriod,
    /// A second window answered alongside `period` in the same response — the
    /// range a delta or a half-against-half comparison is measured against.
    /// Carried by the `period` and `breakdown` views only; every other view
    /// kind answers over `period` alone.
    pub compare_to: Option<MetricResultsPeriod>,
    pub metrics: Vec<MetricRequest>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum MetricResultsEntity {
    Person {
        ids: Vec<String>,
    },
    Tenant {},
    #[serde(other, skip_serializing)]
    Unknown,
}

impl MetricResultsEntity {
    pub(crate) fn is_tenant(&self) -> bool {
        match self {
            Self::Tenant {} => true,
            Self::Person { .. } | Self::Unknown => false,
        }
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct MetricResultsPeriod {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct MetricRequest {
    pub metric_key: String,
    #[serde(default)]
    pub filters: Vec<MetricDimensionFilterRequest>,
    pub views: Vec<MetricViewRequest>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct MetricDimensionFilterRequest {
    pub dimension: String,
    pub values: Vec<String>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct MetricGroupLimitRequest {
    pub count: usize,
    pub rank_by_metric: Option<String>,
    pub include_remainder: bool,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[serde(tag = "view", rename_all = "snake_case")]
pub enum MetricViewRequest {
    Period,
    Peer {
        cohort_key: Option<String>,
    },
    Timeseries {
        bucket: Option<Bucket>,
        #[serde(default)]
        dimensions: Vec<String>,
        group_limit: Option<MetricGroupLimitRequest>,
    },
    Breakdown {
        dimensions: Vec<String>,
    },
    Rollup {
        dimensions: Vec<String>,
        group_limit: Option<MetricGroupLimitRequest>,
    },
    Histogram {
        #[serde(default)]
        dimensions: Vec<String>,
    },
}

impl MetricViewRequest {
    pub fn kind(&self) -> MetricResultViewKind {
        match self {
            Self::Period => MetricResultViewKind::Period,
            Self::Peer { .. } => MetricResultViewKind::Peer,
            Self::Timeseries { .. } => MetricResultViewKind::Timeseries,
            Self::Breakdown { .. } => MetricResultViewKind::Breakdown,
            Self::Rollup { .. } => MetricResultViewKind::Rollup,
            Self::Histogram { .. } => MetricResultViewKind::Histogram,
        }
    }
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct MetricResultsResponse {
    pub metrics: Vec<MetricResultDto>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct MetricResultDto {
    pub metric_key: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub short_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explanation: Option<String>,
    pub unit: Option<String>,
    pub format: MetricFormat,
    pub direction: MetricDirection,
    #[serde(flatten)]
    pub computation: ComputationDto,
    pub views: Vec<MetricResultViewDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub drilldown: Option<MetricDrilldownCapability>,
    pub selection: MetricResultSelectionDto,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct MetricResultSelectionDto {
    pub metric_key: String,
    pub entity: MetricResultsEntityDto,
    pub period: MetricResultsPeriodDto,
    pub filters: Vec<MetricDimensionFilterDto>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MetricResultsEntityDto {
    Person { ids: Vec<String> },
    Tenant {},
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct MetricResultsPeriodDto {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct MetricDimensionFilterDto {
    pub dimension: String,
    pub values: Vec<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
#[serde(tag = "computation", rename_all = "snake_case")]
pub enum ComputationDto {
    Sum,
    Ratio {
        scale: f64,
    },
    Median,
    Percentile {
        /// The quantile — a probability, matching the definition validation.
        #[schema(minimum = 0, maximum = 1)]
        q: f64,
    },
    Stddev,
    DistinctCount,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
#[serde(tag = "view", rename_all = "snake_case")]
pub enum MetricResultViewDto {
    Period {
        values: Vec<PeriodValueDto>,
    },
    Timeseries {
        bucket: Bucket,
        series: Vec<TimeseriesDto>,
    },
    Peer {
        values: Vec<PeerValueDto>,
    },
    Breakdown {
        dimensions: Vec<String>,
        values: Vec<BreakdownValueDto>,
    },
    Rollup {
        dimensions: Vec<String>,
        values: Vec<RollupValueDto>,
    },
    Histogram {
        /// Present only for the pooled (dimensioned) shape; absent for the
        /// per-entity shape, keeping that wire form unchanged.
        #[serde(skip_serializing_if = "Vec::is_empty")]
        dimensions: Vec<String>,
        values: Vec<HistogramValueDto>,
    },
    /// This view's computation failed; sibling views and metrics are
    /// unaffected. `message` detail depends on the caller's role: admins get
    /// the underlying description, everyone else a generic one.
    Error {
        code: MetricViewErrorCode,
        message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MetricViewErrorCode {
    SourceRelationMissing,
    ResourceExhausted,
    QueryTimeout,
    ResultParseFailed,
    QueryFailed,
}

/// One histogram row. Per-entity shape: `entity_id` set, `dimensions` absent,
/// every requested entity listed. Pooled shape (dimensioned request):
/// `dimensions` set, `entity_id` absent, one row per observed dimension tuple
/// over all selected entities' events — no entity grain, like rollup.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct HistogramValueDto {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_id: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub dimensions: Vec<MetricDimensionDto>,
    /// Empty when a listed entity has no events in the period — the entity is
    /// still listed, mirroring the period view's every-requested-entity rule.
    pub bins: Vec<HistogramBinDto>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct HistogramBinDto {
    pub lo: f64,
    pub hi: f64,
    pub count: u64,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct MetricDimensionDto {
    pub key: String,
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub href: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct PeriodValueDto {
    pub entity_id: String,
    pub value: Option<f64>,
    /// The same reading over `compare_to`. Omitted both when no comparison
    /// window was asked for and when the entity has no value in it — the two
    /// are not distinguished on the wire, and a reader that asked knows which
    /// case it is in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compare_to: Option<f64>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct TimeseriesDto {
    pub entity_id: String,
    pub dimensions: Vec<MetricDimensionDto>,
    #[schema(required)]
    pub total: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rank: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remainder: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub points: Vec<TimeseriesPointDto>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct TimeseriesPointDto {
    pub bucket_start: String,
    pub value: Option<f64>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct PeerValueDto {
    pub entity_id: String,
    pub target_value: Option<f64>,
    pub p25: Option<f64>,
    pub median: Option<f64>,
    pub p75: Option<f64>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub n: u64,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct BreakdownValueDto {
    pub entity_id: String,
    pub dimensions: Vec<MetricDimensionDto>,
    pub value: Option<f64>,
    /// Whether this group has any observation inside the primary period.
    /// Present only on a windowed response, where the group set spans every
    /// window and a reader has to know which of them each group belongs to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub present: Option<bool>,
    /// This group's reading over `compare_to`; absent when no comparison window
    /// was asked for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compare_to: Option<BreakdownWindowValueDto>,
}

/// One group's reading over the comparison window.
///
/// `value` and `present` are independent: a ratio over a group that IS in the
/// window reads NULL whenever its denominator is zero, so absence cannot be
/// inferred from the value. A reader that wants what a standalone request over
/// that window would have returned keeps the rows with `present` and renders
/// their `value` as it stands, NULL included.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct BreakdownWindowValueDto {
    pub value: Option<f64>,
    pub present: bool,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct RollupValueDto {
    pub dimensions: Vec<MetricDimensionDto>,
    pub value: Option<f64>,
    pub contributing_entity_count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rank: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remainder: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

impl toolkit::api::api_dto::RequestApiDto for MetricResultsRequest {}
impl toolkit::api::api_dto::ResponseApiDto for MetricResultsResponse {}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::MetricResultsRequest;

    #[test]
    fn tenant_entity_needs_no_client_supplied_identifier() {
        let request = serde_json::from_value::<MetricResultsRequest>(json!({
            "entity": { "type": "tenant" },
            "period": { "from": "2026-01-01", "to": "2026-01-31" },
            "metrics": [{ "metric_key": "ci.runs", "views": [{ "view": "period" }] }]
        }));

        assert!(request.is_ok());
    }

    #[test]
    fn tenant_entity_rejects_client_supplied_identifiers() {
        let request = serde_json::from_value::<MetricResultsRequest>(json!({
            "entity": { "type": "tenant", "ids": ["default"] },
            "period": { "from": "2026-01-01", "to": "2026-01-31" },
            "metrics": [{ "metric_key": "ci.runs", "views": [{ "view": "period" }] }]
        }));

        assert!(request.is_err());
    }

    #[test]
    fn unknown_entity_type_reaches_domain_validation() {
        let request = serde_json::from_value::<MetricResultsRequest>(json!({
            "entity": { "type": "team", "ids": ["team"] },
            "period": { "from": "2026-01-01", "to": "2026-01-31" },
            "metrics": [{ "metric_key": "ci.runs", "views": [{ "view": "period" }] }]
        }));

        assert!(request.is_ok());
    }

    #[test]
    fn histogram_view_deserializes_with_and_without_dimensions() {
        // Bare `{"view": "histogram"}` must keep parsing exactly as before the
        // pooled shape existed (dimensions default to empty).
        let bare = serde_json::from_value::<MetricResultsRequest>(json!({
            "entity": { "type": "person", "ids": ["019e27bc-dec0-7626-81a9-c5524662a6a9"] },
            "period": { "from": "2026-01-01", "to": "2026-01-31" },
            "metrics": [{ "metric_key": "git.x", "views": [{ "view": "histogram" }] }]
        }));
        assert!(bare.is_ok());

        let pooled = serde_json::from_value::<MetricResultsRequest>(json!({
            "entity": { "type": "person", "ids": ["019e27bc-dec0-7626-81a9-c5524662a6a9"] },
            "period": { "from": "2026-01-01", "to": "2026-01-31" },
            "metrics": [{
                "metric_key": "git.x",
                "views": [{ "view": "histogram", "dimensions": ["repository"] }]
            }]
        }));
        assert!(pooled.is_ok());
    }

    #[test]
    fn rollup_view_deserializes_with_an_optional_group_limit() {
        let request = serde_json::from_value::<MetricResultsRequest>(json!({
            "entity": {
                "type": "person",
                "ids": ["019e27bc-dec0-7626-81a9-c5524662a6a9"]
            },
            "period": { "from": "2026-01-01", "to": "2026-01-31" },
            "metrics": [{
                "metric_key": "git.commits",
                "views": [{
                    "view": "rollup",
                    "dimensions": ["repository"],
                    "group_limit": {
                        "count": 25,
                        "rank_by_metric": "git.commits",
                        "include_remainder": true
                    }
                }]
            }]
        }));

        assert!(request.is_ok());
    }
}

use std::collections::BTreeMap;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::domain::metric_definitions::definition::MetricInputRole;

use super::cursor::CursorKey;
use crate::domain::metric_definitions::{EvidenceGranularity, EvidenceRelation, MetricDefinition};

pub(super) const DEFAULT_PAGE_LIMIT: usize = 100;
pub(super) const MAX_PAGE_LIMIT: usize = 250;
pub(super) const MAX_PERIOD_DAYS: i64 = 400;
pub(super) const MAX_FILTERS: usize = 10;
pub(super) const MAX_DISPLAY_DIMENSIONS: usize = 10;
pub(super) const MAX_FILTER_VALUES: usize = 100;
pub(super) const MAX_FILTER_VALUE_BYTES: usize = 512;
pub const MAX_EXPORT_ROWS: usize = 50_000;
pub const EVIDENCE_QUERY_TIMEOUT_SECS: u64 = 45;
pub const EVIDENCE_QUERY_MEMORY_BYTES: usize = 256 * 1024 * 1024;
pub const EVIDENCE_QUERY_READ_BYTES: usize = 512 * 1024 * 1024;
pub const EVIDENCE_QUERY_RESULT_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone, Deserialize, Serialize, utoipa::ToSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum MetricDrilldownEntity {
    Person {
        id: String,
    },
    Tenant {},
    #[serde(other, skip_serializing)]
    Unknown,
}

impl MetricDrilldownEntity {
    pub(crate) fn entity_type(&self) -> &'static str {
        match self {
            Self::Person { .. } => "person",
            Self::Tenant {} => "tenant",
            Self::Unknown => "unknown",
        }
    }

    pub(crate) fn person_id(&self) -> Option<&str> {
        match self {
            Self::Person { id } => Some(id),
            Self::Tenant {} | Self::Unknown => None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, utoipa::ToSchema)]
pub struct MetricDrilldownPeriod {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, utoipa::ToSchema)]
pub struct MetricDrilldownFilter {
    pub dimension: String,
    pub values: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct MetricDrilldownRequest {
    pub metric_key: String,
    pub entity: MetricDrilldownEntity,
    pub period: MetricDrilldownPeriod,
    #[serde(default)]
    pub filters: Vec<MetricDrilldownFilter>,
    #[serde(default)]
    pub display_dimensions: Vec<String>,
    pub limit: Option<usize>,
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum MetricDrilldownExportFormat {
    Csv,
    Xlsx,
}

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct MetricDrilldownExportRequest {
    pub metric_key: String,
    pub entity: MetricDrilldownEntity,
    pub period: MetricDrilldownPeriod,
    #[serde(default)]
    pub filters: Vec<MetricDrilldownFilter>,
    #[serde(default)]
    pub display_dimensions: Vec<String>,
    pub format: MetricDrilldownExportFormat,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct MetricDrilldownSelection {
    pub metric_key: String,
    pub entity: MetricDrilldownEntity,
    pub period: MetricDrilldownPeriod,
    pub filters: Vec<MetricDrilldownFilter>,
    pub display_dimensions: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum MetricDrilldownColumnType {
    String,
    Date,
    Number,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct MetricDrilldownColumn {
    pub key: String,
    pub label: String,
    pub r#type: MetricDrilldownColumnType,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct MetricDrilldownRow {
    pub values: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct MetricDrilldownCapability {
    pub granularity: Vec<EvidenceGranularity>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct MetricDrilldownResponse {
    pub selection: MetricDrilldownSelection,
    pub columns: Vec<MetricDrilldownColumn>,
    pub rows: Vec<MetricDrilldownRow>,
    pub next_cursor: Option<String>,
}

impl toolkit::api::api_dto::RequestApiDto for MetricDrilldownRequest {}
impl toolkit::api::api_dto::RequestApiDto for MetricDrilldownExportRequest {}
impl toolkit::api::api_dto::ResponseApiDto for MetricDrilldownResponse {}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::MetricDrilldownRequest;

    #[test]
    fn tenant_entity_needs_no_client_supplied_identifier() {
        let request = serde_json::from_value::<MetricDrilldownRequest>(json!({
            "metric_key": "ci.runs",
            "entity": { "type": "tenant" },
            "period": { "from": "2026-01-01", "to": "2026-01-31" },
            "limit": 100
        }));

        assert!(request.is_ok());
    }

    #[test]
    fn tenant_entity_rejects_client_supplied_identifier() {
        let request = serde_json::from_value::<MetricDrilldownRequest>(json!({
            "metric_key": "ci.runs",
            "entity": { "type": "tenant", "id": "default" },
            "period": { "from": "2026-01-01", "to": "2026-01-31" },
            "limit": 100
        }));

        assert!(request.is_err());
    }

    #[test]
    fn unknown_entity_type_reaches_domain_validation() {
        let request = serde_json::from_value::<MetricDrilldownRequest>(json!({
            "metric_key": "ci.runs",
            "entity": { "type": "team", "id": "team" },
            "period": { "from": "2026-01-01", "to": "2026-01-31" },
            "limit": 100
        }));

        assert!(request.is_ok());
    }
}

#[derive(Debug)]
pub struct ValidatedMetricDrilldown {
    pub selection: MetricDrilldownSelection,
    pub tenant_id: uuid::Uuid,
    /// Same runtime policy switch as metric-results (#1967): the evidence read
    /// leads with `tenant_id = ?` when set, degrades to match-all otherwise.
    pub enforce_tenant_scope: bool,
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub limit: usize,
    pub cursor: Option<CursorKey>,
    pub plan: EvidencePlan,
    pub snapshot_id: String,
    pub fingerprint: String,
}

#[derive(Debug, Clone)]
pub struct EvidencePlan {
    pub definition: MetricDefinition,
    pub relation: EvidenceRelation,
    pub source_key: String,
    pub inputs: Vec<EvidenceInput>,
}

#[derive(Debug, Clone)]
pub struct EvidenceInput {
    pub role: MetricInputRole,
    pub measure_key: String,
    pub presentation: EvidencePresentation,
}

#[derive(Debug, Clone)]
pub struct EvidencePresentation {
    pub detail_keys: &'static [&'static str],
    pub show_value: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EvidenceQueryRow {
    pub role: String,
    pub metric_date: String,
    pub observed_at: String,
    pub source_key: String,
    pub measure_key: String,
    pub record_id: String,
    pub record_kind: String,
    pub contribution: Option<f64>,
    pub numerator: Option<f64>,
    pub denominator: Option<f64>,
    pub subject_key: String,
    pub dimensions_json: String,
    pub details: serde_json::Value,
}

impl MetricDrilldownExportFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Xlsx => "xlsx",
        }
    }
}

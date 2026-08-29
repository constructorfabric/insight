use serde::Deserialize;
use serde::Serialize;
use uuid::Uuid;

use super::columns::ReportColumnMetadata;
use super::row::ReportRow;

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ReportRecipe {
    pub subject: ReportSubject,
    pub period: ReportPeriod,
    pub granularity: ReportGranularity,
    #[schema(max_items = 100)]
    pub metric_keys: Vec<String>,
}

pub type ReportPreviewRequest = ReportRecipe;

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ReportPreviewResponse {
    pub columns: Vec<ReportColumnMetadata>,
    pub rows: Vec<ReportRow>,
    pub total_rows: u64,
}

impl toolkit::api::api_dto::RequestApiDto for ReportRecipe {}
impl toolkit::api::api_dto::RequestApiDto for ReportExportRequest {}
impl toolkit::api::api_dto::ResponseApiDto for ReportPreviewResponse {}

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ReportExportRequest {
    pub subject: ReportSubject,
    pub period: ReportPeriod,
    pub granularity: ReportGranularity,
    #[schema(max_items = 100)]
    pub metric_keys: Vec<String>,
    pub format: ReportExportFormat,
}

impl ReportExportRequest {
    pub(crate) fn into_recipe(self) -> ReportRecipe {
        ReportRecipe {
            subject: self.subject,
            period: self.period,
            granularity: self.granularity,
            metric_keys: self.metric_keys,
        }
    }
}

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReportSubject {
    People {
        #[schema(max_items = 1000)]
        ids: Vec<Uuid>,
    },
    Tenant {},
}

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ReportPeriod {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Copy, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReportGranularity {
    Day,
    Week,
    Month,
    Quarter,
    Year,
}

#[derive(Debug, Clone, Copy, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReportExportFormat {
    Csv,
    Xlsx,
}

#[cfg(test)]
mod tests {
    use super::*;

    const PERSON_ID: &str = "019e27bc-dec0-7626-81a9-c5524662a6a9";

    #[test]
    fn preview_rejects_unknown_fields_and_invalid_subject_ids() {
        let unknown_field = format!(
            r#"{{"subject":{{"type":"people","ids":["{PERSON_ID}"]}},"period":{{"from":"2026-01-01","to":"2026-01-31"}},"granularity":"day","metric_keys":["git.commits"],"tab":"overview"}}"#
        );
        assert!(serde_json::from_str::<ReportPreviewRequest>(&unknown_field).is_err());

        let invalid_id = r#"{"subject":{"type":"people","ids":["not-a-uuid"]},"period":{"from":"2026-01-01","to":"2026-01-31"},"granularity":"day","metric_keys":["git.commits"]}"#;
        assert!(serde_json::from_str::<ReportPreviewRequest>(invalid_id).is_err());

        let tenant_with_ids = format!(
            r#"{{"subject":{{"type":"tenant","ids":["{PERSON_ID}"]}},"period":{{"from":"2026-01-01","to":"2026-01-31"}},"granularity":"day","metric_keys":["ci.builds"]}}"#
        );
        assert!(serde_json::from_str::<ReportPreviewRequest>(&tenant_with_ids).is_err());
    }

    #[test]
    fn export_accepts_only_declared_formats() {
        let csv = r#"{"subject":{"type":"tenant"},"period":{"from":"2026-01-01","to":"2026-01-31"},"granularity":"month","metric_keys":["ci.builds"],"format":"csv"}"#.to_owned();
        assert!(serde_json::from_str::<ReportExportRequest>(&csv).is_ok());

        let invalid = csv.replace("\"csv\"", "\"pdf\"");
        assert!(serde_json::from_str::<ReportExportRequest>(&invalid).is_err());

        let unknown_field = csv.replace(
            "\"format\":\"csv\"",
            "\"format\":\"csv\",\"filename\":\"report.csv\"",
        );
        assert!(serde_json::from_str::<ReportExportRequest>(&unknown_field).is_err());
    }
}

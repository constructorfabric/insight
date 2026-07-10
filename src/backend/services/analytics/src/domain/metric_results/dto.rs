use serde::{Deserialize, Serialize};

use super::view::{Bucket, MetricResultViewKind};
use crate::domain::metric_definitions::{MetricDirection, MetricFormat};

#[derive(Debug, Deserialize)]
pub struct MetricResultsRequest {
    pub entity: MetricResultsEntity,
    pub period: MetricResultsPeriod,
    pub metrics: Vec<MetricRequest>,
}

#[derive(Debug, Deserialize)]
pub struct MetricResultsEntity {
    pub r#type: String,
    pub ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct MetricResultsPeriod {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Deserialize)]
pub struct MetricRequest {
    pub metric_key: String,
    pub views: Vec<MetricViewRequest>,
}

#[derive(Debug, Deserialize)]
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
    },
    Breakdown {
        dimensions: Vec<String>,
    },
    Histogram,
}

impl MetricViewRequest {
    pub fn kind(&self) -> MetricResultViewKind {
        match self {
            Self::Period => MetricResultViewKind::Period,
            Self::Peer { .. } => MetricResultViewKind::Peer,
            Self::Timeseries { .. } => MetricResultViewKind::Timeseries,
            Self::Breakdown { .. } => MetricResultViewKind::Breakdown,
            Self::Histogram => MetricResultViewKind::Histogram,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct MetricResultsResponse {
    pub metrics: Vec<MetricResultDto>,
}

#[derive(Debug, Serialize)]
pub struct MetricResultDto {
    pub metric_key: String,
    pub label: String,
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
}

#[derive(Debug, Serialize)]
#[serde(tag = "computation", rename_all = "snake_case")]
pub enum ComputationDto {
    Sum,
    Ratio { scale: f64 },
    Median,
}

#[derive(Debug, Serialize)]
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
    Histogram {
        values: Vec<HistogramValueDto>,
    },
}

#[derive(Debug, Serialize)]
pub struct HistogramValueDto {
    pub entity_id: String,
    /// Empty when the entity has no events in the period — the entity is
    /// still listed, mirroring the period view's every-requested-entity rule.
    pub bins: Vec<HistogramBinDto>,
}

#[derive(Debug, Serialize)]
pub struct HistogramBinDto {
    pub lo: f64,
    pub hi: f64,
    pub count: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct MetricDimensionDto {
    pub key: String,
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PeriodValueDto {
    pub entity_id: String,
    pub value: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct TimeseriesDto {
    pub entity_id: String,
    pub dimensions: Vec<MetricDimensionDto>,
    pub points: Vec<TimeseriesPointDto>,
}

#[derive(Debug, Serialize)]
pub struct TimeseriesPointDto {
    pub bucket_start: String,
    pub value: Option<f64>,
}

#[derive(Debug, Serialize)]
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

#[derive(Debug, Serialize)]
pub struct BreakdownValueDto {
    pub entity_id: String,
    pub dimensions: Vec<MetricDimensionDto>,
    pub value: Option<f64>,
}

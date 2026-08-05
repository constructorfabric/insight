mod capability;
mod compiler;
mod cursor;
mod dto;
mod error;
mod export;
mod presentation;
mod validation;

pub(crate) use capability::load_capabilities;
pub(crate) use compiler::{compile_query, decode_evidence_rows};
pub(crate) use cursor::verify_evidence_snapshot;
pub(crate) use dto::{
    EVIDENCE_QUERY_MEMORY_BYTES, EVIDENCE_QUERY_READ_BYTES, EVIDENCE_QUERY_RESULT_BYTES,
    EVIDENCE_QUERY_TIMEOUT_SECS, EvidenceQueryRow, MAX_EXPORT_ROWS, MetricDrilldownCapability,
    MetricDrilldownColumn, MetricDrilldownExportFormat, MetricDrilldownExportRequest,
    MetricDrilldownRequest, MetricDrilldownResponse, MetricDrilldownRow, ValidatedMetricDrilldown,
};
pub(crate) use error::{evidence_unavailable, export_internal, export_limit};
pub(crate) use export::{MAX_EXPORT_BYTES, build_export, export_filename};
pub(crate) use presentation::{build_response, presentation};
pub(crate) use validation::{validate_export_request, validate_request};

#[cfg(test)]
mod test_support;

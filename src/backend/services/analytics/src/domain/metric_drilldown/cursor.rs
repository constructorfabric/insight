use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use toolkit_canonical_errors::CanonicalError;
use uuid::Uuid;

use crate::api::error::MetricError;
use crate::domain::metric_definitions::EvidenceRelation;

use super::dto::{EvidenceQueryRow, MetricDrilldownSelection};
use super::error::{config_error, evidence_unavailable, invalid, invalid_error};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CursorKey {
    pub(super) role: String,
    pub(super) metric_date: String,
    pub(super) observed_at: String,
    pub(super) source_key: String,
    pub(super) measure_key: String,
    pub(super) record_id: String,
    pub(super) record_kind: String,
    pub(super) subject_key: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub(super) struct CursorEnvelope {
    pub(super) version: u8,
    pub(super) fingerprint: String,
    pub(super) snapshot_id: String,
    pub(super) key: CursorKey,
}

#[derive(Debug, Deserialize, clickhouse::Row)]
struct EvidenceSnapshotRow {
    snapshot_id: String,
}

pub(super) fn selection_fingerprint(
    tenant_id: Uuid,
    selection: &MetricDrilldownSelection,
) -> Result<String, CanonicalError> {
    let bytes = serde_json::to_vec(&(tenant_id, selection)).map_err(|_| config_error())?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

pub async fn verify_evidence_snapshot(
    ch: &insight_clickhouse::Client,
    relation: &EvidenceRelation,
    expected: &str,
) -> Result<(), CanonicalError> {
    let current = evidence_snapshot_id(ch, relation).await?;
    if current != expected {
        return Err(MetricError::failed_precondition()
            .with_precondition_violation(
                "metric evidence snapshot",
                "Metric evidence was rebuilt while the request was running.",
                "EVIDENCE_SNAPSHOT_EXPIRED",
            )
            .create());
    }
    Ok(())
}

pub(super) async fn evidence_snapshot_id(
    ch: &insight_clickhouse::Client,
    relation: &EvidenceRelation,
) -> Result<String, CanonicalError> {
    let (database, table) = relation.table_ref();
    ch.query(
        "SELECT toString(uuid) AS snapshot_id \
         FROM system.tables WHERE database = ? AND name = ?",
    )
    .bind(database)
    .bind(table)
    .fetch_one::<EvidenceSnapshotRow>()
    .await
    .map(|row| row.snapshot_id)
    .map_err(|error| {
        tracing::error!(
            error = %error,
            database,
            table,
            "metric evidence snapshot lookup failed"
        );
        evidence_unavailable()
    })
}

pub(super) fn encode_cursor(
    fingerprint: &str,
    snapshot_id: &str,
    row: &EvidenceQueryRow,
) -> Result<String, CanonicalError> {
    let envelope = CursorEnvelope {
        version: 1,
        fingerprint: fingerprint.to_owned(),
        snapshot_id: snapshot_id.to_owned(),
        key: CursorKey {
            role: row.role.clone(),
            metric_date: row.metric_date.clone(),
            observed_at: row.observed_at.clone(),
            source_key: row.source_key.clone(),
            measure_key: row.measure_key.clone(),
            record_id: row.record_id.clone(),
            record_kind: row.record_kind.clone(),
            subject_key: row.subject_key.clone(),
        },
    };
    let bytes = serde_json::to_vec(&envelope).map_err(|_| config_error())?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
}

pub(super) fn decode_cursor(value: &str) -> Result<CursorEnvelope, CanonicalError> {
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| invalid_error("cursor", "cursor is malformed"))?;
    let envelope: CursorEnvelope = serde_json::from_slice(&bytes)
        .map_err(|_| invalid_error("cursor", "cursor is malformed"))?;
    if envelope.version != 1 {
        return invalid("cursor", "cursor version is unsupported");
    }
    Ok(envelope)
}

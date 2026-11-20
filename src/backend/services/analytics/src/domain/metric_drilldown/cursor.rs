use base64::Engine;
use insight_clickhouse::MissingRelation;
use serde::{Deserialize, Serialize};
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::crypto::hash::Sha256;
use uuid::Uuid;

use crate::api::error::MetricError;
use crate::domain::metric_definitions::EvidenceRelation;
use crate::domain::not_ready::NotReadyReason;
use crate::infra::not_ready;

use super::dto::{EvidenceQueryRow, MetricDrilldownColumn, MetricDrilldownSelection};
use super::error::{config_error, evidence_unavailable, invalid, invalid_error};

const CURSOR_VERSION: u8 = 3;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CursorKey {
    pub(super) sort_flag: bool,
    pub(super) sort_value: String,
    pub(super) role: String,
    pub(super) entity_id: String,
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
    columns: &[MetricDrilldownColumn],
) -> Result<String, CanonicalError> {
    let bytes = serde_json::to_vec(&(tenant_id, selection, columns)).map_err(|_| config_error())?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

pub async fn verify_evidence_snapshot(
    ch: &insight_clickhouse::Client,
    relation: &EvidenceRelation,
    expected: &str,
) -> Result<(), CanonicalError> {
    let EvidenceSnapshot::Pinned(current) = evidence_snapshot_id(ch, relation).await? else {
        return Err(snapshot_expired());
    };
    if current != expected {
        return Err(snapshot_expired());
    }
    Ok(())
}

fn snapshot_expired() -> CanonicalError {
    MetricError::failed_precondition()
        .with_precondition_violation(
            "metric evidence snapshot",
            "Metric evidence was rebuilt while the request was running.",
            "EVIDENCE_SNAPSHOT_EXPIRED",
        )
        .create()
}

#[derive(Debug)]
pub(super) enum EvidenceSnapshot {
    Pinned(String),
    NotReady(NotReadyReason),
}

pub(super) async fn evidence_snapshot_id(
    ch: &insight_clickhouse::Client,
    relation: &EvidenceRelation,
) -> Result<EvidenceSnapshot, CanonicalError> {
    let (database, table) = relation.table_ref();
    let found = ch
        .query(
            "SELECT toString(uuid) AS snapshot_id \
             FROM system.tables WHERE database = ? AND name = ?",
        )
        .bind(database)
        .bind(table)
        .fetch_optional::<EvidenceSnapshotRow>()
        .await
        .map_err(|error| {
            tracing::error!(
                error = %error,
                database,
                table,
                "metric evidence snapshot lookup failed"
            );
            evidence_unavailable()
        })?;

    let Some(evidence) = found.map(|row| row.snapshot_id) else {
        not_ready::warn_relation(&format!("{database}.{table}"), "metric-drilldown:snapshot");
        return Ok(EvidenceSnapshot::NotReady(
            NotReadyReason::SourceRelationMissing,
        ));
    };

    let identity = ch
        .query(
            // SAFETY: CAST to String, and coalesce every scalar subquery. A scalar
            // subquery is Nullable-typed, which makes `concat` return
            // Nullable(String) and the row decode fail against this String field.
            "SELECT CAST(concat(\
                 toString(coalesce((SELECT count() FROM identity.identity_persons), 0)), ':', \
                 toString(coalesce((SELECT max(id) FROM identity.identity_persons), 0)), ':', \
                 toString(coalesce((SELECT max(_version) FROM identity.identity_inputs), 0)) \
             ) AS String) AS snapshot_id",
        )
        .fetch_one::<EvidenceSnapshotRow>()
        .await
        .map(|row| row.snapshot_id);

    let identity = match identity {
        Ok(identity) => identity,
        Err(error) if MissingRelation::from_error(&error).is_some() => {
            not_ready::warn_throttled(&error.to_string(), "metric-drilldown:identity-watermark");
            return Ok(EvidenceSnapshot::NotReady(
                NotReadyReason::SourceRelationMissing,
            ));
        }
        Err(error) => {
            tracing::error!(error = %error, "identity watermark lookup failed");
            return Err(evidence_unavailable());
        }
    };

    Ok(EvidenceSnapshot::Pinned(format!("{evidence}:{identity}")))
}

pub(super) fn encode_cursor(
    fingerprint: &str,
    snapshot_id: &str,
    row: &EvidenceQueryRow,
) -> Result<String, CanonicalError> {
    let envelope = CursorEnvelope {
        version: CURSOR_VERSION,
        fingerprint: fingerprint.to_owned(),
        snapshot_id: snapshot_id.to_owned(),
        key: CursorKey {
            sort_flag: row.sort_flag != 0,
            sort_value: row.sort_value.clone(),
            role: row.role.clone(),
            entity_id: row.entity_id.clone(),
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
    if envelope.version != CURSOR_VERSION {
        return invalid("cursor", "cursor version is unsupported");
    }
    Ok(envelope)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn an_unreachable_warehouse_fails_the_pin_instead_of_answering_not_ready() {
        let ch = insight_clickhouse::Client::new(insight_clickhouse::Config::new(
            "http://127.0.0.1:1",
            "analytics",
        ));
        let relation = EvidenceRelation::parse("git_metric_evidence")
            .unwrap_or_else(|| panic!("evidence relation must parse"));

        let pinned = evidence_snapshot_id(&ch, &relation).await;

        assert!(
            pinned.is_err(),
            "an unreachable warehouse must not read as a relation nobody has built"
        );
    }
}

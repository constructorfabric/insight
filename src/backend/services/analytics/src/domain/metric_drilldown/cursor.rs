use base64::Engine;
use insight_clickhouse::MissingRelation;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use toolkit_canonical_errors::CanonicalError;
use uuid::Uuid;

use crate::api::error::MetricError;
use crate::domain::metric_definitions::EvidenceRelation;
use crate::domain::not_ready::NotReadyReason;
use crate::infra::not_ready;

use super::dto::{EvidenceQueryRow, MetricDrilldownColumn, MetricDrilldownSelection};
use super::error::{config_error, evidence_unavailable, invalid, invalid_error};

/// Bumped when the ordering key changes: an older cursor addresses a page this
/// shape would not produce, so it is refused.
const CURSOR_VERSION: u8 = 3;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CursorKey {
    /// Leads the key because it leads the ORDER BY: blank cells sit past every
    /// filled one, whichever way the sorted column runs.
    ///
    /// INVARIANT: a bool, not the `UInt8` the query projects. A cursor is
    /// caller-held bytes, and any other number would compare against the
    /// query's own 0-or-1 as if it outranked both — serving a page twice or
    /// dropping the rest of the result set. As a bool there is no such value
    /// to send: anything else fails to decode, and a cursor that does not
    /// decode is already refused.
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

/// What a cursor is bound to.
///
/// INVARIANT: the presented columns are in here, not just the selection the
/// caller sent. The ordering key's SQL is decided by a column's declared TYPE —
/// a number sorts numerically and replays through a cast, a string sorts
/// zero-padded and replays as text. That declaration lives in the metric
/// catalog, which no snapshot pins, so a change to it while a walk is in
/// flight would otherwise leave the cursor looking valid and comparing against
/// a differently-shaped key.
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
    // A relation that went absent mid-walk expires the cursor like any other
    // rebuild. A fresh install never reaches here: validation reads the
    // snapshot first and answers not-ready from there.
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

/// The marks a cursor is pinned to, or the reason there is nothing to pin.
#[derive(Debug)]
pub(super) enum EvidenceSnapshot {
    Pinned(String),
    /// A relation the pin reads has not been built yet — the evidence table
    /// itself, or the identity relations the watermark comes from.
    NotReady(NotReadyReason),
}

/// What answers a drilldown: the evidence relation AND the identity map, since
/// the rows a person owns are decided while the query runs. Both are pinned, so
/// a rebuild or correction mid-pagination expires the cursor.
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

    // No row means the relation itself is absent — the fresh-install reading,
    // and the one `system.tables` reports instead of an exception.
    let Some(evidence) = found.map(|row| row.snapshot_id) else {
        not_ready::warn_relation(&format!("{database}.{table}"), "metric-drilldown:snapshot");
        return Ok(EvidenceSnapshot::NotReady(
            NotReadyReason::SourceRelationMissing,
        ));
    };

    // SAFETY: content-derived marks, not the sync stamp — the sync stamp moves on
    // every republish and would expire every cursor on a clock. Both map inputs
    // are pinned: the persons journal is append-only, so (count, max id) moves
    // when a decision is recorded; a claim insert always carries a new high
    // `_version`, so `max(_version)` moves when a connector teaches identity a
    // new alias (which shifts pages without any gold rebuild). No count() on the
    // claims side — merges collapse replaced row versions, and a shrinking count
    // would expire live cursors for nothing.
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

    /// A warehouse that cannot be reached is not an absent relation: the pin
    /// fails rather than reporting the evidence as not built yet.
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

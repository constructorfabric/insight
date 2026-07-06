//! Peer-counter `query_ref` for the collaboration Messaging modality
//! (issue #1527, epic #1516 · release #1526).
//!
//! Clone of #1514's `ai_personal_counters_qr` (`m20260623_000001`): reads the
//! `insight.collab_person_counter_daily` gold view (PR A of #1527), re-aggregates
//! per person over the window with the honest-NULL wrapper
//! `if(countIf(x IS NOT NULL) > 0, sumIf(x, …), NULL)`, unpivots to
//! `(metric_key, value)` long rows via ARRAY JOIN (dropping NULLs so a person
//! with no source is absent, never a fake 0), then LEFT JOINs per-`org_unit_id`
//! cohort bands (`quantileExact` median/p25/p75 + min/max/count).
//!
//! This lays the shared peer-query scaffold; later modalities (#1528–#1532) add
//! their `collab_person_counter_daily.*` keys to the ARRAY JOIN tuple list.
//! Thresholds/bands calibration is out of scope (seed writes initial defaults).

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const ZERO_TENANT: &str = "00000000000000000000000000000000";
/// `metrics.id` for the collab Messaging peer-counter query. Next free slot
/// after the AI-personal counters (`…0052`).
const COLLAB_MESSAGING_COUNTERS_HEX: &str = "00000000000000000001000000000053";

/// Per-person long-format values: wide honest-NULL aggregate over the gold view
/// → ARRAY JOIN unpivot (NULLs dropped) → `org_unit_id` via `insight.people`.
const COLLAB_PERSON_COUNTER_VALUES_QR: &str = r"SELECT metric_values.person_id AS person_id, p.org_unit_id AS org_unit_id, metric_values.metric_key AS metric_key, metric_values.value AS value FROM (SELECT person_id, kv.1 AS metric_key, kv.2 AS value FROM (SELECT person_id, if(countIf(messages_sent IS NOT NULL) > 0, sumIf(messages_sent, messages_sent IS NOT NULL), CAST(NULL AS Nullable(Float64))) AS messages_sent, if(countIf(channel_posts IS NOT NULL) > 0, sumIf(channel_posts, channel_posts IS NOT NULL), CAST(NULL AS Nullable(Float64))) AS channel_posts FROM insight.collab_person_counter_daily GROUP BY person_id) d ARRAY JOIN [('collab_person_counter_daily.messages_sent', messages_sent), ('collab_person_counter_daily.channel_posts', channel_posts)] AS kv WHERE kv.2 IS NOT NULL) metric_values LEFT JOIN insight.people AS p ON metric_values.person_id = p.person_id";

fn collab_messaging_counters_qr() -> String {
    format!(
        "SELECT p.person_id AS person_id, p.org_unit_id AS org_unit_id, p.metric_key AS metric_key, p.value AS value, c.team_median AS median, c.team_p25 AS p25, c.team_p75 AS p75, c.team_n AS n, c.team_min AS range_min, c.team_max AS range_max FROM ({COLLAB_PERSON_COUNTER_VALUES_QR}) p LEFT JOIN (SELECT metric_key, org_unit_id, quantileExact(0.5)(value) AS team_median, quantileExact(0.25)(value) AS team_p25, quantileExact(0.75)(value) AS team_p75, toFloat64(count()) AS team_n, min(value) AS team_min, max(value) AS team_max FROM ({COLLAB_PERSON_COUNTER_VALUES_QR}) person_values GROUP BY metric_key, org_unit_id) c ON c.metric_key = p.metric_key AND c.org_unit_id = p.org_unit_id"
    )
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        let query = collab_messaging_counters_qr();
        db.execute_unprepared(&format!(
            "INSERT INTO metrics (id, insight_tenant_id, name, description, query_ref, is_enabled) \
             VALUES (UNHEX('{COLLAB_MESSAGING_COUNTERS_HEX}'), UNHEX('{ZERO_TENANT}'), \
             'Collab Messaging Peer Counters', 'Per-person collaboration messaging peer counter rows.', '{qr}', 1) \
             ON DUPLICATE KEY UPDATE name=VALUES(name), description=VALUES(description), query_ref=VALUES(query_ref), is_enabled=1",
            qr = query.replace('\'', "''"),
        ))
        .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(&format!(
                "DELETE FROM metrics WHERE id = UNHEX('{COLLAB_MESSAGING_COUNTERS_HEX}')"
            ))
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pins the generated peer-counter SQL shape (feeds #1433 coverage). Mirrors
    /// the assertion style of `handlers.rs::parse_simple_select` — no DB needed.
    #[test]
    fn counters_qr_emits_both_keys_with_honest_null_and_per_org_unit_bands() {
        let qr = collab_messaging_counters_qr();

        // Both Messaging metric keys are unpivoted.
        assert!(qr.contains("'collab_person_counter_daily.messages_sent'"));
        assert!(qr.contains("'collab_person_counter_daily.channel_posts'"));

        // Honest-NULL wrapper on each counter (no fake-0 fallback).
        assert!(qr.contains(
            "if(countIf(messages_sent IS NOT NULL) > 0, sumIf(messages_sent, messages_sent IS NOT NULL), CAST(NULL AS Nullable(Float64)))"
        ));
        assert!(qr.contains(
            "if(countIf(channel_posts IS NOT NULL) > 0, sumIf(channel_posts, channel_posts IS NOT NULL), CAST(NULL AS Nullable(Float64)))"
        ));

        // Reads the PR-A gold view, drops NULL rows on unpivot.
        assert!(qr.contains("FROM insight.collab_person_counter_daily GROUP BY person_id"));
        assert!(qr.contains("WHERE kv.2 IS NOT NULL"));

        // Per-org_unit cohort bands joined on (metric_key, org_unit_id).
        assert!(qr.contains("quantileExact(0.5)(value) AS team_median"));
        assert!(qr.contains("GROUP BY metric_key, org_unit_id"));
        assert!(qr.contains("c.metric_key = p.metric_key AND c.org_unit_id = p.org_unit_id"));
    }
}

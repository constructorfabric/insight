//! Seed the Support (Zendesk) Bullet `query_ref`s for the Team + IC dashboards
//! (INSIGHT-459). New "Support" dashboard domain — the person-activity view of
//! support work, alongside Collaboration.
//!
//! Pairs with the ingestion gold migration
//! `20260611000000_support-bullet-rows.sql` (`insight.support_bullet_rows`).
//! Mirrors the collab/AI bullet pattern (m20260518 / m20260609):
//!   - wide-aggregate one row per person from `support_bullet_rows`
//!   - ARRAY JOIN unpivot → long (`metric_key`, `v_period`)
//!   - team/ic outer query joins a company/team distribution subquery
//!
//! FE-visible `metric_keys` (7):
//!   `support_active`            — DAU marker → `ACTIVE_LIST`, outer sum = #active members
//!   `support_updates`           — counter (period total per person) → outer avg
//!   `support_public_comments`   — counter
//!   `support_private_comments`  — counter
//!   `support_solved`            — counter
//!   `support_csat`              — QUALITY %, computed Σgood/Σtotal (assignee-attributed;
//!                               NULL when no ratings → `ComingSoon`)
//!   `support_kb`                — Knowledge base articles: NULL stub (no Guide stream
//!                               yet) → renders `ComingSoon`, lights up when the stream lands

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const TEAM_BULLET_SUPPORT_ID: &str = "00000000000000000001000000000007";
const IC_BULLET_SUPPORT_ID: &str = "00000000000000000001000000000008";
const ZERO_TENANT: &str = "00000000000000000000000000000000";

/// Active-marker keys — the outer query uses `sum(v_period)` (count of active
/// members); everything else uses `avg(v_period)`.
const ACTIVE_LIST: &str = "'support_active'";

/// Inner wide-aggregate: one row per person, every FE-visible metric in its own
/// column. Counters via `sumIf`; CSAT % via `Σgood/Σtotal` (NULL on zero
/// denominator); `support_kb` a NULL stub (no Guide stream).
fn wide_aggregate_pp() -> &'static str {
    "SELECT person_id, any(org_unit_id) AS org_unit_id, \
         if(countIf(metric_key = 'support_active') > 0, toFloat64(1), CAST(NULL AS Nullable(Float64))) AS support_active_v, \
         sumIf(metric_value, metric_key = 'support_updates') AS support_updates_v, \
         sumIf(metric_value, metric_key = 'support_public_comments') AS support_public_comments_v, \
         sumIf(metric_value, metric_key = 'support_private_comments') AS support_private_comments_v, \
         sumIf(metric_value, metric_key = 'support_solved') AS support_solved_v, \
         if(sumIf(metric_value, metric_key = 'support_csat_total') > 0, \
            round(toFloat64(100) \
                  * sumIf(metric_value, metric_key = 'support_csat_good') \
                  / sumIf(metric_value, metric_key = 'support_csat_total'), 1), \
            CAST(NULL AS Nullable(Float64))) AS support_csat_v, \
         CAST(NULL AS Nullable(Float64)) AS support_kb_v \
     FROM insight.support_bullet_rows \
     GROUP BY person_id"
}

/// ARRAY JOIN unpivot → 7 FE-visible `metric_keys`.
fn array_join_kv() -> &'static str {
    "ARRAY JOIN [ \
         ('support_active',           support_active_v), \
         ('support_updates',          support_updates_v), \
         ('support_public_comments',  support_public_comments_v), \
         ('support_private_comments', support_private_comments_v), \
         ('support_solved',           support_solved_v), \
         ('support_csat',             support_csat_v), \
         ('support_kb',               support_kb_v) \
     ] AS kv"
}

fn team_query() -> String {
    let pp = wide_aggregate_pp();
    let kv = array_join_kv();
    format!(
        "SELECT p.metric_key AS metric_key, \
                multiIf(p.metric_key IN ({ACTIVE_LIST}), sum(p.v_period), avg(p.v_period)) AS value, \
                any(c.company_median) AS median, \
                any(c.company_min) AS range_min, \
                any(c.company_max) AS range_max \
         FROM ( \
             SELECT person_id, org_unit_id, \
                    kv.1 AS metric_key, kv.2 AS v_period \
             FROM ({pp}) pp \
             {kv} \
         ) p \
         LEFT JOIN ( \
             SELECT metric_key, \
                    multiIf(metric_key IN ({ACTIVE_LIST}), \
                            if(count(v_period) = 0, CAST(NULL AS Nullable(Float64)), toFloat64(0)), \
                            quantileExact(0.5)(v_period)) AS company_median, \
                    multiIf(metric_key IN ({ACTIVE_LIST}), \
                            if(count(v_period) = 0, CAST(NULL AS Nullable(Float64)), toFloat64(0)), \
                            min(v_period)) AS company_min, \
                    multiIf(metric_key IN ({ACTIVE_LIST}), \
                            if(count(v_period) = 0, CAST(NULL AS Nullable(Float64)), toFloat64(count())), \
                            max(v_period)) AS company_max \
             FROM ( \
                 SELECT kv.1 AS metric_key, kv.2 AS v_period \
                 FROM ({pp}) ppc \
                 {kv} \
             ) inner_c \
             GROUP BY metric_key \
         ) c ON c.metric_key = p.metric_key \
         GROUP BY p.metric_key"
    )
}

fn ic_query() -> String {
    let pp = wide_aggregate_pp();
    let kv = array_join_kv();
    format!(
        "SELECT p.metric_key AS metric_key, \
                multiIf(p.metric_key IN ({ACTIVE_LIST}), sum(p.v_period), avg(p.v_period)) AS value, \
                any(c.team_median) AS median, \
                any(c.team_min) AS range_min, \
                any(c.team_max) AS range_max \
         FROM ( \
             SELECT person_id, org_unit_id, \
                    kv.1 AS metric_key, kv.2 AS v_period \
             FROM ({pp}) pp \
             {kv} \
         ) p \
         LEFT JOIN ( \
             SELECT metric_key, org_unit_id, \
                    multiIf(metric_key IN ({ACTIVE_LIST}), \
                            if(count(v_period) = 0, CAST(NULL AS Nullable(Float64)), toFloat64(0)), \
                            quantileExact(0.5)(v_period)) AS team_median, \
                    multiIf(metric_key IN ({ACTIVE_LIST}), \
                            if(count(v_period) = 0, CAST(NULL AS Nullable(Float64)), toFloat64(0)), \
                            min(v_period)) AS team_min, \
                    multiIf(metric_key IN ({ACTIVE_LIST}), \
                            if(count(v_period) = 0, CAST(NULL AS Nullable(Float64)), toFloat64(count())), \
                            max(v_period)) AS team_max \
             FROM ( \
                 SELECT person_id, org_unit_id, \
                        kv.1 AS metric_key, kv.2 AS v_period \
                 FROM ({pp}) ppc \
                 {kv} \
             ) inner_c \
             GROUP BY metric_key, org_unit_id \
         ) c ON c.metric_key = p.metric_key AND c.org_unit_id = p.org_unit_id \
         GROUP BY p.metric_key"
    )
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for (hex_id, name, query) in [
            (TEAM_BULLET_SUPPORT_ID, "Team Bullet Support", team_query()),
            (IC_BULLET_SUPPORT_ID, "IC Bullet Support", ic_query()),
        ] {
            db.execute_unprepared(&format!(
                "INSERT INTO metrics (id, insight_tenant_id, name, description, query_ref, is_enabled) \
                 VALUES (UNHEX('{hex_id}'), UNHEX('{ZERO_TENANT}'), '{name}', \
                         'Support (Zendesk) activity bullets — person vs cohort.', '{qr}', 1) \
                 ON DUPLICATE KEY UPDATE name=VALUES(name), description=VALUES(description), query_ref=VALUES(query_ref)",
                qr = query.replace('\'', "''"),
            ))
            .await?;
        }
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "m20260612_000001_support_metrics is irreversible: delete the Support \
             metrics rows (ids …0007 / …0008) manually if needed."
                .to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// All 7 FE-visible support `metric_keys` appear in the ARRAY JOIN unpivot.
    #[test]
    fn array_join_emits_all_seven_keys() {
        let kv = array_join_kv();
        for key in [
            "support_active",
            "support_updates",
            "support_public_comments",
            "support_private_comments",
            "support_solved",
            "support_csat",
            "support_kb",
        ] {
            assert!(
                kv.contains(&format!("('{key}',")),
                "ARRAY JOIN missing {key}"
            );
        }
        assert_eq!(kv.matches("('").count(), 7, "expected exactly 7 keys");
    }

    /// `support_active` is the only active marker; CSAT is a ratio (not a counter
    /// in `ACTIVE_LIST`); `support_kb` is a NULL stub (`ComingSoon`).
    #[test]
    fn classification_is_correct() {
        assert!(ACTIVE_LIST.contains("'support_active'"));
        assert!(!ACTIVE_LIST.contains("'support_solved'"));
        let pp = wide_aggregate_pp();
        // CSAT computed as Σgood/Σtotal, NULL on zero denominator
        assert!(
            pp.contains("metric_key = 'support_csat_total'")
                && pp.contains("metric_key = 'support_csat_good'")
        );
        // support_kb is a NULL stub until a Guide stream lands
        assert!(pp.contains("CAST(NULL AS Nullable(Float64)) AS support_kb_v"));
        // counters via sumIf
        assert!(pp.contains("sumIf(metric_value, metric_key = 'support_solved')"));
    }

    /// Both `query_refs` reference the gold view + the shared unpivot.
    #[test]
    fn queries_reference_gold_view_and_unpivot() {
        for q in [team_query(), ic_query()] {
            assert!(q.contains("insight.support_bullet_rows"));
            assert!(q.contains("ARRAY JOIN"));
            assert!(q.contains("support_csat"));
        }
    }
}

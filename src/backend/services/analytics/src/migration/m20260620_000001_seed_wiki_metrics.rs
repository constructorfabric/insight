//! Seed the Team / IC "Bullet Wiki" metric views (wiki class → frontend).
//!
//! Pairs with ingestion migration `20260620000000_wiki-bullet-rows.sql`
//! which creates `insight.wiki_bullet_rows` (long format: `person_id`,
//! `org_unit_id`, `metric_date`, `metric_key`, `metric_value`) from the wiki Silver
//! classes (Confluence today; Outline later).
//!
//! Adds two new rows to the `metrics` table (the FE references them by id
//! via `METRIC_REGISTRY)`:
//!   • `TEAM_BULLET_WIKI` = ...0040 — company-wide range (median/min/max).
//!   • `IC_BULLET_WIKI`   = ...0041 — per-org-unit (team) range.
//!
//! `query_ref` shape mirrors the AI bullet (`m20260618_000001)`: a per-person
//! wide-aggregate → ARRAY JOIN unpivot → outer dispatch where the marker
//! `wiki_active_authors` aggregates with `sum()` of the per-person 0/1 (= count
//! of active authors) and the counters aggregate with `avg()` per person; the
//! range subquery uses `count()` as the marker's max scale (like the AI
//! active-member markers). 4 `metric_keys` total.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const TEAM_BULLET_WIKI_ID: &str = "00000000000000000001000000000040";
const IC_BULLET_WIKI_ID: &str = "00000000000000000001000000000041";
const ZERO_TENANT: &str = "00000000000000000000000000000000";

/// Active-marker keys — outer uses sum(per-person 0/1) = count of active
/// authors. Counters are NOT here (they average per person).
const ACTIVE_LIST: &str = "'wiki_active_authors'";

/// Per-person wide-aggregate over `insight.wiki_bullet_rows`.
fn wide_aggregate_pp() -> &'static str {
    "SELECT person_id, any(org_unit_id) AS org_unit_id, \
         if(countIf(metric_key = 'wiki_active_authors') > 0, toFloat64(1), CAST(NULL AS Nullable(Float64))) AS wiki_active_authors_v, \
         sumIf(metric_value, metric_key = 'wiki_pages_created') AS wiki_pages_created_v, \
         sumIf(metric_value, metric_key = 'wiki_edits') AS wiki_edits_v, \
         sumIf(metric_value, metric_key = 'wiki_comments') AS wiki_comments_v \
     FROM insight.wiki_bullet_rows \
     GROUP BY person_id"
}

/// ARRAY JOIN unpivot: wide columns → long rows per person. 4 keys.
fn array_join_kv() -> &'static str {
    "ARRAY JOIN [ \
         ('wiki_active_authors', wiki_active_authors_v), \
         ('wiki_pages_created',  wiki_pages_created_v), \
         ('wiki_edits',          wiki_edits_v), \
         ('wiki_comments',       wiki_comments_v) \
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
             SELECT person_id, org_unit_id, kv.1 AS metric_key, kv.2 AS v_period \
             FROM ({pp}) pp {kv} \
         ) p \
         LEFT JOIN ( \
             SELECT metric_key, \
                    multiIf(metric_key IN ({ACTIVE_LIST}), if(count(v_period) = 0, CAST(NULL AS Nullable(Float64)), toFloat64(0)), quantileExact(0.5)(v_period)) AS company_median, \
                    multiIf(metric_key IN ({ACTIVE_LIST}), if(count(v_period) = 0, CAST(NULL AS Nullable(Float64)), toFloat64(0)), min(v_period)) AS company_min, \
                    multiIf(metric_key IN ({ACTIVE_LIST}), if(count(v_period) = 0, CAST(NULL AS Nullable(Float64)), toFloat64(count())), max(v_period)) AS company_max \
             FROM (SELECT kv.1 AS metric_key, kv.2 AS v_period FROM ({pp}) ppc {kv}) inner_c \
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
             SELECT person_id, org_unit_id, kv.1 AS metric_key, kv.2 AS v_period \
             FROM ({pp}) pp {kv} \
         ) p \
         LEFT JOIN ( \
             SELECT metric_key, org_unit_id, \
                    multiIf(metric_key IN ({ACTIVE_LIST}), if(count(v_period) = 0, CAST(NULL AS Nullable(Float64)), toFloat64(0)), quantileExact(0.5)(v_period)) AS team_median, \
                    multiIf(metric_key IN ({ACTIVE_LIST}), if(count(v_period) = 0, CAST(NULL AS Nullable(Float64)), toFloat64(0)), min(v_period)) AS team_min, \
                    multiIf(metric_key IN ({ACTIVE_LIST}), if(count(v_period) = 0, CAST(NULL AS Nullable(Float64)), toFloat64(count())), max(v_period)) AS team_max \
             FROM (SELECT person_id, org_unit_id, kv.1 AS metric_key, kv.2 AS v_period FROM ({pp}) ppc {kv}) inner_c \
             GROUP BY metric_key, org_unit_id \
         ) c ON c.metric_key = p.metric_key AND c.org_unit_id = p.org_unit_id \
         GROUP BY p.metric_key"
    )
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for (hex_id, name, desc, qr) in [
            (
                TEAM_BULLET_WIKI_ID,
                "Team Bullet Wiki",
                "Wiki (Confluence/Outline) per-person bullets — team view, company-wide range.",
                team_query(),
            ),
            (
                IC_BULLET_WIKI_ID,
                "IC Bullet Wiki",
                "Wiki (Confluence/Outline) per-person bullets — IC view, per-org-unit range.",
                ic_query(),
            ),
        ] {
            db.execute_unprepared(&format!(
                "INSERT INTO metrics (id, insight_tenant_id, name, description, query_ref, is_enabled) \
                 VALUES (UNHEX('{hex_id}'), UNHEX('{ZERO_TENANT}'), '{name}', '{desc}', '{q}', 1) \
                 ON DUPLICATE KEY UPDATE name=VALUES(name), description=VALUES(description), query_ref=VALUES(query_ref)",
                q = qr.replace('\'', "''"),
            ))
            .await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for id in [TEAM_BULLET_WIKI_ID, IC_BULLET_WIKI_ID] {
            db.execute_unprepared(&format!("DELETE FROM metrics WHERE id = UNHEX('{id}')"))
                .await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEYS: &[&str] = &[
        "wiki_active_authors",
        "wiki_pages_created",
        "wiki_edits",
        "wiki_comments",
    ];

    #[test]
    fn array_join_has_all_four_keys() {
        let kv = array_join_kv();
        for k in KEYS {
            assert!(kv.contains(&format!("('{k}',")), "ARRAY JOIN missing {k}");
        }
        assert_eq!(kv.matches("('").count(), 4, "expected exactly 4 keys");
    }

    #[test]
    fn marker_is_active_counters_are_not() {
        assert!(ACTIVE_LIST.contains("wiki_active_authors"));
        for c in ["wiki_pages_created", "wiki_edits", "wiki_comments"] {
            assert!(!ACTIVE_LIST.contains(c), "{c} is a counter, not active");
        }
    }

    #[test]
    fn counters_use_sumif_marker_uses_countif() {
        let pp = wide_aggregate_pp();
        assert!(pp.contains("countIf(metric_key = 'wiki_active_authors') > 0"));
        for c in ["wiki_pages_created", "wiki_edits", "wiki_comments"] {
            assert!(pp.contains(&format!("sumIf(metric_value, metric_key = '{c}')")));
        }
    }

    #[test]
    fn queries_reference_view_and_unpivot() {
        for q in [team_query(), ic_query()] {
            assert!(q.contains("insight.wiki_bullet_rows"));
            assert!(q.contains("ARRAY JOIN"));
            assert!(q.contains("wiki_comments"));
        }
        assert!(team_query().contains("company_median"));
        assert!(
            ic_query().contains("team_median")
                && ic_query().contains("c.org_unit_id = p.org_unit_id")
        );
    }
}

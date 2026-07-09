//! Seed catalog rows for the CRM (sales-rep) dashboard family.
//!
//! Five metrics, all with stable UUIDs in the
//! `00000000-0000-0000-0001-00000000002X` block:
//!
//!   * `0020 CRM KPIs`                    — hero strip + pacing (flow metric)
//!   * `0021 CRM Chart Deal Flow`         — weekly opened / closed / won
//!   * `0022 CRM Bullet Velocity Quality` — win rate, cycle, avg deal size, deals opened (vs team)
//!   * `0023 CRM Bullet Activity`         — calls, emails, meetings, comms-per-won (vs team)
//!   * `0028 CRM Pipeline Now`            — date-less open-deal snapshot per rep
//!
//! `0028` is split from `0020` because pipeline-now is a stock metric —
//! one row per rep, no `metric_date` dimension — while the rest of the
//! KPIs sum across the selected period. Folding it back into `0020` would
//! require the analytics-api to skip its date-filter injection for one
//! column (or fanning the snapshot across N dates, which we tried and
//! dropped in review — O(reps × 365) row count for a constant value).
//!
//! UUIDs `…0024 / …0025 / …0026 / …0027` stay RESERVED — earlier drafts
//! seeded a Closing-Soon table (0024), Lost-Reasons composition (0025),
//! Deal-Types composition (0026), and a Sources mix donut (0027). All four
//! were dropped before merge: 0024 because the action-queue UX is off-axis
//! for Insight's person-performance lens; 0025 / 0026 because Constructor's
//! HubSpot doesn't fill `properties_closed_lost_reason` /
//! `properties_dealtype`, so each collapsed to a single `(no reason)` /
//! `(unspecified)` bar in the UI; 0027 deferred pending an `hs_analytics_
//! source` enrichment pass at silver. Reserve them so the UUIDs are
//! available if/when those sections come back.
//!
//! Source views live in CH migration `20260512000000_crm-gold-views.sql`.
//! Frontend UUIDs mirror these in
//! `insight-front/src/screensets/insight/api/metricRegistry.ts`.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// ---------------------------------------------------------------------------
// UUIDs
// ---------------------------------------------------------------------------

const ZERO_TENANT: &str = "00000000000000000000000000000000";
const CRM_KPIS_ID: &str = "00000000000000000001000000000020";
const CRM_CHART_FLOW_ID: &str = "00000000000000000001000000000021";
const CRM_BULLET_QUALITY_ID: &str = "00000000000000000001000000000022";
const CRM_BULLET_ACTIVITY_ID: &str = "00000000000000000001000000000023";
const CRM_PIPELINE_NOW_ID: &str = "00000000000000000001000000000028";

// ---------------------------------------------------------------------------
// Names + descriptions
// ---------------------------------------------------------------------------

const CRM_KPIS_NAME: &str = "CRM KPIs";
const CRM_CHART_FLOW_NAME: &str = "CRM Chart Deal Flow";
const CRM_BULLET_QUALITY_NAME: &str = "CRM Bullet Velocity Quality";
const CRM_BULLET_ACTIVITY_NAME: &str = "CRM Bullet Activity";
const CRM_PIPELINE_NOW_NAME: &str = "CRM Pipeline Now";

const CRM_KPIS_DESC: &str =
    "Sales rep KPIs from HubSpot: open/closed deals, won count + value, communications volume";
const CRM_CHART_FLOW_DESC: &str = "Weekly opened / closed / won deal counts per sales rep";
const CRM_BULLET_QUALITY_DESC: &str = "Sales velocity & quality bullet metrics: win rate, cycle days, avg deal size, \
     deals opened — with team median/min/max for vs-team comparison.";
const CRM_BULLET_ACTIVITY_DESC: &str = "Sales outreach-activity bullet metrics: calls, emails, meetings volumes \
     and comms-per-won-deal efficiency — with team median/min/max for \
     vs-team comparison.";
const CRM_PIPELINE_NOW_DESC: &str = "Open-deal snapshot per rep (count + summed amount_home). Stock metric — \
     point-in-time, no date dimension.";

// ---------------------------------------------------------------------------
// query_ref strings
// ---------------------------------------------------------------------------

const CRM_KPIS_QUERY: &str = "SELECT person_id, \
sum(deals_opened) AS deals_opened, \
sum(deals_closed) AS deals_closed, \
sum(deals_won) AS deals_won, \
round(sum(deals_value_closed)) AS deals_value_closed, \
sum(comms_count) AS comms_count \
FROM insight.crm_kpis GROUP BY person_id";

// Date-less — `insight.crm_pipeline_now` has no `metric_date` column, so
// analytics-api's date-filter injection no-ops on it. Single row per rep.
const CRM_PIPELINE_NOW_QUERY: &str = "SELECT person_id, \
pipeline_count, \
pipeline_value \
FROM insight.crm_pipeline_now";

const CRM_CHART_FLOW_QUERY: &str =
    "SELECT date_bucket, opened, closed, won, person_id, metric_date FROM insight.crm_chart_flow";

// Mirrors the `git_bullet_rows` query_ref pattern: inner per-person wide
// aggregate → ARRAY JOIN to long format → outer JOIN to team distribution
// computed identically. Whitespace flattened so it matches what the seed
// upserts into MariaDB byte-for-byte (keeps `down()` rollback exact).
const CRM_BULLET_QUALITY_QUERY: &str = "SELECT p.metric_key AS metric_key, \
avgIf(p.v_period, isNotNull(p.v_period)) AS value, \
any(c.team_median) AS median, \
any(c.team_min) AS range_min, \
any(c.team_max) AS range_max \
FROM (\
SELECT person_id, org_unit_id, kv.1 AS metric_key, kv.2 AS v_period \
FROM (\
SELECT person_id, any(org_unit_id) AS org_unit_id, \
ifNull(sumIf(metric_value, metric_key = 'deals_opened'), 0) AS deals_opened, \
ifNull(sumIf(metric_value, metric_key = 'deals_closed'), 0) AS deals_closed, \
ifNull(sumIf(metric_value, metric_key = 'deals_won'),    0) AS deals_won, \
avgIf(metric_value, metric_key = 'cycle_days')              AS cycle_days, \
avgIf(metric_value, metric_key = 'deal_size')               AS avg_deal_size \
FROM insight.crm_bullet_rows GROUP BY person_id\
) ARRAY JOIN [\
('deals_opened', toFloat64(deals_opened)), \
('cycle_days', cycle_days), \
('avg_deal_size', avg_deal_size), \
('win_rate', if(deals_closed > 0, deals_won * 100.0 / deals_closed, NULL))\
] AS kv\
) p \
LEFT JOIN (\
SELECT metric_key, org_unit_id, \
quantileExactIf(0.5)(v_period, isNotNull(v_period)) AS team_median, \
minIf(v_period, isNotNull(v_period)) AS team_min, \
maxIf(v_period, isNotNull(v_period)) AS team_max \
FROM (\
SELECT person_id, org_unit_id, kv.1 AS metric_key, kv.2 AS v_period \
FROM (\
SELECT person_id, any(org_unit_id) AS org_unit_id, \
ifNull(sumIf(metric_value, metric_key = 'deals_opened'), 0) AS deals_opened, \
ifNull(sumIf(metric_value, metric_key = 'deals_closed'), 0) AS deals_closed, \
ifNull(sumIf(metric_value, metric_key = 'deals_won'),    0) AS deals_won, \
avgIf(metric_value, metric_key = 'cycle_days')              AS cycle_days, \
avgIf(metric_value, metric_key = 'deal_size')               AS avg_deal_size \
FROM insight.crm_bullet_rows GROUP BY person_id\
) ARRAY JOIN [\
('deals_opened', toFloat64(deals_opened)), \
('cycle_days', cycle_days), \
('avg_deal_size', avg_deal_size), \
('win_rate', if(deals_closed > 0, deals_won * 100.0 / deals_closed, NULL))\
] AS kv\
) inner_c \
GROUP BY metric_key, org_unit_id\
) c ON c.metric_key = p.metric_key AND c.org_unit_id = p.org_unit_id \
GROUP BY p.metric_key";

// Same self-join shape as CRM_BULLET_QUALITY. Inner aggregates also pull
// `tasks` and `deals_won` so the derived `comms_per_won` ratio reflects
// all engagement channels even though `tasks` itself isn't surfaced as a
// bullet row.
const CRM_BULLET_ACTIVITY_QUERY: &str = "SELECT p.metric_key AS metric_key, \
avgIf(p.v_period, isNotNull(p.v_period)) AS value, \
any(c.team_median) AS median, \
any(c.team_min) AS range_min, \
any(c.team_max) AS range_max \
FROM (\
SELECT person_id, org_unit_id, kv.1 AS metric_key, kv.2 AS v_period \
FROM (\
SELECT person_id, any(org_unit_id) AS org_unit_id, \
ifNull(sumIf(metric_value, metric_key = 'calls'), 0)     AS calls, \
ifNull(sumIf(metric_value, metric_key = 'emails'), 0)    AS emails, \
ifNull(sumIf(metric_value, metric_key = 'meetings'), 0)  AS meetings, \
ifNull(sumIf(metric_value, metric_key = 'tasks'), 0)     AS tasks, \
ifNull(sumIf(metric_value, metric_key = 'deals_won'), 0) AS deals_won \
FROM insight.crm_bullet_rows GROUP BY person_id\
) ARRAY JOIN [\
('calls', toFloat64(calls)), \
('emails', toFloat64(emails)), \
('meetings', toFloat64(meetings)), \
('comms_per_won', if(deals_won > 0, (calls + emails + meetings + tasks) / deals_won, NULL))\
] AS kv\
) p \
LEFT JOIN (\
SELECT metric_key, org_unit_id, \
quantileExactIf(0.5)(v_period, isNotNull(v_period)) AS team_median, \
minIf(v_period, isNotNull(v_period)) AS team_min, \
maxIf(v_period, isNotNull(v_period)) AS team_max \
FROM (\
SELECT person_id, org_unit_id, kv.1 AS metric_key, kv.2 AS v_period \
FROM (\
SELECT person_id, any(org_unit_id) AS org_unit_id, \
ifNull(sumIf(metric_value, metric_key = 'calls'), 0)     AS calls, \
ifNull(sumIf(metric_value, metric_key = 'emails'), 0)    AS emails, \
ifNull(sumIf(metric_value, metric_key = 'meetings'), 0)  AS meetings, \
ifNull(sumIf(metric_value, metric_key = 'tasks'), 0)     AS tasks, \
ifNull(sumIf(metric_value, metric_key = 'deals_won'), 0) AS deals_won \
FROM insight.crm_bullet_rows GROUP BY person_id\
) ARRAY JOIN [\
('calls', toFloat64(calls)), \
('emails', toFloat64(emails)), \
('meetings', toFloat64(meetings)), \
('comms_per_won', if(deals_won > 0, (calls + emails + meetings + tasks) / deals_won, NULL))\
] AS kv\
) inner_c \
GROUP BY metric_key, org_unit_id\
) c ON c.metric_key = p.metric_key AND c.org_unit_id = p.org_unit_id \
GROUP BY p.metric_key";

// ---------------------------------------------------------------------------
// Migration
// ---------------------------------------------------------------------------

const SEEDS: &[(&str, &str, &str, &str)] = &[
    (CRM_KPIS_ID, CRM_KPIS_NAME, CRM_KPIS_DESC, CRM_KPIS_QUERY),
    (
        CRM_CHART_FLOW_ID,
        CRM_CHART_FLOW_NAME,
        CRM_CHART_FLOW_DESC,
        CRM_CHART_FLOW_QUERY,
    ),
    (
        CRM_BULLET_QUALITY_ID,
        CRM_BULLET_QUALITY_NAME,
        CRM_BULLET_QUALITY_DESC,
        CRM_BULLET_QUALITY_QUERY,
    ),
    (
        CRM_BULLET_ACTIVITY_ID,
        CRM_BULLET_ACTIVITY_NAME,
        CRM_BULLET_ACTIVITY_DESC,
        CRM_BULLET_ACTIVITY_QUERY,
    ),
    (
        CRM_PIPELINE_NOW_ID,
        CRM_PIPELINE_NOW_NAME,
        CRM_PIPELINE_NOW_DESC,
        CRM_PIPELINE_NOW_QUERY,
    ),
];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for (hex_id, name, desc, query) in SEEDS {
            db.execute_unprepared(&format!(
                "INSERT INTO metrics (id, insight_tenant_id, name, description, query_ref, is_enabled) \
                 VALUES (UNHEX('{hex_id}'), UNHEX('{tenant}'), '{name}', '{desc}', '{qr}', 1) \
                 ON DUPLICATE KEY UPDATE \
                   name = VALUES(name), \
                   description = VALUES(description), \
                   query_ref = VALUES(query_ref), \
                   is_enabled = 1",
                tenant = ZERO_TENANT,
                name   = name.replace('\'', "''"),
                desc   = desc.replace('\'', "''"),
                qr     = query.replace('\'', "''"),
            ))
            .await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for (hex_id, _name, _desc, _query) in SEEDS {
            db.execute_unprepared(&format!("DELETE FROM metrics WHERE id = UNHEX('{hex_id}')"))
                .await?;
        }
        Ok(())
    }
}

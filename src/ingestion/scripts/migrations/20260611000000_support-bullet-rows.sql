-- =====================================================================
-- Support (Zendesk) Gold — bullet rows + person-period + company stats
-- =====================================================================
-- New "Support" dashboard domain (INSIGHT-459), mirroring the collaboration
-- gold shape (collab_bullet_rows → collab_person_period → collab_company_stats).
-- Reads the cross-vendor person×date silver rollup silver.class_support_activity.
--
-- Metric keys emitted (person × date):
--   support_active           — 1 per (person, date): any support activity (DAU marker → max)
--   support_updates          — ticket field updates (actor), period total → sum
--   support_public_comments  — public comments authored, period total → sum
--   support_private_comments — private (internal) comments, period total → sum
--   support_solved           — tickets solved (status→solved), period total → sum
--   support_csat_good        — CSAT good ratings (assignee-attributed) → sum
--   support_csat_total       — CSAT rated (good+bad) → sum
-- The CSAT bullet (% good = Σgood/Σtotal) is computed in the analytics-api
-- query_ref from the two raw counters (same pattern as cc_tool_acceptance) —
-- it is a QUALITY signal, assignee-attributed (not actor; see PRD §4).
-- `support_kb` (Knowledge base articles) is intentionally NOT emitted yet — no
-- Guide/Help-Center stream — so it renders ComingSoon via the catalog/query_ref.
-- =====================================================================
DROP VIEW IF EXISTS insight.support_bullet_rows;
CREATE VIEW insight.support_bullet_rows AS
SELECT
    a.person_key                                    AS person_id,    -- = lower(email)
    p.org_unit_id                                   AS org_unit_id,
    a.date                                          AS metric_date,
    kv.1                                            AS metric_key,
    kv.2                                            AS metric_value
FROM silver.class_support_activity AS a
LEFT JOIN insight.people AS p ON a.person_key = p.person_id
ARRAY JOIN [
    -- support_active = the person did ACTOR work that day (comment/update/solve).
    -- NOT 1-per-row: a class_support_activity row can exist purely from the
    -- assignee-attributed CSAT contribution (a customer rated their ticket),
    -- and a day with only that is not "active support work". So gate on the
    -- actor counters; max()/sum() downstream then count genuinely-active members.
    ('support_active',           toFloat64(if(
        ifNull(a.updates, 0) + ifNull(a.public_comments, 0)
        + ifNull(a.private_comments, 0) + ifNull(a.solved, 0) > 0, 1, 0))),
    ('support_updates',          toFloat64(ifNull(a.updates, 0))),
    ('support_public_comments',  toFloat64(ifNull(a.public_comments, 0))),
    ('support_private_comments', toFloat64(ifNull(a.private_comments, 0))),
    ('support_solved',           toFloat64(ifNull(a.solved, 0))),
    ('support_csat_good',        toFloat64(ifNull(a.csat_good, 0))),
    ('support_csat_total',       toFloat64(ifNull(a.csat_total, 0)))
] AS kv
WHERE a.person_key IS NOT NULL AND a.person_key != '';

-- Per-person period rollup. Counters → sum; the active marker → max
-- (1 if active in the period). Mirrors ai_person_period semantics.
DROP VIEW IF EXISTS insight.support_person_period;
CREATE VIEW insight.support_person_period AS
SELECT
    metric_key,
    person_id,
    any(org_unit_id)                                AS org_unit_id,
    max(metric_date)                                AS metric_date,
    multiIf(
        metric_key IN ('support_active'),
        max(metric_value),
        sum(metric_value))                          AS v
FROM insight.support_bullet_rows
GROUP BY metric_key, person_id;

-- Company-level distribution over the per-person values. active marker sums
-- across persons (= count of active members) with a synthetic distribution;
-- counters use avg/quantiles over persons. Mirrors ai_company_stats.
DROP VIEW IF EXISTS insight.support_company_stats;
CREATE VIEW insight.support_company_stats AS
SELECT
    metric_key,
    multiIf(
        metric_key IN ('support_active'),
        sum(v),
        avg(v))                                                       AS company_value,
    multiIf(
        metric_key IN ('support_active'),
        if(count(v) = 0, CAST(NULL AS Nullable(Float64)), toFloat64(0)),
        quantileExact(0.5)(v))                                        AS company_median,
    multiIf(
        metric_key IN ('support_active'),
        if(count(v) = 0, CAST(NULL AS Nullable(Float64)), toFloat64(0)),
        min(v))                                                       AS company_p5,
    multiIf(
        metric_key IN ('support_active'),
        if(count(v) = 0, CAST(NULL AS Nullable(Float64)), toFloat64(count())),
        max(v))                                                       AS company_p95
FROM insight.support_person_period
GROUP BY metric_key;

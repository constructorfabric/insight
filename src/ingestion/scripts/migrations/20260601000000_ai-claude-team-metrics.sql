-- =====================================================================
-- ai_bullet_rows — Claude Team metric extension (INSIGHT-458)
-- =====================================================================
--
-- Extends Branch 3 (tool = 'claude_code') of ai_bullet_rows with three
-- new metric_keys sourced exclusively from Claude Team:
--
--   cc_cost        ← silver.class_ai_dev_usage.cost_cents
--                    Per-user-per-day cost in cents. Claude Team is the
--                    only source at this grain (all others expose cost at
--                    org/workspace level only). NULL for cursor /
--                    claude_enterprise / claude_admin rows → 0 via
--                    COALESCE; query_ref sums it and exposes as
--                    cc_cost metric.
--
--   prs_with_cc    ← silver.class_ai_dev_usage.prs_with_cc_count
--                    PRs where Claude Code was active at least once.
--                    Populated only on tenants with the Anthropic
--                    GitHub-app connected; structural 0 on orgs without
--                    it (including Constructor Tech dev org). NULL for
--                    all other sources → 0 via COALESCE.
--
--   prs_total      ← silver.class_ai_dev_usage.prs_total_count
--                    Total PRs in the measurement window; denominator
--                    for a future prs_with_cc_pct ratio metric.
--                    Same availability caveat as prs_with_cc.
--
-- Shape change: Branch 3 grows from 5 to 8 metric_keys emitted per
-- (person, date) row via ARRAY JOIN. Branches 1 and 2 are unchanged.
--
-- Paired backend migrations:
--   m20260601_000001_ai_claude_team_metrics.rs — updates query_ref for Team
--     Bullet AI and IC Bullet AI to read the three new metric_keys via sumIf.
--   m20260601_000002_seed_claude_team_metrics_catalog.rs — seeds
--     metric_catalog + product-default metric_threshold rows for the three
--     new keys (label / unit / higher_is_better / good / warn).
--
-- Idempotent: DROP VIEW IF EXISTS + CREATE VIEW.
-- =====================================================================

DROP VIEW IF EXISTS insight.ai_bullet_rows;

CREATE VIEW insight.ai_bullet_rows AS

-- ─── Branch 1: tool-agnostic (any AI tool counts) ────────────────────
-- active_ai_members: emitted once per (person, date) for any tool row.
-- team_ai_loc: sums lines across all tools (Cursor + Claude Code).
SELECT
    lower(c.email)                                 AS person_id,
    p.org_unit_id                                  AS org_unit_id,
    c.day                                          AS metric_date,
    kv.1                                           AS metric_key,
    kv.2                                           AS metric_value
FROM silver.class_ai_dev_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
ARRAY JOIN [
    ('active_ai_members', toFloat64(1)),
    ('team_ai_loc',       toFloat64(coalesce(c.lines_added, 0)))
] AS kv
WHERE c.email IS NOT NULL AND c.email != ''

UNION ALL

-- ─── Branch 2: Cursor (tool = 'cursor') ──────────────────────────────
-- 6 keys: 1 active marker, 3 sum counters, 2 denominator counters.
-- cursor_offered + cursor_total_lines are the raw denominators
-- query_ref uses to reconstruct cursor_acceptance + ai_loc_share2.
SELECT
    lower(c.email)                                 AS person_id,
    p.org_unit_id                                  AS org_unit_id,
    c.day                                          AS metric_date,
    kv.1                                           AS metric_key,
    kv.2                                           AS metric_value
FROM silver.class_ai_dev_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
ARRAY JOIN [
    ('cursor_active',       toFloat64(1)),
    ('cursor_completions',  toFloat64(coalesce(c.tool_use_accepted, 0))),
    ('cursor_agents',       toFloat64(coalesce(c.agent_sessions,    0))),
    ('cursor_lines',        toFloat64(coalesce(c.lines_added,       0))),
    ('cursor_offered',      toFloat64(coalesce(c.tool_use_offered,  0))),
    ('cursor_total_lines',  toFloat64(coalesce(c.total_lines_added, 0)))
] AS kv
WHERE c.tool = 'cursor'
  AND c.email IS NOT NULL AND c.email != ''

UNION ALL

-- ─── Branch 3: Claude Code (tool = 'claude_code') ────────────────────
-- 8 keys: 1 active marker, 2 core counters, 2 offered/accept counters,
-- 3 Claude Team-specific counters (cc_cost, prs_with_cc, prs_total).
--
-- For non-Team sources (claude_enterprise, claude_admin):
--   cost_cents = NULL → COALESCE → 0
--   prs_with_cc_count = NULL → COALESCE → 0
--   prs_total_count = NULL → COALESCE → 0
-- Semantics are correct: the counters accumulate to 0 for sources that
-- do not expose these fields. query_ref aggregates via sumIf — a 0
-- contribution from non-Team rows is harmless.
--
-- ⚠️  prs_total / prs_with_cc scope: both columns are populated ONLY from
-- claude_team (source = 'claude_team') which feeds into tool = 'claude_code'
-- rows here. claude_enterprise also writes tool = 'claude_code' rows but
-- contributes prs_total_count = NULL → 0 (no GitHub-app). This means the
-- prs_total metric reflects only Claude Team PR attribution, NOT all
-- claude_code activity. The name is intentionally scoped to the GitHub-app
-- population, not the broader "all Claude Code users" population.
--
-- cc_offered: denominator query_ref uses to reconstruct cc_tool_acceptance
-- as 100 * Σcc_tool_accept / Σcc_offered.
SELECT
    lower(c.email)                                 AS person_id,
    p.org_unit_id                                  AS org_unit_id,
    c.day                                          AS metric_date,
    kv.1                                           AS metric_key,
    kv.2                                           AS metric_value
FROM silver.class_ai_dev_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
ARRAY JOIN [
    ('cc_active',       toFloat64(1)),
    ('cc_sessions',     toFloat64(coalesce(c.session_count,     0))),
    ('cc_lines',        toFloat64(coalesce(c.lines_added,       0))),
    ('cc_tool_accept',  toFloat64(coalesce(c.tool_use_accepted, 0))),
    ('cc_offered',      toFloat64(coalesce(c.tool_use_offered,  0))),
    ('cc_cost',         toFloat64(coalesce(c.cost_cents,        0))),
    ('prs_with_cc',     toFloat64(coalesce(c.prs_with_cc_count, 0))),
    ('prs_total',       toFloat64(coalesce(c.prs_total_count,   0)))
] AS kv
WHERE c.tool = 'claude_code'
  AND c.email IS NOT NULL AND c.email != '';

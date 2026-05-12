-- =====================================================================
-- ai_bullet_rows: per-tool filtering + populated Claude Code metrics
-- =====================================================================
--
-- The view defined in 20260427120000_views-from-silver.sql assumed
-- silver.class_ai_dev_usage was Cursor-only — Claude Code rows were a
-- future placeholder, so cursor_* metrics had no `tool` filter and
-- cc_* metrics emitted hardcoded NULL.
--
-- After Claude Enterprise was wired up as a canonical feed for
-- class_ai_dev_usage (this PR), that assumption breaks:
--   • cursor_* rows now leak Claude Code activity (e.g. cursor_lines
--     summed Claude lines_added into Cursor totals).
--   • cc_* metrics still NULL despite real Claude rows being available.
--
-- This migration drops and recreates ai_bullet_rows with:
--   • Per-tool WHERE filter on every cursor_* and cc_* row.
--   • Real values for cc_active / cc_sessions / cc_lines /
--     cc_tool_accept / cc_tool_acceptance, computed from the
--     claude_code rows in silver.
--   • active_ai_members and team_ai_loc remain tool-agnostic
--     (any AI tool counts as an active member; team_ai_loc sums
--     Cursor + Claude AI-accepted lines).
--   • ai_loc_share2 stays Cursor-only — Claude Enterprise does not
--     expose a `total_lines_added` denominator, so the share metric
--     is undefined for Claude rows.
--   • codex_*, claude_web, chatgpt continue to emit NULL — those
--     surfaces aren't ingested.

DROP VIEW IF EXISTS insight.ai_bullet_rows;

CREATE VIEW insight.ai_bullet_rows AS
-- active_ai_members: tool-agnostic
SELECT
    lower(c.email)                                AS person_id,
    p.org_unit_id,
    c.day                                         AS metric_date,
    'active_ai_members'                           AS metric_key,
    toFloat64(1)                                  AS metric_value
FROM silver.class_ai_dev_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
UNION ALL
-- cursor_*: filter WHERE tool='cursor'
SELECT lower(c.email), p.org_unit_id, c.day, 'cursor_active', toFloat64(1)
FROM silver.class_ai_dev_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
WHERE c.tool = 'cursor'
UNION ALL
SELECT lower(c.email), p.org_unit_id, c.day, 'cursor_acceptance',
    if(toFloat64(coalesce(c.tool_use_offered, 0)) > 0,
       round((toFloat64(coalesce(c.tool_use_accepted, 0)) /
              toFloat64(c.tool_use_offered)) * 100, 1),
       CAST(NULL AS Nullable(Float64)))
FROM silver.class_ai_dev_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
WHERE c.tool = 'cursor'
UNION ALL
SELECT lower(c.email), p.org_unit_id, c.day, 'cursor_completions',
    toFloat64(coalesce(c.completions_count, 0))
FROM silver.class_ai_dev_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
WHERE c.tool = 'cursor'
UNION ALL
SELECT lower(c.email), p.org_unit_id, c.day, 'cursor_agents',
    toFloat64(coalesce(c.agent_sessions, 0))
FROM silver.class_ai_dev_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
WHERE c.tool = 'cursor'
UNION ALL
SELECT lower(c.email), p.org_unit_id, c.day, 'cursor_lines',
    toFloat64(coalesce(c.lines_added, 0))
FROM silver.class_ai_dev_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
WHERE c.tool = 'cursor'
UNION ALL
-- cc_*: filter WHERE tool='claude_code', compute real values
SELECT lower(c.email), p.org_unit_id, c.day, 'cc_active', toFloat64(1)
FROM silver.class_ai_dev_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
WHERE c.tool = 'claude_code'
UNION ALL
SELECT lower(c.email), p.org_unit_id, c.day, 'cc_sessions',
    toFloat64(coalesce(c.session_count, 0))
FROM silver.class_ai_dev_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
WHERE c.tool = 'claude_code'
UNION ALL
SELECT lower(c.email), p.org_unit_id, c.day, 'cc_lines',
    toFloat64(coalesce(c.lines_added, 0))
FROM silver.class_ai_dev_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
WHERE c.tool = 'claude_code'
UNION ALL
SELECT lower(c.email), p.org_unit_id, c.day, 'cc_tool_accept',
    toFloat64(coalesce(c.tool_use_accepted, 0))
FROM silver.class_ai_dev_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
WHERE c.tool = 'claude_code'
UNION ALL
SELECT lower(c.email), p.org_unit_id, c.day, 'cc_tool_acceptance',
    if(toFloat64(coalesce(c.tool_use_offered, 0)) > 0,
       round((toFloat64(coalesce(c.tool_use_accepted, 0)) /
              toFloat64(c.tool_use_offered)) * 100, 1),
       CAST(NULL AS Nullable(Float64)))
FROM silver.class_ai_dev_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
WHERE c.tool = 'claude_code'
UNION ALL
-- codex_active: not ingested
SELECT lower(c.email), p.org_unit_id, c.day, 'codex_active',
    CAST(NULL AS Nullable(Float64))
FROM silver.class_ai_dev_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
UNION ALL
-- team_ai_loc: tool-agnostic sum (Cursor + Claude = total AI lines accepted)
SELECT lower(c.email), p.org_unit_id, c.day, 'team_ai_loc',
    toFloat64(coalesce(c.lines_added, 0))
FROM silver.class_ai_dev_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
UNION ALL
-- ai_loc_share2: Cursor-only (Claude Enterprise has total_lines_added=NULL)
SELECT lower(c.email), p.org_unit_id, c.day, 'ai_loc_share2',
    if(toFloat64(coalesce(c.total_lines_added, 0)) > 0,
       round((toFloat64(coalesce(c.lines_added, 0)) /
              toFloat64(c.total_lines_added)) * 100, 1),
       CAST(NULL AS Nullable(Float64)))
FROM silver.class_ai_dev_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
WHERE c.tool = 'cursor'
UNION ALL
-- chatgpt, claude_web: not ingested
SELECT lower(c.email), p.org_unit_id, c.day, 'chatgpt',
    CAST(NULL AS Nullable(Float64))
FROM silver.class_ai_dev_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
UNION ALL
SELECT lower(c.email), p.org_unit_id, c.day, 'claude_web',
    CAST(NULL AS Nullable(Float64))
FROM silver.class_ai_dev_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id;

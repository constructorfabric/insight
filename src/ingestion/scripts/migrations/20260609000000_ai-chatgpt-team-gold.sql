-- =====================================================================
-- ai_bullet_rows — ChatGPT Team (Codex + Chat) extension
-- =====================================================================
--
-- Surfaces ChatGPT Team data (connector chatgpt-team) in the AI bullet
-- Gold view. Adds two branches; Branches 1–3 are unchanged from
-- 20260601000000_ai-claude-team-metrics.sql.
--
--   Branch 4 — Codex (tool = 'codex', source = 'chatgpt_team'):
--     codex_active   ← 1 per (person, date)        — Codex DAU marker
--     codex_lines    ← class_ai_dev_usage.lines_added   — AI-accepted lines
--     codex_sessions ← class_ai_dev_usage.session_count — Codex threads
--   (Codex exposes no offered/accepted counters → no acceptance ratio,
--    and no total_lines_added → no AI-LOC-share. Codex also already
--    contributes to the tool-agnostic active_ai_members / team_ai_loc
--    via Branch 1.)
--
--   Branch 5 — ChatGPT chat (tool = 'chatgpt') from class_ai_assistant_usage:
--     chatgpt_active ← 1 per (person, date)        — ChatGPT chat DAU marker
--     chatgpt        ← class_ai_assistant_usage.message_count — interactions
--   (Conversational surface — read from class_ai_assistant_usage, NOT
--    class_ai_dev_usage. Per-bucket splits / credits live in
--    surface_metrics_json and are not promoted to bullet metrics.)
--
-- Paired backend migration:
--   m20260609_000001_ai_chatgpt_team_metrics.rs — un-stubs codex_active /
--   chatgpt in query_ref for Team Bullet AI + IC Bullet AI and adds
--   codex_lines / codex_sessions / chatgpt_active aggregation.
--
-- Idempotent: DROP VIEW IF EXISTS + CREATE VIEW.
-- =====================================================================

DROP VIEW IF EXISTS insight.ai_bullet_rows;

CREATE VIEW insight.ai_bullet_rows AS

-- ─── Branch 1: tool-agnostic (any dev AI tool counts) ────────────────
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
    ('cc_cost',         toFloat64(coalesce(c.cost_cents,        0)))
    -- honest-NULL (issue #1286): the Claude Team connector ships NO PR data
    -- (bronze total_prs / prs_with_cc are 0 for every row — a non-ingested
    -- source, not a measured zero). Emitting literal 0 rendered a fake
    -- "0 PRs with Claude Code" and could trip false alerts. So we do NOT emit
    -- prs_with_cc / prs_total rows at all; the keys stay ComingSoon via the
    -- query_ref guard (if countIf(key) > 0 … else NULL). A future source with
    -- real PR attribution re-introduces the rows and they light up.
] AS kv
WHERE c.tool = 'claude_code'
  AND c.email IS NOT NULL AND c.email != ''

UNION ALL

-- ─── Branch 4: Codex (tool = 'codex', ChatGPT Team) ──────────────────
SELECT
    lower(c.email)                                 AS person_id,
    p.org_unit_id                                  AS org_unit_id,
    c.day                                          AS metric_date,
    kv.1                                           AS metric_key,
    kv.2                                           AS metric_value
FROM silver.class_ai_dev_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
ARRAY JOIN [
    ('codex_active',   toFloat64(1)),
    ('codex_lines',    toFloat64(coalesce(c.lines_added,   0))),
    ('codex_sessions', toFloat64(coalesce(c.session_count, 0)))
] AS kv
WHERE c.tool = 'codex'
  AND c.email IS NOT NULL AND c.email != ''

UNION ALL

-- ─── Branch 5: ChatGPT chat (tool = 'chatgpt') ───────────────────────
-- Conversational surface — sourced from class_ai_assistant_usage.
SELECT
    lower(a.email)                                 AS person_id,
    p.org_unit_id                                  AS org_unit_id,
    a.day                                          AS metric_date,
    kv.1                                           AS metric_key,
    kv.2                                           AS metric_value
FROM silver.class_ai_assistant_usage AS a
LEFT JOIN insight.people AS p ON lower(a.email) = p.person_id
ARRAY JOIN [
    ('chatgpt_active', toFloat64(1)),
    ('chatgpt',        toFloat64(coalesce(a.message_count, 0)))
] AS kv
WHERE a.tool = 'chatgpt'
  AND a.email IS NOT NULL AND a.email != '';

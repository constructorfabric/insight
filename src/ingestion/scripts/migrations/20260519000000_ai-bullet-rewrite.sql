-- =====================================================================
-- ai_bullet_rows — Phase A rewrite (issue #433 §4.1, §4.4)
-- =====================================================================
--
-- Same scan-consolidation + ratio num/den split + Date type rewrite as
-- PR #478 (task_delivery) and PR #480 (collab), applied to the AI
-- section. Three concurrent changes plus a "ComingSoon audit" for the
-- not-yet-ingested surfaces:
--
--   1. SCAN CONSOLIDATION (issue #433 §3.5). View dropped from 16
--      UNION-ALL branches to 3 — one per tool scope. Within each branch
--      multiple metric_keys are emitted via `ARRAY JOIN` over a tuple
--      array, so `silver.class_ai_dev_usage` is read once per scope
--      instead of 16 times.
--
--   2. RATIO num/den SPLIT (issue #433 §3.3). Three daily-ratio metrics
--      were going through `avg(metric_value)` in `query_ref` over the
--      period — mathematically wrong when daily denominators differ:
--
--        cursor_acceptance     daily 100 * tool_use_accepted / tool_use_offered
--        cc_tool_acceptance    daily 100 * tool_use_accepted / tool_use_offered
--        ai_loc_share2         daily 100 * lines_added / total_lines_added
--
--      All three are dropped from this view as standalone `metric_key`s.
--      `query_ref` now reconstructs them as `100 * Σnum / Σden` over
--      the period from the raw counters that are already emitted:
--
--        cursor_acceptance  = Σcursor_completions / Σcursor_offered
--                             (cursor_completions IS Σtool_use_accepted —
--                              see #262 alias note in the old view)
--        cc_tool_acceptance = Σcc_tool_accept     / Σcc_offered
--        ai_loc_share2      = Σcursor_lines       / Σcursor_total_lines
--
--      Two new raw counters are exposed for the denominators:
--        cursor_offered      = tool_use_offered    (tool='cursor')
--        cc_offered          = tool_use_offered    (tool='claude_code')
--        cursor_total_lines  = total_lines_added   (tool='cursor')
--
--   3. `metric_date` type. Previously `c.day` (which is already `Date`
--      in silver) flowed through unchanged — no `toString(...)` wrap in
--      the predecessor either, so no type change is needed for this
--      view. Documented for symmetry with PR #478/#480.
--
--   4. ComingSoon AUDIT (issue #433 §4.4). The predecessor emitted
--      `codex_active`, `chatgpt`, `claude_web` as one NULL-valued row
--      per (person, date) from `silver.class_ai_dev_usage`. Those rows
--      carried no signal — the surfaces aren't ingested — and inflated
--      the view by ~N_persons * N_days per tool. We drop those three
--      branches entirely. The corresponding `metric_key`s remain in the
--      FE-visible response because `query_ref` hardcodes them to NULL
--      in the wide-aggregate — same honest-NULL → ComingSoon contract
--      as `20260423120000_bullet-views-honest-nulls.sql` documents
--      ("flip those to NULL so the FE bullet renders ComingSoon").
--
-- Branch shape after rewrite (3 branches, scope-aligned):
--
--   1. tool-agnostic     → 2 keys via ARRAY JOIN (active_ai_members, team_ai_loc)
--   2. tool = 'cursor'   → 6 keys via ARRAY JOIN
--   3. tool = 'claude_code' → 5 keys via ARRAY JOIN
--
-- 13 distinct metric_keys after rewrite (down from 16 — dropped:
-- codex_active, chatgpt, claude_web). The 3 composite-ratio
-- metric_keys visible on FE (`cursor_acceptance`, `cc_tool_acceptance`,
-- `ai_loc_share2`) live ONLY in the `query_ref` projection — they are
-- not emitted by this view. The 3 ComingSoon `metric_key`s also live
-- only in `query_ref` (as hardcoded NULL columns).
-- =====================================================================

DROP VIEW IF EXISTS insight.ai_bullet_rows;

CREATE VIEW insight.ai_bullet_rows AS

-- ─── Branch 1: tool-agnostic (any AI tool counts) ────────────────────
-- active_ai_members: emitted once per (person, date) for any tool row.
-- team_ai_loc: sums lines across all tools (Cursor + Claude Code).
SELECT
    lower(c.email)                                AS person_id,
    p.org_unit_id                                 AS org_unit_id,
    c.day                                         AS metric_date,
    kv.1                                          AS metric_key,
    kv.2                                          AS metric_value
FROM silver.class_ai_dev_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
ARRAY JOIN [
    ('active_ai_members', toFloat64(1)),
    ('team_ai_loc',       toFloat64(coalesce(c.lines_added, 0)))
] AS kv
WHERE c.email IS NOT NULL AND c.email != ''

UNION ALL

-- ─── Branch 2: Cursor (tool = 'cursor') ──────────────────────────────
-- 6 keys: 1 active marker, 4 sum counters, 1 offered-denominator.
-- cursor_completions is sourced from tool_use_accepted per #262
-- (completions_count was numerically identical and was dropped from
-- silver). cursor_offered + cursor_total_lines are the denominators
-- query_ref uses to reconstruct cursor_acceptance + ai_loc_share2.
SELECT
    lower(c.email)                                AS person_id,
    p.org_unit_id                                 AS org_unit_id,
    c.day                                         AS metric_date,
    kv.1                                          AS metric_key,
    kv.2                                          AS metric_value
FROM silver.class_ai_dev_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
ARRAY JOIN [
    ('cursor_active',       toFloat64(1)),
    ('cursor_completions',  toFloat64(coalesce(c.tool_use_accepted, 0))),
    ('cursor_agents',       toFloat64(coalesce(c.agent_sessions, 0))),
    ('cursor_lines',        toFloat64(coalesce(c.lines_added, 0))),
    ('cursor_offered',      toFloat64(coalesce(c.tool_use_offered, 0))),
    ('cursor_total_lines',  toFloat64(coalesce(c.total_lines_added, 0)))
] AS kv
WHERE c.tool = 'cursor'
  AND c.email IS NOT NULL AND c.email != ''

UNION ALL

-- ─── Branch 3: Claude Code (tool = 'claude_code') ────────────────────
-- 5 keys: 1 active marker, 3 sum counters, 1 offered-denominator.
-- cc_offered is the denominator query_ref uses to reconstruct
-- cc_tool_acceptance as Σcc_tool_accept / Σcc_offered.
SELECT
    lower(c.email)                                AS person_id,
    p.org_unit_id                                 AS org_unit_id,
    c.day                                         AS metric_date,
    kv.1                                          AS metric_key,
    kv.2                                          AS metric_value
FROM silver.class_ai_dev_usage AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
ARRAY JOIN [
    ('cc_active',       toFloat64(1)),
    ('cc_sessions',     toFloat64(coalesce(c.session_count, 0))),
    ('cc_lines',        toFloat64(coalesce(c.lines_added, 0))),
    ('cc_tool_accept',  toFloat64(coalesce(c.tool_use_accepted, 0))),
    ('cc_offered',      toFloat64(coalesce(c.tool_use_offered, 0)))
] AS kv
WHERE c.tool = 'claude_code'
  AND c.email IS NOT NULL AND c.email != '';

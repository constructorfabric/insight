-- =====================================================================
-- ai_person_period — fix period-rollup classification (issue #1286)
-- =====================================================================
-- The multiIf in 20260423120000_bullet-views-honest-nulls.sql was never
-- updated as new bullet keys landed (20260519 rewrite + claude-team + the
-- chatgpt-team work in this PR). Unlisted keys silently defaulted to
-- avg(metric_value):
--   • codex_lines / codex_sessions — counters (twins of cc_lines / cc_sessions,
--       which are sum) → divided by each person's active-day count, so Codex
--       was understated ~4-8x vs Claude Code (e.g. codex_lines 23,618 → 5,998).
--   • cc_offered / cc_tool_accept   — tool-use counters → understated volumes.
--   • cc_cost                        — period spend → classified as sum to
--       match cc_lines (a "period total" sibling; was avg-per-person daily).
--       UNIT: cents end-to-end — gold sums cents, catalog unit='¢' /
--       "…cost · cents · period total", and the FE renders cents as-is (no
--       /100). Whether to display $ instead of ¢ is an open PRODUCT decision
--       (would need /100 + unit='$' aligned across gold ↔ catalog ↔ FE); it is
--       NOT a hidden ×100 bug today.
--   • prs_total / prs_with_cc        — counters (honest-NULL handled upstream;
--       classified as sum so real PR data sums correctly when a source ships it).
--   • chatgpt_active                 — a 0/1 active-member flag (twin of
--       cc_active / codex_active, which are max) → avg produced a meaningless
--       fraction; moved to the max branch.
--
-- Counters → sum, active flags → max, ratios (acceptance / loc-share) → avg.
-- ai_company_stats is intentionally NOT recreated here: it reads
-- ai_person_period.v and computes a per-person company distribution
-- (avg / quantiles), which is correct once the per-person value is correct.
-- =====================================================================
DROP VIEW IF EXISTS insight.ai_person_period;
CREATE VIEW insight.ai_person_period AS
SELECT
    metric_key,
    person_id,
    any(org_unit_id)                                              AS org_unit_id,
    max(metric_date)                                              AS metric_date,
    multiIf(
        metric_key IN ('chatgpt','cc_lines','cc_sessions','cursor_agents',
                       'cursor_lines','claude_web','cursor_completions','team_ai_loc',
                       'codex_lines','codex_sessions','cc_offered','cc_tool_accept',
                       'cc_cost','prs_total','prs_with_cc',
                       -- cursor offered/total-lines are counters too (twins of
                       -- cc_offered); they were missing from the sum list and
                       -- would have defaulted to avg. No data yet (bronze_cursor
                       -- empty) but classified correctly for when it ships.
                       'cursor_offered','cursor_total_lines'),
        sum(metric_value),
        metric_key IN ('active_ai_members','cursor_active','cc_active','codex_active',
                       'chatgpt_active'),
        max(metric_value),
        avg(metric_value))                                        AS v
FROM insight.ai_bullet_rows
GROUP BY metric_key, person_id;

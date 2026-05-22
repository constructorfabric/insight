-- =====================================================================
-- code_quality_bullet_rows — Phase A rewrite (issue #433 §4.1, §4.5)
-- =====================================================================
--
-- Same scan-consolidation + Date-type rewrite + ComingSoon audit as
-- PR #478 (task_delivery), PR #480 (collab), PR #490 (ai), applied to
-- the code-quality section. Two concurrent changes:
--
--   1. SCAN CONSOLIDATION (issue #433 §3.5). View dropped from 4
--      UNION-ALL branches to 1 — all four predecessor branches read
--      from `insight.jira_closed_tasks`, but three of them emitted
--      hardcoded NULL (no ingestion source). Re-scanning the same
--      table 3 times to produce NULL rows is pure waste. Only the
--      `bugs_fixed` branch carries signal; that is the only branch
--      this rewrite keeps.
--
--      No `ARRAY JOIN` is needed at this layer — only one
--      `metric_key` is emitted, so the simple `SELECT` shape is the
--      consolidated form. ARRAY JOIN appears one level up in
--      `query_ref`, where 1 raw `metric_key` + 3 hardcoded-NULL
--      `metric_key`s are unpivoted into the FE response.
--
--      Ratio num/den split (issue #433 §3.3) does NOT apply here —
--      there are no daily-ratio metrics in this section.
--
--   2. ComingSoon AUDIT (issue #433 §4.5). The predecessor emitted
--      `prs_per_dev`, `pr_cycle_time`, `build_success` as one
--      NULL-valued row per (person, date) from
--      `insight.jira_closed_tasks` for each — pure noise. The
--      surfaces aren't ingested:
--
--        prs_per_dev    Bitbucket PR ingestion not wired
--        pr_cycle_time  same
--        build_success  CI build results not ingested
--
--      We drop those three branches entirely. The corresponding
--      `metric_key`s remain in the FE-visible response because the
--      paired `query_ref` hardcodes them to NULL columns in the
--      wide-aggregate — same honest-NULL → ComingSoon contract as
--      `20260423120000_bullet-views-honest-nulls.sql` ("flip those to
--      NULL so the FE bullet renders ComingSoon"). When any of these
--      sources land in silver (Bitbucket / CI), the right place to
--      wire them is back here as new branches, not into
--      task_delivery_bullet_rows (which would mix sections).
--
--   3. `metric_date` type. Previously `String` via
--      `toString(j.metric_date)`; now `j.metric_date` directly (the
--      native `Date` type of `insight.jira_closed_tasks.metric_date`).
--      Same change as PR #478 / #480. Unlocks downstream MergeTree
--      min/max statistics.
--
-- Branch shape after rewrite (1 branch):
--
--   1. `jira_closed_tasks` → 1 key (`bugs_fixed`)
--
-- 1 distinct metric_key after rewrite (down from 4 — dropped:
-- prs_per_dev, pr_cycle_time, build_success). The 3 ComingSoon
-- `metric_key`s visible on FE live ONLY in the `query_ref` projection
-- — they are not emitted by this view.
--
-- Note on duplication with `task_delivery_bullet_rows`: that view
-- (after PR #478 rewrite) also emits `bugs_fixed` from the same
-- `insight.jira_closed_tasks` source. We deliberately do NOT
-- consolidate the two reads into a single view — keeping
-- `code_quality_bullet_rows` as its own surface preserves a place to
-- wire future ingestion sources (Bitbucket PRs for `prs_per_dev` /
-- `pr_cycle_time`, CI for `build_success`) without dragging
-- code-quality concerns into the task-delivery view. The trade-off is
-- one extra scan of `jira_closed_tasks` per render.
-- =====================================================================

DROP VIEW IF EXISTS insight.code_quality_bullet_rows;

CREATE VIEW insight.code_quality_bullet_rows AS

-- ─── Branch 1: jira_closed_tasks (per-person-per-day aggregate) ──────
-- Emits 1 key: bugs_fixed. Single SELECT — no ARRAY JOIN at this
-- layer since there's only one metric to unpack.
SELECT
    j.person_id                                                  AS person_id,
    p.org_unit_id                                                AS org_unit_id,
    j.metric_date                                                AS metric_date,
    'bugs_fixed'                                                 AS metric_key,
    CAST(toFloat64(j.bugs_fixed) AS Nullable(Float64))           AS metric_value
FROM insight.jira_closed_tasks AS j
LEFT JOIN insight.people AS p ON j.person_id = p.person_id;

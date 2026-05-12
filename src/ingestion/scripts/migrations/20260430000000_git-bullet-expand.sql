-- Expand insight.git_bullet_rows from a single metric (`commits`) to the full
-- git_output set the FE expects.
--
-- Reads silver.class_git_* directly (NOT silver.fct_git_*) — fct_* are
-- silver-over-silver derivations and not consumable from gold under the
-- "bronze → silver → gold" rule. fct_*'s contributions (state_norm,
-- cycle_time_h, person_key fallback, file_category) are recomputed inline
-- here. Sources are ReplacingMergeTree(_version), so `FINAL` is required
-- for dedup.
--
-- Identity:
--   person_id = if(author_email != '', lower(author_email), lower(author_name))
--   For commits: author_email is reliably populated → person_id resolves to
--   insight.people.person_id and org_unit_id is non-null.
--   For PRs: Bitbucket Cloud REST API does NOT return author email on PR
--   payloads (verified against bronze_bitbucket_cloud.pull_requests in the
--   local dump — 100% empty). We fall back to lower(author_name) as
--   person_key. These rows will not join insight.people, so org_unit_id
--   stays NULL — but the metric is still attributed to a stable per-person
--   string. Drop is not an option (would zero the entire PR section).
--
-- Dedup:
--   class_git_* are ReplacingMergeTree on _version. `FINAL` ensures only
--   the latest version of each unique_key contributes.
--
-- Tenant:
--   No tenant filter at this layer — analytics-api currently runs in
--   single-tenant MVP mode (handlers.rs `MVP: single tenant — skip tenant
--   isolation filter`). When multi-tenant lands, all gold views need the
--   same retrofit; this is consistent with the rest of the layer.
--
-- Not emitted (intentional):
--   pr_review_time        — needs author↔reviewer identity-resolution; the
--                           reviewer-login namespace is not joinable to
--                           email/name. FE renders ComingSoon.
--   unique_files_touched  — distinct-over-period semantics don't fit a
--                           sum-aggregator bullet view. Future work.
--
-- Granularity:
--   Counter keys emit one row per (person, day): commits, loc, clean_loc,
--   prs_created, prs_merged. Distribution keys emit one row per event:
--   pr_size (per-PR LOC, dated by created_on), pr_cycle_time_h (per merged
--   PR, dated by closed_on). Period filter applied via metric_date works
--   for both.
--
-- Depends on 20260422000000_gold-views.sql (insight.people exists).

DROP VIEW IF EXISTS insight.git_bullet_rows;

CREATE VIEW insight.git_bullet_rows
(
    `person_id`    String,
    `org_unit_id`  Nullable(String),
    `metric_date`  String,
    `metric_key`   String,
    `metric_value` Float64
)
AS
-- ── commits per day ─────────────────────────────────────────────────────
SELECT
    toString(lower(c.author_email))                  AS person_id,
    p.org_unit_id                                    AS org_unit_id,
    toString(assumeNotNull(toDate(c.date)))                         AS metric_date,
    'commits'                                        AS metric_key,
    toFloat64(countDistinct(c.commit_hash))          AS metric_value
FROM silver.class_git_commits AS c FINAL
LEFT JOIN insight.people AS p ON lower(c.author_email) = p.person_id
WHERE c.is_merge_commit = 0
  AND c.author_email != ''
  AND c.date IS NOT NULL
GROUP BY lower(c.author_email), p.org_unit_id, toDate(c.date)

UNION ALL

-- ── total LOC per day (used to derive lines_per_commit) ─────────────────
SELECT
    toString(lower(c.author_email))                  AS person_id,
    p.org_unit_id                                    AS org_unit_id,
    toString(assumeNotNull(toDate(c.date)))                         AS metric_date,
    'loc'                                            AS metric_key,
    toFloat64(sum(c.lines_added + c.lines_removed))  AS metric_value
FROM silver.class_git_commits AS c FINAL
LEFT JOIN insight.people AS p ON lower(c.author_email) = p.person_id
WHERE c.is_merge_commit = 0
  AND c.author_email != ''
  AND c.date IS NOT NULL
GROUP BY lower(c.author_email), p.org_unit_id, toDate(c.date)

UNION ALL

-- ── clean_loc per day: lines added in code files (not spec/config) ──────
-- file_category is computed inline (mirrors fct_git_file_change logic);
-- person/date come from the joined commit (file_changes has no author).
SELECT
    toString(lower(c.author_email))                  AS person_id,
    p.org_unit_id                                    AS org_unit_id,
    toString(assumeNotNull(toDate(c.date)))                         AS metric_date,
    'clean_loc'                                      AS metric_key,
    toFloat64(sum(fc.lines_added))                   AS metric_value
FROM silver.class_git_file_changes AS fc FINAL
INNER JOIN silver.class_git_commits AS c FINAL
       ON c.tenant_id   = fc.tenant_id
      AND c.commit_hash = fc.commit_hash
      AND c.project_key = fc.project_key
      AND c.repo_slug   = fc.repo_slug
LEFT JOIN insight.people AS p ON lower(c.author_email) = p.person_id
WHERE c.is_merge_commit = 0
  AND c.author_email != ''
  AND c.date IS NOT NULL
  AND multiIf(
        match(fc.file_path, '(?i)(\\.spec\\.|\\.test\\.|__tests__/|/tests?/)'), 'spec',
        match(fc.file_path, '(?i)(\\.lock$|package-lock\\.json|yarn\\.lock|poetry\\.lock|\\.ya?ml$|\\.toml$|\\.cfg$|\\.ini$)'), 'config',
        'code'
      ) = 'code'
GROUP BY lower(c.author_email), p.org_unit_id, toDate(c.date)

UNION ALL

-- ── prs_created per day (by PR open date) ───────────────────────────────
SELECT
    toString(if(pr.author_email != '', lower(pr.author_email), lower(pr.author_name))) AS person_id,
    p.org_unit_id                                    AS org_unit_id,
    toString(assumeNotNull(toDate(pr.created_on)))                  AS metric_date,
    'prs_created'                                    AS metric_key,
    toFloat64(count())                               AS metric_value
FROM silver.class_git_pull_requests AS pr FINAL
LEFT JOIN insight.people AS p
       ON if(pr.author_email != '', lower(pr.author_email), lower(pr.author_name)) = p.person_id
WHERE (pr.author_email != '' OR pr.author_name != '')
  AND pr.created_on IS NOT NULL
GROUP BY person_id, p.org_unit_id, toDate(pr.created_on)

UNION ALL

-- ── prs_merged per day (by merge date, state=merged) ────────────────────
SELECT
    toString(if(pr.author_email != '', lower(pr.author_email), lower(pr.author_name))) AS person_id,
    p.org_unit_id                                    AS org_unit_id,
    toString(assumeNotNull(toDate(pr.closed_on)))                   AS metric_date,
    'prs_merged'                                     AS metric_key,
    toFloat64(count())                               AS metric_value
FROM silver.class_git_pull_requests AS pr FINAL
LEFT JOIN insight.people AS p
       ON if(pr.author_email != '', lower(pr.author_email), lower(pr.author_name)) = p.person_id
WHERE (pr.author_email != '' OR pr.author_name != '')
  AND lower(pr.state) = 'merged'
  AND pr.closed_on IS NOT NULL
GROUP BY person_id, p.org_unit_id, toDate(pr.closed_on)

UNION ALL

-- ── pr_size: one row per PR, value = LOC of that PR ─────────────────────
-- Aggregator computes period median (quantileExact 0.5).
SELECT
    toString(if(pr.author_email != '', lower(pr.author_email), lower(pr.author_name))) AS person_id,
    p.org_unit_id                                    AS org_unit_id,
    toString(assumeNotNull(toDate(pr.created_on)))                  AS metric_date,
    'pr_size'                                        AS metric_key,
    toFloat64(pr.lines_added + pr.lines_removed)     AS metric_value
FROM silver.class_git_pull_requests AS pr FINAL
LEFT JOIN insight.people AS p
       ON if(pr.author_email != '', lower(pr.author_email), lower(pr.author_name)) = p.person_id
WHERE (pr.author_email != '' OR pr.author_name != '')
  AND pr.created_on IS NOT NULL

UNION ALL

-- ── pr_cycle_time_h: one row per merged PR, value = hours opened→merged ─
-- This is opened → merged (not opened → first review). Real "review time"
-- needs author↔reviewer identity-resolution which is not yet built.
-- Negative diffs (closed_on < created_on, dirty data) are clamped to NULL.
SELECT
    toString(if(pr.author_email != '', lower(pr.author_email), lower(pr.author_name))) AS person_id,
    p.org_unit_id                                    AS org_unit_id,
    toString(assumeNotNull(toDate(pr.closed_on)))                   AS metric_date,
    'pr_cycle_time_h'                                AS metric_key,
    assumeNotNull(toFloat64(dateDiff('second', pr.created_on, pr.closed_on) / 3600.0)) AS metric_value
FROM silver.class_git_pull_requests AS pr FINAL
LEFT JOIN insight.people AS p
       ON if(pr.author_email != '', lower(pr.author_email), lower(pr.author_name)) = p.person_id
WHERE (pr.author_email != '' OR pr.author_name != '')
  AND lower(pr.state) = 'merged'
  AND pr.closed_on IS NOT NULL
  AND pr.created_on IS NOT NULL
  AND pr.closed_on >= pr.created_on
;

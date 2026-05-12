{{ config(
    materialized='table',
    schema='silver',
    engine='ReplacingMergeTree',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    tags=['silver']
) }}

-- All CTEs bucket by commit-date week (toStartOfWeek(commit.date, 1)) so the
-- LEFT JOINs on `week` align rows from the same activity window.
-- For prs_merged the week is taken from the merge commit's date (when the
-- merge_commit_hash resolves to a commit row), falling back to the PR
-- closed_on week. This avoids commit-week vs PR-close-week drift.
--
-- Why anchor + LEFT JOINs instead of FULL OUTER JOIN ... USING:
-- ClickHouse 25.3 fills unmatched per-side columns with the column's default
-- value (empty string for non-Nullable String, 1970-01-01 for Date) instead
-- of NULL after FOJ. A subsequent `coalesce(commits.person_key, …)` then
-- picks the default and the joining key is lost. We anchor on the union of
-- all (tenant_id, person_key, week) tuples and LEFT JOIN each metric CTE
-- onto it, which keeps the join key authoritative on the anchor side.

WITH commits AS (
    SELECT
        tenant_id,
        person_key,
        week,
        count() AS commits
    FROM {{ ref('fct_git_commit') }}
    WHERE is_merge_commit = 0
      AND person_key != ''
      AND date IS NOT NULL
    GROUP BY tenant_id, person_key, week
),
loc AS (
    SELECT
        tenant_id,
        person_key,
        week,
        SUM(if(file_category = 'code', lines_added, 0)) AS code_loc,
        SUM(if(file_category = 'spec', lines_added, 0)) AS spec_lines
    FROM {{ ref('fct_git_file_change') }}
    WHERE is_merge_commit = 0
      AND person_key != ''
      AND week IS NOT NULL
    GROUP BY tenant_id, person_key, week
),
prs AS (
    SELECT
        pr.tenant_id,
        pr.person_key,
        coalesce(mc.week, toStartOfWeek(pr.closed_on, 1)) AS week,
        count() AS prs_merged
    FROM {{ ref('fct_git_pr') }} AS pr
    LEFT JOIN {{ ref('fct_git_commit') }} AS mc
        ON  mc.tenant_id   = pr.tenant_id
        AND mc.source_id   = pr.source_id
        AND mc.project_key = pr.project_key
        AND mc.repo_slug   = pr.repo_slug
        AND mc.commit_hash = pr.merge_commit_hash
    WHERE pr.state_norm = 'merged'
      AND pr.closed_on IS NOT NULL
      AND pr.person_key != ''
    GROUP BY pr.tenant_id, pr.person_key, week
),
all_keys AS (
    SELECT tenant_id, person_key, week FROM commits
    UNION DISTINCT
    SELECT tenant_id, person_key, week FROM loc
    UNION DISTINCT
    SELECT tenant_id, person_key, week FROM prs
)
SELECT
    ak.tenant_id                                                 AS tenant_id,
    ak.person_key                                                AS person_key,
    ak.week                                                      AS week,
    concat(
        coalesce(ak.tenant_id, ''),
        '|',
        ak.person_key,
        '|',
        toString(ak.week)
    )                                                            AS unique_key,
    coalesce(commits.commits, 0)                                 AS commits,
    coalesce(prs.prs_merged, 0)                                  AS prs_merged,
    coalesce(loc.code_loc, 0)                                    AS code_loc,
    coalesce(loc.spec_lines, 0)                                  AS spec_lines
FROM all_keys ak
LEFT JOIN commits USING (tenant_id, person_key, week)
LEFT JOIN loc     USING (tenant_id, person_key, week)
LEFT JOIN prs     USING (tenant_id, person_key, week)

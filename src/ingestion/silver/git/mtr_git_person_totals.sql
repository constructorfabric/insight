{{ config(
    materialized='table',
    schema='silver',
    engine='ReplacingMergeTree',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    tags=['silver']
) }}

-- reviews_given intentionally excluded: fct_git_review.person_key lives in
-- the reviewer-login namespace (GitHub login, Bitbucket display_name), which
-- does not match the email-based person_key used by commits/PRs. Returns
-- once identity resolution bridges the two namespaces.
--
-- Why anchor + LEFT JOINs instead of FULL OUTER JOIN ... USING:
-- ClickHouse 25.3 fills unmatched per-side columns with the column's default
-- value (empty string for non-Nullable String) instead of NULL after FOJ.
-- A subsequent `coalesce(prs.person_key, commits.person_key, …)` then picks
-- the empty string and the joining key is lost. We anchor on the union of
-- all (tenant_id, person_key) tuples and LEFT JOIN each metric CTE onto it,
-- which keeps the join key authoritative on the anchor side.

WITH all_keys AS (
    SELECT tenant_id, person_key
    FROM {{ ref('fct_git_pr') }}
    WHERE person_key != ''
    UNION DISTINCT
    SELECT tenant_id, person_key
    FROM {{ ref('fct_git_commit') }}
    WHERE is_merge_commit = 0 AND person_key != ''
    UNION DISTINCT
    SELECT tenant_id, person_key
    FROM {{ ref('fct_git_file_change') }}
    WHERE is_merge_commit = 0
      AND file_category = 'code'
      AND person_key != ''
),
prs AS (
    SELECT
        tenant_id,
        person_key,
        count()                                                AS prs_created,
        countIf(state_norm = 'merged')                         AS prs_merged,
        avgIf(cycle_time_h, cycle_time_h IS NOT NULL)          AS avg_pr_cycle_time_h
    FROM {{ ref('fct_git_pr') }}
    WHERE person_key != ''
    GROUP BY tenant_id, person_key
),
commits AS (
    SELECT
        tenant_id,
        person_key,
        count()                           AS commits,
        SUM(lines_added + lines_removed)  AS loc
    FROM {{ ref('fct_git_commit') }}
    WHERE is_merge_commit = 0
      AND person_key != ''
    GROUP BY tenant_id, person_key
),
clean AS (
    SELECT
        tenant_id,
        person_key,
        SUM(lines_added) AS clean_loc
    FROM {{ ref('fct_git_file_change') }}
    WHERE is_merge_commit = 0
      AND file_category = 'code'
      AND person_key != ''
    GROUP BY tenant_id, person_key
)
SELECT
    ak.tenant_id                          AS tenant_id,
    ak.person_key                         AS person_key,
    concat(coalesce(ak.tenant_id, ''), '|', ak.person_key) AS unique_key,
    coalesce(prs.prs_created, 0)          AS prs_created,
    coalesce(prs.prs_merged, 0)           AS prs_merged,
    prs.avg_pr_cycle_time_h               AS avg_pr_cycle_time_h,
    coalesce(commits.commits, 0)          AS commits,
    coalesce(commits.loc, 0)              AS loc,
    coalesce(clean.clean_loc, 0)          AS clean_loc
FROM all_keys ak
LEFT JOIN prs     USING (tenant_id, person_key)
LEFT JOIN commits USING (tenant_id, person_key)
LEFT JOIN clean   USING (tenant_id, person_key)

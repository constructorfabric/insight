-- =====================================================================
-- git_bullet_rows — Phase A rewrite (issue #433 §4.1, §4.6)
-- =====================================================================
--
-- Same scan-consolidation + Date-type rewrite as PR #478
-- (task_delivery), PR #480 (collab), PR #490 (ai), PR #491
-- (code_quality), applied to the git section. This PR is **view-only**:
--
--   1. SCAN CONSOLIDATION (issue #433 §3.5). View dropped from 7
--      UNION-ALL branches to 6 — branches 1 and 2 of the predecessor
--      (`commits` and `loc`) shared the same source class
--      (`silver.class_git_commits FINAL`), the same filter, the same
--      `GROUP BY (person, day)`. They are now consolidated into a
--      single branch that reads `class_git_commits` once and emits
--      both `metric_key`s via `ARRAY JOIN` over a tuple array.
--
--      The remaining 5 source branches (`clean_loc`, `prs_created`,
--      `prs_merged`, `pr_size`, `pr_cycle_time_h`) each have a
--      genuinely different shape — different sources, different
--      grains (per-day counter vs per-PR distribution), and/or
--      different date columns (`created_on` vs `closed_on`) — and
--      cannot be safely consolidated without changing observable
--      behavior. Tried in an earlier draft of this rewrite and
--      backed out: an ARRAY JOIN over different date columns
--      produced row-count drifts vs the predecessor (`pr_size`
--      1057 → 987, `pr_cycle_time_h` 751 → 750 etc), which would
--      have made this PR a behavior change rather than a structural
--      refactor.
--
--      Predecessor scans (7×):
--        silver.class_git_commits FINAL          — 3× (commits, loc, clean_loc join)
--        silver.class_git_pull_requests FINAL    — 4× (prs_created, prs_merged, pr_size, pr_cycle_time_h)
--        silver.class_git_file_changes FINAL     — 1× (clean_loc, joined to commits)
--
--      Rewrite scans (6×):
--        silver.class_git_commits FINAL          — 2× (1 consolidated for commits+loc, 1 for clean_loc join)
--        silver.class_git_pull_requests FINAL    — 4× (unchanged)
--        silver.class_git_file_changes FINAL     — 1× (unchanged)
--
--      Modest improvement (-1 scan), behavior-preserving. Further
--      consolidation of the PR-source branches is a follow-up that
--      requires reworking the date-bucketing semantics (PRs span
--      created_on vs closed_on across different rows).
--
--   2. `metric_date` type. Previously `String` via
--      `toString(assumeNotNull(toDate(...)))`; now native `Date`.
--      Same change as PR #478 / #480 / #490 / #491. Unlocks
--      downstream MergeTree min/max statistics on any materialization.
--
--   No `query_ref` changes. The IC Bullet Git `query_ref` was already
--   rewritten by `m20260430_000001_update_git_bullet` — the early
--   reference implementation of the wide-aggregate + ARRAY JOIN +
--   composite-Σnum/Σden pattern that the rest of the series followed.
--   The new view emits the same 7 `metric_key`s as the predecessor,
--   so the existing query_ref consumes it without modification.
--   Composite ratios (`merge_rate`, `lines_per_commit`,
--   `commits_per_active_day`) live only in `query_ref`.
--
-- Branch shape after rewrite (6 branches):
--
--   1. silver.class_git_commits FINAL                    → 2 keys via ARRAY JOIN
--                                                          (commits, loc)
--   2. silver.class_git_file_changes FINAL ⋈ commits FINAL → 1 key (clean_loc)
--   3. silver.class_git_pull_requests FINAL              → 1 key (prs_created)
--   4. silver.class_git_pull_requests FINAL              → 1 key (prs_merged)
--   5. silver.class_git_pull_requests FINAL              → 1 key (pr_size)
--   6. silver.class_git_pull_requests FINAL              → 1 key (pr_cycle_time_h)
--
-- 7 distinct metric_keys after rewrite (unchanged). Composite-ratio
-- metric_keys visible on FE live ONLY in the `query_ref` projection.
--
-- Note on TEAM bullet absence: there is no `TEAM_BULLET_GIT` seed —
-- only `IC_BULLET_GIT` (`…18`). Adding a Team-scope variant is a
-- feature decision, out of scope for this #433 cleanup. Future work.
--
-- Identity / dedup / tenant / not-emitted notes unchanged from
-- predecessor (`20260430000000_git-bullet-expand.sql` header); see
-- there for the per-PR author_email caveat (Bitbucket Cloud REST API
-- doesn't return author email on PR payloads → person_id falls back
-- to lower(author_name)).
-- =====================================================================

DROP VIEW IF EXISTS insight.git_bullet_rows;

CREATE VIEW insight.git_bullet_rows
(
    `person_id`    String,
    `org_unit_id`  Nullable(String),
    `metric_date`  Date,
    `metric_key`   String,
    `metric_value` Float64
)
AS

-- ─── Branch 1: class_git_commits — commits + loc via ARRAY JOIN ──────
-- Per-person-per-day aggregate over commits. Emits 2 metric_keys from
-- one scan:
--   commits = countDistinct(commit_hash)
--   loc     = Σ(lines_added + lines_removed)
SELECT
    pp.person_id                                     AS person_id,
    pp.org_unit_id                                   AS org_unit_id,
    pp.metric_date                                   AS metric_date,
    kv.1                                             AS metric_key,
    kv.2                                             AS metric_value
FROM (
    SELECT
        lower(c.author_email)                        AS person_id,
        any(p.org_unit_id)                           AS org_unit_id,
        assumeNotNull(toDate(c.date))                               AS metric_date,
        toFloat64(countDistinct(c.commit_hash))      AS commits_v,
        toFloat64(sum(c.lines_added + c.lines_removed)) AS loc_v
    FROM silver.class_git_commits AS c FINAL
    LEFT JOIN insight.people AS p ON lower(c.author_email) = p.person_id
    WHERE c.is_merge_commit = 0
      AND c.author_email != ''
      AND c.date IS NOT NULL
    GROUP BY lower(c.author_email), assumeNotNull(toDate(c.date))
) AS pp
ARRAY JOIN [
    ('commits', pp.commits_v),
    ('loc',     pp.loc_v)
] AS kv

UNION ALL

-- ─── Branch 2: class_git_file_changes ⋈ class_git_commits ────────────
-- clean_loc: Σ(file_changes.lines_added) for non-spec / non-config
-- files (file_category='code'). Inline-recomputed file_category mirrors
-- `fct_git_file_change` logic (no reads of `fct_*` per the
-- bronze→silver→gold rule).
SELECT
    lower(c.author_email)                            AS person_id,
    p.org_unit_id                                    AS org_unit_id,
    assumeNotNull(toDate(c.date))                                   AS metric_date,
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
GROUP BY lower(c.author_email), p.org_unit_id, assumeNotNull(toDate(c.date))

UNION ALL

-- ─── Branch 3: prs_created (per-day counter, dated by created_on) ────
SELECT
    if(pr.author_email != '', lower(pr.author_email), lower(pr.author_name)) AS person_id,
    p.org_unit_id                                    AS org_unit_id,
    assumeNotNull(toDate(pr.created_on))                            AS metric_date,
    'prs_created'                                    AS metric_key,
    toFloat64(count())                               AS metric_value
FROM silver.class_git_pull_requests AS pr FINAL
LEFT JOIN insight.people AS p
       ON if(pr.author_email != '', lower(pr.author_email), lower(pr.author_name)) = p.person_id
WHERE (pr.author_email != '' OR pr.author_name != '')
  AND pr.created_on IS NOT NULL
GROUP BY person_id, p.org_unit_id, assumeNotNull(toDate(pr.created_on))

UNION ALL

-- ─── Branch 4: prs_merged (per-day counter, dated by closed_on) ──────
SELECT
    if(pr.author_email != '', lower(pr.author_email), lower(pr.author_name)) AS person_id,
    p.org_unit_id                                    AS org_unit_id,
    assumeNotNull(toDate(pr.closed_on))                             AS metric_date,
    'prs_merged'                                     AS metric_key,
    toFloat64(count())                               AS metric_value
FROM silver.class_git_pull_requests AS pr FINAL
LEFT JOIN insight.people AS p
       ON if(pr.author_email != '', lower(pr.author_email), lower(pr.author_name)) = p.person_id
WHERE (pr.author_email != '' OR pr.author_name != '')
  AND lower(pr.state) = 'merged'
  AND pr.closed_on IS NOT NULL
GROUP BY person_id, p.org_unit_id, assumeNotNull(toDate(pr.closed_on))

UNION ALL

-- ─── Branch 5: pr_size (per-PR distribution, dated by created_on) ────
-- One row per PR, value = LOC of that PR. query_ref aggregates as a
-- period median via `quantileExactIf(0.5)`.
SELECT
    if(pr.author_email != '', lower(pr.author_email), lower(pr.author_name)) AS person_id,
    p.org_unit_id                                    AS org_unit_id,
    assumeNotNull(toDate(pr.created_on))                            AS metric_date,
    'pr_size'                                        AS metric_key,
    toFloat64(pr.lines_added + pr.lines_removed)     AS metric_value
FROM silver.class_git_pull_requests AS pr FINAL
LEFT JOIN insight.people AS p
       ON if(pr.author_email != '', lower(pr.author_email), lower(pr.author_name)) = p.person_id
WHERE (pr.author_email != '' OR pr.author_name != '')
  AND pr.created_on IS NOT NULL

UNION ALL

-- ─── Branch 6: pr_cycle_time_h (per-merged-PR, dated by closed_on) ───
-- One row per merged PR, value = hours opened→merged. Negative diffs
-- (closed_on < created_on, dirty data) are excluded by the
-- `closed_on >= created_on` filter — same as the predecessor.
SELECT
    if(pr.author_email != '', lower(pr.author_email), lower(pr.author_name)) AS person_id,
    p.org_unit_id                                    AS org_unit_id,
    assumeNotNull(toDate(pr.closed_on))                             AS metric_date,
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

-- Operational guardrail, not a semantic invariant: a connector instance that
-- has stopped collecting file changes fails here instead of quietly serving
-- commit-level sizes under an Unknown file grain.
--
-- Both bounds are vars because both are policy. `git_coverage_min_pct` is the
-- floor a healthy instance stays above; `git_coverage_min_sample` keeps a new
-- or tiny instance out of it, where one lost diff out of three reads as 66%
-- and means nothing statistically.
--
-- Recent window only. The all-time share carries history no re-read can reach,
-- so it is a floor that never fully recovers and would fail forever on any
-- installation that predates the collection fix.
SELECT
    tenant_id,
    data_source,
    source_id,
    recent_commits_requiring_file_changes,
    recent_collected_pct
FROM {{ ref('git_file_change_coverage') }}
WHERE recent_commits_requiring_file_changes >= {{ var('git_coverage_min_sample') }}
  AND recent_collected_pct < {{ var('git_coverage_min_pct') }}

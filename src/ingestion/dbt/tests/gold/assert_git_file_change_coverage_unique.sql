-- Build-integrity check (untagged → error severity under `dbt build`).
-- One row per connector instance is the premise the coverage alarm rests on:
-- assert_git_file_change_coverage_holds gates on
-- recent_commits_requiring_file_changes >= git_coverage_min_sample, so a finer
-- grain divides an instance's sample below that gate and disarms the alarm
-- instead of failing it. The grain holds by construction; a widened GROUP BY
-- is what would break it.
SELECT
    tenant_id,
    data_source,
    source_id,
    count() AS row_count
FROM {{ ref('git_file_change_coverage') }}
GROUP BY tenant_id, data_source, source_id
HAVING count() > 1

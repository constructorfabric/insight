{{ config(
    tags=['data_quality'],
    severity='warn',
    store_failures=true,
    meta={
        'title': 'ic_chart_loc LOC values non-negative',
        'domain': 'gold',
        'category': 'physical_bound',
        'tier': 'error',
        'remediation': 'A Gold LOC bucket is negative: code_loc, spec_lines and config_loc are git file-category line counts and must be >= 0. Inspect the insight.ic_chart_loc view definition — a row here usually means the view is deriving a count by subtraction instead of summing git lines_added directly.'
    }
) }}
-- Gold-layer check. `insight.ic_chart_loc` is a database view (not a dbt model);
-- a singular test reads it via the registered `gold` source. The three LOC
-- buckets are sums of git lines_added per file_category, so they must always be
-- non-negative by construction. Any returned row means the view math is wrong.
SELECT
    person_id,
    metric_date,
    code_loc,
    spec_lines,
    config_loc
FROM {{ source('gold', 'ic_chart_loc') }}
WHERE code_loc   < 0
   OR spec_lines < 0
   OR config_loc < 0

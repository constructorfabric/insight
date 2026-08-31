{{ metric_observations_table() }}

SELECT
    tenant_id,
    source_key,
    entity_type,
    entity_id,
    -- The read-time resolution key: rows only merge within one account
    -- binding, or a bot's contributions would ride a human's summary row.
    account_source_type,
    account_source_id,
    account_id,
    metric_date,
    CAST(NULL AS Nullable(DateTime64(3))) AS observed_at,
    measure_key,
    toNullable(sum(contribution)) AS value,
    CAST(NULL AS Nullable(String)) AS subject_key,
    dimensions
FROM {{ ref('git_metric_evidence') }}
WHERE measure_key NOT IN (
    'commit_day',
    'commit_change_size',
    'pr_commit_count',
    'pr_cycle_hours',
    'pr_change_size',
    'pr_first_review_hours',
    'pr_review_wait_share',
    'pr_review_to_merge_hours',
    'pr_approval_to_merge_hours'
)
GROUP BY tenant_id, source_key, entity_type, entity_id, account_source_type, account_source_id, account_id, metric_date, measure_key, dimensions

UNION ALL

SELECT
    tenant_id,
    source_key,
    entity_type,
    entity_id,
    account_source_type,
    account_source_id,
    account_id,
    metric_date,
    CAST(NULL AS Nullable(DateTime64(3))) AS observed_at,
    measure_key,
    contribution AS value,
    subject_key,
    dimensions
FROM {{ ref('git_metric_evidence') }}
WHERE measure_key IN (
    'commit_day',
    'commit_change_size',
    'pr_commit_count',
    'pr_cycle_hours',
    'pr_change_size',
    'pr_first_review_hours',
    'pr_review_wait_share',
    'pr_review_to_merge_hours',
    'pr_approval_to_merge_hours'
)

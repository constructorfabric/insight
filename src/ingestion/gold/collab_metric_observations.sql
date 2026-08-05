{{ metric_observations_table() }}

SELECT
    tenant_id,
    source_key,
    entity_type,
    -- entity_id arrives ALREADY canonical from evidence (resolved once per
    -- build); '' marks a row identity could not resolve, which stays out of
    -- every serving relation and is counted by identity_resolution_coverage.
    entity_id,
    metric_date,
    CAST(NULL AS Nullable(DateTime64(3))) AS observed_at,
    measure_key,
    -- Grouped to CANONICAL grain: a person's several source accounts land in
    -- ONE row per (person, day, measure, subject, dims). Additive measures sum
    -- across accounts; day flags collapse by their own semantics (max — active
    -- under any account; min for meeting_free_day — free only if every account
    -- was); distinct subjects stay their own rows via subject_key in the key.
    toNullable({{ collapsed_value('contribution', max_keys=['chat_active_day'], min_keys=['meeting_free_day']) }}) AS value,
    subject_key,
    dimensions
FROM {{ ref('collab_metric_evidence') }}
WHERE entity_id != ''
GROUP BY tenant_id, source_key, entity_type, entity_id, metric_date, measure_key, subject_key, dimensions

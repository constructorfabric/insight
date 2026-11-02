{{ metric_observations_table() }}

SELECT
    tenant_id,
    source_key,
    entity_type,
    entity_id,
    -- The read-time resolution key: a guest account's rows merge only within
    -- that account, so a binding moves exactly its contributions.
    account_source_type,
    account_source_id,
    account_id,
    metric_date,
    CAST(NULL AS Nullable(DateTime64(3))) AS observed_at,
    measure_key,
    -- One row per (source identity, day, measure, subject, dims). The runtime
    -- resolves identities to persons as it serves and applies each measure's
    -- alias collapse there; distinct subjects stay their own rows via
    -- subject_key in the key, so a per-subject collapse can see them.
    toNullable(sum(contribution)) AS value,
    subject_key,
    dimensions
FROM {{ ref('collab_metric_evidence') }}
GROUP BY tenant_id, source_key, entity_type, entity_id, account_source_type, account_source_id, account_id, metric_date, measure_key, subject_key, dimensions

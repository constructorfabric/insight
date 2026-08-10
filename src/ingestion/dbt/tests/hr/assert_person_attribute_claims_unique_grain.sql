-- Build-integrity guard: at most one claim per (tenant, source type, source
-- instance, account, field, observed_at). A surplus row means two producers
-- encoded the same logical claim under different unique_key values, so RMT
-- cannot collapse them and the gold interval builder sees a same-instant tie
-- with undefined lead() ordering.
SELECT
    insight_tenant_id,
    insight_source_type,
    insight_source_id,
    source_account_id,
    field_id,
    observed_at,
    count()                   AS row_count,
    uniqExact(unique_key)     AS distinct_unique_keys
FROM {{ ref('class_person_attribute_claims') }} FINAL
GROUP BY
    insight_tenant_id,
    insight_source_type,
    insight_source_id,
    source_account_id,
    field_id,
    observed_at
HAVING count() > 1

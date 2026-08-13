{{ config(
    tags=['data_quality'],
    severity='warn',
    store_failures=true,
    meta={
        'title': 'One class_people row per person',
        'domain': 'hr',
        'category': 'grain',
        'tier': 'error',
        'remediation': 'class_people is a current-state snapshot: exactly one row per (workspace, source, source_person_id). More than one means a staging `__to_class_people` view reintroduced a version axis into `unique_key` (e.g. appending lastChanged), so silver RMT can no longer collapse the versions and every changed record becomes a second permanently-current row — inflating headcount. Check the `unique_key` expression in the hr-directory staging views; it must stay entity-level. See ADR-0004 and silver/_shared/class_people.sql.'
    }
) }}
-- class_people is a snapshot, not an SCD2 history table: HR attribute history
-- lives in the per-source `*_snapshot` / `*_fields_history` chain. So the grain
-- here is one row per person per source, and any surplus row is a duplicate
-- that inflates every row-counting consumer (headcount, join fan-out).
--
-- FINAL still catches the regression this guards against: if a version axis
-- comes back into `unique_key`, the version rows have *different* keys, so FINAL
-- cannot collapse them and they surface here. A genuine transient RMT duplicate
-- (same key, pre-merge) is correctly collapsed and is not a violation.
SELECT
    workspace_id,
    source,
    source_person_id,
    count()                                AS row_count,
    uniqExact(unique_key)                  AS distinct_unique_keys,
    groupArray(toString(valid_from))        AS valid_from_values
FROM silver.class_people FINAL
GROUP BY
    workspace_id,
    source,
    source_person_id
HAVING count() > 1

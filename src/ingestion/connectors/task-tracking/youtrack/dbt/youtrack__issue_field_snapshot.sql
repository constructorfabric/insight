{{ config(materialized='table', schema='staging', engine=insight_engine('MergeTree'), order_by=['insight_source_id', 'issue_id', 'field_id', 'observed_at'], tags=['youtrack', 'staging']) }}

WITH observations AS (
    -- INVARIANT: deduplicated by key, not FINAL; FINAL merges whole payloads in memory.
    SELECT insight_source_id, issue_id, id_readable, created_at, reporter_id, observed_at, custom_fields, payload
    FROM {{ ref('youtrack__issue_observations') }}
    LIMIT 1 BY unique_key
), custom AS (
    SELECT insight_source_id, issue_id, id_readable, created_at, reporter_id, observed_at,
        arrayJoin(JSONExtractArrayRaw(custom_fields)) AS f
    FROM observations
), field_types AS (
    SELECT insight_source_id, field_id, observed_at, field_type
    FROM {{ ref('youtrack__field_observations') }} FINAL
), first_field_types AS (
    SELECT insight_source_id, field_id, argMin(field_type, observed_at) AS field_type
    FROM field_types GROUP BY insight_source_id, field_id
), custom_fields AS (
    -- The value's $type names the cardinality only for the Multi*/Single*
    -- families; State, Date, Owned and the like fall back to the field's type
    -- as observed at or before the snapshot, as youtrack__activities reads it.
    SELECT c.insight_source_id AS insight_source_id, c.issue_id AS issue_id, c.id_readable AS id_readable,
        c.created_at AS created_at, c.reporter_id AS reporter_id, c.observed_at AS observed_at, c.f AS f,
        coalesce(nullIf(t.field_type, ''), ff.field_type, '') AS observed_field_type
    FROM (SELECT *, JSONExtractString(f, 'projectCustomField', 'field', 'id') AS custom_field_id FROM custom) AS c
    ASOF LEFT JOIN field_types AS t
        ON t.insight_source_id = c.insight_source_id AND t.field_id = c.custom_field_id AND c.observed_at >= t.observed_at
    LEFT JOIN first_field_types AS ff ON ff.insight_source_id = c.insight_source_id AND ff.field_id = c.custom_field_id
), builtin_fields AS (
    SELECT insight_source_id, issue_id, id_readable, created_at, reporter_id, observed_at,
        builtin.1 AS field_id, builtin.2 AS field_name, builtin.3 AS field_cardinality,
        builtin.4 AS raw_value, builtin.5 AS value_types
    FROM observations
    ARRAY JOIN [
        ('summary', 'Summary', 'single', JSONExtractRaw(payload, 'summary'), ['string_literal']),
        ('description', 'Description', 'single', JSONExtractRaw(payload, 'description'), ['string_literal']),
        ('tags', 'Tags', 'multi', JSONExtractRaw(payload, 'tags'), ['opaque_id']),
        ('project', 'Project', 'single', JSONExtractRaw(payload, 'project'), ['opaque_id'])
    ] AS builtin
), fields AS (
    SELECT insight_source_id, issue_id, id_readable, created_at, reporter_id, observed_at,
        JSONExtractString(f, 'projectCustomField', 'field', 'id') AS field_id,
        JSONExtractString(f, 'name') AS field_name,
        multiIf(startsWith(JSONExtractString(f, '$type'), 'Multi'), 'multi',
                startsWith(JSONExtractString(f, '$type'), 'Single'), 'single',
                JSONExtractString(f, '$type') IN ('SimpleIssueCustomField', 'TextIssueCustomField', 'PeriodIssueCustomField'), 'single',
                observed_field_type = '', 'unknown',
                endsWith(observed_field_type, '[*]'), 'multi',
                'single') AS field_cardinality,
        {{ youtrack_values("JSONExtractRaw(f, 'value')") }} AS pairs,
        {{ youtrack_value_types("JSONExtractRaw(f, 'value')") }} AS value_types,
        toUInt8(JSONExtractBool(f, 'value', 'isResolved')) AS value_is_resolved
    FROM custom_fields
    UNION ALL
    SELECT insight_source_id, issue_id, id_readable, created_at, reporter_id, observed_at,
        field_id, field_name, field_cardinality, {{ youtrack_values('raw_value') }}, value_types, toUInt8(0)
    FROM builtin_fields
)
SELECT * FROM fields WHERE field_id != ''

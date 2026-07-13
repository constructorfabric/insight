-- depends_on: {{ ref('youtrack__bronze_promoted') }}
{{ config(
    materialized='view',
    alias='youtrack__task_statuses',
    schema='staging',
    tags=['youtrack', 'silver:class_task_statuses']
) }}

-- Per-source status dimension; unioned into `silver.class_task_statuses` via `union_by_tag`.
-- YouTrack has no global status table and no Jira-style statusCategory. The
-- equivalent signal is the State custom field's bundle values, each carrying a
-- boolean `isResolved`. We explode the State bundle(s) and map:
--   isResolved = true  -> status_category = 'done'   (task is closed)
--   isResolved = false -> status_category = 'in_progress'  (best-effort; see below)
-- See docs youtrack DESIGN (`cpt-insightspec-principle-youtrack-status-category`)
-- and issue #1541.
--
-- BEST-EFFORT lifecycle: `isResolved` cleanly identifies the terminal (done)
-- states, but YouTrack does not tag non-resolved states as "new" vs
-- "in progress". We default non-resolved states to `in_progress`; a finer split
-- would need a per-instance convention (e.g. bundle ordinal 0 = new) and is
-- deferred. The `done` signal — the only one Gold close-detection needs — is exact.
--
-- Requires `isResolved` in the State bundle values, added to the
-- `youtrack_project_custom_fields` stream field selection (connector.yaml).
--
-- `bundle_values_json` is the raw JSON array of bundle value objects. A State
-- bundle may be shared across projects, so the same value id appears in several
-- rows; `union_by_tag` dedups by `unique_key` (= source + status_id) to one row.

WITH state_fields AS (
    SELECT
        pcf.source_id                                       AS source_id,
        pcf.bundle_values_json                              AS bundle_values_json,
        pcf._airbyte_extracted_at                           AS _airbyte_extracted_at
    FROM {{ source('bronze_youtrack', 'youtrack_project_custom_fields') }} pcf
    WHERE lower(toString(pcf.value_type)) = 'state'
       OR toString(pcf.field_type_id) LIKE 'state%'
)
SELECT
    concat(toString(sf.source_id), '-', JSONExtractString(val_raw, 'id')) AS unique_key,
    sf.source_id                                            AS insight_source_id,
    CAST('youtrack' AS String)                              AS data_source,
    JSONExtractString(val_raw, 'id')                        AS status_id,
    JSONExtractString(val_raw, 'name')                      AS status_name,
    CAST(NULL AS Nullable(Int32))                           AS category_id,
    CAST(NULL AS Nullable(String))                          AS category_key,
    if(JSONExtractBool(val_raw, 'isResolved'), 'done', 'in_progress') AS status_category,
    toDateTime64(sf._airbyte_extracted_at, 3)               AS collected_at,
    toUnixTimestamp64Milli(sf._airbyte_extracted_at)        AS _version
FROM state_fields sf
ARRAY JOIN JSONExtractArrayRaw(ifNull(sf.bundle_values_json, '[]')) AS val_raw
WHERE JSONExtractString(val_raw, 'id') != ''

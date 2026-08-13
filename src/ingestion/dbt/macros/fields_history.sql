{% macro fields_history(snapshot_ref, entity_id_col, fields, fields_raw_data=[]) %}
{#
  Generates a field-level change log from a snapshot model.
  One row per changed field per version transition.

  Args:
    snapshot_ref:    ref() to the snapshot incremental model
    entity_id_col:   column name for the entity identifier
    fields:          list of top-level column names to track
    fields_raw_data: list of field names inside `raw_data` JSON column to track
                     (extracted via JSONExtractString; missing keys yield '')

  Output columns:
    entity_id, tenant_id, source_id, field_name, old_value, new_value, updated_at

  Note: field_name values from both lists appear without prefixes — consumers
  do not distinguish between top-level and raw_data fields.
#}

{% set all_fields = fields + fields_raw_data %}

WITH versioned AS (
    SELECT
        unique_key,
        {{ entity_id_col }} AS entity_id,
        tenant_id,
        source_id,
        {% for f in fields %}
        toString({{ f }}) AS {{ f }},
        {% endfor %}
        {% for f in fields_raw_data %}
        JSONExtractString(ifNull(toString(raw_data), '{}'), '{{ f }}') AS {{ f }},
        {% endfor %}
        _tracked_at AS updated_at,
        ROW_NUMBER() OVER (
            PARTITION BY unique_key ORDER BY _tracked_at
        ) AS version_num
    FROM {{ snapshot_ref }}
),

consecutive AS (
    SELECT
        curr.entity_id,
        curr.tenant_id,
        curr.source_id,
        curr.updated_at,
        {% for f in all_fields %}
        curr.{{ f }} AS curr_{{ f }},
        prev.{{ f }} AS prev_{{ f }}{{ ',' if not loop.last }}
        {% endfor %}
    FROM versioned curr
    INNER JOIN versioned prev
        ON curr.unique_key = prev.unique_key
        AND curr.version_num = prev.version_num + 1
)

{% for f in all_fields %}
SELECT
    entity_id, tenant_id, source_id,
    '{{ f }}' AS field_name,
    prev_{{ f }} AS old_value,
    curr_{{ f }} AS new_value,
    updated_at
FROM consecutive
WHERE curr_{{ f }} != prev_{{ f }}
UNION ALL
{% endfor %}

{% for f in all_fields %}
SELECT
    entity_id, tenant_id, source_id,
    '{{ f }}' AS field_name,
    '' AS old_value,
    {{ f }} AS new_value,
    updated_at
FROM versioned
WHERE version_num = 1
  AND {{ f }} != ''
{{ 'UNION ALL' if not loop.last }}
{% endfor %}

{% endmacro %}


{% macro fields_history_raw(snapshot_ref, entity_id_col, exclude_keys=[]) %}
{#
  Field-level change log over every key of the snapshot's `raw_data` payload,
  without naming the fields. One row per changed field per version transition.

  Args:
    snapshot_ref:  ref() to the snapshot incremental model
    entity_id_col: column name for the entity identifier
    exclude_keys:  keys to leave untracked

  Output columns:
    entity_id, tenant_id, source_id, field_name, old_value, new_value, updated_at

  Pair this with snapshot(check_raw_data_all=true) and the same exclude_keys: a
  field only carries a correct change timestamp if the snapshot versions on it.
#}

{#-
  Shaped as spillable GROUP BY aggregation, not window functions: aggregation
  state spills to disk under max_bytes_before_external_group_by, window sort
  state cannot spill and holds the whole exploded relation in memory.
-#}
WITH entity_versions AS (
    SELECT
        unique_key,
        arraySort(groupArray(_tracked_at)) AS version_times
    FROM {{ snapshot_ref }}
    GROUP BY unique_key
),

-- One (entity, field) group with the value at every version carrying the key.
-- Exploding pairs before grouping keeps the wide `raw_data` payload out of the
-- ARRAY JOIN output: replicating it once per key is what costs gigabytes.
field_values AS (
    SELECT
        unique_key,
        any({{ entity_id_col }}) AS entity_id,
        any(tenant_id) AS tenant_id,
        any(source_id) AS source_id,
        pair.1 AS field_name,
        CAST(groupArray((_tracked_at, pair.2)), 'Map(DateTime, String)') AS value_at
    FROM {{ snapshot_ref }}
    ARRAY JOIN CAST({{ raw_data_fields(exclude_keys) }}, 'Array(Tuple(String, String))') AS pair
    GROUP BY unique_key, field_name
),

-- INVARIANT: an absent key reads as '' — the timeline spans every version of
-- the entity, so a version that stops carrying the key diffs as a change to ''
-- and the first version diffs against '', both matching fields_history.
timelines AS (
    SELECT
        f.entity_id AS entity_id,
        f.tenant_id AS tenant_id,
        f.source_id AS source_id,
        f.field_name AS field_name,
        e.version_times AS times,
        arrayMap(t -> f.value_at[t], e.version_times) AS vals
    FROM field_values f
    INNER JOIN entity_versions e ON f.unique_key = e.unique_key
)

SELECT
    entity_id,
    tenant_id,
    source_id,
    field_name,
    change.1 AS old_value,
    change.2 AS new_value,
    change.3 AS updated_at
FROM timelines
ARRAY JOIN arrayFilter(c -> c.1 != c.2,
    arrayMap(i -> (if(i = 1, '', vals[i - 1]), vals[i], times[i]), arrayEnumerate(vals))) AS change

{% endmacro %}

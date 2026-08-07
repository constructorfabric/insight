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

WITH versioned AS (
    SELECT
        unique_key,
        {{ entity_id_col }} AS entity_id,
        tenant_id,
        source_id,
        _tracked_at AS updated_at,
        {{ raw_data_fields(exclude_keys) }} AS fields,
        ROW_NUMBER() OVER (
            PARTITION BY unique_key ORDER BY _tracked_at
        ) AS version_num
    FROM {{ snapshot_ref }}
),

transitions AS (
    SELECT
        curr.entity_id AS entity_id,
        curr.tenant_id AS tenant_id,
        curr.source_id AS source_id,
        curr.updated_at AS updated_at,
        curr.fields AS curr_fields,
        prev.fields AS prev_fields
    FROM versioned curr
    INNER JOIN versioned prev
        ON curr.unique_key = prev.unique_key
        AND curr.version_num = prev.version_num + 1

    UNION ALL

    -- The first version has nothing to diff against; an empty map makes every
    -- non-empty field read as a change from '', matching fields_history.
    SELECT
        entity_id,
        tenant_id,
        source_id,
        updated_at,
        fields AS curr_fields,
        CAST(map(), 'Map(String, String)') AS prev_fields
    FROM versioned
    WHERE version_num = 1
)

SELECT
    entity_id,
    tenant_id,
    source_id,
    field_name,
    prev_fields[field_name] AS old_value,
    curr_fields[field_name] AS new_value,
    updated_at
FROM transitions
ARRAY JOIN arrayDistinct(arrayConcat(mapKeys(curr_fields), mapKeys(prev_fields))) AS field_name
WHERE curr_fields[field_name] != prev_fields[field_name]

{% endmacro %}

{% macro identity_inputs_from_history(
    fields_history_ref,
    source_type,
    identity_fields,
    deactivation_condition
) %}
{#
  Generates identity_inputs rows from a fields_history model.
  Produces UPSERT rows for identity-relevant field changes, and DELETE rows
  for all identity fields when a deactivation condition is met.

  In addition, every activity in history yields a `value_type='id'`
  observation carrying `value = entity_id` (= source_account_id); this
  is the ADR-0002 canonical binding row, emitted by the macro so every
  connector contributes it uniformly without repeating boilerplate.

  Designed for incremental models: when is_incremental() is true, only
  processes fields_history rows newer than the last _synced_at in the target.

  Args:
    fields_history_ref:     ref() to the fields_history model
    source_type:            insight_source_type value (e.g., 'bamboohr', 'zoom')
    identity_fields:        list of dicts with keys:
                              - field: source field name in fields_history (e.g., 'workEmail')
                              - value_type: persons value_type (e.g., 'email',
                                'employee_id', 'display_name'). The implicit
                                `value_type='id'` row is emitted in addition to
                                whatever is listed here — do not repeat it.
                              - value_field_name: fully-qualified field path
                                (e.g., 'bronze_bamboohr.employees.workEmail')
    deactivation_condition: SQL expression evaluated against fields_history row
                            that returns true when the entity is deactivated.
                            Available columns: entity_id, tenant_id, source_id,
                            field_name, old_value, new_value, updated_at.
                            Example: "field_name = 'status' AND new_value = 'Inactive'"

  Output columns (match identity_inputs schema):
    unique_key, insight_tenant_id, insight_source_id, insight_source_type,
    source_account_id, value_type, value, value_field_name, operation_type,
    _synced_at, _version

  unique_key is `{tenant}-{source_type}-{source_account_id}-{value_type}-{operation}-{updated_at_ms}`
  — uniquely identifies one observation event. RMT(_version) deduplicates true
  duplicates (same observation re-emitted) on background merge.
#}

WITH history AS (
    SELECT *
    FROM {{ fields_history_ref }}
    {% if is_incremental() %}
    WHERE updated_at > (SELECT max(_synced_at) FROM {{ this }})
    {% endif %}
),

-- UPSERT: identity field changed
upserts AS (
    {% for f in identity_fields %}
    SELECT
        CAST(concat(
            coalesce(tenant_id, ''), '-',
            '{{ source_type }}', '-',
            coalesce(entity_id, ''), '-',
            '{{ f.value_type }}', '-',
            'UPSERT-',
            toString(toUnixTimestamp64Milli(toDateTime64(updated_at, 3)))
        ) AS String) AS unique_key,
        tenant_id AS insight_tenant_id,
        source_id AS insight_source_id,
        '{{ source_type }}' AS insight_source_type,
        entity_id AS source_account_id,
        '{{ f.value_type }}' AS value_type,
        new_value AS value,
        '{{ f.value_field_name }}' AS value_field_name,
        'UPSERT' AS operation_type,
        updated_at AS _synced_at,
        toUnixTimestamp64Milli(toDateTime64(updated_at, 3)) AS _version
    FROM history
    WHERE field_name = '{{ f.field }}'
      AND new_value != ''
    {{ 'UNION ALL' if not loop.last }}
    {% endfor %}
),

-- DELETE: deactivation detected — emit DELETE for all identity fields
deactivation_events AS (
    SELECT
        tenant_id,
        source_id,
        entity_id,
        updated_at
    FROM history
    WHERE {{ deactivation_condition }}
),

deletes AS (
    {% for f in identity_fields %}
    SELECT
        CAST(concat(
            coalesce(d.tenant_id, ''), '-',
            '{{ source_type }}', '-',
            coalesce(d.entity_id, ''), '-',
            '{{ f.value_type }}', '-',
            'DELETE-',
            toString(toUnixTimestamp64Milli(toDateTime64(d.updated_at, 3)))
        ) AS String) AS unique_key,
        d.tenant_id AS insight_tenant_id,
        d.source_id AS insight_source_id,
        '{{ source_type }}' AS insight_source_type,
        d.entity_id AS source_account_id,
        '{{ f.value_type }}' AS value_type,
        '' AS value,
        '{{ f.value_field_name }}' AS value_field_name,
        'DELETE' AS operation_type,
        d.updated_at AS _synced_at,
        toUnixTimestamp64Milli(toDateTime64(d.updated_at, 3)) AS _version
    FROM deactivation_events d
    {{ 'UNION ALL' if not loop.last }}
    {% endfor %}
),

-- UPSERT: canonical binding row (value_type='id', value=source_account_id) per
-- ADR-0002 — emitted by the macro on every activity so every connector
-- contributes it uniformly. (REC-IR-05: planned to move to per-connector
-- explicit declaration in a follow-up PR.)
id_upserts AS (
    SELECT
        CAST(concat(
            coalesce(tenant_id, ''), '-',
            '{{ source_type }}', '-',
            coalesce(entity_id, ''), '-',
            'id-',
            'UPSERT-',
            toString(toUnixTimestamp64Milli(toDateTime64(updated_at, 3)))
        ) AS String) AS unique_key,
        tenant_id AS insight_tenant_id,
        source_id AS insight_source_id,
        '{{ source_type }}' AS insight_source_type,
        entity_id AS source_account_id,
        'id' AS value_type,
        entity_id AS value,
        '{{ source_type }}.entity_id' AS value_field_name,
        'UPSERT' AS operation_type,
        updated_at AS _synced_at,
        toUnixTimestamp64Milli(toDateTime64(updated_at, 3)) AS _version
    FROM history
    WHERE entity_id IS NOT NULL AND entity_id != ''
),

-- DELETE: mirror id-binding row at deactivation.
id_deletes AS (
    SELECT
        CAST(concat(
            coalesce(d.tenant_id, ''), '-',
            '{{ source_type }}', '-',
            coalesce(d.entity_id, ''), '-',
            'id-',
            'DELETE-',
            toString(toUnixTimestamp64Milli(toDateTime64(d.updated_at, 3)))
        ) AS String) AS unique_key,
        d.tenant_id AS insight_tenant_id,
        d.source_id AS insight_source_id,
        '{{ source_type }}' AS insight_source_type,
        d.entity_id AS source_account_id,
        'id' AS value_type,
        '' AS value,
        '{{ source_type }}.entity_id' AS value_field_name,
        'DELETE' AS operation_type,
        d.updated_at AS _synced_at,
        toUnixTimestamp64Milli(toDateTime64(d.updated_at, 3)) AS _version
    FROM deactivation_events d
)

SELECT * FROM upserts
UNION ALL
SELECT * FROM deletes
UNION ALL
SELECT * FROM id_upserts
UNION ALL
SELECT * FROM id_deletes

{% endmacro %}

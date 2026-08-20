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

  Types:
    `insight_tenant_id` and `insight_source_id` are emitted as UUID, derived
    from the source's raw `tenant_id` / `source_id` strings via sipHash128.
    `_synced_at` is emitted as DateTime64(3). All three match what the seed-
    style identity inputs (seed_identity_inputs_from_cursor /
    _from_claude_admin) emit so the `silver/_shared/identity_inputs.sql`
    UNION ALL type-checks — ClickHouse 25.3 rejects UNION across UUID and
    String with error code 386 NO_COMMON_TYPE. The hashing-to-UUID approach
    is TEMPORARY pending a real tenants registry; the same hash applied to
    the same raw tenant_id maps to the same UUID across all sources, so
    downstream joins on `insight_tenant_id` stay consistent.

  unique_key is
  `{tenant}-{source_id}-{source_type}-{source_account_id}-{value_type}-{operation}-{updated_at_ms}`
  — uniquely identifies one observation event. source_id is part of the key:
  two connections under one tenant can legitimately hold the same entity_id
  (numeric vendor ids collide across hosts), and without it the silver RMT
  would silently keep one scope's row and drop the other's. RMT(_version)
  deduplicates true duplicates (same observation re-emitted) on background
  merge.
#}

WITH history AS (
    SELECT *
    FROM {{ fields_history_ref }}
    {% if is_incremental() %}
    WHERE updated_at > (SELECT max(_synced_at) FROM {{ this }})
    {% endif %}
),

-- UPSERT: identity field changed.
-- DISTINCT: one member appearing as several history entities at one instant
-- (e.g. a GitHub member of two configured orgs) is one observation, not two
-- rows sharing a unique_key.
upserts AS (
    {% for f in identity_fields %}
    SELECT DISTINCT
        CAST(concat(
            coalesce(tenant_id, ''), '-',
            coalesce(source_id, ''), '-',
            '{{ source_type }}', '-',
            coalesce(entity_id, ''), '-',
            '{{ f.value_type }}', '-',
            'UPSERT-',
            toString(toUnixTimestamp64Milli(toDateTime64(updated_at, 3)))
        ) AS String) AS unique_key,
        toUUID(UUIDNumToString(sipHash128(coalesce(tenant_id, '')))) AS insight_tenant_id,
        toUUID(UUIDNumToString(sipHash128(coalesce(source_id, '')))) AS insight_source_id,
        '{{ source_type }}' AS insight_source_type,
        entity_id AS source_account_id,
        '{{ f.value_type }}' AS value_type,
        new_value AS value,
        '{{ f.value_field_name }}' AS value_field_name,
        'UPSERT' AS operation_type,
        toDateTime64(updated_at, 3) AS _synced_at,
        toUnixTimestamp64Milli(toDateTime64(updated_at, 3)) AS _version
    FROM history
    WHERE field_name = '{{ f.field }}'
      AND new_value != ''
      -- A row without an account key can never bind (the persons-seed fails
      -- the whole run on a NULL source_account_id).
      AND entity_id IS NOT NULL AND entity_id != ''
    {{ 'UNION ALL' if not loop.last }}
    {% endfor %}
),

-- DELETE: deactivation detected — emit DELETE for all identity fields
deactivation_events AS (
    SELECT DISTINCT
        tenant_id,
        source_id,
        entity_id,
        updated_at
    FROM history
    WHERE ({{ deactivation_condition }})
      AND entity_id IS NOT NULL AND entity_id != ''
),

deletes AS (
    {% for f in identity_fields %}
    SELECT
        CAST(concat(
            coalesce(d.tenant_id, ''), '-',
            coalesce(d.source_id, ''), '-',
            '{{ source_type }}', '-',
            coalesce(d.entity_id, ''), '-',
            '{{ f.value_type }}', '-',
            'DELETE-',
            toString(toUnixTimestamp64Milli(toDateTime64(d.updated_at, 3)))
        ) AS String) AS unique_key,
        toUUID(UUIDNumToString(sipHash128(coalesce(d.tenant_id, '')))) AS insight_tenant_id,
        toUUID(UUIDNumToString(sipHash128(coalesce(d.source_id, '')))) AS insight_source_id,
        '{{ source_type }}' AS insight_source_type,
        d.entity_id AS source_account_id,
        '{{ f.value_type }}' AS value_type,
        '' AS value,
        '{{ f.value_field_name }}' AS value_field_name,
        'DELETE' AS operation_type,
        toDateTime64(d.updated_at, 3) AS _synced_at,
        toUnixTimestamp64Milli(toDateTime64(d.updated_at, 3)) AS _version
    FROM deactivation_events d
    {{ 'UNION ALL' if not loop.last }}
    {% endfor %}
),

-- UPSERT: canonical binding row (value_type='id', value=source_account_id) per
-- ADR-0002 — emitted by the macro on every activity so every connector
-- contributes it uniformly. (REC-IR-05: planned to move to per-connector
-- explicit declaration in a follow-up PR.)
-- DISTINCT: several fields changing at one instant are several history rows
-- but ONE binding observation — without it they share one unique_key and the
-- staging model's uniqueness test fails.
id_upserts AS (
    SELECT DISTINCT
        CAST(concat(
            coalesce(tenant_id, ''), '-',
            coalesce(source_id, ''), '-',
            '{{ source_type }}', '-',
            coalesce(entity_id, ''), '-',
            'id-',
            'UPSERT-',
            toString(toUnixTimestamp64Milli(toDateTime64(updated_at, 3)))
        ) AS String) AS unique_key,
        toUUID(UUIDNumToString(sipHash128(coalesce(tenant_id, '')))) AS insight_tenant_id,
        toUUID(UUIDNumToString(sipHash128(coalesce(source_id, '')))) AS insight_source_id,
        '{{ source_type }}' AS insight_source_type,
        entity_id AS source_account_id,
        'id' AS value_type,
        entity_id AS value,
        '{{ source_type }}.entity_id' AS value_field_name,
        'UPSERT' AS operation_type,
        toDateTime64(updated_at, 3) AS _synced_at,
        toUnixTimestamp64Milli(toDateTime64(updated_at, 3)) AS _version
    FROM history
    WHERE entity_id IS NOT NULL AND entity_id != ''
),

-- DELETE: mirror id-binding row at deactivation.
id_deletes AS (
    SELECT DISTINCT
        CAST(concat(
            coalesce(d.tenant_id, ''), '-',
            coalesce(d.source_id, ''), '-',
            '{{ source_type }}', '-',
            coalesce(d.entity_id, ''), '-',
            'id-',
            'DELETE-',
            toString(toUnixTimestamp64Milli(toDateTime64(d.updated_at, 3)))
        ) AS String) AS unique_key,
        toUUID(UUIDNumToString(sipHash128(coalesce(d.tenant_id, '')))) AS insight_tenant_id,
        toUUID(UUIDNumToString(sipHash128(coalesce(d.source_id, '')))) AS insight_source_id,
        '{{ source_type }}' AS insight_source_type,
        d.entity_id AS source_account_id,
        'id' AS value_type,
        '' AS value,
        '{{ source_type }}.entity_id' AS value_field_name,
        'DELETE' AS operation_type,
        toDateTime64(d.updated_at, 3) AS _synced_at,
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

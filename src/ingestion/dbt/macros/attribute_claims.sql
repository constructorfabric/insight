{% macro attribute_claims(snapshot_ref, entity_id_col, source_type, fields) %}
{#
  Typed person-attribute claims from an SCD2 snapshot model.
  One row per (source account, field, transition): claim_action='set' when a
  field acquires or changes a non-empty value, 'clear' when it becomes empty.

  NULL and '' both normalize to '' (value absent), so transitions into and out
  of NULL are recorded like any other change.

  A field emits claims only if the snapshot versions on it: keep `fields`
  within the snapshot's check_cols, or a change lands with the observed_at of
  the next tracked change instead of its own. Customer-defined fields in the
  raw payload are out of scope for the same reason — the snapshot versions on
  a configured subset, so claiming the rest would date them wrongly.

  A 'clear' is emitted only when a delivered record carries an empty value.
  Absence of the whole record from a sync never closes values: no
  sync-completeness signal exists, and closing values on possibly-partial
  snapshots would fabricate end dates.

  Args:
    snapshot_ref:   ref() to the SCD2 snapshot (output of the snapshot macro)
    entity_id_col:  source-account identifier column in the snapshot
    source_type:    insight_source_type literal (e.g. 'bamboohr')
    fields:         top-level columns that become attribute claims

  Output columns match the class_person_attribute_claims contract:
    unique_key, insight_tenant_id, insight_source_type, insight_source_id,
    source_account_id, field_id, value_id, value_label, claim_action,
    observed_at, ingested_at, _version

  value_id is reserved for immutable source value identifiers (NULL until a
  source provides them).
#}

WITH versioned AS (
    SELECT
        unique_key,
        coalesce(tenant_id, '')                          AS insight_tenant_id,
        coalesce(source_id, '')                          AS insight_source_id,
        coalesce(toString({{ entity_id_col }}), '')      AS source_account_id,
        toDateTime64(_tracked_at, 3)                     AS observed_at,
        _airbyte_extracted_at                            AS ingested_at,
        CAST(
            [
                {% for f in fields %}
                ('{{ f }}', ifNull(toString({{ f }}), '')){{ ',' if not loop.last }}
                {% endfor %}
            ],
            'Map(String, String)'
        )                                                AS attrs
    FROM {{ snapshot_ref }}
),

-- lagInFrame's out-of-frame default is an empty Map, so the first snapshot
-- version compares against all-absent: every non-empty initial value emits a
-- set and no initial clear is possible.
with_previous AS (
    SELECT
        insight_tenant_id,
        insight_source_id,
        source_account_id,
        observed_at,
        ingested_at,
        attrs                                            AS curr_attrs,
        lagInFrame(attrs) OVER (
            PARTITION BY unique_key
            ORDER BY observed_at
            ROWS BETWEEN 1 PRECEDING AND 1 PRECEDING
        )                                                AS prev_attrs
    FROM versioned
),

claims AS (
    SELECT
        insight_tenant_id,
        insight_source_id,
        source_account_id,
        observed_at,
        ingested_at,
        field_id,
        arrayElement(curr_attrs, field_id)               AS value_label
    FROM with_previous
    ARRAY JOIN arrayDistinct(arrayConcat(mapKeys(curr_attrs), mapKeys(prev_attrs))) AS field_id
    WHERE arrayElement(curr_attrs, field_id) != arrayElement(prev_attrs, field_id)
)

SELECT
    concat(
        claim.insight_tenant_id, '-',
        claim.insight_source_id, '-',
        '{{ source_type }}', '-',
        claim.source_account_id, '-',
        claim.field_id, '-',
        toString(toUnixTimestamp64Milli(claim.observed_at))
    )                                                    AS unique_key,
    claim.insight_tenant_id                              AS insight_tenant_id,
    '{{ source_type }}'                                  AS insight_source_type,
    claim.insight_source_id                              AS insight_source_id,
    claim.source_account_id                              AS source_account_id,
    claim.field_id                                       AS field_id,
    CAST(NULL, 'Nullable(String)')                       AS value_id,
    claim.value_label                                    AS value_label,
    CAST(
        if(claim.value_label = '', 'clear', 'set'),
        'Enum8(\'set\' = 1, \'clear\' = 2)'
    )                                                    AS claim_action,
    claim.observed_at                                    AS observed_at,
    claim.ingested_at                                    AS ingested_at,
    toUnixTimestamp64Milli(claim.observed_at)            AS _version
FROM claims AS claim
{% if is_incremental() %}
-- Watermark per source instance, not per table: one staging relation holds
-- every tenant and connection of this connector, and their sync workflows run
-- concurrently. A table-wide max lets whichever instance commits first filter
-- out another instance's older-stamped claims, which then never reach silver
-- at all.
LEFT JOIN (
    SELECT
        insight_tenant_id,
        insight_source_id,
        max(_version) AS max_version
    FROM {{ this }}
    GROUP BY
        insight_tenant_id,
        insight_source_id
) AS watermark
    ON  claim.insight_tenant_id = watermark.insight_tenant_id
    AND claim.insight_source_id = watermark.insight_source_id
{% endif %}
WHERE claim.source_account_id != ''
{% if is_incremental() %}
  AND toUnixTimestamp64Milli(claim.observed_at) > coalesce(watermark.max_version, 0)
{% endif %}
{% endmacro %}

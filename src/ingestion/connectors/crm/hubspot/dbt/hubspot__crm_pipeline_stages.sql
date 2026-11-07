{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    engine=insight_engine('ReplacingMergeTree', '_version'),
    order_by='(unique_key)',
    settings={'allow_nullable_key': 1},
    tags=['hubspot', 'silver:class_crm_pipeline_stages']
) }}

-- Stages come only inside their pipeline, so a stage HubSpot removed simply
-- stops arriving: each run re-emits the stages known so far that the latest
-- poll no longer carries, as deleted, keeping their labels for history.
WITH polls AS (
    {{ hubspot_pipeline_polls() }}
),

current_stages AS (
    SELECT
        tenant_id,
        source_id,
        pipeline_id,
        object_type,
        coalesce(archived, false)
            OR {{ hubspot_absent_from_poll('_airbyte_extracted_at', 'polled_at') }} AS pipeline_deleted,
        stage,
        data_source,
        polled_at
    FROM polls
    ARRAY JOIN JSONExtractArrayRaw(coalesce(stages, '[]')) AS stage
),

polled AS (
    SELECT
        tenant_id,
        source_id,
        concat(tenant_id, '-', source_id, '-', object_type, '-', JSONExtractString(stage, 'id')) AS unique_key,
        JSONExtractString(stage, 'id')                  AS stage_id,
        pipeline_id,
        object_type,
        JSONExtractString(stage, 'label')               AS label,
        JSONExtract(stage, 'displayOrder', 'Nullable(Int64)') AS display_order,
        toFloat64OrNull(JSONExtractString(stage, 'metadata', 'probability')) AS probability,
        CAST(
            if(JSONHas(stage, 'metadata', 'isClosed'), toUInt8(JSONExtractString(stage, 'metadata', 'isClosed') = 'true'), NULL)
            AS Nullable(UInt8)
        )                                               AS is_closed,
        -- A deal stage is won when it closes at probability 1; ticket stages
        -- carry no probability and stay NULL.
        CAST(
            if(object_type = 'deals', toUInt8(JSONExtractString(stage, 'metadata', 'isClosed') = 'true' AND toFloat64OrNull(JSONExtractString(stage, 'metadata', 'probability')) = 1), NULL)
            AS Nullable(UInt8)
        )                                               AS is_won,
        nullIf(JSONExtractString(stage, 'metadata', 'ticketState'), '') AS ticket_state,
        toUInt8(pipeline_deleted OR JSONExtractBool(stage, 'archived')) AS is_deleted,
        parseDateTime64BestEffortOrNull(JSONExtractString(stage, 'createdAt'), 3) AS created_at,
        parseDateTime64BestEffortOrNull(JSONExtractString(stage, 'updatedAt'), 3) AS updated_at,
        data_source,
        toUnixTimestamp64Milli(polled_at)               AS _version
    FROM current_stages
),

{% if is_incremental() %}
latest_polls AS (
    SELECT tenant_id, source_id, object_type, max(_version) AS polled_version
    FROM polled
    GROUP BY tenant_id, source_id, object_type
),

removed AS (
    SELECT
        known.tenant_id,
        known.source_id,
        known.unique_key,
        known.stage_id,
        known.pipeline_id,
        known.object_type,
        known.label,
        known.display_order,
        known.probability,
        known.is_closed,
        known.is_won,
        known.ticket_state,
        toUInt8(1)                                      AS is_deleted,
        known.created_at,
        known.updated_at,
        known.data_source,
        latest_polls.polled_version                     AS _version
    FROM {{ this }} AS known FINAL
    INNER JOIN latest_polls
        ON latest_polls.tenant_id = known.tenant_id
       AND latest_polls.source_id = known.source_id
       AND latest_polls.object_type = known.object_type
    WHERE known.is_deleted = 0
      AND known.unique_key NOT IN (SELECT unique_key FROM polled)
),
{% endif %}

src AS (
    SELECT * FROM polled
    {% if is_incremental() %}
    UNION ALL
    SELECT * FROM removed
    {% endif %}
)

{{ hubspot_staging_tail('src') }}

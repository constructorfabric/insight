{{ config(
    severity='warn',
    store_failures=true,
    meta={
        'title': 'Cursor per-event cost reaches class_ai_dev_usage',
        'domain': 'ai',
        'category': 'traceability',
        'tier': 'warn',
        'remediation': 'Cursor prices usage per event; class_ai_dev_usage carries that cost at the (person, day) grain anchored on cursor_daily_usage. A row here is priced usage whose person-day has no Silver row, or has one with cost_cents NULL — the amount is missing from every cost surface. Two known causes: cursor_daily_usage reports isActive=false for a day that still carries billable events, and gaps in the daily-usage stream where no row exists for the day at all. Both are daily-usage coverage problems — fix the stream, not the consumers.'
    }
) }}
{#- Reads a bronze-derived staging view, so it is deliberately untagged: the
    data_quality selector is reserved for checks that read silver/gold only
    (see tests/README.md). Runs under `dbt build` at warn severity. -#}

WITH event_cost AS (

    SELECT tenant_key, source_key, email, day, charged_cents
    FROM {{ ref('cursor__event_cost_daily') }}
    {#- Yesterday onward is legitimately mid-flight: events for a day can land
        before that day's daily-usage row syncs. Bounding here keeps the check
        empty in steady state, which is the only state in which it is useful. -#}
    WHERE day < today() - 1
      AND charged_cents > 0

),

attributed AS (

    {#- Two-step on purpose. Inner: read-time dedup at the table's own grain
        (unique_key). Outer: fold the userId siblings that can share one
        email-day — only one of them carries the day's cost, and siblings can
        tie on _version, so an argMax across them could return the NULL sibling
        and report a day that is in fact attributed. max() ignores NULLs, and
        still yields NULL when no sibling carries cost. -#}
    SELECT
        tenant_key,
        source_key,
        email,
        day,
        max(cost_cents)                     AS cost_cents,
        toUInt8(1)                          AS present
    FROM (
        SELECT
            coalesce(insight_tenant_id, '') AS tenant_key,
            coalesce(source_id, '')         AS source_key,
            email,
            day,
            unique_key,
            {#- tuple() keeps a NULL winner: bare argMax skips rows whose value
                argument is NULL, which would hide the very rows this check
                exists to report. -#}
            tupleElement(argMax(tuple(cost_cents), _version), 1) AS cost_cents
        FROM {{ ref('class_ai_dev_usage') }}
        WHERE source = 'cursor'
        GROUP BY tenant_key, source_key, email, day, unique_key
    )
    GROUP BY tenant_key, source_key, email, day

)

SELECT
    e.tenant_key    AS insight_tenant_id,
    e.source_key    AS source_id,
    e.email         AS email,
    e.day           AS day,
    e.charged_cents AS unattributed_cents,
    if(coalesce(a.present, 0) = 1, 'null_cost', 'no_row') AS cause
FROM event_cost AS e
LEFT JOIN attributed AS a
       ON e.tenant_key = a.tenant_key
      AND e.source_key = a.source_key
      AND e.email = a.email
      AND e.day = a.day
WHERE coalesce(a.present, 0) = 0
   OR a.cost_cents IS NULL

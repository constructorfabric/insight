{{ config(
    tags=['data_quality'],
    severity='warn',
    store_failures=true,
    meta={
        'title': 'every stored Claude Team day came from a read that brought the whole roster',
        'domain': 'ai',
        'category': 'completeness',
        'tier': 'error',
        'remediation': 'A row here is a day whose newest read did not bring everyone the vendor counted for it, so the day is either missing people or is being served from an older read. The per-user endpoint pages an offset window over a sort on accepted lines, and everyone with no accepted lines ties on that sort, so a person can fall between two pages while both requests succeed (#3172). The staging model refuses such a read, which is why the day is stale rather than wrong — it clears itself once a read returns the full roster, and the next scheduled sync re-reads the last three days. A day that stays here across several syncs means the roster no longer fits the request page: raise the page size on claude_team_code_metrics. A day reported as unreferenced instead means the read carried no envelope to check it against, which should not happen after the org stream starts reporting total_users.'
    }
) }}

{#- Reads the same reference the gate in claude_team__ai_dev_usage reads, and
    reports what the gate silently withheld. The gate protects the serving
    layer; this makes the withholding visible, because a day quietly served
    from an older read looks exactly like a quiet day.

    Legacy reads — those older than the instance's first reference — are not
    reported: the envelope did not carry a headcount yet, so there is nothing
    they could be judged against. -#}

WITH reference AS (

    SELECT
        coalesce(tenant_id, '')                         AS insight_tenant_id,
        coalesce(source_id, '')                         AS source_id,
        toDate(metric_date)                             AS day,
        JSONExtractInt(_airbyte_meta, 'sync_id')        AS read_id,
        -- Not aliased `total_users`: the alias would shadow the source
        -- column and ClickHouse then resolves the outer WHERE to the
        -- aggregate itself.
        max(toInt64OrNull(toString(total_users)))       AS read_total_users,
        min(_airbyte_extracted_at)                      AS read_at
    FROM {{ source('bronze_claude_team', 'claude_team_code_metrics_org') }}
    WHERE metric_date IS NOT NULL
      AND total_users IS NOT NULL
    GROUP BY insight_tenant_id, source_id, day, read_id

),

first_reference AS (

    SELECT
        insight_tenant_id,
        source_id,
        min(read_at)                                    AS since
    FROM reference
    GROUP BY insight_tenant_id, source_id

),

newest_read AS (

    -- The inner alias is deliberately not `read_at`: ClickHouse resolves the
    -- outer aggregate's argument against the outer name first and then reports
    -- an aggregate inside an aggregate.
    SELECT
        insight_tenant_id,
        source_id,
        day,
        argMax(read_id, last_seen)                      AS read_id,
        argMax(people, last_seen)                       AS people,
        max(last_seen)                                  AS read_at
    FROM (
        SELECT
            coalesce(tenant_id, '')                     AS insight_tenant_id,
            coalesce(source_id, '')                     AS source_id,
            toDate(metric_date)                         AS day,
            JSONExtractInt(_airbyte_meta, 'sync_id')    AS read_id,
            uniqExact(lower(trim(coalesce(email, '')))) AS people,
            max(_airbyte_extracted_at)                  AS last_seen
        FROM {{ source('bronze_claude_team', 'claude_team_code_metrics') }}
        WHERE metric_date IS NOT NULL
        GROUP BY insight_tenant_id, source_id, day, read_id
    )
    GROUP BY insight_tenant_id, source_id, day

)

SELECT
    n.insight_tenant_id                                 AS insight_tenant_id,
    n.source_id                                         AS source_id,
    n.day                                               AS day,
    n.read_id                                           AS read_id,
    n.people                                            AS people_the_read_returned,
    r.read_total_users                                  AS people_the_vendor_counted,
    if(r.read_total_users IS NULL, 'unreferenced', 'short') AS verdict
FROM newest_read AS n
INNER JOIN first_reference AS f
        ON f.insight_tenant_id = n.insight_tenant_id
       AND f.source_id = n.source_id
LEFT JOIN reference AS r
       ON r.insight_tenant_id = n.insight_tenant_id
      AND r.source_id = n.source_id
      AND r.day = n.day
      AND r.read_id = n.read_id
-- Only reads that should have carried a reference: anything older than this
-- instance's first one predates the field.
WHERE n.read_at >= f.since
  AND (r.read_total_users IS NULL OR n.people != r.read_total_users)

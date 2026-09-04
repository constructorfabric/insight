{{ config(
    tags=['data_quality'],
    severity='warn',
    store_failures=true,
    meta={
        'title': 'every stored Claude Team day came from a read that brought the whole roster',
        'domain': 'ai',
        'category': 'completeness',
        'tier': 'error',
        'remediation': 'A row here is a day whose newest read did not bring everyone the vendor counted for it, so the day is either missing people or is being served from an older read. The per-user endpoint pages an offset window over a sort on accepted lines, and everyone with no accepted lines ties on that sort, so a person can fall between two pages while both requests succeed (#3172). The staging model refuses such a read, which is why the day is stale rather than wrong — it clears itself once a read returns the full roster, and the next scheduled sync re-reads the last three days. A day that stays here across several syncs means the roster no longer fits the request page: raise the page size on claude_team_code_metrics. verdict=unreferenced means the read carried no envelope to check it against, which should not happen once the org stream reports total_users. verdict=empty means the envelope counted people and the read stored none at all — the whole day is missing, not part of it.'
    }
) }}

{#- Reads the same reference the gate in claude_team__ai_dev_usage reads, and
    reports what the gate withheld. The gate protects the serving layer; this
    makes the withholding visible, because a day quietly served from an older
    read looks exactly like a quiet day.

    Candidates come from the reference AND the user reads, not from the user
    reads alone: a day whose envelope counted people while no per-user row
    arrived has nothing to group, so judging only what arrived would pass over
    the very case where everything is missing.

    Legacy reads — those before the first reference-carrying read — are not
    reported: the envelope did not carry a headcount yet, so there is nothing
    they could be judged against. -#}

WITH reference AS (

    SELECT
        coalesce(tenant_id, '')                         AS insight_tenant_id,
        coalesce(source_id, '')                         AS source_id,
        toDate(metric_date)                             AS day,
        JSONExtractInt(_airbyte_meta, 'sync_id')        AS read_id,
        max(toInt64OrNull(toString(total_users)))       AS read_total_users
    -- FINAL: the envelope table is a ReplacingMergeTree, so an unmerged part
    -- can still hold a superseded headcount for the same read.
    FROM {{ source('bronze_claude_team', 'claude_team_code_metrics_org') }} FINAL
    WHERE metric_date IS NOT NULL
      AND total_users IS NOT NULL
    GROUP BY insight_tenant_id, source_id, day, read_id

),

first_reference AS (

    -- The boundary is the READ, not its timestamp: both streams are written by
    -- the same job but not at the same instant, so on the first sync after the
    -- upgrade the per-user rows can predate the envelope that judges them.
    SELECT
        insight_tenant_id,
        source_id,
        min(read_id)                                    AS since_read
    FROM reference
    GROUP BY insight_tenant_id, source_id

),

user_reads AS (

    SELECT
        coalesce(tenant_id, '')                         AS insight_tenant_id,
        coalesce(source_id, '')                         AS source_id,
        toDate(metric_date)                             AS day,
        JSONExtractInt(_airbyte_meta, 'sync_id')        AS read_id,
        -- Only people the serving layer could attribute, matching the gate.
        uniqExactIf(lower(trim(email)),
                    email IS NOT NULL AND trim(email) != '') AS people,
        max(_airbyte_extracted_at)                      AS last_seen
    FROM {{ source('bronze_claude_team', 'claude_team_code_metrics') }}
    WHERE metric_date IS NOT NULL
    GROUP BY insight_tenant_id, source_id, day, read_id

),

newest_user_read AS (

    SELECT
        insight_tenant_id,
        source_id,
        day,
        argMax(read_id, last_seen)                      AS read_id,
        argMax(people, last_seen)                       AS people,
        toUInt8(1)                                      AS had_rows
    FROM user_reads
    GROUP BY insight_tenant_id, source_id, day

),

newest_reference_read AS (

    SELECT
        insight_tenant_id,
        source_id,
        day,
        max(read_id)                                    AS read_id
    FROM reference
    GROUP BY insight_tenant_id, source_id, day

),

candidates AS (

    SELECT insight_tenant_id, source_id, day FROM reference
    UNION DISTINCT
    SELECT insight_tenant_id, source_id, day FROM user_reads

),

judged AS (

    SELECT
        c.insight_tenant_id                             AS insight_tenant_id,
        c.source_id                                     AS source_id,
        c.day                                           AS day,
        -- The read that decides the day: the newest one that stored rows, or —
        -- when none did — the newest envelope, so a day that stored nothing is
        -- still judged rather than skipped for having nothing to group.
        if(coalesce(u.had_rows, 0) = 1, u.read_id, nr.read_id) AS read_id,
        coalesce(u.people, 0)                           AS people,
        coalesce(u.had_rows, 0)                         AS had_rows
    FROM candidates AS c
    LEFT JOIN newest_user_read AS u
           ON u.insight_tenant_id = c.insight_tenant_id
          AND u.source_id = c.source_id
          AND u.day = c.day
    LEFT JOIN newest_reference_read AS nr
           ON nr.insight_tenant_id = c.insight_tenant_id
          AND nr.source_id = c.source_id
          AND nr.day = c.day

)

SELECT
    j.insight_tenant_id                                 AS insight_tenant_id,
    j.source_id                                         AS source_id,
    j.day                                               AS day,
    j.read_id                                           AS read_id,
    j.people                                            AS people_the_read_returned,
    r.read_total_users                                  AS people_the_vendor_counted,
    multiIf(r.read_total_users IS NULL, 'unreferenced',
            j.had_rows = 0, 'empty',
            'short')                                    AS verdict
FROM judged AS j
INNER JOIN first_reference AS f
        ON f.insight_tenant_id = j.insight_tenant_id
       AND f.source_id = j.source_id
LEFT JOIN reference AS r
       ON r.insight_tenant_id = j.insight_tenant_id
      AND r.source_id = j.source_id
      AND r.day = j.day
      AND r.read_id = j.read_id
-- Only reads that should have carried a reference: anything before this
-- instance's first reference-carrying read predates the field.
WHERE j.read_id >= f.since_read
  AND (r.read_total_users IS NULL OR j.people != r.read_total_users)

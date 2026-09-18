{{ config(
    tags=['data_quality'],
    severity='warn',
    store_failures=true,
    meta={
        'title': 'A ChatGPT Team instance that reported subscription spend still reports it',
        'domain': 'ai',
        'category': 'coverage',
        'tier': 'error',
        'remediation': 'A row here is a connector instance whose subscription stream produced rows before and none in the last three days. The connector maps 403 and 404 on /api/subscriptions/{org_id}/usage to action: IGNORE, so a session that lost billing visibility, a mistyped chatgpt_org_id or a broken proxy route all leave the sync green and the stream empty. Check chatgpt_org_id in the connector Secret, then the proxy route, then whether the session still has billing access, and re-sync. An instance deliberately decommissioned is the other explanation, and it clears itself once its last reporting day rolls out of the comparison.'
    }
) }}
{#- The IGNORE is deliberate: a workspace with no billing visibility should not
    fail the whole sync. What it costs is the ability to tell "nothing to
    report" from "asking the wrong question", and nothing else in the pipeline
    recovers it — an empty table has no freshness to go stale. Comparing the
    instance against its own past is the one signal available.

    Inert until an instance has reported at least once: a connector installed
    without chatgpt_org_id never appears on either side and never flags. -#}

WITH reported_before AS (

    SELECT DISTINCT
        coalesce(tenant_id, '') AS insight_tenant_id,
        coalesce(source_id, '') AS source_id
    FROM {{ source('bronze_chatgpt_team', 'chatgpt_team_subscription_usage') }}
    WHERE toDateOrNull(snapshot_date) < today() - 3

),

reporting_now AS (

    SELECT DISTINCT
        coalesce(tenant_id, '') AS insight_tenant_id,
        coalesce(source_id, '') AS source_id
    FROM {{ source('bronze_chatgpt_team', 'chatgpt_team_subscription_usage') }}
    WHERE toDateOrNull(snapshot_date) >= today() - 3

)

SELECT
    p.insight_tenant_id AS insight_tenant_id,
    p.source_id         AS source_id
FROM reported_before AS p
LEFT ANTI JOIN reporting_now AS c
    ON p.insight_tenant_id = c.insight_tenant_id
   AND p.source_id = c.source_id

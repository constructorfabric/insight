{{ config(
    tags=['data_quality'],
    severity='warn',
    store_failures=true,
    meta={
        'title': 'A ChatGPT Team instance that reported subscription spend still reports it',
        'domain': 'ai',
        'category': 'coverage',
        'tier': 'error',
        'remediation': 'A row here is a connector instance whose subscription stream produced rows before and none in the last three days. The connector maps 401, 403 and 404 on /api/subscriptions/{org_id}/usage to action: IGNORE, so every way of losing billing leaves the sync green and the stream empty. The sync log distinguishes them and is the place to look first: 401 means the workspace role fell below account-admin and this installation now has no billing capability at all; 403 means the session was gated out of billing; 404 means chatgpt_org_id is blank or wrong, or the proxy route is broken. Fix the one the log names, then re-sync. An instance deliberately decommissioned is the other explanation, and it clears itself once its last reporting day rolls out of the comparison. Note the blind spot this check cannot cover: an installation that has NEVER been able to read billing produces no row on either side of the comparison, so its only signal is the 401 in the sync log of its first run.'
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

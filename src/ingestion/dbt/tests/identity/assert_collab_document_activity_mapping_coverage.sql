{{ config(
    tags=['data_quality'],
    severity='warn',
    store_failures=true,
    meta={
        'title': 'Collab document activity → roster mapping coverage (drift + floor)',
        'domain': 'identity',
        'category': 'coverage',
        'tier': 'warn',
        'remediation': 'For the latest complete day, the share of active users that map to a BambooHR employee (person_key = lower(email)) either fell more than 5 points below the 7-day trailing average (a connector or identity-resolution regression) or dropped under the 75% floor (slow external bleed, e.g. a new contractor domain). Inspect the stored tenant-day rows and the unmapped person_keys. NOTE: person_key is email, so a renamed user (marriage/legal change) orphans here until the identity service lands person_id in silver — that is the deeper fix, not this check.'
    }
) }}
-- Referential mapping-coverage guard for collaboration document activity (#736).
-- Decision 1: BambooHR is the denominator (employment, not access) — Entra-only
-- identities are expected externals and read as unmapped until a steward
-- allow-lists their domain (a separate by-domain rollup feeds that).
-- Decision 3: relative drift AND an absolute safety floor.
--
-- Mapping criterion today is email equality: activity.person_key (= lower(email))
-- must match a BambooHR class_people email for the same tenant. Empty keys are
-- excluded (expected non-mappable); the incomplete current day is excluded so a
-- mid-day partial run can't look like a drop.
--
-- severity=warn + store_failures: advisory, monitored, never blocks a build.
-- Read FINAL on both ReplacingMergeTree tables.
WITH roster AS (
    -- email is Nullable on class_people; coalesce so the join key is a plain
    -- String and NULL/'' emails are excluded.
    SELECT DISTINCT
        tenant_id,
        lower(coalesce(email, '')) AS email
    FROM {{ ref('class_people') }} FINAL
    WHERE source = 'bamboohr' AND coalesce(email, '') != ''
),
daily AS (
    SELECT
        a.tenant_id                                            AS tenant_id,
        a.date                                                 AS date_day,
        count(DISTINCT a.person_key)                           AS total_users,
        count(DISTINCT if(r.email != '', a.person_key, NULL))  AS mapped_users
    FROM {{ ref('class_collab_document_activity') }} FINAL AS a
    LEFT JOIN roster AS r
        ON a.tenant_id = r.tenant_id AND a.person_key = r.email
    WHERE a.person_key != ''        -- empty keys are expected non-mappable
      AND a.date < today()          -- exclude the incomplete current day
    GROUP BY a.tenant_id, a.date
),
stats AS (
    SELECT
        tenant_id,
        date_day,
        total_users,
        mapped_users,
        mapped_users / nullIf(total_users, 0)              AS coverage,
        avg(mapped_users / nullIf(total_users, 0)) OVER w  AS trailing_7d_coverage,
        count()                                    OVER w  AS prior_days
    FROM daily
    WINDOW w AS (
        PARTITION BY tenant_id
        ORDER BY date_day
        ROWS BETWEEN 7 PRECEDING AND 1 PRECEDING
    )
),
latest AS (
    SELECT * FROM stats WHERE date_day = (SELECT max(date_day) FROM stats)
)
SELECT
    tenant_id,
    date_day,
    total_users,
    mapped_users,
    round(coverage, 4)             AS coverage,
    round(trailing_7d_coverage, 4) AS trailing_7d_coverage,
    prior_days
FROM latest
WHERE coverage IS NOT NULL
  AND (
        coverage < 0.75                                   -- absolute safety floor
     OR (prior_days >= 5                                  -- enough baseline history
         AND trailing_7d_coverage IS NOT NULL
         AND trailing_7d_coverage - coverage >= 0.05)     -- >5pt relative drift
  )

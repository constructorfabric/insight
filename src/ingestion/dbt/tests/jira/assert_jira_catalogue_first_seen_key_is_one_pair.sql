-- Each catalogue key names one tenant-and-source pair.
--
-- Raw rows are not unique on purpose: two racing runs may each record the same
-- pair, and the merge keeps the earlier boundary. What must never happen is one
-- key standing for two pairs, which would merge one source's boundary into
-- another's.

SELECT
    unique_key,
    uniqExact(tenant_id, insight_source_id) AS pairs
FROM {{ ref('jira__catalogue_first_seen') }}
GROUP BY unique_key
HAVING pairs > 1

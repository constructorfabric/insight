-- One row per person and day within a source: edits and comments received are
-- summed into that grain, never repeated across it.
SELECT
    tenant_id,
    source_id,
    author_email,
    activity_date,
    count() AS row_count
FROM {{ ref('wiki_person_days') }}
GROUP BY tenant_id, source_id, author_email, activity_date
HAVING count() > 1

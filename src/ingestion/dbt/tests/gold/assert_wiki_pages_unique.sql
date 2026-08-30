-- A page id is unique only within the source that published it.
SELECT
    tenant_id,
    source_id,
    page_id,
    count() AS row_count
FROM {{ ref('wiki_pages') }}
GROUP BY tenant_id, source_id, page_id
HAVING count() > 1

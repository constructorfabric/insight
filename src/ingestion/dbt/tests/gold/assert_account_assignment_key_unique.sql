-- Build-integrity check (untagged → error severity under `dbt build`).
-- One binding per (source_type, source_id, account_id): the view's `LIMIT 1 BY`
-- makes that true by construction, so a second row means the key the analytics
-- join uses stopped identifying one binding — and every joined fact would
-- multiply. Asserted because the runtime joins this view per person read, where
-- a duplicate inflates metrics silently rather than failing.
SELECT
    source_type,
    source_id,
    account_id,
    count() AS binding_count
FROM {{ ref('account_assignment') }}
GROUP BY source_type, source_id, account_id
HAVING count() > 1

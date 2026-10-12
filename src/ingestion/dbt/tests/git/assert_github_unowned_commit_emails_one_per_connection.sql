-- An unowned address is one row per connection. Two tenants, or two
-- connections of one tenant, may each report the same address.
SELECT
    tenant_id,
    source_id,
    email,
    count() AS row_count
FROM {{ ref('github__unowned_commit_emails') }}
GROUP BY
    tenant_id,
    source_id,
    email
HAVING count() > 1

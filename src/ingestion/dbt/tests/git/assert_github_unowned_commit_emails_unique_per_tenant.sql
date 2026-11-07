-- An unowned commit e-mail is published once per tenant.
--
-- Its identity input is keyed by tenant and source, so a second row for the same
-- e-mail and tenant would publish the address as two accounts.

SELECT
    email,
    tenant_id,
    count() AS rows
FROM {{ ref('github__unowned_commit_emails') }}
GROUP BY email, tenant_id
HAVING rows > 1

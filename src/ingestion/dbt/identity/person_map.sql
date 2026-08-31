{{ config(
    materialized='view',
    schema='identity',
    tags=['identity', 'identity:map']
) }}

-- INVARIANT: one row per email. A duplicate multiplies every joined fact, so
-- schema.yml asserts it at error severity instead of this model repairing it.
-- INVARIANT: `identity_inputs` is an RMT read WITHOUT `FINAL`. Sound only while
-- duplicate row versions of one unique_key agree on every column projected or
-- filtered here (the key covers them); `SELECT DISTINCT` then collapses the
-- versions. A feeder that versions a changing `value` under one key, or a new
-- filter on a version-variant column, makes this read need `FINAL`.
-- INVARIANT: both sides of the analytics join store `normalized_email()` output,
-- so the compiled join applies no function to either side.
-- SAFETY: UPSERT only. A closed account keeps claiming, because closure means
-- the account is gone from the source, not that its history changed owner.
-- SAFETY: no tenant join. `identity_inputs` carries a producer-side hashed
-- tenant that never equals the journal's; the account triple is the only key
-- sound across the two stores.

WITH account_emails AS (
    SELECT DISTINCT
        insight_source_type                    AS source_type,
        insight_source_id                      AS source_id,
        lower(trimBoth(source_account_id))     AS account_id,
        {{ normalized_email('value') }}        AS email
    FROM {{ ref('identity_inputs') }}
    WHERE value_type = 'email'
      AND operation_type = 'UPSERT'
      AND coalesce(value, '') != ''
      AND coalesce(source_account_id, '') != ''
)

SELECT
    account_emails.email      AS email,
    any(assignment.person_id) AS person_id
FROM account_emails
INNER JOIN {{ ref('account_assignment') }} AS assignment
    ON assignment.source_type = account_emails.source_type
   AND assignment.source_id = account_emails.source_id
   AND assignment.account_id = account_emails.account_id
WHERE account_emails.email != ''
  AND assignment.person_id != {{ excluded_person_id() }}
GROUP BY account_emails.email
HAVING uniqExact(assignment.person_id) = 1

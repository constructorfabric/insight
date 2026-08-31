-- Build-integrity check (untagged → error severity under `dbt build`).
-- The query-time join applies no function to either side, so an unnormalized
-- map row matches nothing and the person silently misses that activity. Pairs
-- with the per-relation entity-id shape checks on the fact side.
SELECT
    email,
    person_id
FROM {{ ref('person_map') }}
WHERE email != lower(trimBoth(email))
   OR email = ''

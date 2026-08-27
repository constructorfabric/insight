-- Build-integrity check (untagged → error severity under `dbt build`).
-- Asserts the invariant, not the output: the view's terminal GROUP BY makes its
-- own output unique by construction, so a contested email must be ABSENT from
-- the map rather than tie-broken into it.
-- Reads `identity_inputs` without `FINAL` under the same invariant as
-- person_map.sql: duplicate row versions agree on every column read here.
WITH claims AS (
    SELECT
        {{ normalized_email('inputs.value') }} AS email,
        assignment.person_id                   AS person_id
    FROM {{ ref('identity_inputs') }} AS inputs
    INNER JOIN {{ ref('account_assignment') }} AS assignment
        ON assignment.source_type = inputs.insight_source_type
       AND assignment.source_id = inputs.insight_source_id
       AND assignment.account_id = lower(trimBoth(inputs.source_account_id))
    WHERE inputs.value_type = 'email'
      AND inputs.operation_type = 'UPSERT'
      AND coalesce(inputs.value, '') != ''
      AND coalesce(inputs.source_account_id, '') != ''
      AND assignment.person_id != {{ excluded_person_id() }}
),
contested AS (
    SELECT email
    FROM claims
    GROUP BY email
    HAVING uniqExact(person_id) > 1
)
SELECT map.email
FROM {{ ref('person_map') }} AS map
INNER JOIN contested ON contested.email = map.email

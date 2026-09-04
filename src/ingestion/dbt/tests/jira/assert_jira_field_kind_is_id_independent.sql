-- Portability guard for the `jira_field_kind` classifier.
--
-- `customfield_NNNNN` ids are assigned per Jira instance; Jira's type constants
-- are not. A rule keyed on a field id therefore works on exactly one deployment.
-- Rather than grep the macro's source, assert the property that matters:
-- masking a field's id must not change its kind.
--
-- The exceptions are the Jira *system* field names the classifier is documented
-- to match (FIELD-HISTORY-IN-DBT.md §3.1) — those are product constants, stable
-- across instances, and are the reason the ban is on `customfield_` specifically
-- rather than on every literal.
--
-- Adding to this list is a decision, not a fix. Every entry has to be a name
-- Jira itself defines, and the reason structure cannot decide belongs in the
-- spec next to the rule. Reaching for it because a classifier branch fails the
-- test is how the ban erodes.
--
-- Adding to this list is a decision, not a fix. Every entry has to be a name
-- Jira itself defines, and the reason structure cannot decide belongs in the
-- spec next to the rule. Reaching for it because a classifier branch fails the
-- test is how the ban erodes.

WITH masked AS (
    SELECT
        COALESCE(f.field_id, '')      AS field_id,
        COALESCE(f.name, '')          AS field_name,
        {{ jira_field_kind('f.field_id', 'f.schema_type',
                           'f.schema_items', 'f.schema_custom') }}   AS kind_with_real_id,
        {{ jira_field_kind("'customfield_000000'", 'f.schema_type',
                           'f.schema_items', 'f.schema_custom') }}   AS kind_with_masked_id
    FROM {{ source('bronze_jira', 'jira_fields') }} AS f FINAL
    WHERE f.field_id IS NOT NULL
)

SELECT
    field_id,
    field_name,
    kind_with_real_id,
    kind_with_masked_id
FROM masked
WHERE kind_with_real_id != kind_with_masked_id
  AND field_id NOT IN ('parent', 'issuekey', 'thumbnail',
                       'description', 'environment',
                       -- §3.5: zero and absent are the same state for the two
                       -- time-tracking estimates, and no structure separates
                       -- them from a story-point estimate
                       'timeestimate', 'timeoriginalestimate')
LIMIT 100

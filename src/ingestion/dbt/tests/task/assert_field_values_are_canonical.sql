-- `target_value` and `default_value` are low-cardinality strings, not enums,
-- so the database accepts any word an operator writes. Gold guards its own
-- read with the same domain, so a typo classifies as if unmapped without
-- saying so — this test is what makes it visible. Each standardized `field`
-- declares its domain here; a field this test does not know is itself a
-- finding, because its values would constrain nothing.

SELECT
    'out_of_domain' AS problem,
    tenant_id,
    insight_source_id,
    field,
    source_key AS detail,
    target_value AS value
FROM config.field_value_map FINAL
WHERE is_deleted = 0
  AND NOT (field = 'issue_type' AND target_value IN ('bug', 'task', 'unknown'))
  AND NOT (field = 'resolution' AND target_value IN ('fixed', 'duplicate', 'wontfix', 'unknown'))

UNION ALL

SELECT
    'out_of_domain',
    tenant_id,
    insight_source_id,
    field,
    '' AS detail,
    default_value
FROM config.field_value_defaults FINAL
WHERE is_deleted = 0
  AND NOT (field = 'issue_type' AND default_value IN ('bug', 'task', 'unknown'))
  AND NOT (field = 'resolution' AND default_value IN ('fixed', 'duplicate', 'wontfix', 'unknown'))

LIMIT 100

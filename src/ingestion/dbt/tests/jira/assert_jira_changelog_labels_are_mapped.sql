{{ config(
    severity='warn',
    tags=['connector_quality', 'jira'],
    store_failures=true,
    meta={
        'title': 'Jira changelog items without a field id are mapped or knowingly skipped',
        'domain': 'task-tracking',
        'category': 'completeness',
        'tier': 'warn',
        'remediation': 'Jira writes a few changes by label alone, with no fieldId, and jira__changelog_items keeps only those `jira_changelog_item_field_id` maps to a field; every other such item is dropped. A label listed here is neither mapped nor in `jira_changelog_ignored_events`, so its history is being lost. Look at a sample item: if it changes a field (as "Link" changes issuelinks and "IssueParentAssociation" changes parent), map it in `jira_changelog_item_field_id`; if it is a system event no field holds, add it to `jira_changelog_ignored_events`.'
    }
) }}

-- SEVERITY: warn. A new label is a change in how Jira writes its changelog,
-- not a fault of this pipeline; it is reported so that history it carries is
-- not dropped unnoticed, as link and parent history once were.
--
-- One row per unmapped label, with the span of its items and one sample.

WITH items AS (
    SELECT
        COALESCE(h.tenant_id, '')                         AS tenant_id,
        h.source_id                                       AS insight_source_id,
        h.created                                         AS created,
        arrayJoin(JSONExtractArrayRaw(COALESCE(h.items, '[]')))  AS item
    FROM {{ source('bronze_jira', 'jira_issue_history') }} AS h FINAL
)

SELECT
    tenant_id,
    insight_source_id,
    JSONExtractString(item, 'field')                      AS field_label,
    JSONExtractString(item, 'fieldtype')                  AS field_type,
    count()                                               AS items,
    min(created)                                          AS oldest_item,
    max(created)                                          AS newest_item,
    any(item)                                             AS sample_item
FROM items
WHERE JSONExtractString(item, 'fieldId') = ''
  AND {{ jira_changelog_item_field_id('item') }} = ''
  AND JSONExtractString(item, 'field') NOT IN {{ jira_changelog_ignored_events() }}
GROUP BY tenant_id, insight_source_id, field_label, field_type

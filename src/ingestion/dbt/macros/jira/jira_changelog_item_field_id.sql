{#
  The field a changelog item changes, from the raw item JSON.

  Jira names most fields by `fieldId`, but writes a few system fields by label
  alone, with no `fieldId`: a link added or removed is `"field": "Link"`, and a
  change of parent is `"IssueParentAssociation"` (formerly `"Parent"`). Those
  are `issuelinks` and `parent`, and dropping them loses that history outright.
  Every other item without a `fieldId` yields ''; the system events among them
  are listed in `jira_changelog_ignored_events`, and
  `assert_jira_changelog_labels_are_mapped` reports any other.
#}
{% macro jira_changelog_item_field_id(item) -%}
multiIf(
    JSONExtractString({{ item }}, 'fieldId') != '',
        JSONExtractString({{ item }}, 'fieldId'),
    JSONExtractString({{ item }}, 'fieldtype') != 'jira',
        '',
    JSONExtractString({{ item }}, 'field') = 'Link',
        'issuelinks',
    JSONExtractString({{ item }}, 'field') IN ('IssueParentAssociation', 'Parent'),
        'parent',
    ''
)
{%- endmacro %}


{#
  System events Jira writes without a `fieldId` that no field holds, so the
  journal skips them on purpose: worklog bookkeeping, an issue moving to a new
  key, a workflow migration, and remote links, which are a separate resource.
#}
{% macro jira_changelog_ignored_events() -%}
('WorklogId', 'WorklogTimeSpent', 'Key', 'Workflow', 'RemoteIssueLink', 'RemoteWorkItemLink')
{%- endmacro %}

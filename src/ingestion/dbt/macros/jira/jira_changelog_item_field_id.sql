{#
  The field a changelog item changes, from the raw item JSON.

  Jira names most fields by `fieldId`, but writes a few system fields by label
  alone, with no `fieldId`: a link added or removed is `"field": "Link"`, and a
  change of parent is `"IssueParentAssociation"` (formerly `"Parent"`). Those
  are `issuelinks` and `parent`, and dropping them loses that history outright.
  Every other item without a `fieldId` is a system event no field holds
  ("WorklogId", "RemoteIssueLink", "Key", ...) and yields ''.
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

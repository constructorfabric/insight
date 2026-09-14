{#
  The journal's `unique_key`, one formula for every arm of the Jira field
  history: {source}-jira-{issue_id}-{field_id}-{event_id}.

  The issue is named by its immutable id, never by `id_readable`: Jira changes
  the readable key when an issue moves between projects, and a key built from it
  stored the issue's whole history a second time under the new key with nothing
  for ReplacingMergeTree to collapse (#2741). A row that cannot name its issue
  by id is not in the journal at all — see `changelog_items` in the derived
  model. `assert_jira_field_history_key_is_issue_keyed` recomputes this over
  every row.
#}
{% macro jira_history_key(source_id, issue_id, field_id, event_id) -%}
concat({{ source_id }}, '-jira-', {{ issue_id }}, '-', {{ field_id }}, '-', {{ event_id }})
{%- endmacro %}

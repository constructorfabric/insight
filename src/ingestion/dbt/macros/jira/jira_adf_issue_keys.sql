{#-
  Issue keys an Atlassian Document Format body names: in the words of its text
  nodes (`text`), in the URL of a smart link (`inlineCard` / `blockCard` /
  `embedCard`, `attrs.url`) and in the target of a link mark (`attrs.href`).
  Identifiers (`id`, `localId`, account ids) are never read, and a mention's
  `attrs.text` is the person's display name (`@…`), so it is skipped.
-#}
{% macro jira_adf_issue_keys(doc) %}
    arrayFlatten(arrayMap(
        s -> {{ task_issue_keys('s') }},
        arrayFilter(
            s -> NOT startsWith(s, '@'),
            extractAll({{ doc }}, '"(?:text|url|href)"\\s*:\\s*"((?:[^"\\\\]|\\\\.)*)"')
        )
    ))
{% endmacro %}

{#
  The relation a link holds from one issue's side: the phrase its link type
  gives that direction — "blocks" outward, "is blocked by" inward. The issue
  resource states it as the type's `outward` or `inward`; the changelog renders
  it in a sentence, "This issue blocks PROJ-2" or "This work item blocks
  PROJ-2", whose tail is the linked key; the subject carries no meaning.
#}
{% macro jira_link_relation(sentence, linked_key) -%}
    trim(BOTH ' ' FROM replaceRegexpOne(
        if(endsWith({{ sentence }}, concat(' ', {{ linked_key }})),
           substring({{ sentence }}, 1, length({{ sentence }}) - length({{ linked_key }}) - 1),
           {{ sentence }}),
        '^This (issue|work item) ', ''))
{%- endmacro %}


{#
  A Jira link type and direction, as the contract's `link_type`.

  `type_name` is the current name of the type a link was resolved to by its
  id, so a type whose phrases an administrator reworded keeps its meaning; a
  link resolved to no type is matched by Jira's default wording. `Duplicate` inward ("is duplicated by")
  and every other type — Relates, Cloners, any custom type — are `connected`:
  the contract has no inverse of `duplicate_of`, and `connected` says only that
  the link exists.
#}
{% macro jira_link_type(type_name, direction, relation) -%}
    multiIf(
        {{ type_name }} = 'Blocks' AND {{ direction }} = 'outward',      'blocking',
        {{ type_name }} = 'Blocks' AND {{ direction }} = 'inward',       'blocked_by',
        {{ type_name }} = 'Duplicate' AND {{ direction }} = 'outward',   'duplicate_of',
        {{ type_name }} != '',                                           'connected',
        {{ relation }} = 'blocks',                                       'blocking',
        {{ relation }} = 'is blocked by',                                'blocked_by',
        {{ relation }} = 'duplicates',                                   'duplicate_of',
        'connected'
    )
{%- endmacro %}

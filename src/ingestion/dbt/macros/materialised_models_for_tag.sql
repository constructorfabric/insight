{#-
  materialised_models_for_tag(tag_name)

  The names of the models carrying `tag_name` whose relation actually exists in
  the warehouse, as a list. Unlike union_by_tag this returns an empty list
  instead of raising when nothing is materialised: a caller that only reports on
  what is there — a completeness test, say — must stay silent on a deployment
  that runs none of the contributing connectors, not abort the run.
-#}
{% macro materialised_models_for_tag(tag_name) %}
  {%- set names = [] -%}
  {%- if execute -%}
    {%- for node in graph.nodes.values() -%}
      {%- if tag_name in node.tags and node.resource_type == 'model' -%}
        {%- if node.config.materialized == 'ephemeral' -%}
          {#- No relation to probe: dbt inlines these as a CTE on ref(). -#}
          {%- do names.append(node.name) -%}
        {%- elif adapter.get_relation(
                   database=none,
                   schema=node.schema,
                   identifier=node.alias or node.name) -%}
          {%- do names.append(node.name) -%}
        {%- endif -%}
      {%- endif -%}
    {%- endfor -%}
  {%- endif -%}
  {{ return(names) }}
{% endmacro %}

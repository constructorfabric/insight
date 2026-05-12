-- @cpt-principle:cpt-dataflow-principle-staging-then-union:p1
{% macro union_by_tag(tag_name) %}
  {%- if execute -%}
    {%- set models = [] -%}
    {%- for node in graph.nodes.values() -%}
      {%- if tag_name in node.tags and node.resource_type == 'model' and node.unique_id != model.unique_id -%}
        {%- if node.config.materialized == 'ephemeral' -%}
          {#- Ephemeral models have no DB relation; dbt inlines them as CTE on ref(). -#}
          {%- do models.append(node) -%}
        {%- else -%}
          {%- set rel = adapter.get_relation(database=none, schema=node.schema, identifier=node.alias or node.name) -%}
          {%- if rel -%}
            {%- do models.append(node) -%}
          {%- else -%}
            {{ log("union_by_tag: skipping " ~ node.name ~ " (staging table not yet materialised)", info=True) }}
          {%- endif -%}
        {%- endif -%}
      {%- endif -%}
    {%- endfor -%}

    {%- if models | length == 0 -%}
      {%- set this_rel = adapter.get_relation(database=this.database, schema=this.schema, identifier=this.identifier) -%}
      {%- if this_rel -%}
        {#- No source staging tables exist for this tag, but the silver target
            was materialised by a previous run. Emit an empty SELECT against
            the existing target so the surrounding model SQL stays
            schema-compatible (the outer "SELECT * FROM (...) WHERE _version > ..."
            keeps working because we hand it the real target schema, just with
            zero rows). The silver materialise becomes a no-op; downstream
            keeps running smoothly. -#}
        {{ log("union_by_tag: no source tables for tag '" ~ tag_name ~ "' — emitting empty select from existing target to preserve schema", info=True) }}
        SELECT * FROM {{ this }} WHERE 1 = 0
      {%- else -%}
        {{ exceptions.raise_compiler_error(
            "union_by_tag: no source tables for tag '" ~ tag_name ~ "' and target " ~ this ~
            " has not been materialised yet. First run requires at least one configured connector with materialised staging — "
            "configure a connector that contributes to this silver target, or exclude this model from the run."
        ) }}
      {%- endif -%}
    {%- else -%}
      {%- for m in models %}
        SELECT * FROM {{ ref(m.name) }}
        {%- if not loop.last %} UNION ALL {% endif %}
      {%- endfor -%}
    {%- endif -%}
  {%- else -%}
    SELECT 1 AS _placeholder WHERE FALSE
  {%- endif -%}
{% endmacro %}

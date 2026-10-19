{#-
  unbuildable_models()

  Every model that cannot be built against the warehouse as it stands, by name.
  `list_unbuildable_models` prints the list for `dbt run --exclude`:

      dbt --quiet run-operation list_unbuildable_models

  A model is buildable when every relation it reads will be there by the time it
  runs: a bronze table that already exists, or a model this run builds first.
  Silver class models are the exception — `union_by_tag` unions whichever tagged
  contributors are materialised, so ONE buildable contributor is enough and the
  rest are simply left out of the union.

  Only `bronze_*` sources are probed. Bronze is the one layer a run cannot
  create for itself: destination-clickhouse writes it from the connector's own
  catalogue on that connector's first sync. `config` is created by this
  project's own on-run-start hooks, so it always stands by the time a model
  reads it.

  INVARIANT: the probe reads bronze, never staging. Staging is built during the
  same run, so a contributor whose relation does not exist yet is still present
  by the time its class reads it — probing staging here would exclude classes
  that were about to become buildable.
-#}
{% macro unbuildable_models() %}
  {%- set names = [] -%}
  {%- if execute -%}
    {%- set models = [] -%}
    {%- for node in graph.nodes.values() if node.resource_type == 'model' -%}
      {%- do models.append(node) -%}
    {%- endfor -%}

    {#- present: the relation is there now, or this run puts it there. -#}
    {%- set present = {} -%}
    {%- for source in graph.sources.values() -%}
      {%- set probed = not source.schema.startswith('bronze_')
            or adapter.get_relation(database=none, schema=source.schema, identifier=source.identifier) is not none -%}
      {%- do present.update({source.unique_id: probed}) -%}
    {%- endfor -%}
    {%- for node in models -%}
      {#- A relation already in the warehouse counts only while reading it does
          not re-run the model. A table holds its rows and stays readable after
          its sources go; a VIEW re-executes, so a stale one over absent bronze
          fails the consumer that unions it — and an ephemeral model has no
          relation at all, dbt inlines it as a CTE on ref(). Both must earn
          their place below, by being buildable. -#}
      {%- set holds_rows = node.config.materialized not in ['view', 'ephemeral'] -%}
      {%- set materialised = holds_rows
            and adapter.get_relation(database=none, schema=node.schema, identifier=node.alias or node.name) is not none -%}
      {%- do present.update({node.unique_id: materialised}) -%}
    {%- endfor -%}

    {%- set contributors = {} -%}
    {%- for node in models -%}
      {%- set tag = 'silver:' ~ (node.alias or node.name) -%}
      {%- set feeders = [] -%}
      {%- for other in models if tag in other.tags and other.unique_id != node.unique_id -%}
        {%- do feeders.append(other.unique_id) -%}
      {%- endfor -%}
      {%- do contributors.update({node.unique_id: feeders}) -%}
    {%- endfor -%}

    {#- `present` only ever grows, so this settles; the loop filter stops it
        the first time a pass changes nothing. -#}
    {%- set buildable = {} -%}
    {%- set pass = namespace(changed=True) -%}
    {%- for _ in range(models | length) if pass.changed -%}
      {%- set pass.changed = False -%}
      {%- for node in models -%}
        {%- set feeders = contributors[node.unique_id] -%}
        {%- set ok = namespace(value=True) -%}
        {%- for dep in node.depends_on.nodes if dep not in feeders -%}
          {%- if not present.get(dep, True) -%}
            {%- set ok.value = False -%}
          {%- endif -%}
        {%- endfor -%}
        {%- if feeders and ok.value -%}
          {%- set fed = namespace(value=False) -%}
          {%- for dep in feeders if present.get(dep, False) -%}
            {%- set fed.value = True -%}
          {%- endfor -%}
          {%- set ok.value = fed.value -%}
        {%- endif -%}
        {%- do buildable.update({node.unique_id: ok.value}) -%}
        {%- if ok.value and not present.get(node.unique_id, False) -%}
          {%- do present.update({node.unique_id: True}) -%}
          {%- set pass.changed = True -%}
        {%- endif -%}
      {%- endfor -%}
    {%- endfor -%}

    {%- for node in models if not buildable.get(node.unique_id, False) -%}
      {%- do names.append(node.name) -%}
    {%- endfor -%}
  {%- endif -%}
  {{ return(names | sort) }}
{% endmacro %}

{% macro list_unbuildable_models() %}
  {#- print(), not log(): `dbt --quiet run-operation` then emits the list and
      nothing else, so a caller can splice it straight into --exclude. -#}
  {{- print(unbuildable_models() | join(' ')) -}}
{% endmacro %}

{#-
  relation_is_readable(node)

  Whether a consumer can read `node` right now — the per-model half of the
  judgement `unbuildable_models` makes over the whole graph, and the one
  `union_by_tag` asks of each contributor before unioning it. Both live here so
  the union and the exclusion cannot drift apart: a class kept in the run
  because ONE arm is healthy must still leave out the arms that are not.

  A table holds its rows and stays readable once its sources go. A view
  re-executes, so it is readable only while everything it reads is still there;
  an ephemeral model has no relation at all and is inlined, so it answers for
  its parents instead.
-#}
{% macro relation_is_readable(node) %}
  {%- if not execute -%}
    {{ return(False) }}
  {%- endif -%}
  {%- set materialization = node.config.materialized -%}
  {%- if materialization == 'ephemeral' -%}
    {{ return(reads_resolve(node)) }}
  {%- endif -%}
  {%- set relation = adapter.get_relation(
       database=none, schema=node.schema, identifier=node.alias or node.name) -%}
  {%- if relation is none -%}
    {{ return(False) }}
  {%- endif -%}
  {%- if materialization != 'view' -%}
    {{ return(True) }}
  {%- endif -%}
  {{ return(reads_resolve(node)) }}
{% endmacro %}

{#- Every relation `node`'s own query names, still resolvable. Bronze is the
    layer that goes missing; the rest a run creates for itself. -#}
{% macro reads_resolve(node) %}
  {%- set ok = namespace(value=True) -%}
  {%- for dep in node.depends_on.nodes if ok.value -%}
    {%- if dep in graph.sources -%}
      {%- set source = graph.sources[dep] -%}
      {%- if source.schema.startswith('bronze_')
            and adapter.get_relation(
                  database=none, schema=source.schema, identifier=source.identifier) is none -%}
        {%- set ok.value = False -%}
      {%- endif -%}
    {%- elif dep in graph.nodes and not relation_is_readable(graph.nodes[dep]) -%}
      {%- set ok.value = False -%}
    {%- endif -%}
  {%- endfor -%}
  {{ return(ok.value) }}
{% endmacro %}

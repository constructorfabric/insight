{#-
  insight_engine(family, parameters=none)

  The MergeTree engine a model is created with, rendered for the topology the
  install declares: the `Replicated*` family where the warehouse replicates,
  the plain one on a single node (epic #2010).

  Every model, hook and stage routes its engine through here rather than
  spelling a literal, because the adapter copies `engine` into the DDL verbatim
  and a `Replicated` database replays the DDL as written: a plain family stands
  on every replica holding only the rows that reached the node it was created
  on. No statement carries `ON CLUSTER` — inside a `Replicated` database the
  server rejects it on table DDL, and the replay is what reaches the replicas.

  The replicated family is rendered without ZooKeeper coordinates, leaving the
  server's `default_replica_path` / `default_replica_name` to supply them.
-#}
{% macro insight_engine(family, parameters=none) -%}
  {%- if not family.endswith('MergeTree') or family.startswith('Replicated') -%}
    {{- exceptions.raise_compiler_error(
      "insight_engine takes a plain MergeTree family name, not '" ~ family ~ "'. "
      "Pass the family and its parameters apart, as insight_engine('ReplacingMergeTree', '_version')."
    ) -}}
  {%- endif -%}
  {{- 'Replicated' if insight_cluster_mode() else '' }}{{ family }}{{ '(' ~ parameters ~ ')' if parameters else '' -}}
{%- endmacro %}

{#-
  Whether the install declared itself clustered.

  `cluster_mode` reaches dbt as TEXT — Jinja in dbt_project.yml renders to a
  string, so "false" is non-empty and therefore truthy to Jinja, and the value
  has to be read rather than tested. The spellings accepted here are the ones
  every other reader of `CLICKHOUSE_CLUSTER_MODE` accepts; see
  charts/insight/README.md, "ClickHouse topology".
-#}
{% macro insight_cluster_mode() -%}
  {{- return((var('cluster_mode', 'false') | string | trim | lower) in ['1', 'true', 'yes', 'on']) -}}
{%- endmacro %}

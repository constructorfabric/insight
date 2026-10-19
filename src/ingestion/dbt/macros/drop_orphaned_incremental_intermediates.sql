{#-
  drop_orphaned_incremental_intermediates()

  Project-level `on-run-start` hook. dbt-clickhouse's `delete+insert` and
  `insert_overwrite` strategies stage a run's rows in
  `<model>__dbt_new_data_<invocation_id>` and drop it as the materialization's
  last statement, outside any cleanup path. A run killed before that statement
  leaves the relation behind, and no later run looks for it: each one only
  pre-drops the name carrying its own invocation id.

  A relation is dropped only when all of these hold:
    - its name is an incremental model of this project, in that model's schema,
      followed by the suffix the adapter appends;
    - the suffix is not this run's invocation id;
    - it is older than `incremental_intermediate_orphan_min_age_hours`.

  SAFETY: dbt runs are not serialised against each other, so a relation another
  invocation is still writing looks exactly like an orphan. Age is the only
  signal that tells them apart; the threshold must exceed the longest run.
-#}
{% macro drop_orphaned_incremental_intermediates() %}
    {%- if not execute -%}
        {{ return('') }}
    {%- endif -%}

    {%- set targets = [] -%}
    {%- for node in graph.nodes.values() -%}
        {%- if node.resource_type == 'model' and node.config.materialized == 'incremental' -%}
            {%- do targets.append("('" ~ node.schema ~ "', '" ~ (node.alias or node.name) ~ "')") -%}
        {%- endif -%}
    {%- endfor -%}
    {%- if not targets -%}
        {{ return('') }}
    {%- endif -%}

    {%- set suffix_pattern = '__dbt_new_data_[0-9a-f]{8}_[0-9a-f]{4}_[0-9a-f]{4}_[0-9a-f]{4}_[0-9a-f]{12}$' -%}
    {%- set own_suffix = '__dbt_new_data_' ~ invocation_id.replace('-', '_') -%}
    {%- set min_age_hours = var('incremental_intermediate_orphan_min_age_hours') | int -%}

    {%- set orphans_query -%}
        SELECT database, name
        FROM system.tables
        WHERE match(name, '{{ suffix_pattern }}')
          AND (database, replaceRegexpOne(name, '{{ suffix_pattern }}', '')) IN ({{ targets | join(', ') }})
          AND NOT endsWith(name, '{{ own_suffix }}')
          AND metadata_modification_time <= now() - INTERVAL {{ min_age_hours }} HOUR
        ORDER BY database, name
    {%- endset -%}
    {%- set orphans = run_query(orphans_query) -%}

    {%- for orphan in orphans.rows -%}
        {%- do log("drop_orphaned_incremental_intermediates: dropping " ~ orphan[0] ~ "." ~ orphan[1], info=True) -%}
        {%- set relation = api.Relation.create(
             schema=orphan[0],
             identifier=orphan[1],
             type='table',
             can_on_cluster=insight_cluster_mode()) -%}
        {%- do adapter.drop_relation(relation) -%}
    {%- endfor -%}
{% endmacro %}

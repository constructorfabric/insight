-- @cpt-principle:cpt-dataflow-principle-staging-then-union:p1
{#-
  union_by_tag(tag_name, dedup_version_col='_version')

  Builds a UNION ALL of every model tagged `tag_name` and deduplicates the
  result at read time to exactly one row per `unique_key`.

  Why the read-time dedup:
    Every staging/silver table is ReplacingMergeTree, which only collapses
    duplicates during background merges — never guaranteed at query time. If
    an upstream table holds transient pre-merge duplicates (e.g. an erroneous
    Airbyte `full_refresh|append` re-appending every row on each sync), a
    plain `SELECT * FROM staging` leaks those
    duplicates straight into silver and inflates metrics. Deduping here makes
    silver duplicate-free regardless of merge timing. See ADR-0001.

  Args:
    dedup_version_col: version column for "latest wins" dedup (default
      `_version`). Pass `none` for versionless RMT sources (full-refresh
      `class_people`, `class_hr_working_hours`) where any row per key is fine.

  IMPORTANT — "any row per key is fine" holds only when `unique_key` is the
  ENTITY key and the staging views emit one row per entity. The versionless
  branch is a bare `LIMIT 1 BY unique_key` with no `ORDER BY`, so which row wins
  is undefined (empirically it picks the STALE row at `max_threads=1`). If
  staging can emit several rows per entity — e.g. an un-deduped read of an
  append-only bronze table holding multiple unmerged snapshots — the staging
  model MUST dedup its own source (`FINAL` on RMT bronze), or attributes will
  vary between runs. Never encode the version into `unique_key` to work around
  this: that keeps every version as a separate permanently-current row. See
  ADR-0001 and ADR-0004.
-#}
{% macro union_by_tag(tag_name, dedup_version_col='_version') %}
  {%- if execute -%}
    {%- set models = [] -%}
    {%- for node in graph.nodes.values() -%}
      {%- if tag_name in node.tags and node.resource_type == 'model' and node.unique_id != model.unique_id -%}
        {#- Readable, not merely there: a stale view over bronze that has gone
            still has a relation, and unioning it fails the whole class. The
            rule is `unbuildable_models`' own, so an arm this run excludes is
            the same arm the union leaves out. -#}
        {%- if relation_is_readable(node) -%}
          {%- do models.append(node) -%}
        {%- else -%}
          {{ log("union_by_tag: skipping " ~ node.name ~ " (not materialised, or it reads a relation that is gone)", info=True) }}
        {%- endif -%}
      {%- endif -%}
    {%- endfor -%}

    {%- if models | length == 0 -%}
      {#- No source staging tables for this tag: hand the surrounding model SQL
          the target's own schema with zero rows, so the materialise is a no-op
          and the outer "WHERE _version > ..." still compiles.

          A class nobody feeds and no earlier run materialised has no shape to
          emit at all — `unbuildable_models` keeps it out of the run, and
          reaching here means something selected it by hand. -#}
      {{ log("union_by_tag: no source tables for tag '" ~ tag_name ~ "' — emitting empty select from the target to preserve schema", info=True) }}
      SELECT * FROM {{ this }} WHERE 1 = 0
    {%- else -%}
      SELECT * FROM (
      {%- for m in models %}
        SELECT * FROM {{ ref(m.name) }}
        {%- if not loop.last %} UNION ALL {% endif %}
      {%- endfor %}
      ) AS _ubt
      {% if dedup_version_col %}
      {# One row per unique_key, latest version wins. #}
      QUALIFY ROW_NUMBER() OVER (PARTITION BY unique_key ORDER BY {{ dedup_version_col }} DESC) = 1
      {% else %}
      {# Versionless RMT (no _version column): any row per unique_key. #}
      LIMIT 1 BY unique_key
      {% endif %}
    {%- endif -%}
  {%- else -%}
    SELECT 1 AS _placeholder WHERE FALSE
  {%- endif -%}
{% endmacro %}

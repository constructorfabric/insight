{# ---------------------------------------------------------------------------
   drop_silver_placeholders_at_start()
   ---------------------------------------------------------------------------
   Project-level `on-run-start` hook. Iterates every silver target in
   the dbt graph and drops the placeholder created by
   `scripts/create-bronze-placeholders.sh` when a two-factor
   signature matches.

   Why on-run-start (not per-model pre-hook):
   dbt-clickhouse's `materialized='incremental'` captures the target
   relation's existence at the START of materialization (before any
   pre_hook runs) and uses that for `is_incremental()` thereafter. If
   the placeholder is dropped inside a pre_hook, `is_incremental` is
   still `True`, so the compiled SQL still emits
   `WHERE _version > (SELECT max(_version) FROM {{ this }})` —
   referencing the now-dropped target → ClickHouse `SYNTAX_ERROR`
   (the inner SELECT against a missing table parses as an empty
   expression list inside the parens). Running the drop in
   `on-run-start` happens BEFORE any materialization template starts,
   so `existing_relation` is captured as `nil` and the materialization
   takes the "create from scratch" branch with the model's real schema.

   Detection — two-factor signature, BOTH must hold:

     1. `system.tables.comment` matches the literal marker
        `INSIGHT_PLACEHOLDER_v1` set by `create-bronze-placeholders.sh`
        on every silver placeholder it creates. dbt never attaches this
        comment to tables it creates, so a POPULATED table carrying the
        marker is still a placeholder. (Populated placeholders happen:
        when staging first materialises mid-run, this hook has already
        passed, the silver model inserts into the placeholder with
        on_schema_change=ignore — silently truncating every column the
        placeholder lacks — and a row-count guard would then block the
        drop forever. Placeholder contents are always rebuildable from
        staging, so dropping a populated one loses nothing.)
     2. **At least one staging model with the tag `silver:<identifier>`
        is materialised** in the warehouse. The silver dbt models use
        the `union_by_tag` macro, whose no-source-tables fallback
        emits `SELECT * FROM {{ this }} WHERE 1 = 0` to preserve the
        target schema. That fallback only works when the target
        exists. So:
          - if staging IS present → drop placeholder, dbt builds real
            schema from staging output;
          - if staging is ABSENT → leave placeholder, `union_by_tag`'s
            fallback selects from it and the silver materialise
            becomes a no-op (incremental INSERT of zero rows).

   Removal plan: when gold-view migrations are split into a post-dbt
   phase (Variant A in ADR-0007), silver tables will only be created
   by dbt itself — placeholders disappear, this macro becomes dead
   code, and both this macro and the COMMENT clauses in
   `create-bronze-placeholders.sh` can be deleted.
   --------------------------------------------------------------------------- #}
{%- macro drop_silver_placeholders_at_start() -%}
    {%- if execute -%}
        {%- for silver_node in graph.nodes.values() -%}
            {%- if 'silver' in silver_node.tags
                  and silver_node.resource_type == 'model'
                  and silver_node.config.materialized != 'ephemeral' -%}

                {%- set silver_id = silver_node.alias or silver_node.name -%}
                {%- set silver_tag = 'silver:' ~ silver_id -%}

                {#- Factor 2 — at least one staging model for this tag is materialised. -#}
                {%- set staging_present = namespace(found=False) -%}
                {%- for stg in graph.nodes.values() -%}
                    {%- if not staging_present.found
                          and silver_tag in stg.tags
                          and stg.resource_type == 'model'
                          and stg.unique_id != silver_node.unique_id
                          and stg.config.materialized != 'ephemeral' -%}
                        {%- set rel = adapter.get_relation(
                             database=none,
                             schema=stg.schema,
                             identifier=stg.alias or stg.name) -%}
                        {%- if rel -%}
                            {%- set staging_present.found = True -%}
                        {%- endif -%}
                    {%- endif -%}
                {%- endfor -%}

                {%- if staging_present.found -%}
                    {#- Factor 1 — placeholder marker. -#}
                    {%- set check_query -%}
                        SELECT count() AS n
                        FROM system.tables
                        WHERE database   = '{{ silver_node.schema }}'
                          AND name       = '{{ silver_id }}'
                          AND comment    = 'INSIGHT_PLACEHOLDER_v1'
                    {%- endset -%}
                    {%- set result = run_query(check_query) -%}
                    {%- if result and result.rows and (result.rows[0][0] | int) > 0 -%}
                        {%- do log("drop_silver_placeholders_at_start: dropping " ~ silver_node.schema ~ "." ~ silver_id ~ " (placeholder marker + staging materialised)", info=True) -%}
                        {#- Use adapter.drop_relation (not raw run_query DROP TABLE)
                            so dbt invalidates its relation cache. dbt populates
                            the cache at parse-time via `list_relations_without_caching`,
                            and `is_incremental()` reads from that cache. A raw DROP
                            via run_query removes the table from the warehouse but
                            leaves it in the cache — `is_incremental()` then returns
                            True for the now-dropped target and the compiled model
                            SQL emits `WHERE _version > (SELECT max FROM {{ this }})`
                            against the missing table → ClickHouse `SYNTAX_ERROR`. -#}
                        {%- set placeholder_rel = api.Relation.create(
                             database=silver_node.database,
                             schema=silver_node.schema,
                             identifier=silver_id,
                             type='table') -%}
                        {%- do adapter.drop_relation(placeholder_rel) -%}
                    {%- endif -%}
                {%- endif -%}
            {%- endif -%}
        {%- endfor -%}
    {%- endif -%}
{%- endmacro -%}

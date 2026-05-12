{#-
  promote_bronze_to_rmt(table, order_by, version_col, partition_by)

  One-time, idempotent migration of an Airbyte-created MergeTree bronze table
  to ReplacingMergeTree with a deterministic ORDER BY for natural-key dedup.

  Why this exists:
    Airbyte writes bronze tables as plain `MergeTree` with `destinationSyncMode='append'`
    (see `src/ingestion/airbyte-toolkit/connect.sh`). Full-refresh streams append
    every row on every sync, so bronze accumulates N copies per entity. Without
    RMT + a natural-key ORDER BY, downstream stagings either inherit duplicates
    or pay for `QUALIFY ROW_NUMBER` on every read.

  Behavior:
    1. Looks up the table in `system.tables`.
       - Missing (Airbyte hasn't run yet) → log + skip.
       - Engine already contains "Replacing" → log + skip.
    2. CREATE TABLE `<tbl>__rmt_swap` ENGINE = ReplacingMergeTree(version_col)
       ORDER BY <order_by> AS SELECT * FROM <tbl>   -- atomic CTAS in CH
    3. EXCHANGE TABLES <tbl> AND <tbl>__rmt_swap     -- atomic in CH 22.7+
    4. DROP TABLE <tbl>__rmt_swap                    -- now holds the old MT data

  Args:
    table:        Fully-qualified "database.name", e.g. "bronze_jira.jira_projects".
    order_by:     ORDER BY expression. The natural-key prefix here is the dedup key.
                  Example: "(insight_tenant_id, insight_source_id, project_id)".
    version_col:  Optional column for ReplacingMergeTree(<col>). Defaults to
                  `_airbyte_extracted_at` (added by Airbyte to every row, monotonic
                  across syncs). Pass '' for versionless RMT.
    partition_by: Optional PARTITION BY expression. Defaults to none.

  Caveats:
    - Race with concurrent Airbyte sync: rows inserted into the original table
      between CREATE and EXCHANGE land in the version we DROP. Schedule the
      first promotion of each table when no Airbyte sync is active.
    - Disk: CTAS doubles storage of the affected table during migration.
    - Replicated tables (`Replicated*MergeTree`) are NOT handled here — they
      require a zookeeper path and explicit replicated engine. Extend if needed.
    - Subsequent Airbyte schema changes (`ALTER TABLE ADD COLUMN`) are
      compatible with RMT.

  Example:
    {{ promote_bronze_to_rmt(
        table='bronze_jira.jira_projects',
        order_by='(insight_tenant_id, insight_source_id, project_id)'
    ) }}

  Reusable connector wrapper (one macro per connector):
    {% macro promote_jira_bronze() %}
        {{ promote_bronze_to_rmt(
            table='bronze_jira.jira_projects',
            order_by='(insight_tenant_id, insight_source_id, project_id)') }}
        {{ promote_bronze_to_rmt(
            table='bronze_jira.jira_user',
            order_by='(insight_tenant_id, insight_source_id, account_id)') }}
        ...
    {% endmacro %}

  Then in dbt_project.yml:
    on-run-start:
      - "{{ promote_jira_bronze() }}"
      - "{{ promote_bamboohr_bronze() }}"
-#}

-- @cpt-principle:cpt-dataflow-principle-promote-bronze:p1
{% macro promote_bronze_to_rmt(table, order_by, version_col='_airbyte_extracted_at', partition_by=none) %}
    {%- set parts = table.split('.') -%}
    {%- if parts | length != 2 -%}
        {{ exceptions.raise_compiler_error(
            "promote_bronze_to_rmt: `table` must be 'database.name', got '" ~ table ~ "'"
        ) }}
    {%- endif -%}
    {%- set db = parts[0] -%}
    {%- set tbl = parts[1] -%}
    {%- set swap = tbl ~ '__rmt_swap' -%}

    {% if not execute %}
        {%- do return(none) -%}
    {% endif %}

    {%- set engine_query -%}
        SELECT engine
        FROM system.tables
        WHERE database = '{{ db }}' AND name = '{{ tbl }}'
    {%- endset -%}
    {%- set result = run_query(engine_query) -%}

    {%- if result.rows | length == 0 -%}
        {{ log("promote_bronze_to_rmt: " ~ table ~ " does not exist yet (Airbyte hasn't created it); skipping", info=True) }}
        {%- do return(none) -%}
    {%- endif -%}

    {%- set current_engine = result.rows[0][0] -%}

    {%- if 'Replacing' in current_engine -%}
        {{ log("promote_bronze_to_rmt: " ~ table ~ " already " ~ current_engine ~ "; skipping", info=True) }}
        {%- do return(none) -%}
    {%- endif -%}

    {%- set engine_decl = 'ReplacingMergeTree' ~ (('(' ~ version_col ~ ')') if version_col else '') -%}
    {%- set partition_clause = ('PARTITION BY ' ~ partition_by) if partition_by else '' -%}

    {{ log("promote_bronze_to_rmt: migrating " ~ table ~ " — " ~ current_engine ~ " → " ~ engine_decl ~ " ORDER BY " ~ order_by, info=True) }}

    {#- 1. Drop any leftover swap from a previous aborted run. -#}
    {% do run_query("DROP TABLE IF EXISTS `" ~ db ~ "`.`" ~ swap ~ "`") %}

    {#- 2. CREATE + populate in one CTAS — atomic at the CH operation level. -#}
    {% set ctas %}
        CREATE TABLE `{{ db }}`.`{{ swap }}`
        ENGINE = {{ engine_decl }}
        {{ partition_clause }}
        ORDER BY {{ order_by }}
        SETTINGS allow_nullable_key = 1
        AS SELECT * FROM `{{ db }}`.`{{ tbl }}`
    {% endset %}
    {% do run_query(ctas) %}

    {#- 3. Atomic swap. After this the original name points at RMT data. -#}
    {% do run_query("EXCHANGE TABLES `" ~ db ~ "`.`" ~ tbl ~ "` AND `" ~ db ~ "`.`" ~ swap ~ "`") %}

    {#- 4. Drop the old MT table (now under the swap name). -#}
    {% do run_query("DROP TABLE `" ~ db ~ "`.`" ~ swap ~ "`") %}

    {{ log("promote_bronze_to_rmt: " ~ table ~ " is now " ~ engine_decl, info=True) }}
{% endmacro %}

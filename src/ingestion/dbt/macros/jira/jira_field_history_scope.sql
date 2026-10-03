{#
  Which issues one run of the field-history journal derives, and in how many
  statements (FIELD-HISTORY-IN-DBT.md §7.3).
#}


{#- The issues bronze delivered something about since the journal last saw
    them. Narrow columns only: this reads every issue in bronze and every row
    of the journal, and neither the JSON nor the value arrays.

    Two ways of knowing an issue is stale, and the union of them is the scope.

    The issue's own rows are one: bronze holds an extraction the version those
    rows carry does not cover. This is what catches an issue whose rows a run
    never wrote at all, including one bronze re-delivered under an OLDER
    extraction stamp than the journal already held — a restore does that.

    How far the last COMPLETED run had read is the other, and it is what makes a
    failed run recoverable. `delete+insert` is two statements: an insert that
    dies partway leaves an issue holding some of its rows, and those rows carry
    the version the complete set would have carried, so the issue reads as
    current and the first test alone would skip it forever. `processed_ms` is
    written only after a replacement completed, so everything bronze delivered
    since then is in scope again, the half-written issue with it. -#}
{% macro jira_journal_touched(processed_ms) %}
    SELECT
        f.insight_source_id                               AS insight_source_id,
        f.issue_id                                        AS issue_id
    FROM (
        SELECT
            insight_source_id,
            issue_id,
            max(extracted_at)                             AS fresh_at
        FROM (
            SELECT
                COALESCE(source_id, '')                   AS insight_source_id,
                COALESCE(toString(jira_id), '')           AS issue_id,
                toDateTime64(_airbyte_extracted_at, 3)    AS extracted_at
            FROM {{ source('bronze_jira', 'jira_issue') }}
            WHERE jira_id IS NOT NULL

            UNION ALL

            SELECT
                insight_source_id,
                assumeNotNull(jira_id)                    AS issue_id,
                extracted_at
            FROM {{ ref('jira__changelog_items') }}
            WHERE jira_id IS NOT NULL
        )
        GROUP BY insight_source_id, issue_id
    ) AS f
    LEFT JOIN (
        SELECT
            insight_source_id,
            issue_id,
            max(_version)                                 AS journal_version
        FROM {{ this }}
        GROUP BY insight_source_id, issue_id
    ) AS v
        ON v.insight_source_id = f.insight_source_id
       AND v.issue_id = f.issue_id
    WHERE toUnixTimestamp64Milli(f.fresh_at) > toInt64(COALESCE(v.journal_version, 0))
       OR toUnixTimestamp64Milli(f.fresh_at) > toInt64({{ processed_ms }})
{% endmacro %}


{#- Every issue bronze knows: the rebuild scope, counted for batching only. -#}
{% macro jira_journal_all_issues() %}
    SELECT COALESCE(source_id, '') AS insight_source_id, COALESCE(toString(jira_id), '') AS issue_id
    FROM {{ source('bronze_jira', 'jira_issue') }}
    WHERE jira_id IS NOT NULL
    UNION ALL
    SELECT insight_source_id, assumeNotNull(jira_id) AS issue_id
    FROM {{ ref('jira__changelog_items') }}
    WHERE jira_id IS NOT NULL
{% endmacro %}


{#- {touched, batches, narrowed}; `narrowed` is whether the reads carry a scope
    predicate at all.

    The batch count is decided here, at compile time, and travels in the
    statement (`jira_journal_batch_marker`). The post-hook must not count
    again: by then batch 0 is written and the touched set has shrunk. -#}
{% macro jira_journal_scope(catalogue) %}
    {%- set touched = is_incremental() and not catalogue.rebuild -%}
    {%- if not execute -%}
        {{ return({'touched': touched, 'batches': 1, 'narrowed': touched}) }}
    {%- endif -%}
    {%- set scope_sql = jira_journal_touched(catalogue.processed_ms) if touched else jira_journal_all_issues() -%}
    {%- set issues = run_query(
        "SELECT uniqExact(insight_source_id, issue_id) FROM (" ~ scope_sql ~ ")"
    ).rows[0][0] | int -%}
    {%- set per_batch = var('jira_journal_issues_per_batch', 40000) | int -%}
    {%- set batches = [1, (issues + per_batch - 1) // per_batch] | max -%}
    {{ return({'touched': touched, 'batches': batches, 'narrowed': touched or batches > 1}) }}
{% endmacro %}


{#- The literal the post-hook substitutes per batch. Its comment is what the
    hook searches for, and carries the batch count the model compiled. -#}
{% macro jira_journal_batch_marker(batch, batches) -%}
toUInt64({{ batch }}) /* jira-journal batch {{ batch }} of {{ batches }} */
{%- endmacro %}


{#- The scope predicate every issue-scoped read carries. `touched_set` is the
    scalar the model computes once; unpacking it costs the size of the set, not
    a scan. Every read must pass the same normalized strings, or one issue
    would hash into two batches. -#}
{% macro jira_journal_issue_in_scope(scope, source_expr, issue_expr) -%}
    {%- set terms = [] -%}
    {%- if scope.touched -%}
        {%- do terms.append('(' ~ source_expr ~ ', ' ~ issue_expr ~ ') IN (SELECT arrayJoin(touched_set))') -%}
    {%- endif -%}
    {%- if scope.batches > 1 -%}
        {%- do terms.append('cityHash64(' ~ source_expr ~ ', ' ~ issue_expr ~ ') % ' ~ scope.batches
                            ~ ' = ' ~ jira_journal_batch_marker(0, scope.batches)) -%}
    {%- endif -%}
    ({{ terms | join(' AND ') }})
{%- endmacro %}


{#- post_hook, ahead of `jira_journal_record_catalogue`: derives batches
    1..N-1 by replaying the compiled statement, each replacing its issues' rows
    the way the materialization's `delete+insert` replaced batch 0's. After a
    full refresh the table holds batch 0 alone, so rows go straight in.

    Must stay ahead of the record: a failed batch then leaves the previous
    record, or none on a swapped-in table, and the next run rebuilds. -#}
{% macro jira_journal_derive_remaining_batches() %}
    {%- set found = modules.re.search('/\\* jira-journal batch 0 of ([0-9]+) \\*/', sql) if execute else none -%}
    {%- if found -%}
        {%- set batches = found.group(1) | int -%}
        {%- set columns = adapter.get_columns_in_relation(this) | map(attribute='quoted') | join(', ') -%}
        {%- set settings = adapter.get_model_query_settings(model) -%}
        {%- set staged = this.incorporate(path={'identifier': this.identifier ~ '__dbt_batch'}) -%}
        {%- for batch in range(1, batches) -%}
            {%- set batch_sql = sql | replace(jira_journal_batch_marker(0, batches),
                                              jira_journal_batch_marker(batch, batches)) -%}
            {%- set derive = 'SELECT ' ~ columns ~ ' FROM (' ~ batch_sql ~ ')' ~ settings -%}
            {%- if flags.FULL_REFRESH -%}
                {%- do run_query('INSERT INTO ' ~ this ~ ' (' ~ columns ~ ') ' ~ derive) -%}
            {%- else -%}
                {%- do run_query('DROP TABLE IF EXISTS ' ~ staged) -%}
                {%- do run_query('CREATE TABLE ' ~ staged ~ ' AS ' ~ this) -%}
                {%- do run_query('INSERT INTO ' ~ staged ~ ' (' ~ columns ~ ') ' ~ derive) -%}
                {%- do run_query('DELETE FROM ' ~ this ~ ' WHERE (insight_source_id, issue_id) IN ('
                                 ~ 'SELECT insight_source_id, issue_id FROM ' ~ staged ~ ')' ~ settings) -%}
                {%- do run_query('INSERT INTO ' ~ this ~ ' (' ~ columns ~ ') SELECT ' ~ columns
                                 ~ ' FROM ' ~ staged ~ settings) -%}
                {%- do run_query('DROP TABLE ' ~ staged) -%}
            {%- endif -%}
        {%- endfor -%}
    {%- endif -%}
    SELECT 1
{% endmacro %}

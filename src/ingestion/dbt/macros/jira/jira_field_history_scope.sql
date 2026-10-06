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


{#- pre_hook, after `jira_journal_drop_rows_bronze_cannot_account_for`: writes
    the run's scope once, then the stages of batch 0, which the model's own
    statement derives.

    The scope is a table, not a predicate each statement re-evaluates: the
    touched set reads the journal, and once batch 0 is written it has shrunk.
    The batch count follows from the scope table alone, so the post-hook reads
    the same count back. -#}
{% macro jira_journal_prepare_scope() %}
    {%- if execute -%}
        {%- set catalogue = jira_journal_catalogue_state() -%}
        {%- set touched = is_incremental() and not catalogue.rebuild -%}
        {%- set scope_sql = jira_journal_touched(catalogue.processed_ms) if touched else jira_journal_all_issues() -%}
        {%- do jira_journal_write_stage('kinds', jira_journal_kinds_sql()) -%}
        {%- do jira_journal_write_stage('scope',
                'SELECT insight_source_id, issue_id FROM (' ~ scope_sql ~ ') GROUP BY insight_source_id, issue_id') -%}
        {%- do jira_journal_derive_stages(0, jira_journal_batches()) -%}
    {%- endif -%}
    SELECT 1
{% endmacro %}


{% macro jira_journal_batches() %}
    {%- set issues = run_query('SELECT count() FROM ' ~ jira_journal_stage('scope')).rows[0][0] | int -%}
    {%- set per_batch = var('jira_journal_issues_per_batch', 40000) | int -%}
    {{- return([1, (issues + per_batch - 1) // per_batch] | max) -}}
{% endmacro %}


{#- post_hook, ahead of `jira_journal_drop_stages`: derives batches 1..N-1, each
    replacing its issues' rows the way the materialization's `delete+insert`
    replaced batch 0's. The statement reads the stages, so replaying it
    unchanged derives whichever batch they hold. After a full refresh the table
    holds batch 0 alone, so rows go straight in.

    Must stay ahead of the record: a failed batch then leaves the previous
    record, or none on a swapped-in table, and the next run rebuilds. -#}
{% macro jira_journal_derive_remaining_batches() %}
    {%- if execute -%}
        {%- set batches = jira_journal_batches() -%}
        {%- set columns = adapter.get_columns_in_relation(this) | map(attribute='quoted') | join(', ') -%}
        {%- set settings = adapter.get_model_query_settings(model) -%}
        {%- set staged = this.incorporate(path={'identifier': this.identifier ~ '__dbt_batch'}) -%}
        {%- set derive = 'SELECT ' ~ columns ~ ' FROM (' ~ sql ~ ')' ~ settings -%}
        {%- for batch in range(1, batches) -%}
            {%- do jira_journal_derive_stages(batch, batches) -%}
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

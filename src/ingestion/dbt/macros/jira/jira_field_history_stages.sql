{#
  The stages of the field-history journal (FIELD-HISTORY-IN-DBT.md §13.1).

  WORKAROUND: ClickHouse expands a CTE afresh at every reference, and the
  journal's arms reference the event chains dozens of times between them, so
  planning the one statement took seconds per batch whatever the batch held.
  The relations read most are written once per batch into tables beside the
  journal, and every arm reads them back.

  The hooks own them: `jira_journal_prepare_scope` writes the run's scope and
  batch 0, `jira_journal_derive_remaining_batches` every later batch, and
  `jira_journal_drop_stages` removes them. A run that dies leaves them behind,
  and the next run replaces them.
#}


{% macro jira_journal_stage(name) -%}
    {{ return(this.incorporate(path={'identifier': this.identifier ~ '__' ~ name})) }}
{%- endmacro %}


{#- The predicate every issue-scoped read carries. Every read must pass the
    normalized strings the scope holds, or it misses the issue. -#}
{% macro jira_journal_in_batch(source_expr, issue_expr) -%}
    ({{ source_expr }}, {{ issue_expr }}) IN (SELECT insight_source_id, issue_id FROM {{ jira_journal_stage('batch') }})
{%- endmacro %}


{#- The modelled fields. `long_text` IS modelled: its body is content-addressed
    into `jira__task_field_text` and the journal carries the hash plus a prefix.

    The catalogue contains a real `created` field, but `created` is also the
    contract's creation-marker sentinel (§10). Emitting both produces two rows
    with the SAME unique_key, and ReplacingMergeTree then keeps one and drops the
    other. The marker wins: its timestamp is the same value, and
    `task_issue_current_state.created_at` reads it by `event_kind`. -#}
{% macro jira_journal_kinds_sql() %}
    SELECT
        insight_source_id,
        field_id,
        field_name,
        field_kind
    FROM {{ ref('jira__task_field_kind') }}
    WHERE field_kind NOT IN ('ignored', 'UNKNOWN')
      AND field_id != 'created'
{% endmacro %}


{#- Every changelog item of the batch, attributed to its issue by the issue's
    immutable id. The changelog stream stamps `id_readable` as the key at fetch
    time, which a move between projects invalidates; `jira_id` (connector
    6.1.0, filled on older rows by the deploy heal) is the only identity used
    here. An item without one names an issue the issue stream never delivered —
    nothing to attribute it to, so it is not in the journal;
    `assert_jira_substream_rows_without_issue_id` reports how many there are. -#}
{% macro jira_journal_changelog_items_sql() %}
    SELECT
        ci.* EXCEPT (id_readable, jira_id),
        assumeNotNull(ci.jira_id)                         AS issue_id
    FROM {{ ref('jira__changelog_items') }} AS ci
    WHERE ci.jira_id IS NOT NULL
      AND {{ jira_journal_in_batch('ci.insight_source_id', 'assumeNotNull(ci.jira_id)') }}
{% endmacro %}


{#- One stage, replaced. `ORDER BY tuple()`: a stage is read whole, by joins.
    `marker` is a comment the statement carries for the log to name it by. -#}
{% macro jira_journal_write_stage(name, select_sql, marker='') %}
    {%- set stage = jira_journal_stage(name) -%}
    {%- set settings = adapter.get_model_query_settings(model) -%}
    {%- do run_query('DROP TABLE IF EXISTS ' ~ stage) -%}
    {%- do run_query('CREATE TABLE ' ~ stage ~ ' ENGINE = MergeTree ORDER BY tuple() AS '
                     ~ marker ~ select_sql ~ settings) -%}
{% endmacro %}


{#- The stages of batch `batch` of `batches`, in dependency order. -#}
{% macro jira_journal_derive_stages(batch, batches) %}
    {%- set marker = '/* jira-journal batch ' ~ batch ~ ' of ' ~ batches ~ ' */ ' -%}
    {%- do jira_journal_write_stage('batch',
            'SELECT insight_source_id, issue_id FROM ' ~ jira_journal_stage('scope')
            ~ ' WHERE cityHash64(insight_source_id, issue_id) % ' ~ batches ~ ' = ' ~ batch,
            marker) -%}
    {%- do jira_journal_write_stage('ranked_events', jira_journal_ranked_events_sql()) -%}
    {%- do jira_journal_write_stage('ordered_events', jira_journal_ordered_events_sql()) -%}
    {%- do jira_journal_write_stage('element_wise_state', jira_journal_element_wise_state_sql()) -%}
{% endmacro %}


{#- post_hook, after the last batch and ahead of the catalogue record. -#}
{% macro jira_journal_drop_stages() %}
    {%- if execute -%}
        {%- for name in ['scope', 'batch', 'kinds', 'ranked_events', 'ordered_events', 'element_wise_state'] -%}
            {%- do run_query('DROP TABLE IF EXISTS ' ~ jira_journal_stage(name)) -%}
        {%- endfor -%}
    {%- endif -%}
    SELECT 1
{% endmacro %}

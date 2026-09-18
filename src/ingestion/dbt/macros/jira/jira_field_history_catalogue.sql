{#
  The field-history journal recomputes an issue only when bronze delivered
  something new about it. Rows also depend on the field catalogue — which
  fields are modelled, as what kind, under what name — and a catalogue that
  changed makes rows of issues bronze never touched wrong. The classification
  the last build used is therefore recorded on the journal table itself, as its
  comment, and a run whose catalogue differs rebuilds the whole table.

  The record travels with the table: a table that carries none — a fresh
  install, a dropped table, the first run after this mechanism landed — is
  rebuilt in full. It also carries `processed_ms` — how far into bronze the
  last run whose replacement completed had read — and `epoch_ms`, the floor
  every row's `_version` takes (FIELD-HISTORY-IN-DBT.md §7).
#}

{% macro jira_journal_catalogue_marker() %}jira-journal-catalogue{% endmacro %}


{#- The catalogue as one fingerprint plus its extraction stamp, or `none` when
    the classifier relation does not exist yet. One line per (source, field):
    the classification and the name, hashed in a fixed order. The WHOLE
    catalogue, not the modelled subset — an `ignored` field decides what the
    unclassified arm leaves out. -#}
{% macro jira_journal_catalogue_current() %}
    {%- set kinds = ref('jira__task_field_kind') -%}
    {%- set present = run_query(
        "SELECT count() FROM system.tables WHERE database = '" ~ kinds.schema
        ~ "' AND name = '" ~ kinds.identifier ~ "'"
    ) -%}
    {%- if present.rows[0][0] | int == 0 -%}
        {{ return(none) }}
    {%- endif -%}
    {%- set current = run_query(
        "SELECT hex(cityHash64(arrayStringConcat(arraySort(groupArray("
        ~ "    concat(insight_source_id, '\\x1f', field_id, '\\x1f', field_kind, '\\x1f', field_name)"
        ~ ")), '\\x1e'))) AS fingerprint,"
        ~ " (SELECT toUnixTimestamp64Milli(toDateTime64("
        ~ "      COALESCE(max(_airbyte_extracted_at), toDateTime64(0, 3)), 3))"
        ~ "  FROM " ~ source('bronze_jira', 'jira_fields') ~ ") AS catalogue_ms"
        ~ " FROM " ~ kinds
    ) -%}
    {{ return({'fingerprint': current.rows[0][0], 'catalogue_ms': current.rows[0][1] | int}) }}
{% endmacro %}


{% macro jira_journal_catalogue_stored() %}
    {%- set rows = run_query(
        "SELECT comment FROM system.tables WHERE database = '" ~ this.schema
        ~ "' AND name = '" ~ this.identifier ~ "'"
    ) -%}
    {%- if rows.rows | length == 0 -%}
        {{ return(none) }}
    {%- endif -%}
    {%- set words = (rows.rows[0][0] or '').split(' ') -%}
    {%- if words[0] != jira_journal_catalogue_marker() -%}
        {{ return(none) }}
    {%- endif -%}
    {%- set stored = {} -%}
    {%- for word in words[1:] if '=' in word -%}
        {%- set pair = word.split('=', 1) -%}
        {%- do stored.update({pair[0]: pair[1]}) -%}
    {%- endfor -%}
    {{ return({'fingerprint': stored.get('fingerprint', ''),
               'epoch_ms': stored.get('epoch_ms', '0') | int,
               'processed_ms': stored.get('processed_ms', '0') | int}) }}
{% endmacro %}


{#- The newest extraction stamp the journal's inputs carry, in milliseconds:
    how far into bronze a run that completes has read. Bronze does not change
    while a run is in flight, so the value the post-hook records is the one the
    body derived from. -#}
{% macro jira_journal_processed_ms() %}
    {%- set rows = run_query(
        "SELECT toUnixTimestamp64Milli(toDateTime64(greatest("
        ~ "  (SELECT COALESCE(max(_airbyte_extracted_at), toDateTime64(0, 3))"
        ~ "   FROM " ~ source('bronze_jira', 'jira_issue') ~ " WHERE jira_id IS NOT NULL),"
        ~ "  (SELECT COALESCE(max(extracted_at), toDateTime64(0, 3))"
        ~ "   FROM " ~ ref('jira__changelog_items') ~ " WHERE jira_id IS NOT NULL)"
        ~ "), 3))"
    ) -%}
    {{- rows.rows[0][0] | int -}}
{% endmacro %}


{#- {rebuild, fingerprint, epoch_ms, processed_ms}.

    Pure in the catalogue and the table's comment, so the model body and the
    post-hook of one run agree — including across a rebuild, which may swap in
    a table carrying no comment and therefore reads as a rebuild from the hook
    as well.

    Every quantity here lives in bronze's time, never the host's. The journal
    decides what to recompute by comparing extraction stamps, so a clock
    reading among them would compare two different clocks: on a warehouse whose
    bronze is older than the machine's idea of now, a wall-clock floor sits
    above every extraction stamp and the comparison stops selecting anything.

    `flags.FULL_REFRESH`, not `should_full_refresh()`: reading the config back
    would let a value this model set be mistaken for one the operator asked
    for. -#}
{% macro jira_journal_catalogue_state() %}
    {%- if not execute -%}
        {{ return({'rebuild': true, 'fingerprint': '', 'epoch_ms': 0, 'processed_ms': 0}) }}
    {%- endif -%}
    {%- set current = jira_journal_catalogue_current() -%}
    {%- if current is none -%}
        {{ return({'rebuild': true, 'fingerprint': '', 'epoch_ms': 0, 'processed_ms': 0}) }}
    {%- endif -%}
    {%- set stored = jira_journal_catalogue_stored() -%}
    {%- if flags.FULL_REFRESH or stored is none or stored.fingerprint != current.fingerprint -%}
        {{ return({'rebuild': true,
                   'fingerprint': current.fingerprint,
                   'epoch_ms': current.catalogue_ms,
                   'processed_ms': 0}) }}
    {%- endif -%}
    {{ return({'rebuild': false,
               'fingerprint': current.fingerprint,
               'epoch_ms': stored.epoch_ms,
               'processed_ms': stored.processed_ms}) }}
{% endmacro %}


{#- pre_hook, on a rebuild only. A rebuild recomputes every issue, so the
    `delete+insert` that follows replaces every row whose issue bronze still
    knows — which, by the model's own construction, is every row it should
    hold: each arm derives its issues from `jira_issue` or from the changelog,
    and bronze forgets neither. What it cannot replace is a row left by an
    EARLIER shape of the model, keyed by an issue id bronze never had; this
    removes exactly those.

    Emptying the table instead would be shorter and is what the first draft
    did. It is worse where it matters: the expensive half of a rebuild is
    deriving the new rows, and a run that dies there — the memory ceiling this
    whole design exists to stay under — would leave the journal empty until the
    next night. Nothing destructive happens here until the derivation has
    already succeeded.

    A model cannot ask dbt for a real full refresh instead: `config()` is a
    no-op at run time (`RuntimeConfigObject.__call__` returns an empty string),
    so only the CLI flag reaches `should_full_refresh()`, and dropping the table
    breaks the run outright — the materialization reads the target's existence
    before hooks fire. -#}
{% macro jira_journal_drop_rows_bronze_cannot_account_for() %}
    {%- if execute and is_incremental() and jira_journal_catalogue_state().rebuild -%}
        {#- Two predicates rather than one over a union: a lightweight delete
            runs as a mutation, and a mutation rejects an unnormalized UNION
            (`UNION mode UNION_DEFAULT must be normalized`). Absent from both
            sources is absent from their union. -#}
        DELETE FROM {{ this }}
        WHERE (insight_source_id, issue_id) NOT IN (
                  SELECT COALESCE(source_id, ''), COALESCE(toString(jira_id), '')
                  FROM {{ source('bronze_jira', 'jira_issue') }}
                  WHERE jira_id IS NOT NULL
              )
          AND (insight_source_id, issue_id) NOT IN (
                  SELECT insight_source_id, assumeNotNull(jira_id)
                  FROM {{ ref('jira__changelog_items') }}
                  WHERE jira_id IS NOT NULL
              )
    {%- else -%}
        SELECT 1
    {%- endif -%}
{% endmacro %}


{#- post_hook. Runs only after the build succeeded, so `processed_ms` marks how
    far a run whose replacement completed had read, and a failed run leaves the
    previous record in place: the next run reconsiders everything bronze
    delivered since the last one that finished. A
    run that could not read the catalogue records nothing, which reads as
    "unknown" and rebuilds next time. -#}
{% macro jira_journal_record_catalogue() %}
    {%- set state = jira_journal_catalogue_state() -%}
    {%- if execute and state.fingerprint != '' -%}
        ALTER TABLE {{ this }} MODIFY COMMENT '{{ jira_journal_catalogue_marker() }} fingerprint={{ state.fingerprint }} epoch_ms={{ state.epoch_ms }} processed_ms={{ jira_journal_processed_ms() }}'
    {%- else -%}
        SELECT 1
    {%- endif -%}
{% endmacro %}


{#- The issue-scope predicate of an incremental run: every input read narrows
    to the touched issues through this one expression. `touched_set` is the
    scalar the model computes once; unpacking it here costs the size of the
    set, not a scan. -#}
{% macro jira_journal_issue_in_scope(source_expr, issue_expr) -%}
({{ source_expr }}, {{ issue_expr }}) IN (SELECT arrayJoin(touched_set))
{%- endmacro %}

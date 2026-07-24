{# -------------------------------------------------------------------------
   Bootstrap model for Jira bronze → RMT promotion.

   Airbyte writes `bronze_jira.*` tables as plain `MergeTree` with
   `destinationSyncMode='append'` (see src/ingestion/airbyte-toolkit/connect.sh).
   Full-refresh streams accumulate N copies per entity across syncs. This
   model's body invokes `promote_bronze_to_rmt` for each Jira bronze table
   (idempotent). The migration replaces MergeTree with
   `ReplacingMergeTree(_airbyte_extracted_at)` + a natural-key ORDER BY, so
   background merges and `FINAL` collapse duplicates.

   Why { do } in the body, not pre_hook:
     pre_hook entries are rendered to SQL strings then executed; the macro
     emits side effects via `run_query` and renders to an empty string —
     which the adapter may treat as an empty SQL statement. Calling the
     macro inside the model body via the do statement runs side effects
     without producing SQL output, guaranteeing the run_query fires during
     view materialization.

   Ordering guarantee:
     Every other Jira staging model declares a depends_on comment that refs
     this model, so dbt's DAG materializes the view (and triggers the
     migrations) before any model reads bronze_jira.*. The view body is
     just a marker.

   Adding a new bronze stream:
     1. Identify the natural key (issue_id, comment_id, ...).
     2. Append a promote_bronze_to_rmt(...) call below.
     3. The model that reads it must add the depends_on comment.
   ------------------------------------------------------------------------- #}

-- @cpt-principle:cpt-dataflow-principle-promote-bronze:p1
{# `staging` tag (not just `jira`): the prod pipeline's staging step selects
   `tag:staging,tag:jira` (an AND-intersection — see render_cronworkflow.py /
   render_sync_trigger.py). Tagged only `jira`, this promote model was excluded
   from that selection (and, with no `+` in the selector, not pulled in as an
   upstream either), so on a real Airbyte sync bronze stayed plain MergeTree and
   the downstream `jira-enrich` step crashed with `Storage MergeTree doesn't
   support FINAL` (issue #1886). `schema='staging'` sets the target DATABASE, not
   a dbt tag, so it does not participate in tag selection. #}
{{ config(
    materialized='view',
    schema='staging',
    tags=['jira', 'staging']
) }}

{# All Jira bronze tables carry a `unique_key` column added by the connector
   AddFields transformation (formula: `{tenant}-{source}-{natural_id}`), so
   `order_by='unique_key'` is equivalent to the natural-key composite. #}
{% do promote_bronze_to_rmt(table='bronze_jira.jira_projects',      order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_jira.jira_user',          order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_jira.jira_sprints',       order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_jira.jira_fields',        order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_jira.jira_issue',         order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_jira.jira_comments',      order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_jira.jira_worklogs',      order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_jira.jira_issue_history', order_by='unique_key') %}
{# jira_issue_keys is the lightweight substream parent (issue key + updated
   per issue). Reconcile (ADR-0015) auto-selects every discovered stream, so
   it lands as a real bronze table and needs the same RMT dedup. No staging
   model reads it — promotion only caps its growth. #}
{% do promote_bronze_to_rmt(table='bronze_jira.jira_issue_keys',    order_by='unique_key') %}

SELECT 1 AS promoted

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
{{ config(
    materialized='view',
    schema='staging',
    tags=['jira']
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

SELECT 1 AS promoted

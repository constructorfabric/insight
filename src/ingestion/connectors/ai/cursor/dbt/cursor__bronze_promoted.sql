{# -------------------------------------------------------------------------
   Bootstrap model for Cursor bronze → RMT promotion.

   Counterpart of `jira__bronze_promoted` for Cursor. See ADR-0002. The
   `promote_bronze_to_rmt` macro is idempotent — already-RMT tables are
   detected and skipped on subsequent runs.
   ------------------------------------------------------------------------- #}

-- @cpt-principle:cpt-dataflow-principle-promote-bronze:p1
{{ config(
    materialized='view',
    schema='staging',
    tags=['cursor']
) }}

{% do promote_bronze_to_rmt(table='bronze_cursor.cursor_audit_logs',                order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_cursor.cursor_daily_usage',               order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_cursor.cursor_members',                   order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_cursor.cursor_usage_events',              order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_cursor.cursor_usage_events_daily_resync', order_by='unique_key') %}

SELECT 1 AS promoted

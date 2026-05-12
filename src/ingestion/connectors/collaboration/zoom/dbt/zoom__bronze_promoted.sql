{# -------------------------------------------------------------------------
   Bootstrap model for Zoom bronze → RMT promotion.

   Counterpart of `jira__bronze_promoted` for Zoom. See ADR-0002. The
   `promote_bronze_to_rmt` macro is idempotent — already-RMT tables are
   detected and skipped on subsequent runs.
   ------------------------------------------------------------------------- #}

-- @cpt-principle:cpt-dataflow-principle-promote-bronze:p1
{{ config(
    materialized='view',
    schema='staging',
    tags=['zoom']
) }}

{% do promote_bronze_to_rmt(table='bronze_zoom.meetings',     order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_zoom.participants', order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_zoom.users',        order_by='unique_key') %}

SELECT 1 AS promoted

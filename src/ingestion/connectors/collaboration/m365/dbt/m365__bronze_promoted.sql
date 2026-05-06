{# -------------------------------------------------------------------------
   Bootstrap model for Microsoft 365 bronze → RMT promotion.

   Counterpart of `jira__bronze_promoted` for M365. See ADR-0002. The
   `promote_bronze_to_rmt` macro is idempotent — already-RMT tables are
   detected and skipped on subsequent runs.
   ------------------------------------------------------------------------- #}

-- @cpt-principle:cpt-dataflow-principle-promote-bronze:p1
{{ config(
    materialized='view',
    schema='staging',
    tags=['m365']
) }}

{% do promote_bronze_to_rmt(table='bronze_m365.email_activity',      order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_m365.onedrive_activity',   order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_m365.sharepoint_activity', order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_m365.teams_activity',      order_by='unique_key') %}

SELECT 1 AS promoted

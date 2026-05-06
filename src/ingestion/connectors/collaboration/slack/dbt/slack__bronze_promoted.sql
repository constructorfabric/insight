{# -------------------------------------------------------------------------
   Bootstrap model for Slack bronze → RMT promotion.

   Counterpart of `jira__bronze_promoted` for Slack. See ADR-0002. The
   `promote_bronze_to_rmt` macro is idempotent — already-RMT tables are
   detected and skipped on subsequent runs.
   ------------------------------------------------------------------------- #}

-- @cpt-principle:cpt-dataflow-principle-promote-bronze:p1
{{ config(
    materialized='view',
    schema='staging',
    tags=['slack']
) }}

{% do promote_bronze_to_rmt(table='bronze_slack.channels',      order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_slack.messages',      order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_slack.users',         order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_slack.users_details', order_by='unique_key') %}

SELECT 1 AS promoted

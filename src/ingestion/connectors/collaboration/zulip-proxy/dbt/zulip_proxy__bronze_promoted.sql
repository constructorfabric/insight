{# -------------------------------------------------------------------------
   Bootstrap model for Zulip-Proxy bronze → RMT promotion.

   Mirrors zoom__bronze_promoted / jira__bronze_promoted. The
   `promote_bronze_to_rmt` macro is idempotent — already-RMT tables are
   detected and skipped on subsequent runs (see ADR-0002).
   ------------------------------------------------------------------------- #}

-- @cpt-principle:cpt-dataflow-principle-promote-bronze:p1
{{ config(
    materialized='view',
    schema='staging',
    tags=['zulip-proxy']
) }}

{% do promote_bronze_to_rmt(table='bronze_zulip_proxy.users',    order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_zulip_proxy.messages', order_by='unique_key') %}

SELECT 1 AS promoted

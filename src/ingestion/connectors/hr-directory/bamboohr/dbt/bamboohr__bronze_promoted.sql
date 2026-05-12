{# -------------------------------------------------------------------------
   Bootstrap model for BambooHR bronze → RMT promotion.

   Counterpart of `jira__bronze_promoted` for BambooHR. See ADR-0002 for the
   reasoning; the macro `promote_bronze_to_rmt` is idempotent — already-RMT
   tables are detected and skipped on subsequent runs.

   All BambooHR bronze tables carry a `unique_key` column added by the
   connector AddFields transformation (formula:
   `{tenant}-{source}-{natural_id}`), so `order_by='unique_key'` is
   equivalent to the natural-key composite.
   ------------------------------------------------------------------------- #}

-- @cpt-principle:cpt-dataflow-principle-promote-bronze:p1
{{ config(
    materialized='view',
    schema='staging',
    tags=['bamboohr']
) }}

{% do promote_bronze_to_rmt(table='bronze_bamboohr.employees',      order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_bamboohr.leave_requests', order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_bamboohr.meta_fields',    order_by='unique_key') %}

SELECT 1 AS promoted

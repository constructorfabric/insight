{# -------------------------------------------------------------------------
   Bootstrap model for Workday bronze → RMT promotion.

   Counterpart of `bamboohr__bronze_promoted` for Workday. See ADR-0002 for
   the reasoning; the macro `promote_bronze_to_rmt` is idempotent —
   already-RMT tables are detected and skipped on subsequent runs.

   All Workday bronze tables carry a `unique_key` column added by the
   connector AddFields transformation (formula:
   `{tenant}-{source}-{natural_id}`), so `order_by='unique_key'` is
   equivalent to the natural-key composite.
   ------------------------------------------------------------------------- #}

-- @cpt-principle:cpt-dataflow-principle-promote-bronze:p1
{{ config(
    materialized='view',
    schema='staging',
    tags=['workday']
) }}

{% do promote_bronze_to_rmt(table='bronze_workday.workers',        order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_workday.leave_requests', order_by='unique_key') %}

SELECT 1 AS promoted

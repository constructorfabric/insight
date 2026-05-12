{# -------------------------------------------------------------------------
   Bootstrap model for MS Entra bronze → RMT promotion.

   Counterpart of `bamboohr__bronze_promoted` for MS Entra. See ADR-0002 for
   the reasoning; the macro `promote_bronze_to_rmt` is idempotent —
   already-RMT tables are detected and skipped on subsequent runs.

   The `users` Bronze table carries a `unique_key` column added by the
   connector AddFields transformation
   (`{tenant}-{source}-{entra_object_id}`), so `order_by='unique_key'` is
   equivalent to the natural key (Entra `id` / JWT `oid`).
   ------------------------------------------------------------------------- #}

-- @cpt-principle:cpt-dataflow-principle-promote-bronze:p1
{{ config(
    materialized='view',
    schema='staging',
    tags=['ms-entra']
) }}

{% do promote_bronze_to_rmt(table='bronze_ms_entra.users', order_by='unique_key') %}

SELECT 1 AS promoted

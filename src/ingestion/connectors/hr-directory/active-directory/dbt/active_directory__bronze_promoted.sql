{# -------------------------------------------------------------------------
   Bootstrap model for Active Directory bronze → RMT promotion.

   Counterpart of `ms_entra__bronze_promoted`. See ADR-0002 for the reasoning;
   the macro `promote_bronze_to_rmt` is idempotent — already-RMT tables are
   detected and skipped on subsequent runs.

   The `users` Bronze table carries a `unique_key` column added by the connector
   (`{tenant}-{source}-{objectGUID}`), so `order_by='unique_key'` is equivalent
   to the natural key (AD objectGUID).
   ------------------------------------------------------------------------- #}

-- @cpt-principle:cpt-dataflow-principle-promote-bronze:p1
{{ config(
    materialized='view',
    schema='staging',
    tags=['active-directory']
) }}

{% do promote_bronze_to_rmt(table='bronze_active_directory.users', order_by='unique_key') %}

SELECT 1 AS promoted

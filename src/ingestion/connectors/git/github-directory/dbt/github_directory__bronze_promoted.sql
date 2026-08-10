{# -------------------------------------------------------------------------
   Bootstrap model for GitHub directory bronze -> RMT promotion.

   The `promote_bronze_to_rmt` macro is idempotent — already-RMT tables are
   detected and skipped on subsequent runs. Promotion is what makes the
   `FINAL` read inside the snapshot macro well-defined, so it must run before
   github_directory__org_members_snapshot.
   ------------------------------------------------------------------------- #}

-- @cpt-principle:cpt-dataflow-principle-promote-bronze:p1
{{ config(
    materialized='view',
    schema='staging',
    tags=['github_directory']
) }}

{% do promote_bronze_to_rmt(table='bronze_github_directory.org_members', order_by='unique_key') %}

SELECT 1 AS promoted

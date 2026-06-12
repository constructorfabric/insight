{# -------------------------------------------------------------------------
   Bootstrap model for Figma bronze → RMT promotion.

   Counterpart of `confluence__bronze_promoted` for Figma. See ADR-0002. The
   `promote_bronze_to_rmt` macro is idempotent — already-RMT tables are
   detected and skipped on subsequent runs.
   ------------------------------------------------------------------------- #}

-- @cpt-principle:cpt-dataflow-principle-promote-bronze:p1
{{ config(
    materialized='view',
    schema='staging',
    tags=['figma']
) }}

{% do promote_bronze_to_rmt(table='bronze_figma.design_projects',      order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_figma.design_files',         order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_figma.design_file_meta',     order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_figma.design_file_versions', order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_figma.design_file_comments', order_by='unique_key') %}

SELECT 1 AS promoted

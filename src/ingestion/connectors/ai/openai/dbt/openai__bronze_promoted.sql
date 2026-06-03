{# -------------------------------------------------------------------------
   Bootstrap model for OpenAI bronze → RMT promotion.

   Counterpart of `jira__bronze_promoted`. See ADR-0002. The
   `promote_bronze_to_rmt` macro is idempotent — already-RMT tables are
   detected and skipped, and tables not yet created by Airbyte are skipped.
   Every stream emits a deterministic `unique_key` (primary_key in
   connector.yaml), so ORDER BY unique_key is the natural-key dedup.
   ------------------------------------------------------------------------- #}

-- @cpt-principle:cpt-dataflow-principle-promote-bronze:p1
{{ config(
    materialized='view',
    schema='staging',
    tags=['openai']
) }}

{% do promote_bronze_to_rmt(table='bronze_openai.users',                      order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_openai.usage_completions',          order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_openai.usage_embeddings',           order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_openai.usage_moderations',          order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_openai.usage_images',               order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_openai.usage_audio_speeches',       order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_openai.usage_audio_transcriptions', order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_openai.usage_vector_stores',        order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_openai.usage_code_interpreter',     order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_openai.costs',                      order_by='unique_key') %}

SELECT 1 AS promoted

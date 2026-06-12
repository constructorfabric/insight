{# -------------------------------------------------------------------------
   Bootstrap model for chatgpt-team bronze → RMT promotion.

   Counterpart of `claude_team__bronze_promoted`. See ADR-0002. The
   `promote_bronze_to_rmt` macro is idempotent — already-RMT tables are
   detected and skipped on subsequent runs.
   ------------------------------------------------------------------------- #}

-- @cpt-principle:cpt-dataflow-principle-promote-bronze:p1
{{ config(
    materialized='view',
    schema='staging',
    tags=['chatgpt-team']
) }}

{% do promote_bronze_to_rmt(table='bronze_chatgpt_team.chatgpt_team_seats',                 order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_chatgpt_team.chatgpt_team_chat_activity',         order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_chatgpt_team.chatgpt_team_codex_user_daily',      order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_chatgpt_team.chatgpt_team_subscription_usage',    order_by='unique_key') %}
{% do promote_bronze_to_rmt(table='bronze_chatgpt_team.chatgpt_team_subscription_balance',  order_by='unique_key') %}

SELECT 1 AS promoted

{{ config(
    materialized='view',
    alias='jira__task_projects',
    schema='staging',
    tags=['jira', 'staging', 'silver:class_task_projects']
) }}

-- View, not table: bronze `jira_projects` is MergeTree (full_refresh + overwrite),
-- so the current state of bronze is the current state of staging — no incremental
-- accumulation needed. Silver `class_task_projects` is RMT(_version), reads via FINAL.

-- INVARIANT: the destination adds `lead_account_id` to bronze on the connector's
-- next sync; a build before that must still succeed, so the lead waits for it.
{%- set project_columns = adapter.get_columns_in_relation(source('bronze_jira', 'jira_projects')) if execute else [] %}
{%- set projects_have_lead = 'lead_account_id' in (project_columns | map(attribute='name') | list) %}

SELECT
    p.unique_key                                AS unique_key,
    p.source_id                                 AS insight_source_id,
    CAST('jira' AS String)                      AS data_source,
    toString(p.project_id)                      AS project_id,
    p.project_key                               AS project_key,
    p.name                                      AS name,
{%- if projects_have_lead %}
    nullIf(toString(p.lead_account_id), '')     AS lead_id,
{%- else %}
    CAST(NULL AS Nullable(String))              AS lead_id,
{%- endif %}
    p.project_type                              AS project_type,
    p.style                                     AS project_style,
    -- `p.archived` is `Nullable(Bool)` in bronze. `toString(true) = 'true'`, which
    -- `toUInt8OrNull` cannot parse — the old expression silently produced 100% NULL.
    CAST(p.archived AS Nullable(UInt8))         AS archived,
    now64(3)                                    AS collected_at,
    toUnixTimestamp64Milli(now64(3))            AS _version
FROM {{ source('bronze_jira', 'jira_projects') }} p
-- `jira_projects` bronze = MergeTree (full_refresh + overwrite), FINAL not supported.

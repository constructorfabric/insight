-- depends_on: {{ ref('github__bronze_promoted') }}
{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    tags=['github']
) }}

-- SCD2 snapshot of the link sets an issue holds — appends a version only when
-- one of them changes.
--
-- Bronze keeps the present, so it cannot answer when a link first appeared.
-- For hierarchy and dependency links the timeline answers that exactly and
-- this model is redundant. For a pull request that closes an issue it is the
-- only answer there is: that link has no reliable event, so the interval model
-- bounds it by when it was first and last observed here.

{{ snapshot(
    source_ref=source('bronze_github', 'issue_links'),
    unique_key_col='unique_key',
    check_cols=[
        'parent_json', 'sub_issues_json', 'blocked_by_json', 'blocking_json',
        'closed_by_pull_requests_json'
    ]
) }}

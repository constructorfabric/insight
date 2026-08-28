{{ config(
    materialized='table',
    engine='MergeTree',
    order_by=['tenant_id', 'entity_id', 'metric_date'],
    schema=var('gold_database'),
    settings={'allow_nullable_key': 1},
    tags=['gold'],
    query_settings=metric_serving_query_settings()
) }}

-- One row per pull-request review or comment, with everything the evidence
-- build needs already resolved: the actor, the request it is on, and the
-- dimension tuple.
--
-- Materialized so the identity map and the pull-request join run once per
-- build in their own query budget, the same reason git_default_branch_commits
-- and git_derived_commits are their own tables. Inside the evidence model this
-- was a CTE joining two FINAL reads and the identity map, all of it competing
-- for one query's memory with every other git measure.
WITH review_person_map AS (
    {{ resolve_person_id() }}
)

SELECT
    events.tenant_id AS tenant_id,
    events.source_id AS source_id,
    events.event_kind AS event_kind,
    events.unique_key AS event_key,
    events.pr_id AS pr_id,
    events.pr_number AS pr_number,
    lower(trimBoth(events.actor_email)) AS entity_id,
    toDate(events.created_at) AS metric_date,
    events.created_at AS observed_at,
    coalesce(prs.title, '') AS title,
    coalesce(prs.author_name, '') AS author_name,
    lower(trimBoth(coalesce(prs.author_email, ''))) AS author_email,
    coalesce(toString(actor_map.person_id), '') AS actor_person_id,
    coalesce(toString(author_map.person_id), '') AS author_person_id,
    -- Own vs others by canonical person where both emails resolve, by
    -- normalized email otherwise. An undeterminable author reads `others`
    -- — the agreed reading for an absent signal (see branch_scope), which
    -- keeps own + others = total.
    multiIf(
        author_email = '', 'others',
        actor_person_id != '' AND author_person_id != '',
            if(actor_person_id = author_person_id, 'own', 'others'),
        entity_id = author_email, 'own',
        'others'
    ) AS comment_target_value,
    if(comment_target_value = 'own', 'Own PRs', 'Others'' PRs') AS comment_target_label,
    if(coalesce(events.project_key, '') = '', '__unknown__', concat(coalesce(toString(events.source_id), ''), ':', events.project_key)) AS project_value,
    if(coalesce(events.project_key, '') = '', 'Unknown', events.project_key) AS project_label,
    concat(coalesce(toString(events.source_id), ''), ':', coalesce(events.project_key, ''), '/', coalesce(events.repo_slug, '')) AS repository_value,
    if(coalesce(events.project_key, '') = '', coalesce(events.repo_slug, ''), concat(events.project_key, '/', events.repo_slug)) AS repository_label,
    if(coalesce(prs.destination_branch, '') = '', '__unknown__', assumeNotNull(prs.destination_branch)) AS destination_branch_value,
    if(coalesce(prs.destination_branch, '') = '', 'Unknown', assumeNotNull(prs.destination_branch)) AS destination_branch_label,
    replaceOne(events.data_source, 'insight_', '') AS source_value,
    {{ git_source_label('source_value') }} AS source_label,
    CAST(arrayConcat(
        if(
            events.event_kind = 'comment',
            [tuple('comment_target', comment_target_value, comment_target_label)],
            []
        ),
        [
            tuple('destination_branch', destination_branch_value, destination_branch_label),
            tuple('repository', repository_value, repository_label),
            tuple('project', project_value, project_label),
            tuple('source_id', coalesce(toString(events.source_id), ''), coalesce(toString(events.source_id), '')),
            tuple('source', source_value, source_label)
        ]
    ) AS Array(Tuple(key String, value String, label Nullable(String)))) AS source_dimensions
FROM {{ ref('class_git_pr_review_events') }} AS events FINAL
LEFT JOIN {{ ref('class_git_pull_requests') }} AS prs FINAL
    ON prs.tenant_id = events.tenant_id
    AND prs.source_id = events.source_id
    AND prs.project_key = events.project_key
    AND prs.repo_slug = events.repo_slug
    AND prs.pr_id = events.pr_id
LEFT JOIN review_person_map AS actor_map
    ON actor_map.email = lower(trimBoth(events.actor_email))
LEFT JOIN review_person_map AS author_map
    ON author_map.email = lower(trimBoth(coalesce(prs.author_email, '')))
WHERE trimBoth(events.actor_email) != ''
  AND events.created_at IS NOT NULL

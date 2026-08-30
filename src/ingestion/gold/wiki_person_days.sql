{{ config(
    materialized='table',
    engine='MergeTree',
    order_by=['tenant_id', 'author_email', 'activity_date'],
    partition_by='toYYYYMM(activity_date)',
    schema=var('gold_database'),
    settings={'allow_nullable_key': 1},
    tags=['gold'],
    query_settings=metric_serving_query_settings()
) }}

-- Wiki work per person and day: the edits a person made, the pages those edits
-- touched, and the comments other people left on pages that person authored.

WITH
-- The only route from a comment to a person: wiki engagement names the page it
-- sits on and never the person, so authorship of the page carries it.
page_authors AS (
    SELECT
        tenant_id,
        source_id,
        page_id,
        lower(trimBoth(author_email)) AS author_email
    FROM {{ ref('class_wiki_pages') }} FINAL
    WHERE author_email LIKE '%@%'
),
edit_activity AS (
    SELECT
        tenant_id,
        source_id,
        lower(trimBoth(author_email)) AS author_email,
        day AS activity_date,
        total_edits AS edits,
        pages_edited AS pages_edited
    FROM {{ ref('class_wiki_activity') }} FINAL
    WHERE tenant_id IS NOT NULL
      AND author_email LIKE '%@%'
      AND day IS NOT NULL
),
page_comments AS (
    SELECT
        engagement.tenant_id AS tenant_id,
        engagement.source_id AS source_id,
        authors.author_email AS author_email,
        engagement.day AS activity_date,
        engagement.total_comments AS comments_received
    FROM (
        SELECT
            tenant_id,
            source_id,
            page_id,
            day,
            total_comments
        FROM {{ ref('class_wiki_engagement') }} FINAL
        WHERE tenant_id IS NOT NULL
          AND day IS NOT NULL
    ) AS engagement
    INNER JOIN page_authors AS authors
        ON engagement.tenant_id = authors.tenant_id
       AND engagement.source_id = authors.source_id
       AND engagement.page_id = authors.page_id
),
-- A person may have edited without being commented on, or been commented on
-- without editing; the union carries both key sets into one grain.
person_day_contributions AS (
    SELECT
        tenant_id,
        source_id,
        author_email,
        activity_date,
        edits,
        pages_edited,
        toUInt32(0) AS comments_received
    FROM edit_activity

    UNION ALL

    SELECT
        tenant_id,
        source_id,
        author_email,
        activity_date,
        toUInt32(0) AS edits,
        toUInt32(0) AS pages_edited,
        comments_received
    FROM page_comments
)

SELECT
    -- SAFETY: every contributing branch admits only non-null keys.
    assumeNotNull(tenant_id) AS tenant_id,
    source_id AS source_id,
    assumeNotNull(author_email) AS author_email,
    assumeNotNull(activity_date) AS activity_date,
    sum(edits) AS edits,
    sum(pages_edited) AS pages_edited,
    sum(comments_received) AS comments_received
FROM person_day_contributions
GROUP BY tenant_id, source_id, author_email, activity_date

{{ config(
    materialized='table',
    engine=insight_engine('MergeTree'),
    order_by=['metric_date', 'tenant_id', 'entity_id', 'measure_key', 'record_id'],
    schema=var('gold_database'),
    tags=['gold'],
    query_settings=metric_serving_query_settings(join_use_nulls=1)
) }}

WITH
{{ git_evidence_authored_commit_ctes() }},
file_changes_source AS (
    SELECT
        commits.tenant_id AS tenant_id,
        commits.entity_id AS entity_id,
        commits.metric_date AS metric_date,
        file_changes.category AS category,
        {{ git_file_category_label('file_changes.category') }} AS category_label,
        file_changes.file_extension_value AS file_extension,
        file_changes.file_extension_label AS file_extension_label,
        file_changes.change_type_value AS change_type,
        file_changes.change_type_label AS change_type_label,
        file_changes.lines_added AS lines_added,
        file_changes.lines_removed AS lines_removed,
        commits.repository_value AS repository_value,
        commits.repository_label AS repository_label,
        commits.source_id AS commit_source_id,
        commits.source_value AS source_value,
        commits.commit_hash AS commit_hash,
        commits.message AS message,
        commits.author_name AS author_name,
        commits.observed_at AS observed_at,
        -- Inherited, not recomputed: lines belong to the bucket their commit
        -- belongs to. A commit in `default` whose lines read `non_default` is
        -- exactly the column disagreement #2464 was about.
        commits.branch_scope_value AS branch_scope_value,
        commits.branch_scope_label AS branch_scope_label,
        CAST(
            [
                tuple('branch_scope', branch_scope_value, branch_scope_label),
                tuple('file_extension', file_extension, file_extension_label),
                tuple('change_type', change_type, change_type_label),
                tuple('repository', repository_value, repository_label),
                tuple('project', commits.project_value, commits.project_label),
                tuple('source_id', coalesce(toString(commits.source_id), ''), coalesce(toString(commits.source_id), '')),
                tuple('source', commits.source_value, commits.source_label)
            ] AS Array(Tuple(key String, value String, label Nullable(String)))
        ) AS file_source_dimensions,
        CAST(
            [
                tuple('branch_scope', branch_scope_value, branch_scope_label),
                tuple('category', category, category_label),
                tuple('file_extension', file_extension, file_extension_label),
                tuple('change_type', change_type, change_type_label),
                tuple('repository', repository_value, repository_label),
                tuple('project', commits.project_value, commits.project_label),
                tuple('source_id', coalesce(toString(commits.source_id), ''), coalesce(toString(commits.source_id), '')),
                tuple('source', commits.source_value, commits.source_label)
            ] AS Array(Tuple(key String, value String, label Nullable(String)))
        ) AS category_source_dimensions
    FROM (
        SELECT
            tenant_id,
            source_id,
            project_key,
            repo_slug,
            commit_hash,
            {{ git_file_category('file_path') }} AS category,
            if(raw_file_change.file_extension = '', '__unknown__', lower(raw_file_change.file_extension)) AS file_extension_value,
            if(raw_file_change.file_extension = '', 'Unknown', lower(raw_file_change.file_extension)) AS file_extension_label,
            if(raw_file_change.change_type = '', '__unknown__', lower(raw_file_change.change_type)) AS change_type_value,
            multiIf(
                raw_file_change.change_type = '', 'Unknown',
                lower(raw_file_change.change_type) = 'added', 'Added',
                lower(raw_file_change.change_type) = 'modified', 'Modified',
                lower(raw_file_change.change_type) = 'renamed', 'Renamed',
                lower(raw_file_change.change_type) IN ('deleted', 'removed'), 'Deleted',
                lower(raw_file_change.change_type) = 'copied', 'Copied',
                lower(raw_file_change.change_type) = 'type_changed', 'Type changed',
                raw_file_change.change_type
            ) AS change_type_label,
            sum(lines_added) AS lines_added,
            sum(lines_removed) AS lines_removed
        FROM {{ ref('git_deduplicated_file_changes') }} AS raw_file_change
        GROUP BY tenant_id, source_id, project_key, repo_slug, commit_hash, category, file_extension_value, file_extension_label, change_type_value, change_type_label
    ) AS file_changes
    INNER JOIN {{ ref('git_authored_commits') }} AS commits
        ON commits.tenant_id = file_changes.tenant_id
        AND commits.source_id = file_changes.source_id
        AND commits.project_key = file_changes.project_key
        AND commits.repo_slug = file_changes.repo_slug
        AND commits.commit_hash = file_changes.commit_hash
),
-- The size a commit reported when its file changes never arrived. Those lines
-- are in the class contract but reach no file-change row, so without this they
-- are absent from every size measure while the commit still counts — Commits
-- and Lines then describe different work on the same side of the branch split,
-- and a commit that genuinely changed nothing is indistinguishable from one
-- whose diff was never collected. Emitting the reported size separates them:
-- zero means zero, and absent means the source reported no size at all.
--
-- Only the totals: the file grain is genuinely unknown here, so the drilldown
-- dimensions say `__unknown__` rather than inventing a category, and the
-- category-scoped measures (code / test lines) stay out.
unattributed_line_measures AS (
    SELECT
        tenant_id,
        entity_id,
        metric_date,
        source_id AS commit_source_id,
        source_value,
        commit_hash,
        message,
        author_name,
        observed_at,
        repository_label,
        branch_scope_label,
        '__unknown__' AS category,
        line_measure.1 AS measure_key,
        line_measure.2 AS value,
        -- INVARIANT: the same KEYS as category_source_dimensions, which the
        -- file-derived rows of these same measures carry. A breakdown groups
        -- by a hidden source-id key as well as the visible ones, so a key
        -- missing here splits one repository into two rows — a reader
        -- comparing the two literals is what keeps them in step.
        CAST(
            [
                tuple('branch_scope', branch_scope_value, branch_scope_label),
                tuple('category', '__unknown__', 'Unknown'),
                tuple('file_extension', '__unknown__', 'Unknown'),
                tuple('change_type', '__unknown__', 'Unknown'),
                tuple('repository', repository_value, repository_label),
                tuple('project', project_value, project_label),
                tuple('source_id', coalesce(toString(source_id), ''), coalesce(toString(source_id), '')),
                tuple('source', source_value, source_label)
            ] AS Array(Tuple(key String, value String, label Nullable(String)))
        ) AS dimensions
    FROM authored_commits
    ARRAY JOIN CAST(arrayConcat(
        if(
            lines_added IS NOT NULL,
            [
                tuple('lines_added', toFloat64(assumeNotNull(lines_added))),
                tuple(
                    if(branch_scope_value = 'default', 'default_lines_added', 'non_default_lines_added'),
                    toFloat64(assumeNotNull(lines_added))
                )
            ],
            []
        ),
        if(
            lines_removed IS NOT NULL,
            [
                tuple('lines_removed', toFloat64(assumeNotNull(lines_removed))),
                tuple(
                    if(branch_scope_value = 'default', 'default_lines_removed', 'non_default_lines_removed'),
                    toFloat64(assumeNotNull(lines_removed))
                )
            ],
            []
        )
    ) AS Array(Tuple(measure_key String, value Float64))) AS line_measure
    WHERE has_no_file_changes
),
file_change_measures AS (
    SELECT
        tenant_id,
        entity_id,
        metric_date,
        commit_source_id,
        source_value,
        commit_hash,
        message,
        author_name,
        observed_at,
        repository_label,
        branch_scope_label,
        category,
        file_measure.1 AS measure_key,
        file_measure.2 AS value,
        file_measure.3 AS dimensions
    FROM file_changes_source
    ARRAY JOIN CAST(arrayConcat(
        if(
            lines_added IS NOT NULL,
            [tuple('lines_added', toFloat64(assumeNotNull(lines_added)), category_source_dimensions)],
            []
        ),
        if(
            lines_removed IS NOT NULL,
            [tuple('lines_removed', toFloat64(assumeNotNull(lines_removed)), category_source_dimensions)],
            []
        ),
        if(
            category = 'code' AND lines_added IS NOT NULL,
            [tuple('code_lines_added', toFloat64(assumeNotNull(lines_added)), file_source_dimensions)],
            []
        ),
        if(
            category IN ('code', 'test') AND lines_added IS NOT NULL,
            [tuple('test_lines_added', if(category = 'test', toFloat64(assumeNotNull(lines_added)), 0.0), file_source_dimensions)],
            []
        ),
        if(
            category IN ('code', 'test') AND lines_added IS NOT NULL,
            [tuple('test_and_code_lines_added', toFloat64(assumeNotNull(lines_added)), file_source_dimensions)],
            []
        ),
        if(
            lines_added IS NOT NULL,
            [tuple(
                if(branch_scope_value = 'default', 'default_lines_added', 'non_default_lines_added'),
                toFloat64(assumeNotNull(lines_added)),
                category_source_dimensions
            )],
            []
        ),
        if(
            lines_removed IS NOT NULL,
            [tuple(
                if(branch_scope_value = 'default', 'default_lines_removed', 'non_default_lines_removed'),
                toFloat64(assumeNotNull(lines_removed)),
                category_source_dimensions
            )],
            []
        ),
        if(
            category = 'code' AND lines_added IS NOT NULL,
            [tuple(
                if(branch_scope_value = 'default', 'default_code_lines_added', 'non_default_code_lines_added'),
                toFloat64(assumeNotNull(lines_added)),
                file_source_dimensions
            )],
            []
        )
    ) AS Array(Tuple(
        measure_key String,
        value Float64,
        dimensions Array(Tuple(key String, value String, label Nullable(String)))
    ))) AS file_measure
),
commit_line_measures AS (
    SELECT
        tenant_id,
        entity_id,
        metric_date,
        measure_key,
        value,
        dimensions,
        category,
        commit_source_id,
        source_value,
        commit_hash,
        message,
        author_name,
        observed_at,
        repository_label,
        branch_scope_label
    FROM file_change_measures

    UNION ALL

    SELECT
        tenant_id,
        entity_id,
        metric_date,
        measure_key,
        value,
        dimensions,
        category,
        commit_source_id,
        source_value,
        commit_hash,
        message,
        author_name,
        observed_at,
        repository_label,
        branch_scope_label
    FROM unattributed_line_measures
)
SELECT
    assumeNotNull(tenant_id) AS tenant_id,
    'git' AS source_key,
    'person' AS entity_type,
    {{ normalized_email('assumeNotNull(entity_id)') }} AS entity_id,
    '' AS account_source_type,
    '' AS account_source_id,
    '' AS account_id,
    assumeNotNull(metric_date) AS metric_date,
    toNullable(toDateTime64(observed_at, 3)) AS observed_at,
    measure_key,
    concat(
        source_value,
        ':',
        commit_hash,
        ':',
        measure_key,
        ':',
        -- INVARIANT: category is keyed though not a dimension: test lines share
        -- their file dimensions with the code lines of the same commit.
        hex(sipHash128(toString(arrayMap(d -> tuple(d.1, d.2), dimensions)), category))
    ) AS record_id,
    'commit' AS record_kind,
    'event' AS granularity,
    if(message = '', commit_hash, message) AS record_label,
    toNullable(value) AS contribution,
    CAST(NULL AS Nullable(String)) AS subject_key,
    dimensions,
    map(
        'source_id', coalesce(toString(commit_source_id), ''),
        'ref', commit_hash,
        'title', message,
        'repository', repository_label,
        'author', author_name,
        'branch_scope', branch_scope_label
    ) AS details
FROM commit_line_measures
WHERE tenant_id IS NOT NULL
  AND entity_id IS NOT NULL
  AND metric_date IS NOT NULL


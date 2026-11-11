{{ metric_evidence_table(join_use_nulls=1) }}

-- Resolution happens at READ time, and account-first: a row naming its author's
-- account (pull requests) resolves through that binding, everything else
-- through the e-mail map. The account columns below are that key, in the
-- identity store's vocabulary rather than data_source's.
SELECT
    src.tenant_id,
    src.source_key,
    src.entity_type,
    {{ normalized_email('src.entity_id') }} AS entity_id,
    src.account_source_type,
    -- INVARIANT: every account column is a plain String in every evidence
    -- relation. The class PR source_id is Nullable, and one family typing it
    -- differently fails the service's exact-column probe, blanking its metrics.
    coalesce(src.account_source_id, '') AS account_source_id,
    src.account_id,
    src.metric_date,
    src.observed_at,
    src.measure_key,
    src.record_id,
    src.record_kind,
    src.granularity,
    src.record_label,
    src.contribution,
    src.subject_key,
    src.dimensions,
    src.details
FROM (


WITH
-- The default branch NAME per repository, which is what a pull request's
-- destination has to be compared against. Every git connector reports it.
-- min() rather than any() so a repository that somehow claims two default
-- branches resolves the same way on every read.
repository_default_branches AS (
    SELECT
        tenant_id,
        source_id,
        project_key,
        repo_slug,
        min(branch_name) AS branch_name
    FROM {{ ref('class_git_repository_branches') }} FINAL
    WHERE is_default = 1
    GROUP BY tenant_id, source_id, project_key, repo_slug
),
{{ git_evidence_authored_commit_ctes() }},
-- uniqExact, not count(): the link table is append-only per sync, so the same
-- link row can arrive more than once and the count must not inflate.
pr_commit_counts AS (
    SELECT
        tenant_id,
        source_id,
        project_key,
        repo_slug,
        pr_id,
        uniqExact(commit_hash) AS linked_commit_count
    FROM {{ ref('class_git_pull_requests_commits') }} FINAL
    GROUP BY tenant_id, source_id, project_key, repo_slug, pr_id
),
pull_request_review_summary AS (
    SELECT
        tenant_id,
        source_id,
        project_key,
        repo_slug,
        pr_id,
        uniqExactIf(
            reviewer_uuid,
            reviewer_uuid != '' AND (reviewed_at IS NOT NULL OR approved = 1)
        ) AS reviewer_count,
        max(approved) AS has_approval,
        minIfOrNull(reviewed_at, reviewed_at IS NOT NULL) AS first_reviewed_at,
        maxIfOrNull(reviewed_at, approved = 1 AND reviewed_at IS NOT NULL) AS last_approved_at
    FROM {{ ref('class_git_pull_requests_reviewers') }} FINAL
    GROUP BY tenant_id, source_id, project_key, repo_slug, pr_id
),
pull_requests_source AS (
    SELECT
        prs.tenant_id AS tenant_id,
        prs.source_id AS source_id,
        prs.pr_id AS pr_id,
        prs.pr_number AS pr_number,
        prs.title AS title,
        prs.author_name AS author_name,
        -- INVARIANT: only what the source says about the AUTHOR may land here.
        -- The author of a request and the author of a commit it carries are
        -- different identities, so an address read off the commits would credit
        -- the request to whoever wrote them — and a request the source names no
        -- author for is one this metric has no answer for, not one to guess at.
        if(
            trimBoth(prs.author_email) != '',
            lower(trimBoth(prs.author_email)),
            CAST(NULL AS Nullable(String))
        ) AS entity_id,
        -- identity's source_type vocabulary, not data_source's: the binding
        -- rows say 'bitbucket', the class rows say 'insight_bitbucket_cloud'.
        -- '' for a connector with no identity inputs keeps the account join
        -- unmatched and resolution on the email path.
        multiIf(
            prs.data_source = 'insight_github', 'github',
            prs.data_source = 'insight_bitbucket_cloud', 'bitbucket',
            prs.data_source = 'insight_gitlab', 'gitlab',
            ''
        ) AS account_source_type,
        prs.source_id AS account_source_id,
        prs.author_account_id AS account_id,
        prs.state AS state,
        prs.created_on AS created_on,
        prs.closed_on AS closed_on,
        -- INVARIANT: the close time the SOURCE stated, which is not always the
        -- one the class row settles on — Bitbucket recovers a merge the vendor
        -- recorded without an activity entry. A count wants the settled day; a
        -- duration wants an instant somebody observed, so every interval below
        -- reads this column and drops the request where it is null. #3362
        prs.closed_on_reported AS closed_on_reported,
        coalesce(pr_commit_counts.linked_commit_count, 0) AS linked_commit_count,
        coalesce(review_summary.reviewer_count, 0) AS reviewer_count,
        coalesce(review_summary.has_approval, 0) AS has_approval,
        review_summary.first_reviewed_at AS first_reviewed_at,
        review_summary.last_approved_at AS last_approved_at,
        prs.lines_added + prs.lines_removed AS change_size,
        if(
            prs.state = 'MERGED'
                AND prs.closed_on_reported IS NOT NULL
                AND prs.created_on IS NOT NULL
                AND prs.closed_on_reported >= prs.created_on,
            dateDiff('second', prs.created_on, prs.closed_on_reported) / 3600.0,
            CAST(NULL AS Nullable(Float64))
        ) AS cycle_hours,
        if(
            prs.created_on IS NOT NULL
                AND review_summary.first_reviewed_at IS NOT NULL
                AND review_summary.first_reviewed_at >= prs.created_on,
            dateDiff('second', prs.created_on, review_summary.first_reviewed_at) / 3600.0,
            CAST(NULL AS Nullable(Float64))
        ) AS first_review_hours,
        if(
            prs.state = 'MERGED'
                AND prs.closed_on_reported IS NOT NULL
                AND review_summary.first_reviewed_at IS NOT NULL
                AND prs.closed_on_reported >= review_summary.first_reviewed_at,
            dateDiff('second', review_summary.first_reviewed_at, prs.closed_on_reported) / 3600.0,
            CAST(NULL AS Nullable(Float64))
        ) AS review_to_merge_hours,
        if(
            prs.state = 'MERGED'
                AND prs.closed_on_reported IS NOT NULL
                AND review_summary.last_approved_at IS NOT NULL
                AND prs.closed_on_reported >= review_summary.last_approved_at,
            dateDiff('second', review_summary.last_approved_at, prs.closed_on_reported) / 3600.0,
            CAST(NULL AS Nullable(Float64))
        ) AS approval_to_merge_hours,
        if(coalesce(prs.project_key, '') = '', '__unknown__', concat(coalesce(toString(prs.source_id), ''), ':', prs.project_key)) AS project_value,
        if(coalesce(prs.project_key, '') = '', 'Unknown', prs.project_key) AS project_label,
        concat(coalesce(toString(prs.source_id), ''), ':', coalesce(prs.project_key, ''), '/', coalesce(prs.repo_slug, '')) AS repository_value,
        if(coalesce(prs.project_key, '') = '', coalesce(prs.repo_slug, ''), concat(prs.project_key, '/', prs.repo_slug)) AS repository_label,
        if(prs.destination_branch = '', '__unknown__', prs.destination_branch) AS destination_branch_value,
        if(prs.destination_branch = '', 'Unknown', prs.destination_branch) AS destination_branch_label,
        -- A request targets the default branch or it does not. An unreported
        -- destination, and a repository whose default branch is unknown, both
        -- read `non_default` — the agreed reading for an absent signal, which
        -- keeps default + non_default = total.
        if(
            prs.destination_branch != ''
                AND prs.destination_branch = coalesce(defaults.branch_name, ''),
            'default',
            'non_default'
        ) AS branch_scope_value,
        {{ git_branch_scope_label('branch_scope_value') }} AS branch_scope_label,
        replaceOne(prs.data_source, 'insight_', '') AS source_value,
        {{ git_source_label('source_value') }} AS source_label,
        CAST(
            [
                tuple('branch_scope', branch_scope_value, branch_scope_label),
                tuple('destination_branch', destination_branch_value, destination_branch_label),
                tuple('repository', repository_value, repository_label),
                tuple('project', project_value, project_label),
                tuple('source_id', coalesce(toString(source_id), ''), coalesce(toString(source_id), '')),
                tuple('source', source_value, source_label)
            ]
            AS Array(Tuple(key String, value String, label Nullable(String)))
        ) AS source_dimensions
    FROM {{ ref('class_git_pull_requests') }} AS prs FINAL
    LEFT JOIN pr_commit_counts
        ON pr_commit_counts.tenant_id = prs.tenant_id
        AND pr_commit_counts.source_id = prs.source_id
        AND pr_commit_counts.project_key = prs.project_key
        AND pr_commit_counts.repo_slug = prs.repo_slug
        AND pr_commit_counts.pr_id = prs.pr_id
    LEFT JOIN pull_request_review_summary AS review_summary
        ON review_summary.tenant_id = prs.tenant_id
        AND review_summary.source_id = prs.source_id
        AND review_summary.project_key = prs.project_key
        AND review_summary.repo_slug = prs.repo_slug
        AND review_summary.pr_id = prs.pr_id
    LEFT JOIN repository_default_branches AS defaults
        ON defaults.tenant_id = prs.tenant_id
        AND defaults.source_id = prs.source_id
        AND defaults.project_key = prs.project_key
        AND defaults.repo_slug = prs.repo_slug
),
-- The reviewer/commenter perspective: one row per review verdict or comment,
-- attributed to the ACTOR, not the pull request author. The request is joined
-- for display fields, its destination branch, and the comment_target split;
-- an event whose request the source has not reported still counts.
pull_request_measures AS (
    SELECT
        tenant_id,
        source_id,
        pr_id,
        pr_number,
        title,
        author_name,
        -- coalesce, not assumeNotNull: an account-only pull request (author
        -- with no resolvable email anywhere) legitimately carries NULL here
        -- and resolves through the account join instead.
        coalesce(entity_id, '') AS entity_id,
        account_source_type,
        account_source_id,
        account_id,
        toDate(pr_measure.3) AS metric_date,
        pr_measure.3 AS observed_at,
        pr_measure.1 AS measure_key,
        pr_measure.2 AS contribution,
        repository_label,
        repository_value,
        branch_scope_label,
        destination_branch_label,
        source_dimensions
    FROM pull_requests_source AS pull_request
    ARRAY JOIN CAST(arrayConcat(
        if(
            created_on IS NOT NULL,
            [tuple('pr_created', toFloat64(1), toDateTime64(assumeNotNull(created_on), 3))],
            []
        ),
        -- The scope picks the key, so exactly one of the pair receives the
        -- request and default + non_default = total holds by construction
        -- rather than by a later reconciliation.
        if(
            created_on IS NOT NULL,
            [tuple(
                if(branch_scope_value = 'default', 'default_pr_created', 'non_default_pr_created'),
                toFloat64(1),
                toDateTime64(assumeNotNull(created_on), 3)
            )],
            []
        ),
        if(
            created_on IS NOT NULL,
            [tuple(
                'pr_created_merged',
                toFloat64(state = 'MERGED'),
                toDateTime64(assumeNotNull(created_on), 3)
            )],
            []
        ),
        if(
            created_on IS NOT NULL AND branch_scope_value = 'default',
            [tuple(
                'default_pr_created_merged',
                toFloat64(state = 'MERGED'),
                toDateTime64(assumeNotNull(created_on), 3)
            )],
            []
        ),
        if(
            created_on IS NOT NULL,
            [tuple('pr_abandoned', toFloat64(closed_on IS NOT NULL AND state != 'MERGED'), toDateTime64(assumeNotNull(created_on), 3))],
            []
        ),
        if(
            created_on IS NOT NULL,
            [tuple('pr_reviewed', toFloat64(reviewer_count > 0), toDateTime64(assumeNotNull(created_on), 3))],
            []
        ),
        if(
            created_on IS NOT NULL,
            [tuple('pr_reviewer_count', toFloat64(reviewer_count), toDateTime64(assumeNotNull(created_on), 3))],
            []
        ),
        if(
            created_on IS NOT NULL,
            [tuple('pr_multi_reviewed', toFloat64(reviewer_count > 1), toDateTime64(assumeNotNull(created_on), 3))],
            []
        ),
        -- A source that never reported line counts leaves them null and
        -- contributes nothing; a source that reported zero of each observed a
        -- diff of no lines — a rename or a mode change — and that zero is a
        -- value. Only the null separates the two, which is why the class
        -- columns are nullable. #3362
        if(
            created_on IS NOT NULL AND change_size IS NOT NULL,
            [tuple('pr_change_size', toFloat64(assumeNotNull(change_size)), toDateTime64(assumeNotNull(created_on), 3))],
            []
        ),
        if(
            state = 'MERGED' AND closed_on IS NOT NULL,
            [tuple('pr_merged', toFloat64(1), toDateTime64(assumeNotNull(closed_on), 3))],
            []
        ),
        if(
            state = 'MERGED' AND closed_on IS NOT NULL,
            [tuple('pr_merged_without_approval', toFloat64(has_approval = 0), toDateTime64(assumeNotNull(closed_on), 3))],
            []
        ),
        if(
            state = 'MERGED' AND closed_on IS NOT NULL,
            [tuple(
                if(branch_scope_value = 'default', 'default_pr_merged', 'non_default_pr_merged'),
                toFloat64(1),
                toDateTime64(assumeNotNull(closed_on), 3)
            )],
            []
        ),
        -- No linked commit rows means the source did not report the request's
        -- commits, not that it merged empty — such a request contributes no
        -- value rather than a zero.
        if(
            state = 'MERGED' AND closed_on IS NOT NULL AND linked_commit_count > 0,
            [tuple('pr_commit_count', toFloat64(linked_commit_count), toDateTime64(assumeNotNull(closed_on), 3))],
            []
        ),
        if(
            cycle_hours IS NOT NULL AND closed_on_reported IS NOT NULL,
            [tuple('pr_cycle_hours', toFloat64(assumeNotNull(cycle_hours)), toDateTime64(assumeNotNull(closed_on_reported), 3))],
            []
        ),
        if(
            first_review_hours IS NOT NULL AND first_reviewed_at IS NOT NULL,
            [tuple('pr_first_review_hours', toFloat64(assumeNotNull(first_review_hours)), toDateTime64(assumeNotNull(first_reviewed_at), 3))],
            []
        ),
        if(
            review_to_merge_hours IS NOT NULL AND closed_on_reported IS NOT NULL,
            [tuple('pr_review_to_merge_hours', toFloat64(assumeNotNull(review_to_merge_hours)), toDateTime64(assumeNotNull(closed_on_reported), 3))],
            []
        ),
        if(
            approval_to_merge_hours IS NOT NULL AND closed_on_reported IS NOT NULL,
            [tuple('pr_approval_to_merge_hours', toFloat64(assumeNotNull(approval_to_merge_hours)), toDateTime64(assumeNotNull(closed_on_reported), 3))],
            []
        ),
        if(
            first_review_hours IS NOT NULL
                AND review_to_merge_hours IS NOT NULL
                AND cycle_hours IS NOT NULL
                AND cycle_hours > 0
                AND closed_on_reported IS NOT NULL,
            [tuple('pr_review_wait_share', 100.0 * toFloat64(assumeNotNull(first_review_hours)) / toFloat64(assumeNotNull(cycle_hours)), toDateTime64(assumeNotNull(closed_on_reported), 3))],
            []
        )
    ) AS Array(Tuple(measure_key String, contribution Float64, observed_at DateTime64(3)))) AS pr_measure
    -- A row survives on EITHER key the source states about the author: the
    -- address or the account id, which the outer join resolves account-first.
    -- A request the source names neither for drops here — unattributable is
    -- the answer, and a request that survives on an account nothing has bound
    -- stays unresolved at read time rather than reaching some other person.
    WHERE (pull_request.entity_id IS NOT NULL AND pull_request.entity_id != '')
       OR pull_request.account_id != ''
)
SELECT
    tenant_id,
    source_key,
    entity_type,
    entity_id,
    account_source_type,
    account_source_id,
    account_id,
    metric_date,
    observed_at,
    measure_key,
    record_id,
    record_kind,
    granularity,
    record_label,
    contribution,
    subject_key,
    dimensions,
    details
FROM {{ ref('git_commit_line_contributions') }}

UNION ALL

SELECT
    assumeNotNull(tenant_id) AS tenant_id,
    'git' AS source_key,
    'person' AS entity_type,
    assumeNotNull(entity_id) AS entity_id,
    '' AS account_source_type,
    '' AS account_source_id,
    '' AS account_id,
    assumeNotNull(metric_date) AS metric_date,
    toNullable(toDateTime64(observed_at, 3)) AS observed_at,
    commit_measure.1 AS measure_key,
    concat(source_value, ':', commit_hash, ':', commit_measure.1) AS record_id,
    'commit' AS record_kind,
    'event' AS granularity,
    if(message = '', commit_hash, message) AS record_label,
    toNullable(toFloat64(commit_measure.2)) AS contribution,
    if(commit_measure.3, toNullable(toString(metric_date)), CAST(NULL AS Nullable(String))) AS subject_key,
    -- Day rows keep the bare source dimensions: an hour block there splits
    -- one active day into one observation row per block.
    --
    -- INVARIANT: the appended element is CAST to the array's own named tuple
    -- type. `arrayConcat` with an anonymous or wider-nullability tuple widens
    -- the whole column to `Tuple(String, Nullable(String), Nullable(String))`,
    -- which changes this table's DDL and every relation that unions it.
    if(
        commit_measure.3,
        source_dimensions,
        arrayConcat(
            source_dimensions,
            CAST(
                [tuple(
                    'hour_block',
                    coalesce({{ hour_block_value('observed_at') }}, '__unknown__'),
                    toNullable(coalesce({{ hour_block_label('observed_at') }}, 'Unknown'))
                )]
                AS Array(Tuple(key String, value String, label Nullable(String)))
            )
        )
    ) AS dimensions,
    map(
        'source_id', coalesce(toString(source_id), ''),
        'ref', commit_hash,
        'title', message,
        'repository', repository_label,
        'author', author_name,
        'branch_scope', branch_scope_label,
        'lines_added', coalesce(toString(lines_added), ''),
        'lines_removed', coalesce(toString(lines_removed), '')
    ) AS details
FROM authored_commits
ARRAY JOIN arrayConcat(
    [tuple('commit_count', toFloat64(1), false)],
    [tuple(
        if(branch_scope_value = 'default', 'default_commit_count', 'non_default_commit_count'),
        toFloat64(1),
        false
    )],
    if(
        lines_added IS NOT NULL AND lines_removed IS NOT NULL,
        [tuple('commit_change_size', toFloat64(lines_added + lines_removed), false)],
        []
    ),
    [tuple('commit_day', toFloat64(1), true)],
    if(branch_scope_value = 'default', [tuple('default_commit_day', toFloat64(1), true)], [])
) AS commit_measure
WHERE tenant_id IS NOT NULL
  AND entity_id IS NOT NULL
  AND metric_date IS NOT NULL

UNION ALL

SELECT
    assumeNotNull(tenant_id) AS tenant_id,
    'git' AS source_key,
    'person' AS entity_type,
    assumeNotNull(entity_id) AS entity_id,
    account_source_type,
    account_source_id,
    account_id,
    assumeNotNull(metric_date) AS metric_date,
    toNullable(toDateTime64(observed_at, 3)) AS observed_at,
    measure_key,
    concat(repository_value, ':pr:', toString(pr_id), ':', measure_key) AS record_id,
    'pull_request' AS record_kind,
    'event' AS granularity,
    if(title = '', concat('PR #', toString(pr_number)), title) AS record_label,
    toNullable(toFloat64(contribution)) AS contribution,
    CAST(NULL AS Nullable(String)) AS subject_key,
    source_dimensions AS dimensions,
    map(
        'source_id', coalesce(toString(source_id), ''),
        'ref', toString(pr_number),
        'title', title,
        'repository', repository_label,
        'author', author_name,
        'branch_scope', branch_scope_label,
        'destination_branch', destination_branch_label
    ) AS details
FROM pull_request_measures
WHERE tenant_id IS NOT NULL
  AND entity_id IS NOT NULL
  AND metric_date IS NOT NULL

UNION ALL

SELECT
    assumeNotNull(tenant_id) AS tenant_id,
    'git' AS source_key,
    'person' AS entity_type,
    assumeNotNull(entity_id) AS entity_id,
    -- Email-only resolution, like every branch except pull requests: the
    -- account path is the PR rows' own rule, and `comment_target` compares
    -- actor to author through the email map, so resolving the entity another
    -- way could classify a comment against a person the row does not name.
    -- The actor's account id is carried in silver for a later, deliberate
    -- widening of that rule.
    '' AS account_source_type,
    '' AS account_source_id,
    '' AS account_id,
    assumeNotNull(metric_date) AS metric_date,
    toNullable(toDateTime64(observed_at, 3)) AS observed_at,
    if(event_kind = 'review', 'review_submitted', 'pr_comment') AS measure_key,
    -- The silver event key disambiguates several events by one person on one
    -- request; per-PR keying would collapse them into one record.
    concat(repository_value, ':pr:', toString(pr_id), ':', measure_key, ':', coalesce(event_key, '')) AS record_id,
    'pull_request' AS record_kind,
    'event' AS granularity,
    if(title = '', concat('PR #', toString(pr_number)), title) AS record_label,
    toNullable(toFloat64(1)) AS contribution,
    CAST(NULL AS Nullable(String)) AS subject_key,
    source_dimensions AS dimensions,
    map(
        'source_id', coalesce(toString(source_id), ''),
        'ref', toString(pr_number),
        'title', title,
        'repository', repository_label,
        'author', author_name,
        'destination_branch', destination_branch_label
    ) AS details
FROM {{ ref('git_review_events') }}
WHERE tenant_id IS NOT NULL
  AND metric_date IS NOT NULL
) AS src

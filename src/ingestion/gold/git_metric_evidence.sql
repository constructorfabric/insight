{{ metric_evidence_table(join_use_nulls=1) }}

-- Resolution happens HERE, once per gold build: evidence carries BOTH keys —
-- `entity_id` is the canonical person id (or '' when identity cannot place
-- the row: those rows stay for coverage but reach no serving relation), and
-- `source_entity_id` keeps the source-native email for provenance. Rows that
-- carry the author's source account id (pull requests) resolve through the
-- account binding first and fall back to the email map; everything else is
-- email-only. Everything downstream (observations, cohorts, coverage,
-- drilldown) reads THIS snapshot, so one identity mapping answers for the
-- whole build.
SELECT
    src.tenant_id,
    src.source_key,
    src.entity_type,
    -- Null-proof under EITHER join_use_nulls setting (models differ): the
    -- conditions are non-Nullable via coalesce, and person_id is read only on
    -- the matched branch, so entity_id is a plain String fit for the sort key.
    -- Account first: an account binding is the source's own answer to "whose
    -- row is this" and survives an empty profile email; the email map decides
    -- only when the row carries no bound account. A matched account bound to
    -- the excluded person terminates resolution — the row attributes to
    -- nobody even when its emails would resolve, or a bot pull request whose
    -- commits carry a human's email would attribute to that human.
    multiIf(
        coalesce(account_map.account_id, '') != '',
        if(
            assumeNotNull(account_map.person_id) = {{ excluded_person_id() }},
            '',
            toString(assumeNotNull(account_map.person_id))
        ),
        coalesce(identity_map.email, '') != '',
        toString(assumeNotNull(identity_map.person_id)),
        ''
    ) AS entity_id,
    src.entity_id AS source_entity_id,
    src.metric_date,
    src.observed_at,
    src.measure_key,
    -- Account-qualified: several source-day record_ids (date:measure:dims
    -- hash) are identical across one person's accounts once entity_id is
    -- canonical, and both the evidence uniqueness grain and the drilldown
    -- cursor need one row per record key. Hashed, not the raw email — the id
    -- reaches the client and stays opaque. The account id joins the salt so
    -- two email-less accounts cannot collide on one record key.
    concat(src.record_id, ':', hex(sipHash64(concat(src.entity_id, ':', src.account_id)))) AS record_id,
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
-- One row per change CONTENT, not per commit that carries it. The same content
-- entering a repository on two lines of history — a branch whose copy of a
-- tree also landed on the default branch, a cherry-pick, a
-- reverted-then-restored file — is one authored change with one oid pair, and
-- summing both commits' diffs would count those lines twice.
--
-- Earliest commit wins, so the value lands in the period the content was first
-- authored and does not move when a later commit repeats it.
--
-- The commit_hash tie-breaker keeps rows whose identity is UNKNOWN (a source
-- that reports no oid, or a row collected before the proxy did) distinct per
-- commit: without it every such row for one path would collapse into one,
-- because LIMIT 1 BY reads their NULL keys as equal.
deduplicated_file_changes AS (
    SELECT
        tenant_id,
        source_id,
        project_key,
        repo_slug,
        commit_hash,
        data_source,
        file_path,
        file_extension,
        change_type,
        lines_added,
        lines_removed
    FROM {{ ref('git_commit_file_changes') }}
    ORDER BY observed_at, commit_hash
    LIMIT 1 BY
        tenant_id,
        data_source,
        project_key,
        repo_slug,
        file_path,
        lower(change_type),
        pre_image_oid,
        post_image_oid,
        if(
            coalesce(pre_image_oid, '') = ''
                AND coalesce(post_image_oid, '') = '',
            commit_hash,
            ''
        )
),
-- A commit's own line stats, less the lines of the file changes that lost the
-- content dedup. The stats stay the base — a source can report a commit's
-- totals without reporting its file changes at all — and only what the dedup
-- removed is taken back out, so a commit that introduces nothing new reports a
-- size of zero and its drilldown detail agrees with what it contributed.
reported_commit_file_lines AS (
    SELECT
        tenant_id,
        source_id,
        project_key,
        repo_slug,
        commit_hash,
        sum(lines_added) AS lines_added,
        sum(lines_removed) AS lines_removed
    FROM {{ ref('git_commit_file_changes') }}
    GROUP BY tenant_id, source_id, project_key, repo_slug, commit_hash
),
authored_commit_file_lines AS (
    SELECT
        tenant_id,
        source_id,
        project_key,
        repo_slug,
        commit_hash,
        sum(lines_added) AS lines_added,
        sum(lines_removed) AS lines_removed
    FROM deduplicated_file_changes
    GROUP BY tenant_id, source_id, project_key, repo_slug, commit_hash
),
authored_commits AS (
    SELECT
        commits.tenant_id AS tenant_id,
        commits.entity_id AS entity_id,
        commits.metric_date AS metric_date,
        commits.observed_at AS observed_at,
        commits.commit_hash AS commit_hash,
        commits.message AS message,
        commits.author_name AS author_name,
        commits.repository_label AS repository_label,
        commits.source_value AS source_value,
        commits.branch_scope_value AS branch_scope_value,
        commits.source_dimensions AS source_dimensions,
        -- SAFETY: the NULL check is explicit because `greatest` IGNORES NULL
        -- arguments — `greatest(0, NULL)` is 0, which would invent a size for a
        -- commit whose source reported no line stats. `greatest` floors the
        -- result because a commit's own stats and the sum of its file changes
        -- need not agree (binary files, truncated diffs).
        if(
            commits.lines_added IS NULL,
            CAST(NULL AS Nullable(Int64)),
            toNullable(greatest(
                toInt64(0),
                assumeNotNull(commits.lines_added)
                    - (coalesce(reported.lines_added, 0) - coalesce(authored.lines_added, 0))
            ))
        ) AS lines_added,
        if(
            commits.lines_removed IS NULL,
            CAST(NULL AS Nullable(Int64)),
            toNullable(greatest(
                toInt64(0),
                assumeNotNull(commits.lines_removed)
                    - (coalesce(reported.lines_removed, 0) - coalesce(authored.lines_removed, 0))
            ))
        ) AS lines_removed
    FROM {{ ref('git_authored_commits') }} AS commits
    LEFT JOIN reported_commit_file_lines AS reported
        ON reported.tenant_id = commits.tenant_id
        AND reported.source_id = commits.source_id
        AND reported.project_key = commits.project_key
        AND reported.repo_slug = commits.repo_slug
        AND reported.commit_hash = commits.commit_hash
    LEFT JOIN authored_commit_file_lines AS authored
        ON authored.tenant_id = commits.tenant_id
        AND authored.source_id = commits.source_id
        AND authored.project_key = commits.project_key
        AND authored.repo_slug = commits.repo_slug
        AND authored.commit_hash = commits.commit_hash
),
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
                lower(raw_file_change.change_type) = 'deleted', 'Deleted',
                raw_file_change.change_type
            ) AS change_type_label,
            sum(lines_added) AS lines_added,
            sum(lines_removed) AS lines_removed
        FROM deduplicated_file_changes AS raw_file_change
        GROUP BY tenant_id, source_id, project_key, repo_slug, commit_hash, category, file_extension_value, file_extension_label, change_type_value, change_type_label
    ) AS file_changes
    INNER JOIN {{ ref('git_authored_commits') }} AS commits
        ON commits.tenant_id = file_changes.tenant_id
        AND commits.source_id = file_changes.source_id
        AND commits.project_key = file_changes.project_key
        AND commits.repo_slug = file_changes.repo_slug
        AND commits.commit_hash = file_changes.commit_hash
),
pr_commit_emails AS (
    SELECT
        tenant_id,
        source_id,
        project_key,
        repo_slug,
        pr_id,
        if(uniqExact(email) = 1, any(email), CAST(NULL AS Nullable(String))) AS email
    FROM (
        SELECT
            links.tenant_id AS tenant_id,
            links.source_id AS source_id,
            links.project_key AS project_key,
            links.repo_slug AS repo_slug,
            links.pr_id AS pr_id,
            lower(trimBoth(commits.author_email)) AS email,
            uniqExact(commits.commit_hash) AS email_count,
            max(uniqExact(commits.commit_hash)) OVER (
                PARTITION BY links.tenant_id, links.source_id,
                             links.project_key, links.repo_slug, links.pr_id
            ) AS max_count
        FROM {{ ref('class_git_pull_requests_commits') }} AS links
        INNER JOIN {{ ref('class_git_commits') }} AS commits
            ON commits.tenant_id = links.tenant_id
            AND commits.source_id = links.source_id
            AND commits.project_key = links.project_key
            AND commits.repo_slug = links.repo_slug
            AND commits.commit_hash = links.commit_hash
        WHERE trimBoth(commits.author_email) != ''
          AND commits.is_merge_commit = 0
        GROUP BY tenant_id, source_id, project_key, repo_slug, pr_id, email
    )
    WHERE email_count = max_count
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
        multiIf(
            trimBoth(prs.author_email) != '', lower(trimBoth(prs.author_email)),
            pr_commit_emails.email IS NOT NULL AND pr_commit_emails.email != '', pr_commit_emails.email,
            CAST(NULL AS Nullable(String))
        ) AS entity_id,
        -- identity's source_type vocabulary, not data_source's: the binding
        -- rows say 'bitbucket', the class rows say 'insight_bitbucket_cloud'.
        -- '' (gitlab — no identity inputs exist) keeps the account join
        -- unmatched and resolution on the email path.
        multiIf(
            prs.data_source = 'insight_github', 'github',
            prs.data_source = 'insight_bitbucket_cloud', 'bitbucket',
            ''
        ) AS account_source_type,
        prs.source_id AS account_source_id,
        prs.author_account_id AS account_id,
        prs.state AS state,
        prs.created_on AS created_on,
        prs.closed_on AS closed_on,
        coalesce(review_summary.reviewer_count, 0) AS reviewer_count,
        coalesce(review_summary.has_approval, 0) AS has_approval,
        review_summary.first_reviewed_at AS first_reviewed_at,
        review_summary.last_approved_at AS last_approved_at,
        prs.lines_added + prs.lines_removed AS change_size,
        if(
            prs.state = 'MERGED'
                AND prs.closed_on IS NOT NULL
                AND prs.created_on IS NOT NULL
                AND prs.closed_on >= prs.created_on,
            dateDiff('second', prs.created_on, prs.closed_on) / 3600.0,
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
                AND prs.closed_on IS NOT NULL
                AND review_summary.first_reviewed_at IS NOT NULL
                AND prs.closed_on >= review_summary.first_reviewed_at,
            dateDiff('second', review_summary.first_reviewed_at, prs.closed_on) / 3600.0,
            CAST(NULL AS Nullable(Float64))
        ) AS review_to_merge_hours,
        if(
            prs.state = 'MERGED'
                AND prs.closed_on IS NOT NULL
                AND review_summary.last_approved_at IS NOT NULL
                AND prs.closed_on >= review_summary.last_approved_at,
            dateDiff('second', review_summary.last_approved_at, prs.closed_on) / 3600.0,
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
                tuple('source', source_value, source_label)
            ]
            AS Array(Tuple(key String, value String, label Nullable(String)))
        ) AS source_dimensions
    FROM {{ ref('class_git_pull_requests') }} AS prs FINAL
    LEFT JOIN pr_commit_emails
        ON pr_commit_emails.tenant_id = prs.tenant_id
        AND pr_commit_emails.source_id = prs.source_id
        AND pr_commit_emails.project_key = prs.project_key
        AND pr_commit_emails.repo_slug = prs.repo_slug
        AND pr_commit_emails.pr_id = prs.pr_id
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
pull_request_measures AS (
    SELECT
        tenant_id,
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
        if(
            created_on IS NOT NULL AND ifNull(change_size, 0) > 0,
            [tuple('pr_change_size', toFloat64(ifNull(change_size, 0)), toDateTime64(assumeNotNull(created_on), 3))],
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
        if(
            cycle_hours IS NOT NULL AND closed_on IS NOT NULL,
            [tuple('pr_cycle_hours', toFloat64(assumeNotNull(cycle_hours)), toDateTime64(assumeNotNull(closed_on), 3))],
            []
        ),
        if(
            first_review_hours IS NOT NULL AND first_reviewed_at IS NOT NULL,
            [tuple('pr_first_review_hours', toFloat64(assumeNotNull(first_review_hours)), toDateTime64(assumeNotNull(first_reviewed_at), 3))],
            []
        ),
        if(
            review_to_merge_hours IS NOT NULL AND closed_on IS NOT NULL,
            [tuple('pr_review_to_merge_hours', toFloat64(assumeNotNull(review_to_merge_hours)), toDateTime64(assumeNotNull(closed_on), 3))],
            []
        ),
        if(
            approval_to_merge_hours IS NOT NULL AND closed_on IS NOT NULL,
            [tuple('pr_approval_to_merge_hours', toFloat64(assumeNotNull(approval_to_merge_hours)), toDateTime64(assumeNotNull(closed_on), 3))],
            []
        ),
        if(
            first_review_hours IS NOT NULL
                AND review_to_merge_hours IS NOT NULL
                AND cycle_hours IS NOT NULL
                AND cycle_hours > 0
                AND closed_on IS NOT NULL,
            [tuple('pr_review_wait_share', 100.0 * toFloat64(assumeNotNull(first_review_hours)) / toFloat64(assumeNotNull(cycle_hours)), toDateTime64(assumeNotNull(closed_on), 3))],
            []
        )
    ) AS Array(Tuple(measure_key String, contribution Float64, observed_at DateTime64(3)))) AS pr_measure
    -- A row survives on EITHER key: the email (today's path) or the account
    -- id, which the outer join resolves account-first. Only a pull request
    -- with neither — no profile email, no attributable commit email, no
    -- account id — drops here, exactly as before.
    WHERE (pull_request.entity_id IS NOT NULL AND pull_request.entity_id != '')
       OR pull_request.account_id != ''
),
file_change_measures AS (
    SELECT
        tenant_id,
        entity_id,
        metric_date,
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
measure_observations AS (
    SELECT
        tenant_id,
        entity_id,
        metric_date,
        measure_key,
        toNullable(sum(value)) AS value,
        dimensions
    FROM file_change_measures
    GROUP BY tenant_id, entity_id, metric_date, measure_key, dimensions
)
SELECT
    assumeNotNull(tenant_id) AS tenant_id,
    'git' AS source_key,
    'person' AS entity_type,
    assumeNotNull(entity_id) AS entity_id,
    '' AS account_source_type,
    '' AS account_source_id,
    '' AS account_id,
    assumeNotNull(metric_date) AS metric_date,
    CAST(NULL AS Nullable(DateTime64(3))) AS observed_at,
    measure_key,
    concat(
        toString(metric_date),
        ':',
        measure_key,
        ':',
        hex(sipHash128(toString(arrayMap(d -> tuple(d.1, d.2), dimensions))))
    ) AS record_id,
    measure_key AS record_kind,
    'source_summary' AS granularity,
    replaceAll(measure_key, '_', ' ') AS record_label,
    value AS contribution,
    CAST(NULL AS Nullable(String)) AS subject_key,
    dimensions,
    CAST(map() AS Map(String, String)) AS details
FROM measure_observations
WHERE tenant_id IS NOT NULL
  AND entity_id IS NOT NULL
  AND metric_date IS NOT NULL

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
    CAST(NULL AS Nullable(DateTime64(3))) AS observed_at,
    'commit_day' AS measure_key,
    concat(
        toString(metric_date),
        ':commit_day:',
        hex(sipHash128(toString(arrayMap(d -> tuple(d.1, d.2), source_dimensions))))
    ) AS record_id,
    'commit_day' AS record_kind,
    'derived_population' AS granularity,
    'commit day' AS record_label,
    toNullable(toFloat64(1)) AS contribution,
    toNullable(toString(metric_date)) AS subject_key,
    source_dimensions AS dimensions,
    CAST(map() AS Map(String, String)) AS details
FROM {{ ref('git_authored_commits') }}
WHERE tenant_id IS NOT NULL
  AND entity_id IS NOT NULL
  AND metric_date IS NOT NULL
GROUP BY tenant_id, entity_id, metric_date, source_dimensions

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
    CAST(NULL AS Nullable(String)) AS subject_key,
    source_dimensions AS dimensions,
    map(
        'ref', commit_hash,
        'title', message,
        'repository', repository_label,
        'author', author_name,
        'lines_added', coalesce(toString(lines_added), ''),
        'lines_removed', coalesce(toString(lines_removed), '')
    ) AS details
FROM authored_commits
ARRAY JOIN arrayConcat(
    [tuple('commit_count', toFloat64(1))],
    [tuple(
        if(branch_scope_value = 'default', 'default_commit_count', 'non_default_commit_count'),
        toFloat64(1)
    )],
    if(
        lines_added IS NOT NULL AND lines_removed IS NOT NULL,
        [tuple('commit_change_size', toFloat64(lines_added + lines_removed))],
        []
    )
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
        'ref', toString(pr_number),
        'title', title,
        'repository', repository_label,
        'author', author_name
    ) AS details
FROM pull_request_measures
WHERE tenant_id IS NOT NULL
  AND entity_id IS NOT NULL
  AND metric_date IS NOT NULL
) AS src
{{ resolved_person_id_join('src') }}
{{ resolved_person_id_by_account_join('src') }}

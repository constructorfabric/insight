{% macro git_evidence_authored_commit_ctes() %}
-- A commit's own line stats, less the lines of the file changes that lost the
-- content dedup. The stats stay the base — a source can report a commit's
-- totals without reporting its file changes at all — and only what the dedup
-- removed is taken back out, so a commit that introduces nothing new reports a
-- size of zero and its drilldown detail agrees with what it contributed. The
-- collected side is git_commit_file_line_totals, a model rather than a CTE so
-- its aggregate runs once per build instead of once per read.
authored_commit_file_lines AS (
    SELECT
        tenant_id,
        data_source,
        commit_hash,
        sum(lines_added) AS lines_added,
        sum(lines_removed) AS lines_removed
    FROM {{ ref('git_deduplicated_file_changes') }}
    GROUP BY tenant_id, data_source, commit_hash
),
authored_commits AS (
    SELECT
        commits.tenant_id AS tenant_id,
        commits.source_id AS source_id,
        commits.entity_id AS entity_id,
        commits.metric_date AS metric_date,
        commits.observed_at AS observed_at,
        commits.commit_hash AS commit_hash,
        commits.message AS message,
        commits.author_name AS author_name,
        commits.repository_value AS repository_value,
        commits.repository_label AS repository_label,
        commits.project_value AS project_value,
        commits.project_label AS project_label,
        commits.source_value AS source_value,
        commits.source_label AS source_label,
        commits.branch_scope_value AS branch_scope_value,
        commits.branch_scope_label AS branch_scope_label,
        commits.source_dimensions AS source_dimensions,
        -- No file change was ever COLLECTED for this commit, so its size
        -- reaches no size measure while the commit itself still counts. The
        -- test is the collected set, not the deduplicated one: a commit whose
        -- changes were collected and then all lost the content dedup already
        -- reports the zero it earned, and must not have its size restored.
        -- Read off the row count, not a line sum — the line columns are
        -- Nullable, so an all-NULL group sums to NULL and would be
        -- indistinguishable from a commit the stream never reached.
        reported.file_change_rows IS NULL AS has_no_file_changes,
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
    -- The canonical commit grain, the one git_authored_commits collapses to and
    -- git_commit_file_changes already attaches on. Null-safe on tenant_id: it
    -- is Nullable on the class, and a plain `=` never matches NULL to NULL —
    -- which would read a whole tenant's commits as having no collected change.
    LEFT JOIN {{ ref('git_commit_file_line_totals') }} AS reported
        ON reported.tenant_id IS NOT DISTINCT FROM commits.tenant_id
        AND reported.data_source = commits.data_source
        AND reported.commit_hash = commits.commit_hash
    LEFT JOIN authored_commit_file_lines AS authored
        ON authored.tenant_id IS NOT DISTINCT FROM commits.tenant_id
        AND authored.data_source = commits.data_source
        AND authored.commit_hash = commits.commit_hash
)
{% endmacro %}

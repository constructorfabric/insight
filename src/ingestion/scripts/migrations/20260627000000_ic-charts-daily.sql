-- ---------------------------------------------------------------------
-- ic_chart_loc / ic_chart_delivery → daily grain
-- ---------------------------------------------------------------------
-- The IC trend charts bucket on the client by the selected range. Emitting
-- the finest (daily) grain here lets the client aggregate to day / week /
-- month / quarter as needed; the chart's date_bucket is one row per
-- (person, day).
DROP VIEW IF EXISTS insight.ic_chart_loc;
CREATE VIEW insight.ic_chart_loc
(
    `person_id`   Nullable(String),
    `org_unit_id` Nullable(String),
    `date_bucket` Nullable(String),
    `metric_date` Nullable(String),
    `code_loc`    Float64,
    `spec_lines`  Float64,
    `config_loc`  Float64
)
AS SELECT
    fc.person_key                                       AS person_id,
    p.org_unit_id                                       AS org_unit_id,
    toString(toDate(fc.committed_at))                   AS date_bucket,
    toString(toDate(fc.committed_at))                   AS metric_date,
    toFloat64(sum(if(fc.file_category = 'code',   fc.lines_added, 0))) AS code_loc,
    toFloat64(sum(if(fc.file_category = 'spec',   fc.lines_added, 0))) AS spec_lines,
    toFloat64(sum(if(fc.file_category = 'config', fc.lines_added, 0))) AS config_loc
FROM silver.fct_git_file_change AS fc FINAL
LEFT JOIN insight.people AS p ON fc.person_key = p.person_id
WHERE fc.is_merge_commit = 0
  AND fc.person_key != ''
  AND fc.committed_at IS NOT NULL
GROUP BY
    fc.person_key,
    p.org_unit_id,
    toDate(fc.committed_at);

-- ---------------------------------------------------------------------
DROP VIEW IF EXISTS insight.ic_chart_delivery;
CREATE VIEW insight.ic_chart_delivery
(
    `person_id`   Nullable(String),
    `org_unit_id` Nullable(String),
    `date_bucket` Nullable(String),
    `metric_date` Nullable(String),
    `commits`     UInt64,
    `prs_merged`  Nullable(UInt64),
    `tasks_done`  UInt64
)
AS
WITH
    daily_commits AS (
        SELECT
            person_key                            AS person_id,
            toDate(date)                          AS day,
            count()                               AS commits
        FROM silver.fct_git_commit FINAL
        WHERE is_merge_commit = 0
          AND person_key != ''
          AND date IS NOT NULL
        GROUP BY person_id, day
    ),
    daily_prs AS (
        SELECT
            pr.person_key                                   AS person_id,
            coalesce(toDate(mc.date), toDate(pr.closed_on)) AS day,
            uniqExact(pr.unique_key)                        AS prs_merged
        FROM silver.fct_git_pr AS pr FINAL
        LEFT JOIN silver.fct_git_commit AS mc FINAL
            ON  mc.tenant_id   = pr.tenant_id
            AND mc.source_id   = pr.source_id
            AND mc.project_key = pr.project_key
            AND mc.repo_slug   = pr.repo_slug
            AND mc.commit_hash = pr.merge_commit_hash
        WHERE pr.state_norm = 'merged'
          AND pr.closed_on IS NOT NULL
          AND pr.person_key != ''
        GROUP BY person_id, day
    ),
    daily_jira AS (
        SELECT
            person_id,
            metric_date                           AS day,
            sum(tasks_closed)                     AS tasks_done
        FROM insight.jira_closed_tasks
        GROUP BY person_id, day
    ),
    days_all AS (
        SELECT person_id, day FROM daily_commits
        UNION DISTINCT
        SELECT person_id, day FROM daily_prs
        UNION DISTINCT
        SELECT person_id, day FROM daily_jira
    )
SELECT
    d.person_id                                   AS person_id,
    p.org_unit_id                                 AS org_unit_id,
    toString(d.day)                               AS date_bucket,
    toString(d.day)                               AS metric_date,
    toUInt64(ifNull(c.commits, 0))                AS commits,
    CAST(toUInt64(ifNull(pr.prs_merged, 0)) AS Nullable(UInt64))
                                                  AS prs_merged,
    toUInt64(ifNull(j.tasks_done, 0))             AS tasks_done
FROM days_all                           AS d
LEFT JOIN insight.people                AS p  ON d.person_id = p.person_id
LEFT JOIN daily_commits                 AS c  ON c.person_id = d.person_id AND c.day = d.day
LEFT JOIN daily_prs                     AS pr ON pr.person_id = d.person_id AND pr.day = d.day
LEFT JOIN daily_jira                    AS j  ON j.person_id = d.person_id AND j.day = d.day;

-- ---------------------------------------------------------------------
-- ic_chart_loc ← silver.mtr_git_person_weekly
-- ---------------------------------------------------------------------
-- Per-person per-week lines-of-code breakdown for the IC LOC chart.
-- The three series are git file-category buckets from
-- silver.mtr_git_person_weekly: code_loc (file_category='code'),
-- spec_lines (file_category='spec'), config_loc (file_category='config').
-- file_category is a mutually-exclusive classification at the
-- fct_git_file_change layer, so the three buckets partition lines added and
-- are each non-negative. Grain matches the source: (person, week).
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
    g.person_key                                  AS person_id,
    p.org_unit_id                                 AS org_unit_id,
    toString(g.week)                              AS date_bucket,
    toString(g.week)                              AS metric_date,
    toFloat64(g.code_loc)                         AS code_loc,
    toFloat64(g.spec_lines)                       AS spec_lines,
    toFloat64(g.config_loc)                       AS config_loc
FROM silver.mtr_git_person_weekly AS g
LEFT JOIN insight.people AS p ON g.person_key = p.person_id
WHERE g.person_key != ''
  AND g.week IS NOT NULL;

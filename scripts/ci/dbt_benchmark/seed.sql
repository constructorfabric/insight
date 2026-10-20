CREATE DATABASE silver;
CREATE DATABASE insight;
CREATE DATABASE benchmark;

CREATE TABLE silver.class_git_commits
(
    unique_key String,
    tenant_id String,
    source_id String,
    project_key String,
    repo_slug String,
    commit_hash String,
    data_source String,
    author_name String,
    author_email String,
    message String,
    date DateTime64(3),
    committer_date DateTime64(3),
    lines_added Int64,
    lines_removed Int64,
    is_merge_commit UInt8,
    is_default_branch UInt8,
    _version UInt64
)
ENGINE = ReplacingMergeTree(_version)
ORDER BY unique_key;

INSERT INTO silver.class_git_commits
SELECT
    toString(number), 'synthetic', 'source', 'project', 'repo',
    toString(number), 'insight_github', 'Example',
    concat(toString(number % 10000), '@example.com'),
    repeat(hex(cityHash64(number)), 16),
    toDateTime64('2025-01-01', 3) + toIntervalSecond(number),
    toDateTime64('2025-01-01', 3) + toIntervalSecond(number),
    1, 0, 0, number % 2, 1
FROM numbers({rows:UInt64});

INSERT INTO silver.class_git_commits
SELECT
    unique_key, tenant_id, source_id, project_key, repo_slug, commit_hash,
    data_source, author_name, author_email, message, date, committer_date,
    2, lines_removed, is_merge_commit, is_default_branch, 2
FROM silver.class_git_commits FINAL
WHERE cityHash64(unique_key) % 3 = 0;

CREATE TABLE insight.git_default_branch_commits
ENGINE = MergeTree ORDER BY (tenant_id, source_id, project_key, repo_slug, commit_hash)
AS SELECT tenant_id, source_id, project_key, repo_slug, commit_hash, data_source
FROM silver.class_git_commits FINAL WHERE is_default_branch = 1;

CREATE TABLE insight.git_derived_commits
ENGINE = MergeTree ORDER BY (tenant_id, data_source, commit_hash)
AS SELECT tenant_id, data_source, commit_hash
FROM silver.class_git_commits FINAL WHERE cityHash64(unique_key) % 19 = 0;

CREATE TABLE benchmark.input
ENGINE = MergeTree ORDER BY id
AS SELECT
    number AS id,
    number % greatest(intDiv({rows:UInt64}, 2), 1) AS group_id,
    repeat(hex(cityHash64(number)), 16) AS payload
FROM numbers({rows:UInt64});

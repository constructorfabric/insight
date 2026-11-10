-- Applied dbt transform versions: append one row per model per application.
-- Service rows stay beside sync_events, outside the exported metric surface.
CREATE TABLE IF NOT EXISTS ingestion_history.transform_versions
(
    model             String,
    relation          String,
    transform_version UInt64,
    release           Nullable(String),
    applied_at        DateTime64(6, 'UTC'),
    applied_by        String
) ENGINE = /*@@REPLICATED@@*/MergeTree
ORDER BY (model, applied_at);

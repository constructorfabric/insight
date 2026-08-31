-- Materialized work behind the semantic layer's measures: each measure's rows
-- at the finest grain a request can re-aggregate from, every tenant in one
-- relation so authoring a measure never issues DDL.
--
-- `kind` names the row shape: `aggregate` is one row per tenant x entity x day
-- x dimension tuple, `event` is one row per source event (a distribution over
-- pre-folded values is not that distribution), `subject` is one row per counted
-- subject (so a distinct count over any window stays exact).
--
-- The partition key is the invalidation unit: refreshing a hot window replaces
-- (measure, version, month) partitions, and a superseded definition's rows drop
-- by partition rather than by mutation.
--
-- The staging twin carries one build. It must keep the same columns, partition
-- key and sorting key as the served relation, or `REPLACE PARTITION` refuses.

CREATE TABLE IF NOT EXISTS insight.semantic_measure_cache
(
    tenant_id          String,
    measure_key        String,
    definition_version UInt32,
    kind               Enum8('aggregate' = 1, 'event' = 2, 'subject' = 3),
    metric_date        Date,
    entity             String,
    dimensions         Array(Tuple(key String, value String, label Nullable(String))),
    value              Float64,
    subject            Nullable(String),
    built_at           DateTime64(3)
)
ENGINE = MergeTree
PARTITION BY (measure_key, definition_version, toYYYYMM(metric_date))
ORDER BY (tenant_id, measure_key, entity, metric_date);

CREATE TABLE IF NOT EXISTS insight.semantic_measure_cache_staging
(
    tenant_id          String,
    measure_key        String,
    definition_version UInt32,
    kind               Enum8('aggregate' = 1, 'event' = 2, 'subject' = 3),
    metric_date        Date,
    entity             String,
    dimensions         Array(Tuple(key String, value String, label Nullable(String))),
    value              Float64,
    subject            Nullable(String),
    built_at           DateTime64(3)
)
ENGINE = MergeTree
PARTITION BY (measure_key, definition_version, toYYYYMM(metric_date))
ORDER BY (tenant_id, measure_key, entity, metric_date);

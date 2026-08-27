-- Connector sync history: what the data mover reports about every sync, copied
-- by the reconcile sweep so the record outlives the mover's own job retention
-- (a deleted connection takes its job history with it). Its own database:
-- `presentation` is swept by metric exports and customer extracts, which must
-- never carry service rows.
--
-- Three row classes share the table, told apart by `event`. Every column has a
-- defined value on every class — see the design's domain model. Columns are
-- nullable only where "nobody measured" and "measured zero" are different
-- answers; `job_created_at` is nullable because snapshot rows are not about a
-- job at all, and is never NULL on a sync row — the read surface orders jobs
-- along it, and a row without it cannot be placed among them.
--
-- Spec: docs/components/backend/analytics/specs/connector-health.

CREATE DATABASE IF NOT EXISTS ingestion_history;

CREATE TABLE IF NOT EXISTS ingestion_history.sync_events (
    event_id         UUID DEFAULT generateUUIDv4(),
    ts               DateTime64(3, 'UTC') DEFAULT now64(3),
    tick_id          String,
    job_id           String,
    connector        LowCardinality(String),
    event            LowCardinality(String),
    status           LowCardinality(String),
    started_at       Nullable(DateTime64(3, 'UTC')),
    job_created_at   Nullable(DateTime64(3, 'UTC')),
    duration_ms      Nullable(UInt64),
    records_reported Nullable(UInt64)
) ENGINE = MergeTree
PARTITION BY toYYYYMM(ts)
ORDER BY (event, connector, ts, event_id)
TTL toDateTime(ts) + INTERVAL 6 MONTH;

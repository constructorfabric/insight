-- Connector sync history: what the data mover reports about every sync, copied
-- by the reconcile sweep so the record outlives the mover's own job retention
-- (a deleted connection takes its job history with it). Its own database:
-- `presentation` is swept by metric exports and customer extracts, which must
-- never carry service rows.
--
-- Three row classes share the table, told apart by `event`. Every column has a
-- defined value on every class — see the design's domain model. Columns are
-- nullable only where "nobody measured" and "measured zero" are different
-- answers; `job_updated_at` is nullable because snapshot rows are not about a
-- job at all, and is never NULL on a sync row — the read surface orders jobs
-- along it, and a row without it cannot be placed among them.
--
-- `job_updated_at` is the mover's last word about the job — its last update, or
-- its start while it is still in flight — never a creation time: the listing
-- does not report when a job was created, and it is also the field the listing
-- filters on, so the sweep reads back exactly the field it asks by.
--
-- `tenant_id` and `source_id` name the connector INSTANCE. One connector can be
-- installed more than once — a second Secret carrying its own source id — and
-- `connector` alone cannot tell those rows apart. Empty only where no instance
-- can be resolved: rows recorded before the identity was carried whose
-- connector is no longer configured, so nothing states which instance they are.
--
-- Spec: docs/components/backend/analytics/specs/connector-health.


CREATE TABLE IF NOT EXISTS ingestion_history.sync_events (
    event_id           UUID DEFAULT generateUUIDv4(),
    ts                 DateTime64(3, 'UTC') DEFAULT now64(3),
    tick_id            String,
    job_id             String,
    connector          LowCardinality(String),
    tenant_id          LowCardinality(String),
    source_id          LowCardinality(String),
    event              LowCardinality(String),
    status             LowCardinality(String),
    started_at         Nullable(DateTime64(3, 'UTC')),
    job_updated_at     Nullable(DateTime64(3, 'UTC')),
    duration_ms        Nullable(UInt64),
    records_reported   Nullable(UInt64),
    schedules          Array(String) DEFAULT [],
    schedule_timezone  LowCardinality(String) DEFAULT '',
    schedule_origin    LowCardinality(String) DEFAULT '',
    schedule_suspended Bool DEFAULT false
) ENGINE = /*@@REPLICATED@@*/MergeTree
PARTITION BY toYYYYMM(ts)
ORDER BY (event, connector, ts, event_id)
TTL toDateTime(ts) + INTERVAL 6 MONTH;

-- Every earlier shape converges here, since this channel has no ledger and the
-- statement above never alters a table that already stands: the stamp column
-- was first named `job_created_at`, a field the mover's listing does not
-- report, and the instance identity and schedule arrived after the table did.
-- `AFTER` keeps a converged table physically identical to a fresh one.
ALTER TABLE ingestion_history.sync_events
    RENAME COLUMN IF EXISTS job_created_at TO job_updated_at;

ALTER TABLE ingestion_history.sync_events
    ADD COLUMN IF NOT EXISTS tenant_id LowCardinality(String) AFTER connector;

ALTER TABLE ingestion_history.sync_events
    ADD COLUMN IF NOT EXISTS source_id LowCardinality(String) AFTER tenant_id;

ALTER TABLE ingestion_history.sync_events
    ADD COLUMN IF NOT EXISTS schedules Array(String) DEFAULT [] AFTER records_reported;

ALTER TABLE ingestion_history.sync_events
    ADD COLUMN IF NOT EXISTS schedule_timezone LowCardinality(String) DEFAULT '' AFTER schedules;

ALTER TABLE ingestion_history.sync_events
    ADD COLUMN IF NOT EXISTS schedule_origin LowCardinality(String) DEFAULT '' AFTER schedule_timezone;

ALTER TABLE ingestion_history.sync_events
    ADD COLUMN IF NOT EXISTS schedule_suspended Bool DEFAULT false AFTER schedule_origin;

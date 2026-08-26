-- Ingestion run ledger (connector-health spec §3.7). Its own database:
-- `presentation` is swept by metric exports and customer extracts, which must
-- never carry service rows — the same reason product-usage events live apart.
--
-- Append-only. Two writers (the pipeline at run time, the reconcile sweep a
-- tick later) may record the same sync; nothing updates or deletes, and reads
-- resolve per job identity by claim precedence. Idempotent: this channel has
-- no ledger of its own and re-runs on every deploy.

CREATE DATABASE IF NOT EXISTS ingestion_runs;

CREATE TABLE IF NOT EXISTS ingestion_runs.pipeline_events (
    event_id          UUID DEFAULT generateUUIDv4(),
    ts                DateTime64(3, 'UTC') DEFAULT now64(3),
    run_id            String,
    job_id            String,
    connector         LowCardinality(String),
    event             LowCardinality(String),
    status            LowCardinality(String),
    origin            LowCardinality(String),
    claim             LowCardinality(String),
    step              LowCardinality(String),
    -- When the mover says the sync began. Nullable because a job the mover has
    -- not started yet has no start time, and the epoch would be a lie.
    started_at        Nullable(DateTime64(3, 'UTC')),
    -- When the job was CREATED. The mover's listing is ordered and filtered by
    -- this, so it — not `started_at` — is the axis the sweep's frontier moves
    -- along: a job that waited a long time to start would otherwise let the
    -- cursor jump past jobs nothing has read.
    job_created_at    Nullable(DateTime64(3, 'UTC')),
    duration_ms       Nullable(UInt64),
    records_moved     UInt64,
    rows_landed       Nullable(UInt64),
    stream            LowCardinality(String),
    streams           UInt16,
    streams_with_data UInt16,
    rows_total        Nullable(UInt64),
    bytes_on_disk     UInt64
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(ts)
ORDER BY (connector, ts, event_id)
TTL toDateTime(ts) + INTERVAL 6 MONTH
;

-- The whole grant surface this change adds, and it is one line.
--
-- SELECT only, and only for the reader. `presentation_ro` is read-only BY
-- CONSTRUCTION (see bootstrap-db/presentation-role.sql and its adversarial test)
-- — granting it INSERT here would break that guarantee for the sake of a
-- privilege the writers do not take through this role: the pipeline and the
-- sweep authenticate as the ingestion user, which owns these databases already.
--
-- No bronze grant is issued to the reader in any form: row visibility in
-- system.parts follows real data access, so "metadata only" is not a reachable
-- state, and storage facts reach the page as recorded `storage.observed` rows
-- instead (spec §2.2, §3.7).
-- `CREATE TABLE IF NOT EXISTS` never widens a table that already exists, and
-- this file has no ledger of its own: a tree that applied an earlier revision
-- keeps whatever shape that revision created. Stated explicitly so the column
-- means the same thing everywhere — on a non-nullable column the read path's
-- `duration_ms IS NOT NULL` is always true, and absence would read as a
-- measured zero. Idempotent: re-stating the current type is a no-op.
ALTER TABLE ingestion_runs.pipeline_events
    MODIFY COLUMN IF EXISTS duration_ms Nullable(UInt64);
ALTER TABLE ingestion_runs.pipeline_events
    MODIFY COLUMN IF EXISTS started_at Nullable(DateTime64(3, 'UTC'));
ALTER TABLE ingestion_runs.pipeline_events
    ADD COLUMN IF NOT EXISTS job_created_at Nullable(DateTime64(3, 'UTC')) AFTER started_at;

GRANT SELECT ON ingestion_runs.* TO presentation_ro;

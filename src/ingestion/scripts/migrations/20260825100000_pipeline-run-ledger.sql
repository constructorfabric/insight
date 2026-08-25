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
    tenant_id         String,
    source_id         String,
    event             LowCardinality(String),
    status            LowCardinality(String),
    origin            LowCardinality(String),
    claim             LowCardinality(String),
    step              LowCardinality(String),
    started_at        DateTime64(3, 'UTC'),
    duration_ms       UInt64,
    records_moved     UInt64,
    bytes_moved       UInt64,
    rows_landed       Nullable(UInt64),
    stream            LowCardinality(String),
    streams           UInt16,
    streams_with_data UInt16,
    rows_total        Nullable(UInt64),
    bytes_on_disk     UInt64,
    message           String
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(ts)
ORDER BY (connector, ts, event_id)
TTL toDateTime(ts) + INTERVAL 6 MONTH
;

-- The writers run as the ingestion user, which already owns bronze end to end,
-- so its sync-time window counts and system.parts observations need nothing here.
GRANT INSERT ON ingestion_runs.pipeline_events TO presentation_ro;

-- The whole grant surface the read surface needs. No bronze grant is issued to
-- the reader in any form: row visibility in system.parts follows real data
-- access, so "metadata only" is not a reachable state and storage facts reach
-- the page as recorded `storage.observed` rows instead (spec §2.2, §3.7).
GRANT SELECT ON ingestion_runs.* TO presentation_ro;

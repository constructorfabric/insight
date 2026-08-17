-- Product adoption events (#2573). Its own database: `presentation` is swept by
-- metric exports and customer extracts, which must never carry usage rows.

CREATE DATABASE IF NOT EXISTS product_usage;

CREATE TABLE IF NOT EXISTS product_usage.usage_events (
    event_id    UUID DEFAULT generateUUIDv4(),
    ts          DateTime64(3, 'UTC') DEFAULT now64(3),
    tenant_id   UUID,
    person_id   UUID,
    session_id  String,
    event_name  LowCardinality(String),
    path        String,
    target      String,
    app_name    LowCardinality(String),
    app_version String
) ENGINE = MergeTree
PARTITION BY toYYYYMM(ts)
ORDER BY (tenant_id, ts, event_id);

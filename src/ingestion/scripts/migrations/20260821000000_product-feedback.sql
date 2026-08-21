-- In-product feedback. Shares the `product_usage` database with the adoption
-- events and stays out of `presentation`, which metric exports and customer
-- extracts sweep.

CREATE DATABASE IF NOT EXISTS product_usage;

CREATE TABLE IF NOT EXISTS product_usage.feedback (
    feedback_id UUID DEFAULT generateUUIDv4(),
    ts          DateTime64(3, 'UTC') DEFAULT now64(3),
    tenant_id   UUID,
    person_id   UUID,
    category    LowCardinality(String),
    message     String,
    path        String,
    app_name    LowCardinality(String),
    app_version String
) ENGINE = MergeTree
PARTITION BY toYYYYMM(ts)
ORDER BY (tenant_id, ts, feedback_id);

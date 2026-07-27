-- Generic audit + state-tracking table for admin-triggered operations.
-- Phase 1 user: `persons-seed` endpoint (POST /v1/persons-seed) records a
-- row per run, status moves queued → running → completed/failed.
-- Future users (other admin operations) reuse the same table with a
-- different `operation_type` value — no schema change needed.
CREATE TABLE IF NOT EXISTS operations (
    operation_id      BINARY(16)   NOT NULL,
    operation_type    VARCHAR(64)  NOT NULL,
    status            VARCHAR(16)  NOT NULL,
    insight_tenant_id BINARY(16)   NOT NULL,
    author_person_id  BINARY(16)   NOT NULL,
    request_json      JSON         NULL,
    summary_json      JSON         NULL,
    error_message     TEXT         NULL,
    started_at        DATETIME(6)  NOT NULL DEFAULT (UTC_TIMESTAMP(6)),
    completed_at      DATETIME(6)  NULL,
    PRIMARY KEY (operation_id),
    INDEX idx_status        (status, started_at),
    INDEX idx_tenant_type   (insight_tenant_id, operation_type, started_at),
    INDEX idx_author        (author_person_id, started_at)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

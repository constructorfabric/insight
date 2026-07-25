-- Stable source-account → person_id binding (SCD2 cache derived from persons).
-- See docs/domain/identity-resolution/specs/DESIGN.md §"Table: account_person_map"
-- and identity service ADR-0002.
CREATE TABLE IF NOT EXISTS account_person_map (
    insight_tenant_id BINARY(16) NOT NULL,
    insight_source_type VARCHAR(100) NOT NULL,
    insight_source_id BINARY(16) NOT NULL,
    source_account_id VARCHAR(320) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    person_id BINARY(16) NOT NULL,
    author_person_id BINARY(16) NOT NULL,
    reason VARCHAR(50) NOT NULL,
    valid_from TIMESTAMP(6) NOT NULL,
    valid_to TIMESTAMP(6) NULL,
    PRIMARY KEY (
        insight_tenant_id, insight_source_type, insight_source_id,
        source_account_id, valid_from
    ),
    INDEX idx_current (
        insight_tenant_id, insight_source_type, insight_source_id,
        source_account_id, valid_to
    ),
    INDEX idx_person_id (person_id),
    INDEX idx_tenant_person (insight_tenant_id, person_id),
    INDEX idx_valid_from (insight_tenant_id, valid_from)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

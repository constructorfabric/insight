-- Person-attribute registry: connector-discovered attribute definitions and
-- their immutable, actor-attributed policy revisions.
--
-- Key representation: definitions are keyed by the RAW string identifiers the
-- warehouse claim relations carry (tenant, source type, source instance,
-- source field). This deviates from the BINARY(16) convention of `persons`
-- deliberately: the policy snapshot published back to ClickHouse must join
-- claims on byte-equal keys, and the BINARY(16) values in `persons` exist
-- only because the identity_inputs producer pre-hashes free-form ids into
-- UUIDs (documented temporary, #1550). Hashing here would make the registry
-- unjoinable to the very relations it governs.
CREATE TABLE IF NOT EXISTS person_attribute_definitions (
    id                  BINARY(16)   NOT NULL,
    insight_tenant_id   VARCHAR(100) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    insight_source_type VARCHAR(100) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    insight_source_id   VARCHAR(100) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    source_field_id     VARCHAR(255) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
    first_observed_at   DATETIME(6)  NOT NULL,
    last_observed_at    DATETIME(6)  NOT NULL,
    created_at          DATETIME(6)  NOT NULL DEFAULT (UTC_TIMESTAMP(6)),
    PRIMARY KEY (id),
    UNIQUE KEY uq_definition (insight_tenant_id, insight_source_type,
                              insight_source_id, source_field_id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

-- Append-only: a policy change inserts the next revision; rows never mutate.
-- The revision rows ARE the audit trail (actor + reason on every row), the
-- pattern of analytics' semantic_definition_revisions.
CREATE TABLE IF NOT EXISTS person_attribute_policy_revisions (
    id                 BINARY(16)   NOT NULL,
    definition_id      BINARY(16)   NOT NULL,
    revision           INT          NOT NULL,
    label_override     VARCHAR(255) NULL,
    sensitivity_class  VARCHAR(64)  NULL,
    grouping_enabled   BOOLEAN      NOT NULL DEFAULT TRUE,
    comparison_enabled BOOLEAN      NOT NULL DEFAULT FALSE,
    value_mode         ENUM('single','multi') NOT NULL DEFAULT 'single',
    retired            BOOLEAN      NOT NULL DEFAULT FALSE,
    actor_person_id    BINARY(16)   NOT NULL,
    reason             TEXT         NOT NULL,
    created_at         DATETIME(6)  NOT NULL DEFAULT (UTC_TIMESTAMP(6)),
    PRIMARY KEY (id),
    UNIQUE KEY uq_definition_revision (definition_id, revision),
    INDEX idx_definition (definition_id, revision),
    CONSTRAINT chk_person_attribute_policy_revision_positive CHECK (revision >= 1)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

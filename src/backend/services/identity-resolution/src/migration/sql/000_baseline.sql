-- The identity schema as migrations 001-018 left it. Every statement is
-- idempotent, so a run interrupted partway converges when re-run.
-- See docs/domain/identity-resolution/specs/DESIGN.md for the tables.

CREATE TABLE IF NOT EXISTS persons (
    id BIGINT UNSIGNED NOT NULL AUTO_INCREMENT,
    value_type VARCHAR(50) NOT NULL,
    insight_source_type VARCHAR(30) NOT NULL,
    insight_source_id BINARY(16) NOT NULL,
    insight_tenant_id BINARY(16) NOT NULL,
    value_id VARCHAR(320) NULL,
    value_full_text VARCHAR(512) NULL,
    value TEXT NULL,
    value_effective TEXT
        GENERATED ALWAYS AS (COALESCE(value_id, value_full_text, value)) STORED,
    value_hash CHAR(64) CHARACTER SET ascii COLLATE ascii_bin
        GENERATED ALWAYS AS (SHA2(COALESCE(value_id, value_full_text, value), 256)) STORED,
    person_id BINARY(16) NOT NULL,
    author_person_id BINARY(16) NOT NULL,
    reason TEXT NULL,
    created_at DATETIME(6) NOT NULL DEFAULT (UTC_TIMESTAMP(6)),

    PRIMARY KEY (id),
    UNIQUE KEY uq_person_observation (
        insight_tenant_id, person_id, insight_source_type, insight_source_id,
        value_type, created_at
    ),
    INDEX idx_value_id (insight_tenant_id, value_type, value_id),
    INDEX idx_value_full_text (insight_tenant_id, value_type, value_full_text),
    INDEX idx_person_id (person_id),
    INDEX idx_tenant_person (insight_tenant_id, person_id),
    INDEX idx_source (insight_source_type, insight_source_id),
    INDEX idx_value_id_any_tenant (value_type, value_id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS org_chart (
    insight_tenant_id BINARY(16) NOT NULL,
    insight_source_type VARCHAR(30) NOT NULL,
    insight_source_id BINARY(16) NOT NULL,
    child_person_id BINARY(16) NOT NULL,
    parent_person_id BINARY(16) NULL,
    author_person_id BINARY(16) NOT NULL,
    reason VARCHAR(50) NULL,
    valid_from DATETIME(6) NOT NULL,
    valid_to DATETIME(6) NULL,

    PRIMARY KEY (
        insight_tenant_id, insight_source_type, insight_source_id,
        child_person_id, valid_from
    ),
    INDEX idx_current_parent (
        insight_tenant_id, insight_source_type, insight_source_id,
        child_person_id, valid_to
    ),
    INDEX idx_current_children (
        insight_tenant_id, insight_source_type, insight_source_id,
        parent_person_id, valid_to
    ),
    INDEX idx_child_any_source (insight_tenant_id, child_person_id, valid_to),
    INDEX idx_parent_any_source (insight_tenant_id, parent_person_id, valid_to),
    INDEX idx_valid_from (insight_tenant_id, valid_from),
    CONSTRAINT chk_no_self_loop
        CHECK (parent_person_id IS NULL OR child_person_id <> parent_person_id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS visibility (
    visibility_id BINARY(16) NOT NULL,
    insight_tenant_id BINARY(16) NOT NULL,
    viewer_person_id BINARY(16) NOT NULL,
    viewed_person_id BINARY(16) NULL,
    valid_from DATETIME(6) NOT NULL,
    valid_to DATETIME(6) NULL,
    author_person_id BINARY(16) NOT NULL,
    reason VARCHAR(500) NULL,
    created_at DATETIME(6) NOT NULL DEFAULT (UTC_TIMESTAMP(6)),

    PRIMARY KEY (visibility_id),
    INDEX idx_viewer_current
        (insight_tenant_id, viewer_person_id, valid_to, viewed_person_id),
    CONSTRAINT chk_visibility_interval
        CHECK (valid_to IS NULL OR valid_from <= valid_to)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS roles (
    role_id BINARY(16) NOT NULL,
    name VARCHAR(64) NOT NULL,

    PRIMARY KEY (role_id),
    UNIQUE KEY uk_name (name)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

-- INVARIANT: the role ids are stable API; `admin` is mirrored by
-- roles_repo::ADMIN_ROLE_ID, and consumers gate on the names.
INSERT INTO roles (role_id, name)
VALUES
    (UNHEX('a4d11000000040008000000000000001'), 'admin'),
    (UNHEX('a4d11000000040008000000000000002'), 'previews-admin')
ON DUPLICATE KEY UPDATE name = name;

CREATE TABLE IF NOT EXISTS person_roles (
    person_role_id BINARY(16) NOT NULL,
    insight_tenant_id BINARY(16) NOT NULL,
    person_id BINARY(16) NOT NULL,
    role_id BINARY(16) NOT NULL,
    valid_from DATETIME(6) NOT NULL,
    valid_to DATETIME(6) NULL,
    author_person_id BINARY(16) NOT NULL,
    reason VARCHAR(500) NULL,
    created_at DATETIME(6) NOT NULL DEFAULT (UTC_TIMESTAMP(6)),

    PRIMARY KEY (person_role_id),
    INDEX idx_person_current (insight_tenant_id, person_id, role_id, valid_to),
    INDEX idx_role_current (insight_tenant_id, role_id, valid_to),
    CONSTRAINT chk_person_roles_interval
        CHECK (valid_to IS NULL OR valid_from <= valid_to)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS operations (
    operation_id BINARY(16) NOT NULL,
    operation_type VARCHAR(64) NOT NULL,
    status VARCHAR(16) NOT NULL,
    insight_tenant_id BINARY(16) NOT NULL,
    author_person_id BINARY(16) NOT NULL,
    request_json JSON NULL,
    summary_json JSON NULL,
    error_message TEXT NULL,
    started_at DATETIME(6) NOT NULL DEFAULT (UTC_TIMESTAMP(6)),
    completed_at DATETIME(6) NULL,

    PRIMARY KEY (operation_id),
    INDEX idx_status (status, started_at),
    INDEX idx_tenant_type (insight_tenant_id, operation_type, started_at),
    INDEX idx_author (author_person_id, started_at)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS people (
    id BIGINT UNSIGNED NOT NULL AUTO_INCREMENT,
    insight_tenant_id BINARY(16) NOT NULL,
    person_id BINARY(16) NOT NULL,
    email VARCHAR(320) NULL,
    username VARCHAR(320) NULL,
    display_name VARCHAR(512) NULL,
    first_name VARCHAR(512) NULL,
    last_name VARCHAR(512) NULL,
    attributes JSON NOT NULL,
    valid_from DATETIME(6) NOT NULL,
    valid_to DATETIME(6) NULL,
    current_person_id BINARY(16)
        GENERATED ALWAYS AS (CASE WHEN valid_to IS NULL THEN person_id ELSE NULL END) STORED,

    PRIMARY KEY (id),
    UNIQUE KEY uq_people_current (insight_tenant_id, current_person_id),
    INDEX idx_people_current (insight_tenant_id, valid_to, person_id),
    CONSTRAINT chk_people_interval CHECK (valid_to IS NULL OR valid_to >= valid_from)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

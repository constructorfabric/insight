-- person ↔ role assignments. Soft-delete + author audit, symmetric with
-- the `visibility` table — keeps a full trail of who granted/revoked
-- which role to whom and when.
--
-- An assignment is "active" when valid_to IS NULL. To revoke, UPDATE
-- valid_to = UTC_TIMESTAMP(6); a re-grant after revoke = new INSERT
-- with a fresh person_role_id.
--
-- Tenant scoping is per-row even though `roles` is global: an admin in
-- tenant T1 is NOT automatically an admin in T2; the grant has to be
-- INSERTed per tenant the operator should administer.
CREATE TABLE IF NOT EXISTS person_roles (
    person_role_id    BINARY(16) NOT NULL,
    insight_tenant_id BINARY(16) NOT NULL,
    person_id         BINARY(16) NOT NULL,
    role_id           BINARY(16) NOT NULL,
    valid_from        DATETIME(6) NOT NULL,
    valid_to          DATETIME(6) NULL,
    author_person_id  BINARY(16) NOT NULL,
    reason            VARCHAR(500) NULL,
    created_at        DATETIME(6) NOT NULL DEFAULT (UTC_TIMESTAMP(6)),

    PRIMARY KEY (person_role_id),

    -- Reject backward intervals (see same constraint on `visibility`).
    CONSTRAINT chk_person_roles_interval
        CHECK (valid_to IS NULL OR valid_from <= valid_to),

    -- Hot path: "is person A an admin in tenant T?" — single bounded
    -- lookup with role_id pinned to the seeded admin UUID.
    INDEX idx_person_current (insight_tenant_id, person_id, role_id, valid_to),

    -- "Who currently holds role R in tenant T?" — used by future
    -- read endpoints (`GET /v1/person-roles?role={rid}`) and to
    -- enumerate admins for the self-removal guard.
    INDEX idx_role_current   (insight_tenant_id, role_id, valid_to)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

-- Role catalogue — strict-minimum RBAC primitive (#346 design rev 3.1).
-- Currently one row: 'admin', which gates CRUD on the visibility,
-- roles, and person_roles tables themselves. Future roles (auditor,
-- hr_admin, ...) can be added by INSERTing further rows; the schema
-- supports them without further migration.
--
-- Global scope: no `insight_tenant_id` — the same `admin` role applies
-- across all tenants. Per-tenant grants happen in `person_roles`.
--
-- No audit columns by design: mutations to this table are rare ops
-- actions; the assignment history lives in `person_roles`, which keeps
-- a full SCD2-style trail there.
CREATE TABLE IF NOT EXISTS roles (
    role_id BINARY(16) NOT NULL,
    name    VARCHAR(64) NOT NULL,

    PRIMARY KEY (role_id),
    UNIQUE KEY uk_name (name)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

-- Hand-crafted deterministic seed UUID for `admin` so application
-- code can reference the role by a fixed constant
-- (`Insight.Identity.Domain.Services.Roles.Admin`) without a runtime
-- SELECT round-trip on every authz check. The shape "a4d1…0001" is
-- intentionally recognisable as the admin seed and is NOT the output
-- of `uuidgen`. DO NOT change this value — person_roles rows
-- reference it exactly.
INSERT INTO roles (role_id, name)
VALUES (UNHEX(REPLACE('a4d11000-0000-4000-8000-000000000001', '-', '')), 'admin')
ON DUPLICATE KEY UPDATE name = name;

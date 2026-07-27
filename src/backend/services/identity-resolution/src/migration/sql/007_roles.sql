-- Role catalogue — strict-minimum OrgChart Visibility primitive (#346 design rev 3.1).
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

-- Admin seed; the constant is mirrored by Domain.Services.Roles.Admin.
INSERT INTO roles (role_id, name)
VALUES (UNHEX('a4d11000000040008000000000000001'), 'admin')
ON DUPLICATE KEY UPDATE name = name;

-- Identity Resolution & Person domain: database and table DDL
-- First migration. Idempotent — safe to re-run.
-- Phase 1 (Initial Seed) of Identity Resolution DECOMPOSITION.

-- ============================================================
-- Databases
-- ============================================================

CREATE DATABASE IF NOT EXISTS identity;
CREATE DATABASE IF NOT EXISTS person;

-- ============================================================
-- person.persons
-- Canonical person record with inlined golden record fields.
-- Source: docs/domain/person/specs/DESIGN.md §3.7
-- ============================================================

CREATE TABLE IF NOT EXISTS person.persons
(
    id                        UUID DEFAULT generateUUIDv7(),
    insight_tenant_id         UUID,
    display_name              String DEFAULT '',
    display_name_source       LowCardinality(String) DEFAULT '',
    status                    LowCardinality(String) DEFAULT 'active',
    email                     String DEFAULT '',
    email_source              LowCardinality(String) DEFAULT '',
    username                  String DEFAULT '',
    username_source           LowCardinality(String) DEFAULT '',
    role                      String DEFAULT '',
    role_source               LowCardinality(String) DEFAULT '',
    manager_person_id         UUID DEFAULT toUUID('00000000-0000-0000-0000-000000000000'),
    manager_person_id_source  LowCardinality(String) DEFAULT '',
    org_unit_id               UUID DEFAULT toUUID('00000000-0000-0000-0000-000000000000'),
    org_unit_id_source        LowCardinality(String) DEFAULT '',
    location                  String DEFAULT '',
    location_source           LowCardinality(String) DEFAULT '',
    completeness_score        Float32 DEFAULT 0.0,
    conflict_status           LowCardinality(String) DEFAULT 'clean',
    created_at                DateTime64(3, 'UTC') DEFAULT now64(3),
    updated_at                DateTime64(3, 'UTC') DEFAULT now64(3),
    is_deleted                UInt8 DEFAULT 0
)
ENGINE = ReplacingMergeTree(updated_at)
ORDER BY (insight_tenant_id, id);

-- ============================================================
-- identity.identity_inputs — created by dbt (silver/_shared/identity_inputs.sql
-- + connector models like bamboohr__identity_inputs, zoom__identity_inputs).
-- NOT created here. See: docs/domain/identity-resolution/specs/DESIGN.md §3.7
-- ============================================================

-- ============================================================
-- identity.aliases
-- Resolved alias-to-person mapping.
-- Source: docs/domain/identity-resolution/specs/DESIGN.md §3.7
-- ============================================================

CREATE TABLE IF NOT EXISTS identity.aliases
(
    id                  UUID DEFAULT generateUUIDv7(),
    insight_tenant_id   UUID,
    person_id           UUID,
    value_type          LowCardinality(String),
    value               String,
    value_field_name    String DEFAULT '',
    insight_source_id   UUID DEFAULT toUUID('00000000-0000-0000-0000-000000000000'),
    insight_source_type LowCardinality(String) DEFAULT '',
    source_account_id   String DEFAULT '',
    confidence          Float32 DEFAULT 1.0,
    is_active           UInt8 DEFAULT 1,
    effective_from      DateTime64(3, 'UTC') DEFAULT now64(3),
    effective_to        DateTime64(3, 'UTC') DEFAULT toDateTime64('1970-01-01 00:00:00.000', 3, 'UTC'),
    first_observed_at   DateTime64(3, 'UTC') DEFAULT now64(3),
    last_observed_at    DateTime64(3, 'UTC') DEFAULT now64(3),
    created_at          DateTime64(3, 'UTC') DEFAULT now64(3),
    updated_at          DateTime64(3, 'UTC') DEFAULT now64(3),
    is_deleted          UInt8 DEFAULT 0
)
ENGINE = ReplacingMergeTree(updated_at)
ORDER BY (insight_tenant_id, value_type, value, insight_source_id, id);

---
id: cpt-ingestion-adr-service-owned-migrations
status: accepted
date: 2026-04-22
---

# ADR-0006 — Service-owned MariaDB migrations

## Context

As new backend services acquire MariaDB-resident tables, we need a
clear rule for **who authors and applies the schema**. Options include:

- A single global migration runner operating over a shared schema
  directory, invoked at deploy time.
- Per-service migrations, owned and applied by the service itself.

Our one existing precedent — `analytics` — follows the second
pattern (SeaORM `Migrator` embedded in the Rust service, applied via
`Migrator::up()` at startup). The open question for this ADR is
whether to extend that pattern to every other service with MariaDB
tables, or to introduce a separate mechanism alongside it. Services
in this project are written in different languages (Rust for
`analytics`, .NET 9 for `identity`); the policy must allow each
service to pick the idiomatic migration library for its language
while keeping the ownership rules identical.

## Decision

**Every backend service that owns MariaDB tables:**

1. **Owns its own database** inside the shared MariaDB instance.
   Cross-service access is explicit (cross-database JOINs / separate
   connections), never implicit via shared-schema layout.

2. **Owns its own migrations**, stored inside the service directory
   (`src/backend/services/<name>/.../Migrations/` for .NET services,
   `src/backend/services/<name>/src/migration/` for Rust services).
   Each service picks the migration tool idiomatic for its language:
   - Rust: SeaORM migration DSL (raw SQL via `manager.get_connection().
     execute_unprepared(...)` is acceptable when column-level
     properties — charset, collation — are not cleanly expressible in
     the DSL).
   - .NET: DbUp with plain SQL files (no embedded DSL).

3. **Applies its migrations at startup**, before opening the HTTP
   listener. Rust services call `Migrator::up(db, None)` from `main`;
   .NET services call the DbUp `MigrationRunner` from `Program.cs`.

4. **Tracks applied versions** in its own tracker table inside its own
   database. The flavour follows the migrator (`seaql_migrations` for
   SeaORM, `SchemaVersions` for DbUp). Different services' trackers
   live in different databases and never collide.

5. **Excludes one-shot data seeds from the Migrator**. Seeds
   (operator-triggered data bootstrap from external stores like
   ClickHouse) are stand-alone scripts in
   `src/backend/services/<name>/seed/`, invoked explicitly by
   operators after migrations and the source data are in place.
   They are not schema migrations and must not enter the migration
   history.

6. **Is responsible for its schema lifecycle**. The umbrella Helm
   chart provisions the **empty database + user grants** (infra
   concern) via the bitnami MariaDB subchart's `initdbScriptsConfigMap`
   (see `charts/insight/templates/mariadb-initdb-scripts.yaml`); the
   service itself never runs cross-service DDL and never issues
   `CREATE DATABASE` / `GRANT`.

## Applied to `persons` / `account_person_map`

- The `identity` (.NET 9) service owns the MariaDB database `identity`.
- Schema defined in
  `src/backend/services/identity/src/Insight.Identity.Infrastructure/Migrations/001_persons.sql`
  and `002_account_person_map.sql` (plain SQL, applied via DbUp).
- Migration runner: `Insight.Identity.Infrastructure.MigrationRunner`,
  invoked from `Program.cs` before the HTTP listener starts.
- Applied on every pod startup (idempotent — DbUp tracks applied
  scripts in a `SchemaVersions` table inside the service's own
  database).
- One-shot seed scripts (bash + Python) live at
  `src/backend/services/identity/seed/`.

## Consequences

- The umbrella's bitnami MariaDB initdb ConfigMap
  (`charts/insight/templates/mariadb-initdb-scripts.yaml`) creates the
  `identity` database + user grants once on the first pod boot; the
  service then applies its own schema. There is no global migration
  step in any other chart or script.
- Adding a new service-owned MariaDB table means adding a new
  migration file inside that service's `Migrations/` (or `migration/`)
  directory — no ingestion-side changes required.
- Cross-service schema dependencies become explicit: if service A
  needs data from service B's table, it either reads via service B's
  API or via an explicit cross-database query. No accidental shared
  table layouts.
- Each service uses the migrator native to its language; new services
  pick whichever fits (SeaORM, DbUp, Flyway-Java, alembic, …) as long
  as the ownership rules above hold. There is no project-wide
  requirement to consolidate on a single migrator.

## Alternatives considered

- **Global bash migration runner** (the pre-revert state). Rejected
  after review: see Context §1 and §2.
- **Single migrator across all languages**. Rejected: would force
  Rust-style tooling on .NET services (or vice versa) for no real
  benefit beyond uniformity. Per-service ownership already keeps the
  blast radius small.
- **Migrations in a shared crate or package across services**.
  Rejected: defeats the per-service-database decision; all services
  would end up re-importing the same migration registry. Service-local
  is simpler.
- **`schema_migrations` bash runner per service** (a runner copy
  inside each service directory). Rejected: still means a bash runner
  at all, and duplicates logic; the native migrator for the service's
  language is always available.

## Related

- `docs/components/backend/specs/ADR/` (analytics SeaORM Migrator
  is the source pattern this ADR generalises).
- `src/backend/services/identity/src/Insight.Identity.Infrastructure/Migrations/`
  — first .NET service-owned migration set under this policy.
- `docs/domain/identity-resolution/specs/ADR/0002-stable-person-id-via-persons-observations.md`
  — seed contract, unchanged by this ADR (seed stays one-shot, not
  a migration).

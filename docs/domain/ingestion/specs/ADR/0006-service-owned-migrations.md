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

Our one existing precedent — `analytics-api` — already follows the
second pattern (SeaORM `Migrator` embedded in the Rust service,
applied via `Migrator::up()` at startup). The open question for this
ADR is whether to extend that pattern to every other service with
MariaDB tables, or to introduce a separate mechanism alongside it.

## Decision

**Every backend service that owns MariaDB tables:**

1. **Owns its own database** inside the shared MariaDB instance.
   Cross-service access is explicit (cross-database JOINs / separate
   connections), never implicit via shared-schema layout.

2. **Owns its own migrations**, stored inside the service directory
   (`src/backend/services/<name>/src/migration/`), authored with the
   SeaORM migration DSL (raw SQL via `manager.get_connection().
   execute_unprepared(...)` is acceptable when column-level
   properties — charset, collation — are not cleanly expressible in
   the DSL).

3. **Applies its migrations at startup**, via `Migrator::up(db,
   None)` invoked from `main`. A helm `initContainer` using the
   service image's `migrate` CLI subcommand runs the same path
   separately for deploy-time ordering (same pattern as
   `analytics-api`).

4. **Tracks applied versions** in its own `seaql_migrations` table
   inside its own database. Different services' trackers live in
   different databases and never collide.

5. **Excludes one-shot data seeds from the Migrator**. Seeds
   (operator-triggered data bootstrap from external stores like
   ClickHouse) are stand-alone scripts in
   `src/backend/services/<name>/seed/`, invoked explicitly by
   operators after migrations and the source data are in place.
   They are not schema migrations and must not enter the migration
   history.

6. **Is responsible for its schema lifecycle**. The umbrella Helm
   chart provisions the **database + user grants** (infra concern)
   via dedicated pre-install / pre-upgrade Jobs (e.g.
   `charts/insight/templates/identity-db-init-job.yaml`); the service
   itself never applies these grants and never runs cross-service
   DDL.

## Applied to `persons`

- `identity-resolution` service owns the MariaDB database `identity`.
- Schema defined in
  `src/backend/services/identity/src/migration/m20260421_000001_persons.rs`.
- Migrator registered in
  `src/backend/services/identity/src/migration/mod.rs`.
- Applied on every pod startup via `run_migrations(&db)` in
  `src/main.rs` (idempotent — sea-orm tracks applied migrations in
  `seaql_migrations` inside the service's own database). The
  `migrate` CLI subcommand applies the same migrations and exits;
  it is intended for use as a Helm `initContainer` if/when an
  install needs deterministic ordering separate from pod-startup.
- One-shot seed scripts (bash + Python) live at
  `src/backend/services/identity/seed/`.

## Consequences

- The umbrella Helm Job (`charts/insight/templates/identity-db-init-job.yaml`)
  creates the `identity` database + user grants once before the
  identity-resolution pod starts; the service then applies its own
  schema. There is no global migration step in any other chart or
  script.
- Adding a new service-owned MariaDB table means adding a new
  migration file in that service's `migration/` directory — no
  ingestion-side changes required.
- Cross-service schema dependencies become explicit: if service A
  needs data from service B's table, it either reads via service B's
  API or via an explicit cross-database query. No accidental shared
  table layouts.
- Rust becomes the required toolchain for authoring new schema for
  Rust-backed domains. Non-Rust domains (if any arise) would need to
  pick a different migrator — deliberately out of scope for this ADR.

## Alternatives considered

- **Global bash migration runner** (the pre-revert state). Rejected
  after review: see Context §1 and §2.
- **SeaORM migrations in a shared crate across services**. Rejected:
  defeats the per-service-database decision; all services would end
  up re-importing the same migration registry. Service-local is
  simpler.
- **Rust migration library + SQL files** (no SeaORM DSL). Rejected:
  analytics-api already uses SeaORM; a second pattern doubles the
  mental-model load for no benefit.
- **`schema_migrations` bash runner per service** (a runner copy
  inside each service directory). Rejected: still means a bash
  runner at all, and duplicates logic; SeaORM is the canonical Rust
  path.

## Related

- `docs/components/backend/specs/ADR/` (analytics-api Migrator is
  the source pattern this ADR generalises).
- `src/backend/services/identity/src/migration/` — first service-
  owned migration set under this policy.
- `docs/domain/identity-resolution/specs/ADR/0002-stable-person-id-via-persons-observations.md`
  — seed contract, unchanged by this ADR (seed stays one-shot, not
  a migration).

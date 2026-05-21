# ADR-0002: Read From the MariaDB `persons` Table

**ID**: `cpt-insightspec-adr-0002-read-from-mariadb-persons`



<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
  - [Read MariaDB persons exclusively (chosen)](#read-mariadb-persons-exclusively-chosen)
  - [Read ClickHouse bronze BambooHR employees directly](#read-clickhouse-bronze-bamboohr-employees-directly)
  - [Read both — MariaDB primary, ClickHouse fallback](#read-both--mariadb-primary-clickhouse-fallback)
- [More Information](#more-information)
- [Traceability](#traceability)

<!-- /toc -->

**Status:** Accepted

## Context and Problem Statement

PR #214 introduces a service-owned MariaDB `persons` table (append-only
observation log) populated from `identity_inputs` for every connector
the platform integrates. The service needs a query target that unifies
every source behind one schema, survives a fresh-cluster install, and
does not depend on ClickHouse availability for synchronous lookups.

## Decision Drivers

- Multi-source coverage is a hard requirement.
- The synchronous lookup path must not introduce a dependency on
  ClickHouse availability.
- The first-install path must not crash-loop when no data has been
  seeded yet.
- The service surface must stay small (one config block per backing
  store).

## Considered Options

- Read MariaDB persons exclusively.
- Read ClickHouse bronze BambooHR employees directly.
- Read both — MariaDB as primary, ClickHouse as fallback.

## Decision Outcome

`insight-identity` reads exclusively from MariaDB `persons` via
`MySqlConnector`. ClickHouse access is not part of the service; bronze
tables remain the upstream input to the dbt pipeline that feeds
`identity_inputs`, but the service does not see them.

### Consequences

- The service depends on MariaDB being reachable at startup; the
  Helm readiness probe wires this through `/health`.
- A first-install cluster needs the seed to run before lookups
  succeed; the readiness probe still passes (DB is reachable, table
  is empty, every lookup just returns 404).
- Future multi-database deployments are out of scope; one MariaDB URL
  per service instance.

### Confirmation

Confirmed by the Phase 1 integration test (`PersonsEndpointTests`)
which exercises the full read path against a Testcontainers MariaDB
seeded with `persons` rows and asserts the assembled response. The
absence of any ClickHouse client in `Insight.Identity.Infrastructure.csproj`
is enforced at build time.

## Pros and Cons of the Options

### Read MariaDB persons exclusively (chosen)

- Good, because the seed pipeline already unifies every connector
  behind `persons`.
- Good, because the synchronous request path has one dependency
  (MariaDB) instead of two (MariaDB + ClickHouse).
- Good, because `BINARY(16)` UUIDs round-trip natively and the index
  surface is well-understood.
- Bad, because lookups against a fresh cluster return 404 until the
  operator runs the seed.

### Read ClickHouse bronze BambooHR employees directly

- Good, because the data is already materialised in bronze by the
  BambooHR connector.
- Bad, because it is BambooHR-only; multi-source coverage is lost.
- Bad, because the synchronous path depends on ClickHouse
  availability — a slower store with eventual-consistency semantics
  for late-arriving connector rows.
- Bad, because `bronze_bamboohr.employees` is a snapshot view — first
  install lacks the table entirely, which forces a startup-load
  pattern that crash-loops the pod until the first connector sync.

### Read both — MariaDB primary, ClickHouse fallback

- Good, because it preserves data access during MariaDB outages.
- Bad, because it doubles the configuration surface and complicates
  the cache-coherence story.
- Bad, because the two stores carry different semantics (observation
  log vs bronze snapshot) so the fallback is rarely the same answer.
- Bad, because the fallback path is hard to exercise in tests and
  tends to rot.

## More Information

- `docs/domain/identity-resolution/specs/DESIGN.md` §"Table: persons"
  for the canonical column reference.
- PR #214 — `persons` table introduction.

## Traceability

- [`cpt-insightspec-fr-identity-lookup-resolve-by-email`](../PRD.md#resolve-email-to-person_id)
- [`cpt-insightspec-fr-identity-lookup-hydrate`](../PRD.md#hydrate-person-attributes)
- [`cpt-insightspec-component-identity-infra`](../DESIGN.md#insightidentityinfrastructure)

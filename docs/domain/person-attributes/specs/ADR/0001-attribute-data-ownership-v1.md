---
status: proposed
date: 2026-08-05
decision-makers: Insight engineering
---

# ADR-0001: Keep Account Attribute Facts in ClickHouse and Governance in Identity


<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
  - [Identity MariaDB as the Attribute System of Record](#identity-mariadb-as-the-attribute-system-of-record)
  - [ClickHouse as the Complete Attribute System of Record](#clickhouse-as-the-complete-attribute-system-of-record)
  - [Split Ownership with Immutable Publication](#split-ownership-with-immutable-publication)
- [More Information](#more-information)
- [Traceability](#traceability)

<!-- /toc -->

**ID**: `cpt-person-attributes-adr-attribute-data-ownership`
## Context and Problem Statement

Person attributes originate in connector data or person-scoped derivations, require temporal analytical joins, and must be governed by tenant admins. The architecture must decide where source claims, derived values, editable policy, and query-oriented values are authoritative without creating request-time cross-database joins or duplicate editable state.

This decision affects connector ingestion, Identity, dbt, ClickHouse, and Analytics. It is needed before #2028 implementation because storage ownership determines the publication contract and failure model.

## Decision Drivers

- Analytical grouping must join attributes to ClickHouse metric facts efficiently.
- Admin policy and audit require transactional writes and clear ownership.
- Source facts must remain independent of corrective person assignment.
- Analytics reads must not depend on a request-time Identity service or MariaDB call.
- Runtime population counts must come from the actual requested period and conditions.
- The solution should add no new datastore or deployable service.

## Considered Options

- Store claims, policy, and temporal values in Identity MariaDB, then copy query-ready values to ClickHouse.
- Store claims, policy, and temporal values entirely in ClickHouse.
- Split ownership: claims and analytical projections in ClickHouse; editable policy and audit in Identity MariaDB; publish immutable policy snapshots to ClickHouse.

## Decision Outcome

Chosen option: **Split ownership**, because each datastore then owns the workload it is designed to serve without duplicating authority. ClickHouse retains connector claims, person-scoped derived values, and query-oriented temporal values. Identity MariaDB owns transactional definitions, policy revisions, and audit. Identity publishes complete immutable policy revisions into ClickHouse. Account values remain independent of current person assignment, which is joined during each cohort query; derived producers publish directly against canonical people without synthetic accounts.

Named-group definitions are not part of this decision's Identity boundary. They remain tenant configuration in Analytics MariaDB because their consumers and lifecycle are owned by analytics.

### Consequences

- Good, because peer queries read attributes and metrics from one analytical store.
- Good, because connector history does not take an unnecessary ClickHouse-to-MariaDB-to-ClickHouse round trip.
- Good, because admin policy has one transactional, audited source of truth.
- Good, because identity corrections require only the current assignment projection to refresh; attribute history is not rebuilt.
- Good, because immutable snapshots remove request-time Identity availability from the analytics path.
- Bad, because the system is eventually consistent across MariaDB and ClickHouse.
- Bad, because policy, assignment, and attribute facts have independent revisions that requests must pin and expose through diagnostics.
- Risk: policy may reference an attribute before its values arrive, or values may arrive before policy enables them. These states produce `no_data` or remain unselectable rather than authorizing a comparison.
- Risk: ClickHouse could be mistaken for an editable policy source. Analytics treats the projection as immutable and all writes remain in Identity.

### Confirmation

The decision is confirmed by design and implementation review showing:

- Connector attribute claims are retained in ClickHouse silver.
- Person-scoped derived values are retained in ClickHouse without synthetic source-account identity.
- Identity MariaDB contains definitions, immutable policy revisions, and audit, but no analytical claim history.
- Analytics query compilation reads policy, current assignment, account values, and metrics from ClickHouse without an Identity call.
- Account values contain source-account identity and no canonical `person_id`.
- The attribute catalog contains no persisted fill rate, distinct-value count, or largest-group statistics.
- Value discovery and metric requests calculate the counts required for their actual period and population.

## Pros and Cons of the Options

### Identity MariaDB as the Attribute System of Record

Claims, policy, and effective values are persisted in Identity, with query-oriented values replicated to ClickHouse.

- Good, because identity and attribute mutation share one transactional database.
- Good, because policy and values can be inspected through one operational store.
- Bad, because connector facts already in ClickHouse must be copied into MariaDB and then back into ClickHouse.
- Bad, because high-volume temporal facts and analytical scans become an Identity storage concern.
- Bad, because two fact copies need reconciliation and retention coordination.

### ClickHouse as the Complete Attribute System of Record

Claims, editable policy, audit, and values are all stored in ClickHouse.

- Good, because all analytical and governance reads use one database.
- Good, because no policy publication step exists.
- Bad, because ClickHouse is a weak fit for transactional admin edits, optimistic concurrency, and actor-attributed audit.
- Bad, because analytical jobs and governance writes share ownership and failure semantics.
- Bad, because policy state becomes easier to mutate outside the Identity authorization boundary.

### Split Ownership with Immutable Publication

ClickHouse owns claims and analytical projections. Identity MariaDB owns editable definitions, policy revisions, and audit. Complete policy snapshots are published to ClickHouse.

- Good, because storage aligns with transactional and analytical workloads.
- Good, because read queries remain local to ClickHouse.
- Good, because each data class has one editable authority.
- Good, because revisions make freshness and compatibility explicit.
- Bad, because eventual consistency and publication monitoring are required.
- Bad, because operators must understand source ownership plus independent policy, assignment, and attribute revisions.

## More Information

- **Scope:** Claims, derived values, policy, analytical values, and publication. Named groups and resolution semantics are separate decisions.
- **Read path:** ClickHouse serves policy, values, assignment, and metrics without an Identity call. No catalog-wide statistics are persisted.
- **Operations:** Policy, assignment, and value publication lag independently; complete snapshots are rollback units.
- **Compatibility:** Connector and metric contracts are additive, and the legacy cohort path remains during migration.
- **Review trigger:** Policy needs transactional coupling to facts, metrics leave ClickHouse, or benchmarks justify persisted catalog statistics.

## Traceability

- **Requirements**: [GitHub issue #2028](https://github.com/constructorfabric/insight/issues/2028) — requires governed attributes in analytics for grouping and comparison.
- **DESIGN**: [Person Attributes and Cohorting](../DESIGN.md) — applies this ownership and publication boundary.
- **Related ADR**: [ADR-0002](./0002-identity-and-time-semantics-v1.md) — defines query-time resolution through the current assignment projection.

This decision directly constrains:

- `cpt-person-attributes-principle-analytical-facts-in-clickhouse`
- `cpt-person-attributes-principle-request-scoped-measurement`
- `cpt-person-attributes-component-claim-store`
- `cpt-person-attributes-component-policy-publisher`
- `cpt-person-attributes-component-account-value-builder`
- `cpt-person-attributes-db-storage`

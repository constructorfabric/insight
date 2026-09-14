---
status: accepted
date: 2026-09-09
---

# ADR-0007: The Assistant's Warehouse Role Reads Every Database


<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Traceability](#traceability)

<!-- /toc -->

**ID**: `cpt-insightspec-v3-adr-the-query-role-reads-every-database`

## Context and Problem Statement

Metrics compile to `SELECT` and run as a read-only warehouse user, separate
from the one this service writes with. A stand gains a database per connector it
turns on, so whatever that user was granted at install time will not cover the
data that lands next — and a metric over it fails for want of a grant, which
reads to everyone as a broken metric.

## Decision Drivers

* A metric over newly connected data must run without a grant change.
* That connection must never write.

## Considered Options

* `SELECT` per database, extended per connector — narrow; every connector
  needs a migration nobody remembers, and the failure looks like a broken
  metric.
* `SELECT` on everything — every metric works the day its data lands; the read
  surface is the whole instance. **Chosen.**

## Decision Outcome

`GRANT SELECT ON *.*` to the read-only user.

### Consequences

* The role reads every database on the instance, other services' included.
* Reads only: a bad query cannot change anything.
* Adding a connector needs no grant work.

### Confirmation

The ledger-grant tests assert the reader cannot write, and that a caller with no
role is refused.

## Traceability

- **PRD**: [PRD.md](../PRD.md)
- **DESIGN**: [DESIGN.md](../DESIGN.md)

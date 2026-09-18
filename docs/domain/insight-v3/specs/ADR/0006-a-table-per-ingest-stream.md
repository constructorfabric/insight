---
status: accepted
date: 2026-09-08
---

# ADR-0006: One Physical Table per Ingest Stream


<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Traceability](#traceability)

<!-- /toc -->

**ID**: `cpt-insightspec-v3-adr-a-table-per-ingest-stream`

> **Open (2026-09-10)**: this is under review. A first-class `dataset`
> entity would likely replace the per-stream table as the thing a
> caller creates and writes to, and would give duplicate-data
> ownership somewhere to live. Until that exists the code does what
> this ADR describes.

## Context and Problem Statement

Raw data arrives as JSON named by its stream. The migration creates `raw_data`
and `PUT /v1/tables/{table}` creates further tables with the identical columns.

## Decision Drivers

* A stream's rows should be readable without scanning every other stream.
* The schema is ours: a caller asks for a table, never for its columns.
* A table name reaches DDL.

## Considered Options

* One `raw_data` table, streams told apart by a column — nothing to create;
  every read pays for every stream.
* A table per stream with a caller-described schema — fits the source; makes
  the request body our DDL.
* A table per stream with one fixed schema — reads stay per stream and the
  shape stays ours; one request before the first write. **Chosen.**

## Decision Outcome

A table per stream, same four columns each, creation admin-only, names matched
against `^[A-Za-z0-9_]{1,128}$`.

### Consequences

* The table count grows with the streams a stand ingests.
* A stream's first write needs an administrator once.
* The catalogue recognises our tables by the sorting key the schema gives them.

### Confirmation

`tables::tests::table_creation_uses_the_fixed_raw_data_schema` pins the DDL; the
k3s functional test writes to a stream's table and counts the row there.

## Traceability

- **PRD**: [PRD.md](../PRD.md)
- **DESIGN**: [DESIGN.md](../DESIGN.md)

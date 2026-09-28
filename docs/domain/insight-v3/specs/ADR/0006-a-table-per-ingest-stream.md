---
status: superseded
superseded_by: cpt-insightspec-v3-adr-dataset-as-unit-of-ingest-and-query
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
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
  - [One `raw_data` table, streams told apart by a column](#one-raw_data-table-streams-told-apart-by-a-column)
  - [A table per stream with a caller-described schema](#a-table-per-stream-with-a-caller-described-schema)
  - [A table per stream with one fixed schema](#a-table-per-stream-with-one-fixed-schema)
- [Traceability](#traceability)

<!-- /toc -->

**ID**: `cpt-insightspec-v3-adr-a-table-per-ingest-stream`

> **Superseded (2026-09-15)** by
> [ADR-0009](0009-a-dataset-is-the-unit-of-ingest-and-query.md): a dataset
> replaces the per-stream table as the thing a caller creates, writes to and
> queries, and its row identity is where duplicate-data ownership lives. The
> physical table per uploaded dataset stays as described here; it is created
> when the dataset is, no longer on first write. Until ADR-0009 ships the code
> does what this ADR describes.

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

## Pros and Cons of the Options

### One `raw_data` table, streams told apart by a column

* Good, because nothing has to be created before a stream can be written.
* Bad, because every read pays for every stream.

### A table per stream with a caller-described schema

* Good, because the table fits whatever the source sends.
* Bad, because the request body becomes our DDL.

### A table per stream with one fixed schema

* Good, because reads stay per stream and the schema stays ours.
* Bad, because a stream's first write needs an administrator once.

## Traceability

- **PRD**: [PRD.md](../PRD.md)
- **DESIGN**: [DESIGN.md](../DESIGN.md)

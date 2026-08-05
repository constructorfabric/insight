---
status: proposed
date: 2026-08-05
---

# ADR-0004: Static Stream Schemas with Full-Record `raw_data`

**ID**: `cpt-insightspec-adr-connector-static-schema-raw-data`

<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
  - [Option 1: Static columns plus full-record `raw_data`](#option-1-static-columns-plus-full-record-raw_data)
  - [Option 2: Discovery-derived columns](#option-2-discovery-derived-columns)
  - [Option 3: Static columns, residual-only overflow blob](#option-3-static-columns-residual-only-overflow-blob)
  - [Option 4: `raw_data` only, no typed columns](#option-4-raw_data-only-no-typed-columns)
- [More Information](#more-information)
- [Traceability](#traceability)

<!-- /toc -->

## Context and Problem Statement

A connector whose advertised schema is computed from a live discovery call
against the source instance makes the Bronze table shape a function of that
instance. Two consequences follow. Bronze DDL cannot be produced without
credentials, so tables cannot be created before the first sync. And two
instances of the same source yield different column sets for the same stream,
so downstream models cannot rely on a column existing.

Restricting columns to a curated list fixes the shape but discards every field
outside the list — including fields the source adds later. Fields that were
never captured cannot be recovered retroactively, which forecloses metrics
that would otherwise be derivable from data already collected.

What schema contract should a connector advertise so that Bronze is
instance-independent and no source field is lost?

## Decision Drivers

* Offline DDL — bronze tables must be creatable from the repository alone, with no credentials and no discovery call
* Instance independence — the same stream must produce the same columns against any instance of the source
* No field loss — a field the source returns must reach Bronze even when no column is declared for it
* Retroactive analysis — a metric conceived after ingestion must be answerable from history already stored
* Query ergonomics — the fields models actually consume should be plain typed columns, not JSON extraction

## Considered Options

* Option 1: Static columns plus full-record `raw_data`
* Option 2: Discovery-derived columns
* Option 3: Static columns, residual-only overflow blob
* Option 4: `raw_data` only, no typed columns

## Decision Outcome

Chosen option: "Static columns plus full-record `raw_data`".

A connector declares each stream's columns statically, in the repository. The
declaration is the source of truth for both the advertised catalog and the
generated Bronze DDL. Every record additionally carries `raw_data`: the whole
source record, serialized as a compact JSON string.

Rules:

1. The advertised schema is loaded from a static declaration. Discovery calls
   may build fetch lists (which fields to request from the API), never the
   schema.
2. `raw_data` holds the record as received, minus source metadata envelopes
   that carry no data. It is present on every stream.
3. Fields the stream does not declare are emitted only inside `raw_data`, never
   as top-level keys. Emitting them would let the destination create columns
   for them and restore instance-dependent drift.
4. String values inside `raw_data` are capped per value. The serialized blob is
   never truncated, so it always parses.
5. Adding a column is a repository change: extend the static declaration.

### Consequences

* Good, because Bronze DDL derives from the repository and needs no credentials
* Good, because the column set is identical across instances of a source
* Good, because a field with no declared column still reaches Bronze and stays
  available to models written later
* Good, because the fields models consume stay typed columns, so existing
  queries need no JSON extraction
* Bad, because declared values are stored twice, once as a column and once
  inside `raw_data`
* Bad, because a source that adds a field no longer surfaces it as a column
  automatically; promoting it is a deliberate repository change
* Neutral, because `raw_data` is a JSON string rather than a native JSON
  column — the destination materializes both identically today

### Confirmation

* Building a stream catalog performs no network call, and repeated builds
  against different instances produce byte-identical schemas
* Every stream's advertised properties equal the static declaration plus the
  envelope fields
* A record carrying a field with no declared column emits no top-level key for
  it, and the field is present in the parsed `raw_data`

## Pros and Cons of the Options

### Option 1: Static columns plus full-record `raw_data`

Columns declared in the repository; whole record additionally preserved as JSON.

* Good, because it satisfies offline DDL and no-field-loss simultaneously
* Good, because typed columns keep the common query path ergonomic
* Bad, because declared values are duplicated inside the blob

### Option 2: Discovery-derived columns

The advertised schema is computed per instance from a live discovery call.

* Good, because a newly added source field becomes a column with no code change
* Bad, because Bronze DDL cannot be produced without credentials
* Bad, because column sets differ across instances of the same source
* Bad, because a fetch failure during discovery fails the whole sync

### Option 3: Static columns, residual-only overflow blob

Only undeclared fields go to the blob; declared values are not duplicated.

* Good, because it avoids duplicate storage
* Bad, because the record cannot be reconstructed from one column; consumers
  must join columns and blob and know which is which
* Bad, because promoting a field to a column changes where historical values
  live, so a query must read both shapes

### Option 4: `raw_data` only, no typed columns

Envelope plus one JSON column; all fields extracted downstream.

* Good, because Bronze DDL becomes identical for every stream
* Good, because it stores each value once
* Bad, because every existing downstream model must be rewritten to extract
  from JSON
* Bad, because wide blobs flowing through sort buffers are a known source of
  memory exhaustion in downstream aggregation

## More Information

Connectors whose column set is already curated and instance-independent satisfy
this ADR by adding `raw_data`; their existing declaration becomes the static
schema. Connectors that compute schemas from discovery must move the
declaration into the repository.

`raw_data` is the only overflow carrier. A connector emits no second blob for a
subset of the record — a column holding just the instance-defined fields is
contained in `raw_data`, and two representations of the same values drift:
they are written by different rules and can disagree on truncation, on null
handling, and on which fields they consider in scope. A consumer that wants
only the instance-defined subset filters `raw_data` at read time.

## Traceability

This decision directly addresses the following requirements or design elements:

* `cpt-insightspec-fr-cn-custom-fields` — a field with no declared column is
  preserved in `raw_data`
* `cpt-insightspec-adr-connector-responsibility-scope` — the connector emits the
  full payload alongside extracted fields, as that ADR requires

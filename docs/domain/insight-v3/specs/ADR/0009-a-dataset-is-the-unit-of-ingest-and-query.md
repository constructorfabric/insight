---
status: proposed
date: 2026-09-15
---

# ADR-0009: A Dataset Is the Unit of Ingest and Query

<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
  - [Keep per-stream tables and store a description beside each](#keep-per-stream-tables-and-store-a-description-beside-each)
  - [A dataset entity over the existing JSON tables](#a-dataset-entity-over-the-existing-json-tables)
  - [A dataset entity with fields materialized as typed columns](#a-dataset-entity-with-fields-materialized-as-typed-columns)
- [More Information](#more-information)
- [Traceability](#traceability)

<!-- /toc -->

**ID**: `cpt-insightspec-v3-adr-dataset-as-unit-of-ingest-and-query`

Supersedes [ADR-0006](0006-a-table-per-ingest-stream.md). The physical table
per uploaded dataset that ADR-0006 describes stays; what changes is what a
caller creates, writes to, reads and is told about.

A dataset is the unit of ingest, and the unit of query wherever a reader
wants what a declaration knows. It is not the only thing a metric may read:
a metric names a dataset or any table the warehouse holds.

## Context and Problem Statement

Raw data arrives as JSON named by its stream, and the stream is a physical
table that appears on the first write of anyone holding the ingest token. The
table says nothing about its rows: no owner, no description, no field types, no
notion of which rows are the same record. A metric over it repeats, per field,
how to extract the value from the payload; the assistant learns the fields by
sampling recent rows and guessing; nothing in the product shows or removes a
stream; and the read path has no point at which "may this caller read these
rows" could ever be asked.

Issue constructorfabric/insight#3408 asks for a dataset that an administrator
creates and removes, that the portal shows, that the assistant reads, and that
later becomes the unit access is restricted by.

## Decision Drivers

* A caller must create the place data lands before sending it; a typo must not
  create a table.
* One declaration of the data, read by the compiler, the portal and the model.
* A re-sent record must not count twice.
* Every read must pass through one entity, so that an access policy has a home.
* The first iteration must not require migrating stored rows or changing the
  physical shape of the ingest tables.
* Automatic migration of what a stand already holds is more risk than the data
  is worth while this is a proof of concept, and a wrong guess about an
  existing stream is silent.

## Considered Options

* Keep per-stream tables and store a description beside each — the description
  is advisory; writes still create tables and metrics still address tables.
* A dataset entity over the existing JSON tables — the declaration supplies the
  field expressions, ingest and metrics address the dataset, the rows stay as
  they are. **Chosen.**
* A dataset entity whose declared fields are materialized as typed columns —
  faster reads and typed storage, at the price of table alterations per
  declaration change. **Deferred**, not rejected: the declaration already
  carries what a later materialization needs.

## Decision Outcome

Chosen option: "a dataset entity over the existing JSON tables", because it
closes every driver without touching stored rows, and leaves the column
materialization as an optimisation that no metric would notice.

The decisions the option carries, taken on 2026-09-15:

* A dataset is a definition like a metric, a widget or a dashboard: stored in
  the definitions store by name, replaced on write, no versions or history.
  The registry lives in `insight-v3-core` only.
* A declaration holds a title, a description, the fields — each with a payload
  key path, a type, and optionally a role, a description, an absent value or a
  person handle — one default clock, and a row identity.
* **A field's type decides what a metric may do with it; its role is
  descriptive.** Sum and average need a numeric field, a clock needs a
  datetime; grouping, filtering, counting, min and max are open to every type.
  The role guides the assistant and the catalogue, so a numeric identifier can
  be grouped by without being mislabelled.
* **Every uploaded dataset's table lives in a ClickHouse database this service
  owns**, separate from the warehouse the rest of Insight builds. Nothing
  outside it is created, read, written or dropped, and a name held there by a
  table of any other shape is refused rather than adopted.
* Only an administrator creates or removes a dataset, under the admin role.
  Ingest keeps its own token and can no longer create anything, and that token
  is refused on the lifecycle surfaces — whoever is trusted to send records is
  not thereby trusted to declare a dataset or to remove one with its records.
  The lifecycle gets no credential of its own: a portal session is the way
  in, by decision. Declaring a dataset is a person saying what their records
  mean, and an agent is never given a way to do it.
* **The dataset's row is the lock, and a create or a removal owns a lease on
  it.** Every decision that depends on a dataset's state holds that row for the
  whole of the decision; a replacement touches no table and so commits as one
  transaction. A create or a removal must outlive one transaction, so it takes
  a leased operation: only its owner may create or drop that dataset's tables,
  a second attempt is refused while the lease holds and may take over once it
  lapses, and an attempt writes its outcome only if it still owns the operation
  when it finishes.
* **A dataset's physical table is named for the attempt that created it**, not
  for the dataset. A statement arriving after its attempt lost the dataset
  therefore names a table that no longer exists, instead of one a later attempt
  provisioned — which is what makes a late drop harmless without asking the
  service to know whether one is still in flight. A table is ours only when a
  stored declaration names it; nothing is dropped on the strength of its name.
* **Only a ready dataset is reachable.** Ingest, metric runs, the catalogue,
  the record preview and every description given to the assistant and to MCP
  ask for that state rather than excluding the states they know about, so a
  dataset mid-create is invisible and unwritable and one mid-removal leaves
  every surface at once.
* Records are stored whole as JSON. A record missing a declared field, or
  carrying keys no field declares, is stored the same way; the declaration
  says what may be asked, not what is refused. A key that is absent, null or
  unconvertible reads as the empty value for its type, never as a zero — which
  changes what an existing metric answers, because today such a key reads as a
  zero and drags an average towards it. The change is deliberate and the
  migration reports every metric it can move.
* **A substitute and a person handle change what a reader is shown, not what a
  record is.** Row identity reads the raw value, so two records that merely
  lack the key are not collapsed through one substitute.
* A declared row identity collapses duplicates on read: one record per
  identity, the one received last, ties broken by record id so repeated runs
  agree. A record whose identity is incomplete stands alone rather than
  collapsing with others. The table engine is unchanged.
* A metric with no clock of its own inherits the dataset's default clock, and
  the API reports that effective clock so the portal windows the card instead
  of reading the metric body.
* **A metric names one of two things: a dataset, or a table the warehouse
  holds.** Which it is follows from which the body names; naming both, or
  neither, is refused. A dataset gives the metric a declaration — where each
  value sits, its type, the date that windows it, which records count as one —
  and the metric names only declared fields. A table gives it columns, read as
  written, and a key path inside a column that holds JSON. This is what a
  metric was before datasets, kept rather than replaced: nothing a stand has
  already built stops working, and a reader who wants a number off a warehouse
  table does not have to declare a dataset to get it. The analytics service and
  its metrics are outside this decision.
* **A dataset is over records sent into it, and nothing else.** Binding one to
  a relation the warehouse already builds was tried and taken out: it asked the
  declaration to describe something that already exists, which made a dataset
  two things held under one name, and the reason for wanting it — reading a
  warehouse table — is answered by the metric naming the table directly.
* **A field says where its value sits** as a key path into the record's
  payload.
* **Nothing is carried over.** No stream table is adopted, moved or renamed,
  and no stored metric is rewritten. A stand that already holds them keeps them
  where they are, unreachable: the metrics name tables and are refused, the
  tables sit outside the datasets database and are never addressed. Datasets
  are declared anew and records are sent again through the ordinary ingest
  path, addressed to a dataset, with no compatibility alias. The service also
  stops creating its landing table in the warehouse database, so that it owns
  nothing outside its own.

### Consequences

* Good, because the model, the portal and the compiler read one declaration
  written by a person, and a change at the source is one edit.
* Good, because a write cannot create a table, and a dataset can be removed
  from the product with its rows.
* Good, because every read resolves a dataset first — the place an access
  policy will attach to.
* Good, because a separate database makes ownership decidable: the service can
  never create or drop a relation the warehouse built, whatever a dataset is
  named.
* Bad, because every run still parses JSON for every record; the deferred
  materialization is the remedy when volumes call for it.
* Bad, because deduplication on read costs a subquery per run over datasets
  with a row identity.
* Bad, because migrated metrics can answer differently: an average over records
  missing a key rises once that key stops reading as a zero.
* Bad, because a metric over a warehouse table is checked for shape and not
  for existence: a table that is not there, or a column that has gone, is met
  by the reader on the run rather than by the author on the write. The
  catalogue is offered so the author can see what is there, but it is a help
  and not a gate — a table made minutes ago is readable before it is listed.
* Bad, because whatever sends records must address a dataset from that moment,
  with no alias to ease it across.
* Bad, because there are now two ways to write the same metric, and which one
  a stand should prefer is a matter of judgement rather than of rule: a
  declaration pays off where several metrics read the same records, and costs
  more than it returns for one number off one table.

### Confirmation

Unit tests over the handlers, the validator, the operation protocol and the
compiler: a record into an
unknown dataset is refused without a create statement; a metric names a
dataset or a table and is refused for naming both or neither, and one over a
table reads a column, a json key inside one, and an engine that keeps
superseded rows through `FINAL`; a declaration's
violations are reported together; a run over a dataset with a row identity
reads one record per identity and keeps incomplete identities apart; a create
whose provisioning fails stores nothing and holds its name; a second attempt is
refused while a lease holds and takes over once it lapses; an attempt that
finishes after losing its operation writes nothing; two removals drop one
table once; a dataset that is not ready is refused by every read and write
path; and every statement that writes names the datasets
database and the physical table its declaration records. A stand check confirms that a
non-administrator cannot create or remove a dataset, and a component test
confirms a card follows the board's window on an inherited clock. The feature
and its scenarios are in
[feature-datasets/FEATURE.md](../feature-datasets/FEATURE.md).

## Pros and Cons of the Options

### Keep per-stream tables and store a description beside each

* Good, because nothing about ingest or metrics changes.
* Bad, because a write still creates a table and a metric still addresses one,
  so the description binds nothing.
* Bad, because there is still no entity to remove, to show, or to attach a
  policy to.

### A dataset entity over the existing JSON tables

* Good, because the declaration is the single source for the compiler, the
  portal and the model.
* Good, because stored rows and the ingest table shape are untouched, so the
  adoption of existing streams is a declaration per table.
* Neutral, because read cost is unchanged from today.
* Bad, because a mistyped value in a record is discovered on read, not on
  write.

### A dataset entity with fields materialized as typed columns

* Good, because reads hit typed, compressible columns and the engine can dedup.
* Bad, because every declaration change is a table alteration the service must
  own and reconcile.
* Bad, because it is more machinery than the first iteration needs.

## More Information

Two earlier explorations informed the declaration vocabulary and the validator
rules: the datasets and query-engine branch in the analytics service
(constructorfabric/insight#3321) and the semantic-layer stack closed on
2026-09-02. Neither is merged; their declaration roles, "report every
violation" discipline and storage-hiding discovery are borrowed, their
build-time embedding and dbt coupling are not.

## Traceability

- **PRD**: [PRD.md](../PRD.md)
- **DESIGN**: [DESIGN.md](../DESIGN.md)

This decision directly addresses the following requirements or design elements:

* `cpt-insightspec-v3-fr-create-dataset` — the dataset is what an administrator creates
* `cpt-insightspec-v3-fr-remove-dataset` — removal takes the declaration and the table together
* `cpt-insightspec-v3-fr-ingest-into-dataset` — a record lands only in an existing dataset
* `cpt-insightspec-v3-fr-metrics-over-datasets` — metrics address datasets, never tables
* `cpt-insightspec-v3-fr-assistant-reads-datasets` — the assistant is told the declaration
* `cpt-insightspec-v3-principle-shape-agnostic-ingest` — records stay arbitrary JSON; interpretation moves into the declaration

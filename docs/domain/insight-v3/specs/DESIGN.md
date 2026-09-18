---
status: draft
date: 2026-09-07
---

> [!NOTE]
> Keep this DESIGN very brief — short sections, no filler.

# DESIGN -- Insight v3 Core Service

- [ ] `p3` - **ID**: `cpt-insightspec-v3-design-core`

<!-- toc -->

- [1. Architecture Overview](#1-architecture-overview)
  - [1.1 Architectural Vision](#11-architectural-vision)
  - [1.2 Architecture Drivers](#12-architecture-drivers)
  - [1.3 Architecture Layers](#13-architecture-layers)
- [2. Principles & Constraints](#2-principles--constraints)
  - [2.1 Design Principles](#21-design-principles)
  - [2.2 Constraints](#22-constraints)
- [3. Technical Architecture](#3-technical-architecture)
  - [3.1 Domain Model](#31-domain-model)
  - [3.2 Component Model](#32-component-model)
  - [3.3 API Contracts](#33-api-contracts)
  - [3.4 Internal Dependencies](#34-internal-dependencies)
  - [3.5 External Dependencies](#35-external-dependencies)
  - [3.6 Interactions & Sequences](#36-interactions--sequences)
  - [3.7 Database schemas & tables](#37-database-schemas--tables)
  - [3.8 Deployment Topology](#38-deployment-topology)
  - [3.9 Declaration and Conversion Contract](#39-declaration-and-conversion-contract)
- [4. Additional context](#4-additional-context)
  - [Test strategy](#test-strategy)
  - [Open questions](#open-questions)
- [5. Traceability](#5-traceability)

<!-- /toc -->

## 1. Architecture Overview

### 1.1 Architectural Vision

Build on gears as much as possible: where a gear covers a capability, compose it instead of writing our own.

### 1.2 Architecture Drivers

TBD

#### Functional Drivers

Functional requirements and their IDs are in [PRD §5](./PRD.md#5-functional-requirements). Design responses: TBD.

#### Architecture Decision Records

| ADR | Decision |
|-----|----------|
| `cpt-insightspec-v3-adr-separate-service` | A separate service rather than extending analytics |
| `cpt-insightspec-v3-adr-static-ingest-token` | A static per-environment ingest token rather than the platform OIDC path |
| `cpt-insightspec-v3-adr-frontend-left-untouched` | The existing frontend is left alone; the new pages sit beside it |
| `cpt-insightspec-v3-adr-schema-driven-widgets` | Widgets are described by a schema, not written as code |
| `cpt-insightspec-v3-adr-a-table-per-ingest-stream` | One physical table per ingest stream (superseded by the decision below, though each dataset still owns a table) |
| `cpt-insightspec-v3-adr-dataset-as-unit-of-ingest-and-query` | A dataset, not a per-stream table, is what a caller creates, writes to and queries |

#### NFR Allocation

| NFR | Design response |
|-----|-----------------|
| `cpt-insightspec-v3-nfr-efficiency` | Definitions are rows read on demand; a record is stored once, whole, and interpreted on read, so a dataset costs one row plus the records it holds |
| `cpt-insightspec-v3-nfr-reliability` | A dataset's row is the lock and the ownership record: one owner per operation, a lease that expires, an outcome written only by an owner that still owns it, and physical tables named per attempt so a late statement reaches nothing |
| `cpt-insightspec-v3-nfr-performance` | The budget is the dashboard's, measured where a board is opened, and no part of it is apportioned to a single definition. This design adds two costs against it — a payload parsed per record on every run, and a collapsing read over a dataset that declares an identity — and removes neither; the deferred column materialization is what removes them, and is the lever if the budget is missed |
| `cpt-insightspec-v3-nfr-security` | One admin-gated surface, one database the service owns and nothing outside it, a read-only principal for every query, and a declaration that decides what may be asked |
| `cpt-insightspec-v3-nfr-versatility` | New data is a declaration, not a release: a dataset, a metric over its fields, a widget and a board are all stored documents |

### 1.3 Architecture Layers

| Layer | Responsibility | Technology |
|-------|---------------|------------|
| Presentation | Any page renders widgets from a JSON schema | React 19 + TypeScript |
| Application | Raw-data ingest and read; metric, widget and dashboard definitions | Rust service `insight-v3-core` |
| Infrastructure | Raw-data storage | ClickHouse |

## 2. Principles & Constraints

### 2.1 Design Principles

#### Gears First

- [ ] `p2` - **ID**: `cpt-insightspec-v3-principle-gears-first`

Where a gear covers a capability, compose it instead of writing our own.

#### Widgets Are Schema, Not Code

- [ ] `p1` - **ID**: `cpt-insightspec-v3-principle-schema-driven`

Widgets are described by a JSON schema and the frontend renders from that schema, so a new widget type is a schema change.

#### Ingest Does Not Know the Shape

- [ ] `p1` - **ID**: `cpt-insightspec-v3-principle-shape-agnostic-ingest`

The ingest side accepts data without knowing its shape; interpretation happens on read.

### 2.2 Constraints

#### TBD

- [ ] `p1` - **ID**: `cpt-insightspec-v3-constraint-tbd`

TBD



## 3. Technical Architecture

### 3.1 Domain Model

- Raw record — arbitrary JSON received into a dataset and stored whole.
- Dataset — a named, described set of raw records at one grain: fields with a payload key and a type, one default clock, a row identity. The only relation a metric reads; its records live in a table of the datasets database, which this service owns.
- Dataset field — a name, a key path into the payload, a type, and optionally a descriptive role, an absent value, a person handle. The **type** decides what a metric may do with the field; the role only guides the catalogue and the assistant.
- Metric — a named calculation over one dataset's fields.
- Widget — any visual representation of data over chosen columns.
- Dashboard — a named, addressable page arranging widgets.
- Alert — a condition on a metric plus where to send it when it fires.

### 3.2 Component Model

#### insight-v3-core Service

- [ ] `p1` - **ID**: `cpt-insightspec-v3-component-core-service`

##### Why this component exists

TBD

##### Responsibility scope

Ingests raw data, serves reads, and owns metric, widget, dashboard and alert definitions.

##### Responsibility boundaries

TBD

##### Related components (by ID)

- `cpt-insightspec-v3-component-ai-chat` — serves it the same contracts the UI uses

#### AI Chat

- [ ] `p2` - **ID**: `cpt-insightspec-v3-component-ai-chat`

##### Why this component exists

TBD

##### Responsibility scope

Answers questions, and creates metrics, widgets, dashboards and alerts.

##### Responsibility boundaries

TBD

##### Related components (by ID)

- `cpt-insightspec-v3-component-core-service` — reads and writes definitions through it

### 3.3 API Contracts

Shapes are the OpenAPI document the service emits and commits, drift-checked by
its `committed_openapi_document_is_current` test. The surface itself is listed
here, because a generated document cannot say which endpoints ought to exist.

**Technology**: OpenAPI, served by the api-gateway gear

**Location**: `docs/components/backend/insight-v3-core/openapi.json`

| Method and path | Purpose | Who may call it |
|---|---|---|
| `POST /v1/raw-data` | Send one record into a dataset | ingest token |
| `PUT /v1/datasets/{name}` | Declare a dataset, or replace its declaration | admin role, or administration token |
| `GET /v1/datasets` | The ready datasets, searched and paged | admin role |
| `GET /v1/datasets/{name}` | One declaration | admin role |
| `GET /v1/datasets/{name}/records` | The latest records, newest first, capped | admin role |
| `GET /v1/datasets/{name}/dependents` | Every metric reading this dataset, unpaged | admin role |
| `DELETE /v1/datasets/{name}` | Remove a dataset and its records | admin role, or administration token |
| `GET/PUT/DELETE /v1/{metrics,widgets,dashboards}/{name}`, `POST /v1/metrics/{name}/run`, `POST /v1/chat` | Unchanged in surface; `PUT /v1/metrics` gains the dataset rules and `GET /v1/metrics/{name}` gains the effective clock | as today |

`PUT /v1/tables/{table}` is withdrawn: a dataset is what a caller creates.

**Answers.** A refusal the caller can act on carries every violation at once
(§3.9). A name that is not free, a dataset an operation holds, and a removal a
metric still reads are conflicts. A dataset that is not ready is not found,
whichever surface asked. Records and dependents are bounded by named
configuration, not by the caller.

### 3.4 Internal Dependencies

- Usage recording and reporting already exist in the analytics service: [src/backend/services/analytics/src/api/usage.rs](../../../../src/backend/services/analytics/src/api/usage.rs). Its summary reports opens and people per event target, per page and per person.

### 3.5 External Dependencies

- ClickHouse — raw-data storage.

### 3.6 Interactions & Sequences

#### TBD

**ID**: `cpt-insightspec-v3-seq-tbd`

TBD

### 3.7 Database schemas & tables

- [ ] `p1` - **ID**: `cpt-insightspec-v3-db-clickhouse`

#### Table: raw_data

**ID**: `cpt-insightspec-v3-dbtable-raw-data`

The schema lives in the migration, not here: [src/backend/services/insight-v3-core/src/migration.rs](../../../../src/backend/services/insight-v3-core/src/migration.rs).

`raw_data` is the table the migration brings up. Every uploaded dataset gets a
physical table of its own with the identical columns, created when the dataset
is created and dropped when it is removed — see
[ADR-0009](./ADR/0009-a-dataset-is-the-unit-of-ingest-and-query.md), which
supersedes [ADR-0006](./ADR/0006-a-table-per-ingest-stream.md).

#### Table: datasets

**ID**: `cpt-insightspec-v3-dbtable-datasets`

The declaration store: one MariaDB row per dataset, in the definitions store
beside `metrics`, `widgets` and `dashboards` — `name` (primary key), `body`
(the JSON declaration), `state` (`claimed`, `ready` or `removing`),
`physical_table` (the table in the datasets database this dataset's records
live in), `operation` and `operation_token` with `lease_until` (the attempt
that owns an in-flight create or removal, empty when none is running) and
`updated_at`. The row is both the lock and the ownership
record: every writer that depends on a dataset's state holds it for the whole
of its decision, and a create or a removal — which must outlive one
transaction to touch a table — owns a lease on it. The schema lives in the
definitions migration, not here:
[src/backend/services/insight-v3-core/src/definitions/migration.rs](../../../../src/backend/services/insight-v3-core/src/definitions/migration.rs).

#### Configuration

- [ ] `p1` - **ID**: `cpt-insightspec-v3-design-dataset-settings`

Four values this feature introduces are configuration: the **datasets
database** name; the **operation lease** bound, which decides how long an
abandoned create or removal holds its dataset before another attempt may take
it over; the **record preview** cap, the most records one dataset page asks
for; and the **administration token**, the credential the dataset lifecycle
accepts over the API. The first three ship with defaults an installation may
override. The last is a secret an installation sets, and it is deliberately
not the ingest token: one is handed to whoever sends data, the other to
whoever may declare and remove it, and they rotate apart.

#### Database: the datasets database

**ID**: `cpt-insightspec-v3-db-datasets-database`

A ClickHouse database of this service's own, holding one table per uploaded
dataset in the `raw_data` shape. It is separate from the warehouse the rest of
Insight builds, and that separation is what makes ownership decidable: the
service addresses nothing outside this database, and inside it a table is its
own only when a stored declaration names it *and* its columns and sorting key
are the ingest schema.

A table's physical name is never the dataset's name. It carries the generation
the creating attempt minted — a dataset `qa_test_runs` recreated twice holds
two different physical names over its life, one of them at a time. Two attempts
at one dataset therefore address two tables, and a statement that arrives after
its attempt lost the dataset names a table that no longer exists instead of one
a later attempt provisioned.

### 3.8 Deployment Topology

TBD


### 3.9 Declaration and Conversion Contract

- [ ] `p1` - **ID**: `cpt-insightspec-v3-design-dataset-contract`

**The declaration.** A dataset body carries `title`, an optional
`description`, `fields`, and an optional `row_identity`. A field carries
`name` (an identifier, unique in the dataset, never `bucket`), `path` (a key
path into the record's JSON), `type` (`string`, `int`, `float`, `bool` or
`datetime`), and optionally `role` (`dimension`, `measurable` or `time`, which
is descriptive only), `description`, `absent_value` (a string substitute shown
and matched where the value is empty) and `person` (`email` or `id`).
At most one field carries `default_clock`, and it must be a `datetime`.
`row_identity` lists field names.

**Key paths.** A path is one or more segments separated by `.`, each a key in
the record's JSON — `commit.author.email` reads the nested object. A segment
containing a literal dot is escaped with a backslash. A declaration does not
pick an element out of an array in this iteration, so a field whose value is
an array or an object reads as empty for every scalar type; the compiler can
already select an array element for a metric written against a table, and a
declaration may borrow that later.

**What a metric names — two name spaces, not one.** A metric carries `dataset`
and no `table`, `database`, key path or type: those come from the declaration.
Beyond that it uses two kinds of name, and conflating them is the mistake this
paragraph exists to prevent.

| Name space | Where it appears | What it may name |
|---|---|---|
| Field reference | the source of each selected field, each filter, the clock | a `name` declared by the dataset |
| Output name | each selected field's own name, and what `group_by` and `order_by` name | a column this metric produces |

A metric that reads `lines` and calls the result `total` groups and orders by
`total`, never by `lines` — which is how stored metrics already read, so the
rule adds a check rather than a migration. The columns a metric produces are
its selected fields' own names plus `bucket`, the time bucket a windowed run
injects. A metric may name a clock field explicitly; otherwise it inherits the
dataset's `default_clock`, and the metric read endpoint reports the effective
clock and whether it came from the metric or the dataset.

**Conversion.** One rule decides what a record's value becomes, and the table
below is the whole of it. "Empty" is ClickHouse's NULL, never a substituted
zero or epoch.

| In the record | string | int / float | bool | datetime |
|---|---|---|---|---|
| Key absent | empty | empty | empty | empty |
| JSON `null` | empty | empty | empty | empty |
| `""` | `""`, distinct from empty | empty | empty | empty |
| A number | its text | the number | non-zero is true | empty |
| A string that parses | itself | the parsed number | `true`/`false` parse | the parsed instant, read as UTC |
| A string that does not parse | itself | empty | empty | empty |
| An object or an array | empty | empty | empty | empty |

**Raw and presented values.** `absent_value` and a `person` handle change what
a reader is *shown*: the substitute stands in for an empty value, and a handle
resolves to the name identity knows. Anything deciding whether two records are
the same record reads the value before both — so a row identity over a field
with a substitute does not collapse two records that merely lack the key, and
one over a person handle does not collapse two accounts of one human into one
event.

**What empty means to a calculation.** `count` over a field counts records
where it is not empty, while `count` over no field counts records. `sum` and
`avg` skip empty values, so an unconvertible number lowers neither a total nor
an average towards zero. `min` and `max` ignore them. A filter on a field
matches only records where the value is present and comparable, except a
filter against a field's declared `absent_value`, which matches exactly the
records where it is empty. A grouping shows empty as the declared
`absent_value` where one is declared, and as an empty group label otherwise.
A record whose clock is empty falls outside every window, and a windowed run
reports how many such records it left out.

**This is not how the service read a payload before.** Until now a missing key
read as a zero, because the extraction functions have no other empty value to
return, and a comparison against an absent key compared against an empty
string. Under the contract above both read as empty. A total is unaffected, but
an average over records that lack the key rises, a count of that field falls,
and an inequality stops matching those records. The change is deliberate: a
zero nobody wrote is not a measurement. Nothing reconciles it with what a stand
answered before, because no metric written before this change is carried over —
see [the datasets feature](../feature-datasets/FEATURE.md).

**Worked examples.** The shapes below are the contract this feature adds; the
emitted document under its drift gate,
[openapi.json](../../../components/backend/insight-v3-core/openapi.json), is
generated from the implementation and describes the service as it stands today,
so it follows these rather than defining them.

A declaration, as `PUT /v1/datasets/qa_test_runs` carries it and
`GET /v1/datasets/qa_test_runs` answers it:

```json
{
  "title": "QA test runs",
  "description": "One record per test in one CI run.",
  "fields": [
    { "name": "run_id", "path": "run_id", "type": "string", "role": "dimension" },
    { "name": "suite", "path": "suite", "type": "string", "role": "dimension" },
    { "name": "test_name", "path": "test.name", "type": "string", "role": "dimension" },
    { "name": "status", "path": "status", "type": "string", "role": "dimension",
      "description": "passed, failed or skipped" },
    { "name": "branch", "path": "branch", "type": "string", "role": "dimension",
      "absent_value": "(no branch)" },
    { "name": "duration_ms", "path": "duration_ms", "type": "int", "role": "measurable",
      "description": "Milliseconds." },
    { "name": "started_at", "path": "started_at", "type": "datetime", "role": "time",
      "default_clock": true }
  ],
  "row_identity": ["run_id", "test_name"]
}
```

A metric over it, as `PUT /v1/metrics/pass_rate_by_day` carries it. `field`
names a declared field; `as_name` names a column this metric produces, and
`group_by` and `order_by` name those:

```json
{
  "dataset": "qa_test_runs",
  "fields": [
    { "field": "status", "agg": "count", "as_name": "total" },
    { "field": "status", "agg": "count", "as_name": "passed",
      "when": [{ "field": "status", "op": "eq", "value": "passed" }] },
    { "divide": ["passed", "total"], "percent": true, "as_name": "pass_rate" }
  ],
  "order_by": { "field": "pass_rate", "direction": "desc" }
}
```

`GET /v1/metrics/pass_rate_by_day` answers the stored body beside what a reader
needs to draw it. `clock` is the addition this feature makes: the portal reads
it instead of looking for a clock inside the body, so a metric that declares
none but inherits one is still windowed.

```json
{
  "name": "pass_rate_by_day",
  "body": { "dataset": "qa_test_runs", "fields": ["..."] },
  "clock": { "field": "started_at", "source": "dataset" },
  "columns": ["bucket", "total", "passed", "pass_rate"]
}
```

`"source"` is `"metric"` when the body names its own clock, and `"dataset"`
when it inherits the default. A metric whose dataset declares no clock answers
`"clock": null`, and a card over it says it covers all time.

`POST /v1/metrics/pass_rate_by_day/run` answers the rows, with `undated`
counting what the window left out for want of a clock value:

```json
{
  "columns": ["bucket", "total", "passed", "pass_rate"],
  "rows": [["2026-09-10T00:00:00Z", 412, 380, 92.2]],
  "percents": ["pass_rate"],
  "undated": 3
}
```

A refusal carries every violation at once, each naming where it is and what
would have been admissible:

```json
{
  "title": "The dataset declaration is not valid",
  "violations": [
    { "field": "fields[5].type", "reason": "UNKNOWN",
      "detail": "`number` is not a type; admissible: string, int, float, bool, datetime" },
    { "field": "row_identity[1]", "reason": "UNKNOWN",
      "detail": "`test` is not a declared field; declared: run_id, suite, test_name, status, branch, duration_ms, started_at" }
  ]
}
```

## 4. Additional context

### Test strategy

Five families, matching what Insight already runs in CI:

- Unit tests per crate and per frontend package, line coverage above 80%.
- API contract tests against the OpenAPI spec, covering the status codes each endpoint declares.
- Metric tests — one per built-in metric definition, as today. Metrics created at runtime are validated when created; there is no test per user-created metric.
- End-to-end UI journeys against the deployed stand.
- Dependency, code, secret and image scans on every pull request.

Metrics come from three sources: the dev stand during regression, our own usage, and customer stands that opt in.

### Open questions

- Can a customer bring their own AI subscription — a team plan of their own — instead of using ours?

## 5. Traceability

- **PRD**: [PRD.md](./PRD.md)
- **DECOMPOSITION**: [DECOMPOSITION.md](./DECOMPOSITION.md)
- **ADRs**: [ADR/](./ADR/)

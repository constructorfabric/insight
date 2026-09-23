---
status: proposed
date: 2026-09-15
---

# Feature: Datasets as the Unit of Ingest and Query

- [ ] `p1` - **ID**: `cpt-insightspec-v3-featstatus-datasets`

<!-- toc -->

- [1. Feature Context](#1-feature-context)
  - [1.1 Overview](#11-overview)
  - [1.2 Purpose](#12-purpose)
  - [1.3 Actors](#13-actors)
  - [1.4 References](#14-references)
- [2. Actor Flows (CDSL)](#2-actor-flows-cdsl)
  - [Create a Dataset](#create-a-dataset)
  - [Send Records Into a Dataset](#send-records-into-a-dataset)
  - [Browse Datasets in the Portal](#browse-datasets-in-the-portal)
  - [Remove a Dataset](#remove-a-dataset)
  - [Author a Metric Over a Dataset](#author-a-metric-over-a-dataset)
  - [Ask the Assistant About a Dataset](#ask-the-assistant-about-a-dataset)
  - [Edit a Definition by Hand](#edit-a-definition-by-hand)
  - [Read a Board Over a Window](#read-a-board-over-a-window)
- [3. Processes / Business Logic (CDSL)](#3-processes--business-logic-cdsl)
  - [Validate a Declaration](#validate-a-declaration)
  - [Validate a Metric Against a Declaration](#validate-a-metric-against-a-declaration)
  - [Resolve a Metric's Effective Clock](#resolve-a-metrics-effective-clock)
  - [Serialize the Writers of One Dataset](#serialize-the-writers-of-one-dataset)
  - [Take an Operation on a Dataset](#take-an-operation-on-a-dataset)
  - [Finish an Operation on a Dataset](#finish-an-operation-on-a-dataset)
  - [Decide Whether a Table Is Ours](#decide-whether-a-table-is-ours)
  - [Provision a Dataset's Table](#provision-a-datasets-table)
  - [Resolve a Field to Its Read Expression](#resolve-a-field-to-its-read-expression)
  - [Collapse Duplicates by Row Identity](#collapse-duplicates-by-row-identity)
  - [Compile a Metric Over a Dataset](#compile-a-metric-over-a-dataset)
  - [Dependents of a Dataset](#dependents-of-a-dataset)
  - [Revalidate the Dependents of a Changed Declaration](#revalidate-the-dependents-of-a-changed-declaration)
  - [Describe Datasets for the Assistant and MCP](#describe-datasets-for-the-assistant-and-mcp)
- [4. States (CDSL)](#4-states-cdsl)
  - [Dataset Lifecycle](#dataset-lifecycle)
- [5. Definitions of Done](#5-definitions-of-done)
  - [Declaration Store and Validation](#declaration-store-and-validation)
  - [A Dataset Says What It Is Over](#a-dataset-says-what-it-is-over)
  - [A Relation Is Read and Never Owned](#a-relation-is-read-and-never-owned)
  - [Datasets Own Their Own Database](#datasets-own-their-own-database)
  - [One Owner per Operation, and Every Step Repeatable](#one-owner-per-operation-and-every-step-repeatable)
  - [Operations Can See and Tune What This Adds](#operations-can-see-and-tune-what-this-adds)
  - [Only a Ready Dataset Is Reachable](#only-a-ready-dataset-is-reachable)
  - [Ingest Addresses a Dataset](#ingest-addresses-a-dataset)
  - [Metrics Read Datasets Only](#metrics-read-datasets-only)
  - [An Inherited Clock Is Visible to the Reader](#an-inherited-clock-is-visible-to-the-reader)
  - [Duplicates Collapse on Read](#duplicates-collapse-on-read)
  - [Assistant and MCP Read Declarations](#assistant-and-mcp-read-declarations)
  - [Portal Dataset Catalogue](#portal-dataset-catalogue)
  - [One Editor for Every Definition](#one-editor-for-every-definition)
  - [Refusals Say Where They Belong](#refusals-say-where-they-belong)
  - [Existing Streams and Definitions Are Not Carried Over](#existing-streams-and-definitions-are-not-carried-over)
- [6. Acceptance Criteria](#6-acceptance-criteria)
- [7. Testing](#7-testing)

<!-- /toc -->

## 1. Feature Context

- [ ] `p1` - `cpt-insightspec-v3-feature-datasets`

### 1.1 Overview

A dataset is what an administrator creates before anything is sent, what a connector writes into, what the portal shows, what the assistant is told about, and the only thing a custom metric reads. This feature adds the dataset entity to the custom surfaces of `insight-v3-core`, moves ingest and metric compilation onto it, and gives the Custom zone a dataset catalogue.

### 1.2 Purpose

Today a stream is a physical table that appears on the first write of anyone holding the ingest token, describes nothing about its rows, cannot be viewed or removed from the product, counts a re-sent record twice, and leaves the assistant to guess field names and types from a sample of recent rows. Every metric over the stream repeats how to extract each field from the payload, so a change at the source is a change to every metric. Nothing in the read path is a place where "may this caller read these rows" could be asked.

The feature implements the decision in [ADR-0009](../ADR/0009-a-dataset-is-the-unit-of-ingest-and-query.md): the declaration of the data lives in one dataset, the rows keep their raw JSON shape, and everything that reads or writes rows goes through the dataset.

**Scope of this iteration**: a dataset says what it is over, and it is one of two things.

*Over a stream*: records are sent into it, this service owns the table they land in, they are stored whole as JSON and read through the declared fields, and duplicates collapse on read by the declared row identity.

*Over a relation*: it names a relation the warehouse already builds — `database` and `table` — and this service only ever reads it. Nothing is provisioned when it is declared and nothing is ever dropped when it is removed; no records may be sent into it; its rows carry neither an identity of their own nor an instant they arrived; and the relation decides for itself which of its rows are current, so no row identity is declared over one.

Both ways: declarations replaced on write like every other definition, without versions; datasets created and removed by administrators only, in the portal and over the API; metrics, the assistant and MCP read the declaration and cannot tell the two apart except where the difference is the point; the portal reads a metric's effective clock instead of guessing it from the metric body.

**The portal also gains a hand editor for every definition.** A dataset needs one, and a metric, a widget and a dashboard have never had one — each is authored only by the assistant or an agent today. They are one editor over four descriptions rather than four editors, because what differs between them is the shape of the document, not the act of editing it. Typed columns, declaration history, access policies, anything list-shaped over a value that is not a scalar ([#3499](https://github.com/constructorfabric/insight/issues/3499)) and a credential of the lifecycle's own — one that lets a dataset be declared or removed over the API without a portal session, and is rotated apart from the ingest token ([#3496](https://github.com/constructorfabric/insight/issues/3496)) — are later iterations. The analytics service and its own metrics are untouched.

**Nothing is carried over.** This is a clean break: no stream table is adopted, moved or renamed, and no stored metric is rewritten. On a stand that already holds them, the tables stay where they are and the metrics stay as they are, refused from the moment this ships because they name a table. Datasets are declared anew and records are sent again through the ordinary ingest path. Whoever sends records addresses a dataset from that point on; there is no compatibility alias.

**Requirements**:

- `cpt-insightspec-v3-fr-create-dataset`
- `cpt-insightspec-v3-fr-remove-dataset`
- `cpt-insightspec-v3-fr-view-dataset`
- `cpt-insightspec-v3-fr-ingest-into-dataset`
- `cpt-insightspec-v3-fr-metrics-over-datasets`
- `cpt-insightspec-v3-fr-assistant-reads-datasets`
- `cpt-insightspec-v3-fr-accept-data`
- `cpt-insightspec-v3-fr-author-by-hand`
- `cpt-insightspec-v3-fr-create-metrics`
- `cpt-insightspec-v3-fr-create-widgets`
- `cpt-insightspec-v3-fr-create-dashboards`
- `cpt-insightspec-v3-nfr-security`
- `cpt-insightspec-v3-nfr-reliability`
- `cpt-insightspec-v3-nfr-efficiency`
- `cpt-insightspec-v3-nfr-versatility`

**Principles**:

- `cpt-insightspec-v3-principle-shape-agnostic-ingest`
- `cpt-insightspec-v3-principle-schema-driven`

**Rollout and what it costs.** Nothing migrates, so there is nothing to undo in the data: a stand's stream tables and stored definitions are left exactly where they are. What the release does change is what the service answers, and that is not reversible by itself — every custom metric a stand held is refused from the moment it ships, because each one names a table. An installation that wants what it had must declare the datasets again and send the records again. The only clean case is a fresh install, where there is nothing to lose and the step is a no-op.

**Not applicable in this feature**: caching, because declarations are read from the definitions store on every request as the other definitions are; events and messaging, because every path is a synchronous request; external integrations beyond the two stores the service already holds; tenant scoping, because the service is single-tenant and no dataset carries a tenant field yet; accessibility beyond the Custom zone's existing components, which the catalogue reuses; regulatory compliance, because the feature moves no personal data and adds no new place it is shown — a person handle on a field resolves through the identity lookup the compiler already performs.

### 1.3 Actors

| Actor | Role in Feature |
|-------|-----------------|
| `cpt-insightspec-v3-actor-administrator` | Creates, reviews and removes datasets, and is the only role the Custom zone opens to, so every flow here is theirs |
| `cpt-insightspec-v3-actor-dashboard-author` | The reader these surfaces are built for; in this iteration the role is held by an administrator, because the zone opens to no one else |
| `cpt-insightspec-v3-actor-external-connector` | Sends records into a dataset that already exists |
| `cpt-insightspec-v3-actor-internal-connector` | Same contract as the external connector |

### 1.4 References

- **PRD**: [PRD.md](../PRD.md)
- **Design**: [DESIGN.md](../DESIGN.md)
- **Decisions**: [ADR-0009](../ADR/0009-a-dataset-is-the-unit-of-ingest-and-query.md) (this feature), [ADR-0006](../ADR/0006-a-table-per-ingest-stream.md) (superseded; the physical table per uploaded dataset stays), [ADR-0002](../ADR/0002-static-ingest-token.md) (the static-token path both credentials follow)
- **Dependencies**: `cpt-insightspec-v3-feature-data-ingestion` (the ingest endpoint and the definitions store this feature extends)
- **Tracking**: constructorfabric/insight#3408

## 2. Actor Flows (CDSL)

**Use cases**: `cpt-insightspec-v3-usecase-dataset-to-dashboard`

### Create a Dataset

- [ ] `p1` - **ID**: `cpt-insightspec-v3-flow-datasets-create`

**Actor**: `cpt-insightspec-v3-actor-administrator`

**Success Scenarios**:
- A new dataset is declared, its table exists in the datasets database, and it appears in the catalogue and in the assistant's context at once
- A create whose table step failed leaves the name claimed and nothing readable, and repeating it once the failed attempt's lease lapses completes it
- A replacement of a ready dataset changes no table, so it commits as one transaction and cannot interleave with a removal
- An existing dataset's declaration is replaced by one every dependent metric still validates against

**Error Scenarios**:
- The name is not an identifier, or is one of the catalogue's reserved path segments
- The body fails validation: an unknown type, a default clock that is not a datetime, two default clocks, a row identity naming an undeclared field
- The replacement would change what the dataset is over, which is not a change a replacement makes
- The declaration names a relation the warehouse does not have, a column it does not hold, or one whose engine a plain read cannot count once per row
- The replacement leaves a stored metric invalid — a field it reads is gone, retyped, or the default clock it inherits has changed
- The replacement would silently move numbers: a field a metric reads now reads from somewhere else, or the row identity changed
- The name is held by a removal, or another attempt owns an operation on it, so it is not free
- The caller is not an administrator; the ingest token does not open this

**Steps**:
1. [ ] - `p1` - Administrator submits a declaration: name, title, description, fields, default clock, row identity - `inst-ds-create-submit`
2. [ ] - `p1` - API: PUT /v1/datasets/{name} (declaration body; a session carrying the admin role) - `inst-ds-create-api`
3. [ ] - `p1` - **IF** the caller does not hold the admin role — the ingest token is not a way in, and presenting it here is refused - `inst-ds-create-authz`
   1. [ ] - `p1` - **RETURN** permission denied, nothing written - `inst-ds-create-authz-reject`
4. [ ] - `p1` - Validate the body per `cpt-insightspec-v3-algo-datasets-validate-declaration` - `inst-ds-create-validate`
   1. [ ] - `p1` - **IF** the declaration is over a relation - `inst-ds-create-relation`
      1. [ ] - `p1` - Warehouse: SELECT system.columns (the columns of the named relation) - `inst-ds-create-relation-columns`
      2. [ ] - `p1` - **IF** it holds none — the warehouse has no such relation - `inst-ds-create-relation-absent`
         1. [ ] - `p1` - Record a violation against `source.table` - `inst-ds-create-relation-absent-violation`
      3. [ ] - `p1` - Warehouse: SELECT system.tables (the relation's engine) - `inst-ds-create-relation-engine`
      4. [ ] - `p1` - **IF** the engine is not one a plain read counts once per row - `inst-ds-create-relation-collapses`
         1. [ ] - `p1` - Record a violation against `source.table`, naming a view over it as the way through - `inst-ds-create-relation-collapses-violation`
      5. [ ] - `p1` - **FOR EACH** declared field whose column the relation does not hold - `inst-ds-create-relation-column-unknown`
         1. [ ] - `p1` - Record a violation against that field's `column` - `inst-ds-create-relation-column-violation`
5. [ ] - `p1` - **IF** validation reports violations - `inst-ds-create-invalid`
   1. [ ] - `p1` - **RETURN** every violation with its field path, nothing written - `inst-ds-create-invalid-reject`
6. [ ] - `p1` - DB: BEGIN and hold the dataset's row per `cpt-insightspec-v3-algo-datasets-serialize` - `inst-ds-create-hold`
7. [ ] - `p1` - **IF** the row is ready — this is a replacement, and a replacement changes no table - `inst-ds-create-replace`
   1. [ ] - `p1` - **IF** any dependent metric fails to validate against the submitted declaration, per `cpt-insightspec-v3-algo-datasets-revalidate-dependents` - `inst-ds-create-breaking`
      1. [ ] - `p1` - **RETURN** conflict naming each metric and why it would break, nothing written - `inst-ds-create-breaking-reject`
   2. [ ] - `p1` - DB: UPDATE datasets (the body) and COMMIT — the whole replacement is one transaction, so no removal can interleave with it - `inst-ds-create-replace-store`
   3. [ ] - `p1` - **RETURN** the stored declaration - `inst-ds-create-replace-return`
8. [ ] - `p1` - **ELSE** the dataset must be brought into being, which needs a table and therefore an operation that outlives one transaction - `inst-ds-create-new`
   1. [ ] - `p1` - Take the operation per `cpt-insightspec-v3-algo-datasets-take-operation`, which writes the row as claimed and returns this attempt's token - `inst-ds-create-take`
   2. [ ] - `p1` - **IF** the operation is refused because another attempt owns it - `inst-ds-create-busy`
      1. [ ] - `p1` - **RETURN** conflict: an operation on this dataset is in progress - `inst-ds-create-busy-reject`
   3. [ ] - `p1` - **IF** the operation is refused because a removal owns the name - `inst-ds-create-removing`
      1. [ ] - `p1` - **RETURN** conflict: the name is not free - `inst-ds-create-removing-reject`
9. [ ] - `p1` - Provision a table of this attempt's own generation per `cpt-insightspec-v3-algo-datasets-provision-table` - `inst-ds-create-table`
10. [ ] - `p1` - **IF** provisioning failed - `inst-ds-create-provision-failed`
    1. [ ] - `p1` - **RETURN** a server error; the row stays claimed and the request is repeatable once this attempt's lease lapses - `inst-ds-create-provision-failed-return`
11. [ ] - `p1` - Finish the operation per `cpt-insightspec-v3-algo-datasets-finish-operation`, storing the body, the table it provisioned and the ready state - `inst-ds-create-store`
12. [ ] - `p1` - **IF** finishing reports the attempt is stale - `inst-ds-create-stale`
    1. [ ] - `p1` - Drop the table this attempt provisioned, which nothing points at, and **RETURN** conflict: another attempt owns the dataset now - `inst-ds-create-stale-return`
13. [ ] - `p1` - **RETURN** the stored declaration; the portal refreshes the catalogue and the rail - `inst-ds-create-return`

### Send Records Into a Dataset

- [ ] `p1` - **ID**: `cpt-insightspec-v3-flow-datasets-ingest`

**Actor**: `cpt-insightspec-v3-actor-external-connector`

**Success Scenarios**:
- A record of any JSON shape lands in the named dataset's table, stored whole
- A record whose payload lacks a declared field, or carries a key no field declares, is stored the same way

**Error Scenarios**:
- The ingest token is missing or wrong
- The dataset is not ready — absent, still being created, or being removed — or the name is misspelled: nothing is created
- The request names no dataset, or still names a table as earlier releases accepted
- The dataset was removed between the lookup and the write: the record is refused, not accepted
- The dataset reads a relation the warehouse builds: there is nowhere to write, and the refusal says so rather than saying the dataset is absent

**Steps**:
1. [ ] - `p1` - Connector sends one record naming the dataset - `inst-ds-ingest-send`
2. [ ] - `p1` - API: POST /v1/raw-data ({dataset, raw_data}) - `inst-ds-ingest-api`
3. [ ] - `p1` - **IF** the ingest token does not verify - `inst-ds-ingest-token`
   1. [ ] - `p1` - **RETURN** unauthenticated - `inst-ds-ingest-token-reject`
4. [ ] - `p1` - DB: SELECT datasets (the declaration under the given name, and its state) - `inst-ds-ingest-lookup`
5. [ ] - `p1` - **IF** the dataset is over a relation — there is nowhere to write, and it is not the dataset that is missing - `inst-ds-ingest-relation`
   1. [ ] - `p1` - **RETURN** a refusal against the field that named it, saying what the dataset is - `inst-ds-ingest-relation-refuse`
6. [ ] - `p1` - **IF** the dataset is not ready — absent, claimed by an unfinished create, or removing - `inst-ds-ingest-unknown`
   1. [ ] - `p1` - **RETURN** not found naming the dataset; no table is created, and a claimed dataset has no table to write into - `inst-ds-ingest-unknown-reject`
7. [ ] - `p1` - ClickHouse: INSERT into the table the declaration names (id, table_name, raw_data, received_at); the row is read without a hold, because a write per record cannot wait on a lock a removal might be holding - `inst-ds-ingest-insert`
8. [ ] - `p1` - **IF** the insert reports the table is gone, which is what a removal racing this write looks like - `inst-ds-ingest-vanished`
   1. [ ] - `p1` - **RETURN** not found naming the dataset, never accepted: a record that reached no table was not stored - `inst-ds-ingest-vanished-reject`
9. [ ] - `p1` - **RETURN** accepted - `inst-ds-ingest-return`

### Browse Datasets in the Portal

- [ ] `p1` - **ID**: `cpt-insightspec-v3-flow-datasets-browse`

**Actor**: `cpt-insightspec-v3-actor-administrator`

**Success Scenarios**:
- The catalogue lists datasets by title with search over name and declaration body, paged like the other catalogues
- A dataset page shows its title, description, fields with roles and types, default time field, row identity, the records it holds as a table of its declared fields, and the metrics that read it
- The records table pages through the whole dataset and orders by any declared field or by the instant records arrived, so a reader checking that what arrives is what was meant can look past the newest few

**Error Scenarios**:
- The caller is not an administrator: the Custom zone is not shown, and the API refuses
- The dataset does not exist, or is not ready: the page says so instead of failing

**Steps**:
1. [ ] - `p1` - Author opens the Datasets catalogue in the Custom zone - `inst-ds-browse-open`
2. [ ] - `p1` - API: GET /v1/datasets (q, limit, offset; the ready datasets and their total, never one mid-create or mid-removal) - `inst-ds-browse-list-api`
3. [ ] - `p1` - Author opens one dataset - `inst-ds-browse-open-one`
4. [ ] - `p1` - API: GET /v1/datasets/{name} (the declaration, and not found unless the dataset is ready) - `inst-ds-browse-get-api`
5. [ ] - `p1` - **IF** the dataset is not found or not ready - `inst-ds-browse-missing`
   1. [ ] - `p1` - **RETURN** an empty state naming the dataset, asking for neither its records nor its dependents - `inst-ds-browse-missing-state`
6. [ ] - `p1` - API: GET /v1/datasets/{name}/records (one page of records, ordered as asked; arrival order newest first when nothing is asked) - `inst-ds-browse-preview-api`
   1. [ ] - `p1` - **IF** `order_by` names neither a declared field nor the arrival column - `inst-ds-browse-order-unknown`
      1. [ ] - `p1` - Refuse against `order_by`, listing the fields the dataset declares - `inst-ds-browse-order-refuse`
   2. [ ] - `p1` - **IF** `limit` is outside 1 to the configured cap - `inst-ds-browse-limit-range`
      1. [ ] - `p1` - Refuse against `limit`, naming the range - `inst-ds-browse-limit-refuse`
   3. [ ] - `p1` - **RETURN** the records, the total behind them, and the page size applied - `inst-ds-browse-page-return`
7. [ ] - `p1` - API: GET /v1/datasets/{name}/dependents (every metric whose body names this dataset, exactly, unpaged) - `inst-ds-browse-dependents-api`
8. [ ] - `p1` - **RETURN** the page: declaration, records, dependents, and the remove action - `inst-ds-browse-return`

### Remove a Dataset

- [ ] `p1` - **ID**: `cpt-insightspec-v3-flow-datasets-remove`

**Actor**: `cpt-insightspec-v3-actor-administrator`

**Success Scenarios**:
- A dataset nothing reads is removed with its rows and leaves the catalogue, the rail and the assistant's context
- A removal whose table drop failed leaves the dataset removing, and repeating it once the failed attempt's lease lapses completes it
- Two removals issued together drop the table once; the second reports the removal already under way

**Error Scenarios**:
- A metric still reads the dataset: the refusal names the metrics
- No dataset of that name is declared: nothing is dropped, whatever tables exist
- A create of that dataset is under way: removing it waits until that settles

**Steps**:
1. [ ] - `p1` - Administrator removes the dataset from its page or over the API - `inst-ds-remove-submit`
2. [ ] - `p1` - API: DELETE /v1/datasets/{name} (a session carrying the admin role) - `inst-ds-remove-api`
3. [ ] - `p1` - **IF** the caller does not hold the admin role — the ingest token is not a way in, and presenting it here is refused - `inst-ds-remove-authz`
   1. [ ] - `p1` - **RETURN** permission denied - `inst-ds-remove-authz-reject`
4. [ ] - `p1` - DB: BEGIN and hold the dataset's row per `cpt-insightspec-v3-algo-datasets-serialize` - `inst-ds-remove-lock`
5. [ ] - `p1` - **IF** no row is stored under the name - `inst-ds-remove-missing`
   1. [ ] - `p1` - **RETURN** not found; no table is dropped, because only a declaration says which table is ours - `inst-ds-remove-missing-reject`
6. [ ] - `p1` - **IF** the row is claimed by a create that never finished - `inst-ds-remove-claimed`
   1. [ ] - `p1` - Remove it as an abandoned create: nothing may read it, so it has no dependents to protect, and the table to drop is whichever generation its claim recorded, if any - `inst-ds-remove-claimed-note`
7. [ ] - `p1` - Collect the metrics reading the dataset per `cpt-insightspec-v3-algo-datasets-dependents`, inside the same transaction, so a metric written concurrently either precedes this read or waits for it - `inst-ds-remove-dependents`
8. [ ] - `p1` - **IF** any metric reads it - `inst-ds-remove-in-use`
   1. [ ] - `p1` - **RETURN** conflict naming the metrics, nothing removed - `inst-ds-remove-in-use-reject`
9. [ ] - `p1` - Take the operation per `cpt-insightspec-v3-algo-datasets-take-operation`, marking the row removing and returning this attempt's token - `inst-ds-remove-take`
10. [ ] - `p1` - **IF** the operation is refused - `inst-ds-remove-busy`
    1. [ ] - `p1` - **IF** a removal holds it - `inst-ds-remove-busy-removal`
       1. [ ] - `p1` - **RETURN** accepted without acting: this removal is already under way - `inst-ds-remove-busy-removal-return`
    2. [ ] - `p1` - **ELSE** a create holds it - `inst-ds-remove-busy-create`
       1. [ ] - `p1` - **RETURN** conflict: the dataset is being created, and removing it is a decision to take once that settles - `inst-ds-remove-busy-create-return`
11. [ ] - `p1` - **IF** the row records a table - `inst-ds-remove-has-table`
    1. [ ] - `p1` - Confirm it is ours per `cpt-insightspec-v3-algo-datasets-table-is-ours`, then ClickHouse: DROP TABLE IF EXISTS that table — that one and no other, so an attempt whose drop arrives late cannot reach a table some later dataset provisioned - `inst-ds-remove-table`
12. [ ] - `p1` - **IF** the drop failed - `inst-ds-remove-drop-failed`
    1. [ ] - `p1` - **RETURN** a server error; the dataset stays removing and the request is repeatable once this attempt's lease lapses - `inst-ds-remove-drop-failed-return`
13. [ ] - `p1` - Finish the operation per `cpt-insightspec-v3-algo-datasets-finish-operation`, deleting the row - `inst-ds-remove-declaration`
14. [ ] - `p1` - **IF** finishing reports the attempt is stale - `inst-ds-remove-stale`
    1. [ ] - `p1` - **RETURN** conflict without deleting anything: another attempt owns the dataset now - `inst-ds-remove-stale-return`
15. [ ] - `p1` - **RETURN** removed; the portal refreshes the catalogue and the rail - `inst-ds-remove-return`

### Author a Metric Over a Dataset

- [ ] `p1` - **ID**: `cpt-insightspec-v3-flow-datasets-author-metric`

**Actor**: `cpt-insightspec-v3-actor-administrator`

**Success Scenarios**:
- A metric names a dataset, reads its fields by their declared names, and orders and groups by the output names it gives them; the widgets and dashboards over it are unchanged
- A metric with no time field of its own is windowed by the dataset's default time field

**Error Scenarios**:
- The body names a table or a database: metrics read datasets only
- The dataset does not exist, is being removed, or a field, filter, grouping or time reference names an undeclared field
- A sum or an average is applied to a field that is not numeric, or the clock names a field that is not a datetime

**Steps**:
1. [ ] - `p1` - Author submits a metric body naming `dataset` and fields by their declared names - `inst-ds-metric-submit`
2. [ ] - `p1` - API: PUT /v1/metrics/{name} (also reached by the chat's create tool and the MCP put_metric tool) - `inst-ds-metric-api`
3. [ ] - `p1` - **IF** the body names `table` or `database` - `inst-ds-metric-table`
   1. [ ] - `p1` - **RETURN** refusal saying metrics read datasets, with the dataset catalogue as the admissible set - `inst-ds-metric-table-reject`
4. [ ] - `p1` - DB: BEGIN, then SELECT datasets holding the named dataset's row against a concurrent removal per `cpt-insightspec-v3-algo-datasets-serialize` - `inst-ds-metric-dataset`
5. [ ] - `p1` - **IF** the dataset is not ready - `inst-ds-metric-unknown`
   1. [ ] - `p1` - **RETURN** refusal naming the dataset and listing the ones a metric may read - `inst-ds-metric-unknown-reject`
6. [ ] - `p1` - Check the body against the declaration per `cpt-insightspec-v3-algo-datasets-validate-metric` - `inst-ds-metric-fields`
7. [ ] - `p1` - **IF** any violation was recorded - `inst-ds-metric-invalid`
   1. [ ] - `p1` - **RETURN** every violation, nothing stored - `inst-ds-metric-invalid-reject`
8. [ ] - `p1` - DB: UPSERT metrics (the body as submitted) and COMMIT, so the metric and the dataset it depends on cannot disagree - `inst-ds-metric-store`
9. [ ] - `p1` - **RETURN** the stored metric with its effective clock, per `cpt-insightspec-v3-algo-datasets-effective-clock`; a run compiles per `cpt-insightspec-v3-algo-datasets-compile-metric` - `inst-ds-metric-return`

### Ask the Assistant About a Dataset

- [ ] `p1` - **ID**: `cpt-insightspec-v3-flow-datasets-chat`

**Actor**: `cpt-insightspec-v3-actor-administrator`

**Success Scenarios**:
- The assistant answers a question from a dataset's rows, naming fields exactly as declared
- The assistant builds a metric, widgets and a dashboard over a dataset in one turn
- An MCP client lists and describes datasets and runs a metric over one

**Error Scenarios**:
- The model proposes a query over a table: the refusal is handed back for one repair round
- No dataset exists yet: the assistant says so instead of inventing one
- The model asks to create a dataset: refused, datasets are created by administrators

**Steps**:
1. [ ] - `p1` - Author sends a message in the Custom zone's chat panel - `inst-ds-chat-send`
2. [ ] - `p1` - API: POST /v1/chat (message and the thread so far) - `inst-ds-chat-api`
3. [ ] - `p1` - Build the system prompt's data section per `cpt-insightspec-v3-algo-datasets-describe` - `inst-ds-chat-context`
4. [ ] - `p1` - Model calls one tool: look_up (describe named datasets), answer (a query over one dataset) or create (definitions over datasets) - `inst-ds-chat-tool`
5. [ ] - `p1` - **IF** the proposal names a table or a database, or a dataset the catalogue does not hold - `inst-ds-chat-table`
   1. [ ] - `p1` - Hand the refusal back to the model for one repair round - `inst-ds-chat-repair`
6. [ ] - `p1` - **IF** the proposal creates a dataset - `inst-ds-chat-create-dataset`
   1. [ ] - `p1` - **RETURN** the reply with the refusal: datasets are created by an administrator - `inst-ds-chat-create-dataset-reject`
7. [ ] - `p1` - Run or store the proposal through the same paths the portal uses - `inst-ds-chat-apply`
8. [ ] - `p1` - **RETURN** the reply, the rows or the created names - `inst-ds-chat-return`

### Edit a Definition by Hand

- [ ] `p1` - **ID**: `cpt-insightspec-v3-flow-datasets-edit-definition`

**Actor**: `cpt-insightspec-v3-actor-administrator`

**Success Scenarios**:
- A dataset is declared from nothing, field by field, and stored
- A stored widget is opened, its type changed, and the fields offered change with it
- A declaration carried from another installation is pasted as text and stored unchanged

**Error Scenarios**:
- The text does not parse, so there is nothing to send
- The service refuses the document, naming places inside it
- The service refuses the document as a whole, naming no place in it

**Steps**:
1. [ ] - `p1` - Administrator opens the editor for a kind, on a new definition or a stored one - `inst-ds-edit-open`
2. [ ] - `p1` - **IF** a stored definition was opened - `inst-ds-edit-stored`
   1. [ ] - `p1` - API: GET /v1/{kind}/{name} (the body as stored, which the editor holds as the document) - `inst-ds-edit-read`
3. [ ] - `p1` - Administrator edits the fields, or the text beside them - `inst-ds-edit-change`
4. [ ] - `p1` - **IF** the fields were edited - `inst-ds-edit-from-fields`
   1. [ ] - `p1` - The text is rewritten from the document - `inst-ds-edit-text-follows`
5. [ ] - `p1` - **ELSE IF** the text parses - `inst-ds-edit-from-text`
   1. [ ] - `p1` - The document becomes what the text says, and the fields follow - `inst-ds-edit-fields-follow`
6. [ ] - `p1` - **ELSE** - `inst-ds-edit-unparsed`
   1. [ ] - `p1` - Leave the fields as they stand, say the text does not parse, and refuse to send - `inst-ds-edit-blocked`
7. [ ] - `p1` - Administrator sends the document - `inst-ds-edit-send`
8. [ ] - `p1` - API: PUT /v1/{kind}/{name} (the document, including any property the fields do not know) - `inst-ds-edit-api`
9. [ ] - `p1` - **IF** the service refuses with places inside the document - `inst-ds-edit-violations`
   1. [ ] - `p1` - **RETURN** each message beside the place it names, in the fields and in the text - `inst-ds-edit-violations-shown`
10. [ ] - `p1` - **IF** the service refuses without naming a place - `inst-ds-edit-whole`
    1. [ ] - `p1` - **RETURN** the message above the document - `inst-ds-edit-whole-shown`
11. [ ] - `p1` - **RETURN** the stored definition; the catalogue and the rail refresh - `inst-ds-edit-return`

### Read a Board Over a Window

- [ ] `p1` - **ID**: `cpt-insightspec-v3-flow-datasets-read-board`

**Actor**: `cpt-insightspec-v3-actor-administrator`

**Success Scenarios**:
- A card whose metric names no clock of its own is read over the picked window, because its dataset declares a default time field
- A card whose dataset declares no time field at all says so and ignores the picker

**Error Scenarios**:
- The metric's dataset was removed while the board was open: the card says the dataset is gone rather than drawing nothing

**Steps**:
1. [ ] - `p1` - Author picks a window on a custom dashboard - `inst-ds-board-pick`
2. [ ] - `p1` - API: GET /v1/metrics/{name} (the stored body and its effective clock: the field and whether it came from the metric or the dataset) - `inst-ds-board-metric-api`
3. [ ] - `p1` - **IF** the metric has an effective clock - `inst-ds-board-clocked`
   1. [ ] - `p1` - API: POST /v1/metrics/{name}/run (the window and the bucket mode) - `inst-ds-board-run-windowed`
4. [ ] - `p1` - **ELSE** - `inst-ds-board-clockless`
   1. [ ] - `p1` - Mark the card as covering all time and run it without a window - `inst-ds-board-run-all-time`
5. [ ] - `p1` - **IF** the run reports the dataset is not ready — removed, or mid-removal - `inst-ds-board-missing-dataset`
   1. [ ] - `p1` - **RETURN** a card saying which dataset the metric names and that it is no longer readable - `inst-ds-board-missing-state`
6. [ ] - `p1` - **RETURN** the drawn card - `inst-ds-board-return`

## 3. Processes / Business Logic (CDSL)

### Validate a Declaration

- [ ] `p1` - **ID**: `cpt-insightspec-v3-algo-datasets-validate-declaration`

**Input**: the dataset name from the path and the declaration body

**Output**: a validated declaration, or the complete list of violations with field paths

**Steps**:
1. [ ] - `p1` - Parse the name with the definitions' identifier rule; refuse the catalogue's reserved segments (`metrics`, `widgets`, `dashboards`, `datasets`) - `inst-ds-validate-name`
2. [ ] - `p1` - Require a non-empty title; accept an optional description - `inst-ds-validate-title`
3. [ ] - `p1` - **FOR EACH** field in the declaration - `inst-ds-validate-fields`
   1. [ ] - `p1` - Require a unique identifier name, a non-empty key path, and a type in {string, int, float, bool, datetime} - `inst-ds-validate-field-shape`
   2. [ ] - `p1` - Accept an optional role in {dimension, measurable, time}; the role guides the assistant and the catalogue and never widens or narrows what a metric may do with the field, which its type decides - `inst-ds-validate-role`
   3. [ ] - `p1` - **IF** the field is marked the default clock **AND** its type is not datetime - `inst-ds-validate-clock-type`
      1. [ ] - `p1` - Record a violation - `inst-ds-validate-clock-violation`
   4. [ ] - `p1` - **IF** the field carries `person` **AND** its type is not string - `inst-ds-validate-person`
      1. [ ] - `p1` - Record a violation - `inst-ds-validate-person-violation`
   5. [ ] - `p1` - **IF** the field carries `absent_value` **AND** its type is not string - `inst-ds-validate-absent`
      1. [ ] - `p1` - Record a violation: the substitute stands in for a missing value in a grouping, which reads as text - `inst-ds-validate-absent-violation`
4. [ ] - `p1` - **IF** more than one field is marked the default clock - `inst-ds-validate-default-time`
   1. [ ] - `p1` - Record a violation; a declaration with no datetime field simply has no default clock - `inst-ds-validate-default-time-violation`
5. [ ] - `p1` - **FOR EACH** name in the row identity - `inst-ds-validate-identity`
   1. [ ] - `p1` - **IF** it names no declared field - `inst-ds-validate-identity-field`
      1. [ ] - `p1` - Record a violation - `inst-ds-validate-identity-violation`
6. [ ] - `p1` - **IF** the field name `bucket` is declared - `inst-ds-validate-bucket`
   1. [ ] - `p1` - Record a violation: the name is reserved for a run's time bucket - `inst-ds-validate-bucket-violation`
7. [ ] - `p1` - **RETURN** the declaration when no violation was recorded, otherwise every violation - `inst-ds-validate-return`

### Validate a Metric Against a Declaration

- [ ] `p1` - **ID**: `cpt-insightspec-v3-algo-datasets-validate-metric`

**Input**: a metric body and the declaration of the dataset it names

**Output**: nothing, or the violations that make the metric unanswerable

> A metric uses two kinds of name. A **field reference** names a declared field of the dataset, and appears as the source of each selected field, each filter, and the clock. An **output name** names a column this metric produces, and appears as each selected field's own name and in grouping and ordering. A field computed from others sits in the second space on both sides: its operands are columns earlier fields produce, and it produces one of its own.

**Steps**:
1. [ ] - `p1` - **FOR EACH** field reference - `inst-ds-vm-refs`
   1. [ ] - `p1` - **IF** it names no declared field - `inst-ds-vm-unknown`
      1. [ ] - `p1` - Record a violation naming the reference and the declared field names - `inst-ds-vm-unknown-violation`
   2. [ ] - `p1` - **IF** it is summed or averaged **AND** the declared type is neither int nor float - `inst-ds-vm-numeric`
      1. [ ] - `p1` - Record a violation - `inst-ds-vm-numeric-violation`
   3. [ ] - `p1` - **IF** it is the clock **AND** the declared type is not datetime - `inst-ds-vm-clock`
      1. [ ] - `p1` - Record a violation - `inst-ds-vm-clock-violation`
   4. [ ] - `p1` - **IF** it is a filter **AND** the filter's value does not parse as the declared type - `inst-ds-vm-filter-value`
      1. [ ] - `p1` - Record a violation - `inst-ds-vm-filter-violation`
2. [ ] - `p1` - Admit grouping, filtering, counting, min and max on any type, so a numeric identifier groups as readily as a name - `inst-ds-vm-any-type`
3. [ ] - `p1` - **FOR EACH** output name in grouping and ordering - `inst-ds-vm-outputs`
   1. [ ] - `p1` - **IF** it names no column this metric produces — its selected and computed fields, plus the time bucket a windowed run injects - `inst-ds-vm-output-unknown`
      1. [ ] - `p1` - Record a violation naming the reference and the columns the metric produces; an aggregate's own name is one of them, so ordering by a total the metric computes is admitted - `inst-ds-vm-output-violation`
4. [ ] - `p1` - **IF** the metric asks to be windowed **AND** `cpt-insightspec-v3-algo-datasets-effective-clock` finds none - `inst-ds-vm-clockless`
   1. [ ] - `p1` - Record a violation - `inst-ds-vm-clockless-violation`
5. [ ] - `p1` - **RETURN** every violation recorded - `inst-ds-vm-return`

### Resolve a Metric's Effective Clock

- [ ] `p1` - **ID**: `cpt-insightspec-v3-algo-datasets-effective-clock`

**Input**: a metric body and its dataset's declaration

**Output**: the clock field and where it came from, or nothing

**Steps**:
1. [ ] - `p1` - **IF** the metric names a clock field - `inst-ds-clock-own`
   1. [ ] - `p1` - **RETURN** that field, sourced from the metric - `inst-ds-clock-own-return`
2. [ ] - `p1` - **IF** the declaration marks a default clock - `inst-ds-clock-inherited`
   1. [ ] - `p1` - **RETURN** that field, sourced from the dataset - `inst-ds-clock-inherited-return`
3. [ ] - `p1` - **RETURN** nothing: the metric answers every record whatever window is picked - `inst-ds-clock-none`

### Serialize the Writers of One Dataset

- [ ] `p1` - **ID**: `cpt-insightspec-v3-algo-datasets-serialize`

**Input**: a dataset name and the writing operation about to run

**Output**: a hold on that dataset's row for the rest of the transaction

> Which operations this covers: creating a dataset, replacing its declaration, removing it, and storing a metric that reads it. A metric write holds the row until it commits, so a removal cannot pass its dependency check while that metric is being written. A replacement of a ready dataset needs no table work, so it begins and ends inside one transaction and nothing interleaves with it. No hold spans a warehouse call: a create or a removal that must touch a table owns an operation instead.

**Steps**:
1. [ ] - `p1` - DB: BEGIN a transaction of the definitions store - `inst-ds-serial-transaction`
2. [ ] - `p1` - DB: SELECT the dataset's row for update, so a second writer of the same dataset waits rather than reading a state that is about to change - `inst-ds-serial-hold`
3. [ ] - `p1` - Read the state inside that hold and act on it there; a state read before the hold is worthless - `inst-ds-serial-read`
4. [ ] - `p1` - Release on commit or rollback - `inst-ds-serial-release`

### Take an Operation on a Dataset

- [ ] `p1` - **ID**: `cpt-insightspec-v3-algo-datasets-take-operation`

**Input**: a dataset name and the kind of operation, create or remove

**Output**: this attempt's token, or a refusal naming the operation that holds the dataset

> A dataset that has no row yet is claimed by the insert itself: the name is the key, so of two attempts at one new dataset exactly one inserts and the other is refused. Nothing else in this feature can lock a row that does not exist.

**Steps**:
1. [ ] - `p1` - Hold the row per `cpt-insightspec-v3-algo-datasets-serialize` - `inst-ds-take-hold`
2. [ ] - `p1` - **IF** the row records an operation whose lease has not lapsed **AND** it is not this attempt - `inst-ds-take-busy`
   1. [ ] - `p1` - **RETURN** refused, naming which kind of operation holds the dataset, so the caller can say what is under way rather than guessing - `inst-ds-take-busy-return`
3. [ ] - `p1` - **IF** the row is removing **AND** the operation asked for is create - `inst-ds-take-removing`
   1. [ ] - `p1` - **RETURN** refused: the name belongs to the removal until it finishes - `inst-ds-take-removing-return`
4. [ ] - `p1` - Mint a token for this attempt, and for a create also the generation its table will carry - `inst-ds-take-token`
5. [ ] - `p1` - **IF** no row exists - `inst-ds-take-insert`
   1. [ ] - `p1` - DB: INSERT the row, which fails if another attempt inserted it first because the name is the key; a failed insert is the same refusal as a held operation - `inst-ds-take-insert-write`
6. [ ] - `p1` - **ELSE** DB: UPDATE the row with the state the operation implies — claimed for a create, removing for a removal — this attempt's token, and a lease that lapses after a bounded time - `inst-ds-take-update`
7. [ ] - `p1` - COMMIT - `inst-ds-take-commit`
8. [ ] - `p1` - **RETURN** the token; from here this attempt is the only one that may touch this dataset's tables - `inst-ds-take-return`

> The lease is what makes an abandoned attempt recoverable without making a running one interruptible: a repeat is refused while the lease holds, and only once it lapses may another attempt take over. A lapsed lease never means the first attempt succeeded — it means its outcome no longer counts, which `cpt-insightspec-v3-algo-datasets-finish-operation` enforces.

### Finish an Operation on a Dataset

- [ ] `p1` - **ID**: `cpt-insightspec-v3-algo-datasets-finish-operation`

**Input**: this attempt's token and what it means to write — a ready declaration, or the row's deletion

**Output**: the operation completed, or the report that this attempt is stale

**Steps**:
1. [ ] - `p1` - Hold the row per `cpt-insightspec-v3-algo-datasets-serialize` - `inst-ds-finish-hold`
2. [ ] - `p1` - **IF** the row is gone, or records a token other than this attempt's - `inst-ds-finish-stale`
   1. [ ] - `p1` - **RETURN** stale, writing nothing: this attempt lost its operation while it was working, and completing now would publish a result the dataset has moved past - `inst-ds-finish-stale-return`
3. [ ] - `p1` - **IF** the operation is a create - `inst-ds-finish-create`
   1. [ ] - `p1` - DB: UPDATE datasets (the declaration, the table this attempt provisioned, state ready, no operation) and COMMIT - `inst-ds-finish-create-write`
4. [ ] - `p1` - **IF** the operation is a removal - `inst-ds-finish-remove`
   1. [ ] - `p1` - DB: DELETE datasets (the row) and COMMIT - `inst-ds-finish-remove-write`
5. [ ] - `p1` - **RETURN** completed - `inst-ds-finish-return`

### Decide Whether a Table Is Ours

- [ ] `p1` - **ID**: `cpt-insightspec-v3-algo-datasets-table-is-ours`

**Input**: a physical table name

**Output**: whether the service may drop or write that table

> Asked before a drop and before a write, never before a create: a table being created has no declaration naming it yet, and provisioning decides by shape alone.

**Steps**:
1. [ ] - `p1` - Resolve it in `cpt-insightspec-v3-db-datasets-database` alone; nothing outside that database is ever addressed - `inst-ds-ours-database`
2. [ ] - `p1` - **IF** no stored declaration names this table - `inst-ds-ours-unclaimed`
   1. [ ] - `p1` - **RETURN** not ours: a declaration naming a table is the only thing that makes it ours, so nothing is dropped on the strength of its name alone - `inst-ds-ours-unclaimed-return`
3. [ ] - `p1` - ClickHouse: SELECT system.tables (the table, with its engine and sorting key) - `inst-ds-ours-lookup`
4. [ ] - `p1` - **IF** it is absent - `inst-ds-ours-absent`
   1. [ ] - `p1` - **RETURN** absent; a removal treats that as a drop already done - `inst-ds-ours-absent-return`
5. [ ] - `p1` - **IF** its columns and sorting key are the ingest schema - `inst-ds-ours-schema`
   1. [ ] - `p1` - **RETURN** ours - `inst-ds-ours-yes`
6. [ ] - `p1` - **RETURN** not ours - `inst-ds-ours-no`

### Provision a Dataset's Table

- [ ] `p1` - **ID**: `cpt-insightspec-v3-algo-datasets-provision-table`

**Input**: a dataset name and the generation this attempt minted

**Output**: a table of the ingest schema, named for this generation, or a failure

**Steps**:
1. [ ] - `p1` - Derive the physical name from the dataset name and the generation, so that two attempts at one dataset never address one table and a name a reader sees is never the name a statement carries - `inst-ds-provision-name`
2. [ ] - `p1` - ClickHouse: SELECT system.tables for that name in the datasets database — ownership is decided here by shape alone, because the declaration that will name this table is written only once the create finishes - `inst-ds-provision-check`
3. [ ] - `p1` - **IF** a table of that name exists **AND** its columns and sorting key are not the ingest schema - `inst-ds-provision-foreign`
   1. [ ] - `p1` - **RETURN** a refusal naming the table - `inst-ds-provision-foreign-return`
4. [ ] - `p1` - ClickHouse: CREATE TABLE IF NOT EXISTS that name in the datasets database, with the ingest schema; an existing table of the right shape is this attempt repeating itself, so the step creates nothing twice - `inst-ds-provision-create`
5. [ ] - `p1` - DB: record the provisioned table on the row **while this attempt still owns the operation**, as `cpt-insightspec-v3-algo-datasets-finish-operation` does - `inst-ds-provision-record`
6. [ ] - `p1` - **IF** that write reaches no row, because the operation moved to another attempt - `inst-ds-provision-stale`
   1. [ ] - `p1` - Drop the table this attempt just made, which nothing points at, and **RETURN** stale - `inst-ds-provision-stale-return`
7. [ ] - `p1` - **RETURN** provisioned, or the failure, so the caller finishes nothing - `inst-ds-provision-return`

> Because the physical name carries the generation, a drop that arrives long after its attempt lost the dataset names a table that no longer exists, and cannot reach the one a later attempt provisioned. That is what keeps a late statement harmless without making the service guess whether one is still in flight.

### Resolve a Field to Its Read Expression

- [ ] `p1` - **ID**: `cpt-insightspec-v3-algo-datasets-field-expression`

**Input**: a dataset declaration, the name of one of its fields, and whether the reader wants the raw value or the presented one

**Output**: the expression that reads the field out of a stored record

**Steps**:
1. [ ] - `p1` - Look the field up in the declaration by name and take its key path and type - `inst-ds-expr-lookup`
2. [ ] - `p1` - Choose the payload extraction by type, per `cpt-insightspec-v3-design-dataset-contract`: text for string, integer or float for the numeric types, boolean for bool, a lenient timestamp parse for datetime - `inst-ds-expr-extract`
3. [ ] - `p1` - A key that is absent, JSON null, or present but unconvertible, yields the empty value for the type — never a zero standing in for a number - `inst-ds-expr-empty`
4. [ ] - `p1` - **IF** the caller asked for the raw value - `inst-ds-expr-raw`
   1. [ ] - `p1` - **RETURN** the extraction alone, with no substitute and no identity lookup: it is the record's own value, and an empty one stays distinguishable from every real value - `inst-ds-expr-raw-return`
5. [ ] - `p1` - **IF** the field declares an `absent_value` - `inst-ds-expr-absent`
   1. [ ] - `p1` - Wrap the expression so an empty value reads as the declared substitute, and a filter comparing against that substitute matches those records - `inst-ds-expr-absent-wrap`
6. [ ] - `p1` - **IF** the field carries `person` - `inst-ds-expr-person`
   1. [ ] - `p1` - Mark the expression for the identity lookup the compiler already performs for person handles - `inst-ds-expr-person-mark`
7. [ ] - `p1` - **RETURN** the presented expression - `inst-ds-expr-return`

> Only what a reader sees is substituted and resolved. Row identity, and anything else deciding whether two records are the same record, reads the raw form.

### Collapse Duplicates by Row Identity

- [ ] `p1` - **ID**: `cpt-insightspec-v3-algo-datasets-collapse-duplicates`

**Input**: a dataset declaration

**Output**: the relation a run reads: one record per identity

**Steps**:
1. [ ] - `p1` - **IF** the declaration names no row identity - `inst-ds-collapse-none`
   1. [ ] - `p1` - **RETURN** the table itself; every record is its own - `inst-ds-collapse-none-return`
2. [ ] - `p1` - Read each identity field through `cpt-insightspec-v3-algo-datasets-field-expression` in its raw form: a declared substitute would make two records with no key look like one record sharing a value, and a resolved person would group two accounts of one human as one event - `inst-ds-collapse-fields`
3. [ ] - `p1` - **IF** a record's identity is incomplete — any identity field is absent, null or unconvertible - `inst-ds-collapse-incomplete`
   1. [ ] - `p1` - Keep it as its own record, collapsed with nothing: an absent key is not evidence that two events are the same event - `inst-ds-collapse-incomplete-keep`
4. [ ] - `p1` - **FOR EACH** group of records sharing a complete identity - `inst-ds-collapse-group`
   1. [ ] - `p1` - Keep the one received last; on an equal receipt instant keep the greater record id, so the survivor is the same on every run - `inst-ds-collapse-winner`
5. [ ] - `p1` - **RETURN** the collapsed relation - `inst-ds-collapse-return`

### Compile a Metric Over a Dataset

- [ ] `p1` - **ID**: `cpt-insightspec-v3-algo-datasets-compile-metric`

**Input**: a stored metric, its dataset's declaration, the requested window

**Output**: one ClickHouse statement with bound parameters, and the undated count a windowed run needs

**Steps**:
1. [ ] - `p1` - Take the database and the physical table from the declaration, never from the metric, and refuse a dataset that is not ready - `inst-ds-compile-table`
2. [ ] - `p1` - Resolve the clock per `cpt-insightspec-v3-algo-datasets-effective-clock` - `inst-ds-compile-clock`
3. [ ] - `p1` - **IF** a window is requested **AND** there is no effective clock - `inst-ds-compile-clockless`
   1. [ ] - `p1` - **RETURN** the existing clockless-window refusal - `inst-ds-compile-clockless-reject`
4. [ ] - `p1` - **FOR EACH** field, filter and grouping reference - `inst-ds-compile-fields`
   1. [ ] - `p1` - Replace the name with its expression per `cpt-insightspec-v3-algo-datasets-field-expression` - `inst-ds-compile-field-expr`
5. [ ] - `p1` - Read from the relation `cpt-insightspec-v3-algo-datasets-collapse-duplicates` returns - `inst-ds-compile-source`
6. [ ] - `p1` - Apply the window, bucket, aggregation, grouping, ordering and limit exactly as today - `inst-ds-compile-rest`
7. [ ] - `p1` - **IF** the run is windowed - `inst-ds-compile-undated`
   1. [ ] - `p1` - Count the records the window left out because their clock is empty, over the collapsed relation and not the raw table, so the count and the rows describe the same records - `inst-ds-compile-undated-count`
8. [ ] - `p1` - Resolve a named range against the wall clock, as the service already does, using the effective clock only to decide which records fall inside it - `inst-ds-compile-window`
9. [ ] - `p1` - **RETURN** the statement and its parameters - `inst-ds-compile-return`

### Dependents of a Dataset

- [ ] `p1` - **ID**: `cpt-insightspec-v3-algo-datasets-dependents`

**Input**: a dataset name

**Output**: every metric that reads it, by name

**Steps**:
1. [ ] - `p1` - DB: SELECT metrics (every stored body) - `inst-ds-deps-scan`
2. [ ] - `p1` - **FOR EACH** metric whose body names this dataset in its dataset field - `inst-ds-deps-match`
   1. [ ] - `p1` - Add it to the result; a name appearing anywhere else in the body is not a dependency - `inst-ds-deps-add`
3. [ ] - `p1` - **RETURN** the result, ordered by metric name and never paged; this reads metrics, not the dataset's state, so it answers the same during a removal - `inst-ds-deps-return`

### Revalidate the Dependents of a Changed Declaration

- [ ] `p1` - **ID**: `cpt-insightspec-v3-algo-datasets-revalidate-dependents`

**Input**: the submitted declaration and the one it would replace

**Output**: the metrics the change would break, and the changes that would move a number without breaking anything

**Steps**:
1. [ ] - `p1` - Collect the dependents per `cpt-insightspec-v3-algo-datasets-dependents` - `inst-ds-reval-collect`
2. [ ] - `p1` - **FOR EACH** dependent metric - `inst-ds-reval-each`
   1. [ ] - `p1` - Run `cpt-insightspec-v3-algo-datasets-validate-metric` against the submitted declaration — the same check its own write passed, so a dropped field, a retyped one and a lost clock are all caught by one rule - `inst-ds-reval-validate`
   2. [ ] - `p1` - **IF** its effective clock changes field - `inst-ds-reval-clock`
      1. [ ] - `p1` - Record it: the metric would answer a different question over the same window - `inst-ds-reval-clock-record`
   3. [ ] - `p1` - **IF** a field it reads keeps its name and type but changes where it reads from - `inst-ds-reval-path`
      1. [ ] - `p1` - Record it: nothing about the metric is invalid, and every number it has ever answered changes - `inst-ds-reval-path-record`
3. [ ] - `p1` - **IF** the row identity gains, loses or changes a field - `inst-ds-reval-identity`
   1. [ ] - `p1` - Record it against every dependent: records that counted once begin to count separately, or the reverse, over the whole history the dataset holds - `inst-ds-reval-identity-record`
4. [ ] - `p1` - A change of this kind is refused rather than reported, because a reader cannot see it happen: the declaration is the only place it shows, and the numbers move underneath every board at once - `inst-ds-reval-refuse`
5. [ ] - `p1` - Stored records are never rewritten by any of this: a declaration says how records are read, so a change reinterprets what is already there - `inst-ds-reval-no-rewrite`
6. [ ] - `p1` - **RETURN** every metric that recorded something, with what it was - `inst-ds-reval-return`

### Describe Datasets for the Assistant and MCP

- [ ] `p1` - **ID**: `cpt-insightspec-v3-algo-datasets-describe`

**Input**: every stored declaration, or the names a lookup asked for

**Output**: the data section of the chat's system prompt, the look_up tool's answer, and the MCP describe payload

**Steps**:
1. [ ] - `p1` - DB: SELECT datasets (the names and bodies of the ready datasets only, so an unfinished create is never described and nothing is offered that cannot be queried) - `inst-ds-describe-read`
2. [ ] - `p1` - **FOR EACH** dataset - `inst-ds-describe-each`
   1. [ ] - `p1` - Emit the name, title and description - `inst-ds-describe-head`
   2. [ ] - `p1` - Emit each field as name, type, role, description and absent value, and mark the default clock - `inst-ds-describe-fields`
   3. [ ] - `p1` - Emit the row identity - `inst-ds-describe-identity`
3. [ ] - `p1` - **IF** no dataset exists - `inst-ds-describe-none`
   1. [ ] - `p1` - Emit that nothing has been declared yet and that an administrator creates datasets - `inst-ds-describe-none-text`
4. [ ] - `p1` - Leave out the database, the table, the key paths and the read expressions - `inst-ds-describe-hide`
5. [ ] - `p1` - **RETURN** the rendered section - `inst-ds-describe-return`

## 4. States (CDSL)

### Dataset Lifecycle

- [ ] `p1` - **ID**: `cpt-insightspec-v3-state-datasets-lifecycle`

**States**: Absent, Claimed, Ready, Removing

**Initial State**: Absent

**Transitions**:
1. [ ] - `p1` - **FROM** Absent **TO** Claimed **WHEN** a create takes the name, before any table exists - `inst-ds-state-claimed`
2. [ ] - `p1` - **FROM** Claimed **TO** Ready **WHEN** the table is provisioned and the declaration is stored - `inst-ds-state-created`
3. [ ] - `p1` - **FROM** Claimed **TO** Claimed **WHEN** provisioning fails, or another attempt takes the operation over after the lease lapses - `inst-ds-state-claim-retry`
4. [ ] - `p1` - **FROM** Ready **TO** Ready **WHEN** the declaration is replaced by one every dependent metric validates against - `inst-ds-state-replaced`
5. [ ] - `p1` - **FROM** Ready **TO** Ready **WHEN** a removal is refused because a metric reads it - `inst-ds-state-remove-refused`
6. [ ] - `p1` - **FROM** Ready **TO** Removing **WHEN** an administrator removes a dataset nothing reads - `inst-ds-state-removing`
7. [ ] - `p1` - **FROM** Removing **TO** Removing **WHEN** dropping the table fails, or a second removal finds the first still under way - `inst-ds-state-removing-retry`
8. [ ] - `p1` - **FROM** Claimed **TO** Removing **WHEN** an administrator removes a create that was abandoned; it has no readers and so no dependents to protect - `inst-ds-state-claimed-removing`
9. [ ] - `p1` - **FROM** Removing **TO** Absent **WHEN** the table is gone and the row is deleted - `inst-ds-state-removed`

> Only a **Ready** dataset is written to, run over, listed, previewed and described — every path asks for that state rather than excluding the states it knows about, so a dataset added to this list later cannot leak through one that was missed. Claimed and Removing both hold the name against every other writer: one so an unfinished create can be repeated, the other so nothing new takes the name mid-removal.
>
> Each transition is taken under the hold of `cpt-insightspec-v3-algo-datasets-serialize`. A transition that needs a table — into Ready from Claimed, and out of Removing — belongs to one leased attempt, and is written only if that attempt still owns the dataset when it finishes.

## 5. Definitions of Done

### Declaration Store and Validation

- [ ] `p1` - **ID**: `cpt-insightspec-v3-dod-datasets-store`

The system **MUST** store dataset declarations as a fourth definition kind in the definitions store, keyed by name and replaced on write like the others, and **MUST** validate every declaration before storing it, reporting all violations together. A field's type **MUST** decide what a metric may do with it; its role **MUST** remain descriptive, so that a numeric identifier can be grouped by. A replacement **MUST** be refused when any dependent metric fails to validate against it, when the clock a dependent inherits would change, when a field a dependent reads would read from a different place in the record, or when the row identity changes — naming each metric and why. The first two break a metric; the last two leave it valid and move every number it has answered, which a reader cannot see happen. Stored records **MUST NOT** be rewritten by any declaration change: a declaration says how records are read, so a change reinterprets what is already stored. Creation and removal **MUST** require the admin role, and the ingest token **MUST** be refused on both, so that whoever is trusted to send records cannot declare a dataset or remove one with its records. A credential of its own for the lifecycle, so that the two can be handed to different parties and rotated apart, is a later iteration: in this one the only way in is a session carrying the role.

**Implements**:
- `cpt-insightspec-v3-flow-datasets-create`
- `cpt-insightspec-v3-algo-datasets-validate-declaration`
- `cpt-insightspec-v3-algo-datasets-validate-metric`
- `cpt-insightspec-v3-algo-datasets-revalidate-dependents`

**Touches**:
- API: `PUT /v1/datasets/{name}`, `GET /v1/datasets`, `GET /v1/datasets/{name}`
- DB: `datasets`
- Entities: `Dataset`, `DatasetField`

### A Dataset Says What It Is Over

- [ ] `p1` - **ID**: `cpt-insightspec-v3-dod-datasets-source`

A declaration **MUST** say what its dataset is over, and **MUST NOT** leave it to be inferred from whether a relation happens to be named. Who provisions the relation, who may drop it, whether records may be sent and how every field is read all follow from it, which is more than the presence of a property should decide. A replacement **MUST NOT** change any of it, neither which of the two it is nor which relation it names. Turning a stream into a relation strands the records already sent, with a table still recorded against a dataset that no longer reads it; the other way round leaves a dataset whose row says ready and which has nothing to read, for good; and pointing it at another relation leaves every field valid and every dependent metric reading somewhere else, which is the same silent move as a field that changed where it reads from. None of the three is a state the rest of this reasons about, so each is refused rather than handled — a dataset is removed and declared anew.

A field **MUST** say where its value sits in the terms its mode uses: a key path into the record's payload, or a column of the relation. A field carrying both, or neither, **MUST** be refused before the body is read, so that nothing below has to referee one; a field carrying the wrong one for its mode **MUST** be refused against the key that carries it rather than read as though it were the other.

**Implements**:
- `cpt-insightspec-v3-flow-datasets-create`
- `cpt-insightspec-v3-algo-datasets-validate-declaration`

**Touches**:
- API: `PUT /v1/datasets/{name}`
- Entities: `Dataset`, `DatasetField`

### A Relation Is Read and Never Owned

- [ ] `p1` - **ID**: `cpt-insightspec-v3-dod-datasets-relation`

A dataset over a relation **MUST** be published without provisioning anything, and its removal **MUST** take the declaration and nothing else. That **MUST** hold by construction rather than by a check at the drop: only the stream mode may reach the table this service makes, a table name **MUST** be recorded against a dataset in one place only, and the connection that may create or drop a table **MUST** be bound to the datasets database — so the warehouse's own relations are out of its reach whatever a declaration says. An attempt that takes a name over from one that had already provisioned a table **MUST** forget that table as it publishes, or a dataset over a relation would inherit one and ingest, which decides by the table, would take records into it.

A declaration over a relation **MUST** be checked against the warehouse when it is written: a relation the warehouse does not have is refused against the name that asked for it, a column it does not hold against the field that named it, and a column whose own type the declared type cannot read against the type that asked for it. That last one **MUST** be an allow-list of the types a read is known to answer from, so a type nobody thought of is refused rather than trusted: a value that is not one value refuses the conversion outright rather than answering nothing, and that refusal would otherwise meet the reader on every run. Every value has a text form, so a string reads any column at all. A declaration over a stream describes records that have not arrived, so there is nothing to hold it to; one over a relation describes something that exists now, and a column it does not have compiles into every metric over the dataset and then fails on every run, far from where the mistake was made.

The relation **MUST** be one a plain read is known to count once per row. Its rows **MUST NOT** be collapsed by a declared identity — the relation decides for itself which of its rows are current, and a second rule here would quietly disagree with it — so a relation on an engine that keeps superseded rows **MUST** be refused when it is declared, naming a view over it as the way through. What counts as such an engine **MUST** be an allow-list: an engine nobody thought of, one fronting another, or a view whose rows are really an inner table's, then defaults to refused rather than to a number a reader cannot tell is wrong. `FINAL` cannot stand in for this, because an engine that does not collapse refuses it outright.

Records **MUST NOT** be sent into a dataset over a relation, and the refusal **MUST** say what the dataset is rather than that it is not there: it is there, it is ready, and saying otherwise sends a sender looking for a name in front of them.

**Implements**:
- `cpt-insightspec-v3-flow-datasets-create`
- `cpt-insightspec-v3-flow-datasets-remove`
- `cpt-insightspec-v3-flow-datasets-ingest`
- `cpt-insightspec-v3-algo-datasets-table-is-ours`

**Touches**:
- API: `PUT /v1/datasets/{name}`, `DELETE /v1/datasets/{name}`, `POST /v1/raw-data`
- DB: `datasets`, the warehouse relation named by the declaration

### Datasets Own Their Own Database

- [ ] `p1` - **ID**: `cpt-insightspec-v3-dod-datasets-database`

Every uploaded dataset's table **MUST** live in `cpt-insightspec-v3-db-datasets-database`, a ClickHouse database this service owns, separate from the warehouse the rest of Insight builds, and the service **MUST NOT** create, read, write or drop a table outside it. A table **MUST** be recognised as the service's own only when a stored declaration names it and its schema is the ingest schema, and every statement **MUST** address the physical name the declaration records rather than the dataset's own name.

**Implements**:
- `cpt-insightspec-v3-algo-datasets-table-is-ours`
- `cpt-insightspec-v3-algo-datasets-provision-table`

**Touches**:
- DB: the datasets database in ClickHouse, `system.tables`
- Entities: service configuration for the datasets database

### One Owner per Operation, and Every Step Repeatable

- [ ] `p1` - **ID**: `cpt-insightspec-v3-dod-datasets-recovery`

Creating, replacing and removing a dataset, and storing a metric that reads one, **MUST** each hold that dataset's row for the whole of their decision, so a state read is still true when it is acted on. A replacement of a ready dataset touches no table and **MUST** therefore commit as one transaction.

A create or a removal, which must touch a table, **MUST** own a leased operation on the dataset: only its owner may create or drop that dataset's tables; a second attempt **MUST** be refused while the lease holds and **MUST** be able to take over once it lapses; and an attempt **MUST** write its outcome only if it still owns the operation when it finishes, discarding it otherwise. Each dataset's physical table **MUST** be named for the attempt's own generation, so a statement arriving after its attempt lost the dataset cannot reach a table some later attempt provisioned. A table **MUST NOT** be dropped on the strength of its name: only a table a stored declaration names is ours.

**Implements**:
- `cpt-insightspec-v3-flow-datasets-create`
- `cpt-insightspec-v3-flow-datasets-remove`
- `cpt-insightspec-v3-algo-datasets-serialize`
- `cpt-insightspec-v3-algo-datasets-take-operation`
- `cpt-insightspec-v3-algo-datasets-finish-operation`
- `cpt-insightspec-v3-state-datasets-lifecycle`

**Touches**:
- API: `PUT /v1/datasets/{name}`, `DELETE /v1/datasets/{name}`, `PUT /v1/metrics/{name}`
- DB: `datasets`, `metrics`, the datasets database in ClickHouse

### Operations Can See and Tune What This Adds

- [ ] `p1` - **ID**: `cpt-insightspec-v3-dod-datasets-operations`

Three values **MUST** be configuration rather than constants: the datasets database, the operation lease bound, and the record-preview cap. The lease decides how long an abandoned create or removal holds its dataset, so an installation that cannot wait must be able to shorten it; the cap decides the widest page of records the service will serve.

Every write that changes what a dataset is **MUST** record who asked for it: creating, replacing and removing are administrator-only and a removal takes a dataset's records with it, so "who removed this" **MUST** have an answer. The service **MUST** log, at least, an operation it refused because another attempt held it, an attempt that finished stale, and a drop that failed — the three states an operator has to recognise to know whether to wait or to repeat.

**Implements**:
- `cpt-insightspec-v3-algo-datasets-take-operation`
- `cpt-insightspec-v3-algo-datasets-finish-operation`
- `cpt-insightspec-v3-flow-datasets-remove`

**Constraints**: `cpt-insightspec-v3-design-dataset-settings`

**Touches**:
- DB: `datasets`
- Entities: service configuration, the service's logs

### Only a Ready Dataset Is Reachable

- [ ] `p1` - **ID**: `cpt-insightspec-v3-dod-datasets-ready-gate`

Every path that reads or writes a dataset's records or its declaration — ingest, running a metric, the catalogue, one dataset's page, the record preview, and the descriptions given to the assistant and to MCP — **MUST** require the ready state rather than excluding the states it happens to know about, so that a dataset mid-create is never written into, listed, described or queried, and one mid-removal disappears from all of them at once.

**Implements**:
- `cpt-insightspec-v3-flow-datasets-ingest`
- `cpt-insightspec-v3-flow-datasets-browse`
- `cpt-insightspec-v3-flow-datasets-read-board`
- `cpt-insightspec-v3-algo-datasets-describe`
- `cpt-insightspec-v3-state-datasets-lifecycle`

**Touches**:
- API: `POST /v1/raw-data`, `GET /v1/datasets`, `GET /v1/datasets/{name}`, `GET /v1/datasets/{name}/records`, `POST /v1/metrics/{name}/run`, `POST /v1/chat`, MCP tools on `/mcp/v3`
- DB: `datasets`

### Ingest Addresses a Dataset

- [ ] `p1` - **ID**: `cpt-insightspec-v3-dod-datasets-ingest`

The system **MUST** accept a record only into a dataset that is Ready, refusing an unknown or removing name without creating anything, and **MUST** create the dataset's table when the dataset is created rather than on first write. The record **MUST** be stored whole whatever its shape.

**Implements**:
- `cpt-insightspec-v3-flow-datasets-ingest`

**Touches**:
- API: `POST /v1/raw-data`
- DB: the dataset's table in the datasets database

### Metrics Read Datasets Only

- [ ] `p1` - **ID**: `cpt-insightspec-v3-dod-datasets-metrics`

A metric **MUST** name a dataset and read its declared fields by name in its selected fields, its filters and its clock, while its grouping and ordering **MUST** name the columns the metric itself produces, as they do today; a body naming a table or a database **MUST** be refused with the dataset catalogue as the admissible set. Admissibility **MUST** follow the field's type: sum and average require a numeric field and a clock requires a datetime, while grouping, filtering, counting, min and max are open to every type. A metric **MUST** be able to filter by a field it also aggregates: over a relation a field is one of the relation's own columns, and an output name the metric chose would otherwise stand in front of the column it was computed from, refusing the whole query rather than reading it. Compilation **MUST** read every value per `cpt-insightspec-v3-design-dataset-contract`, and **MUST** take the database, the table, every field's read expression and the effective clock from the declaration, and **MUST NOT** offer the warehouse catalogue to metrics or to the chat in this iteration.

**Implements**:
- `cpt-insightspec-v3-flow-datasets-author-metric`
- `cpt-insightspec-v3-algo-datasets-validate-metric`
- `cpt-insightspec-v3-algo-datasets-field-expression`
- `cpt-insightspec-v3-algo-datasets-compile-metric`

**Touches**:
- API: `PUT /v1/metrics/{name}`, `POST /v1/metrics/{name}/run`
- DB: `metrics`, `datasets`
- Entities: `MetricQuery`

### An Inherited Clock Is Visible to the Reader

- [ ] `p1` - **ID**: `cpt-insightspec-v3-dod-datasets-effective-clock`

A metric with no clock of its own **MUST** be windowed by its dataset's default clock, and the metric read endpoint **MUST** report the effective clock and where it came from. The portal **MUST** decide whether a card is windowed from that report rather than from the presence of a clock in the metric body, so a card over an inheriting metric follows the board's window instead of showing all time.

**Implements**:
- `cpt-insightspec-v3-flow-datasets-read-board`
- `cpt-insightspec-v3-algo-datasets-effective-clock`

**Touches**:
- API: `GET /v1/metrics/{name}`, `POST /v1/metrics/{name}/run`
- Entities: the custom dashboard page and its widget cards

### Duplicates Collapse on Read

- [ ] `p1` - **ID**: `cpt-insightspec-v3-dod-datasets-dedup`

When a dataset declares a row identity, every run over it **MUST** count one record per identity, keeping the one received last and breaking an equal receipt instant by record id so that repeated runs agree. The count of records a window left out for want of a clock **MUST** be read over the collapsed relation, so that it describes the same records the run returns. Identity **MUST** be read from each field's raw value, before any declared substitute and before any person is resolved, so that two records missing the key are not collapsed through one substitute and two accounts of one person are not collapsed into one event. A record whose identity is incomplete — any identity field absent, null or unconvertible — **MUST** stand alone rather than collapse with others, and a dataset without a row identity **MUST** read every record as today.

**Implements**:
- `cpt-insightspec-v3-algo-datasets-collapse-duplicates`
- `cpt-insightspec-v3-algo-datasets-compile-metric`

**Touches**:
- DB: the dataset's table in the datasets database

### Assistant and MCP Read Declarations

- [ ] `p1` - **ID**: `cpt-insightspec-v3-dod-datasets-assistant`

The chat's system prompt, its look_up tool and the MCP describe tools **MUST** be built from dataset declarations rather than from sampled records, **MUST** leave out physical detail, and the create tools **MUST** refuse to create a dataset.

Refusing that is not tidiness. A dataset is where the boundary of what an agent may read is drawn, and a dataset over a relation is a read path onto the warehouse — so a server that could declare one could widen its own reach to any relation there, without asking anybody. This service already keeps those two apart: reading the warehouse is a grant of its own, held by a different server under a read-only warehouse user, and the authoring server holds the grant to write definitions over what a person has already declared. An agent therefore works inside a boundary somebody drew, and moving it stays an administrator's act. The MCP list_tables tool **MUST** be replaced by dataset listing and description.

**Implements**:
- `cpt-insightspec-v3-flow-datasets-chat`
- `cpt-insightspec-v3-algo-datasets-describe`

**Touches**:
- API: `POST /v1/chat`, MCP tools on `/mcp/v3`
- DB: `datasets`

### Portal Dataset Catalogue

- [ ] `p1` - **ID**: `cpt-insightspec-v3-dod-datasets-portal`

The Custom zone **MUST** gain a Datasets catalogue beside Metrics, Widgets and Dashboards, a dataset page with the declaration, the records it holds and the dependent metrics, and a remove action that shows the dependents when refused. The catalogue **MUST** say what each dataset is over, and name the relation where it reads one: that is what decides whether records may be sent into it, and a reader should not have to open it to find out. The rail **MUST** list the catalogue for administrators only. The dependent list **MUST** come from an exact dependency lookup, never from a catalogue search over stored bodies.

The records **MUST** be shown as a table whose columns are the declared fields, read exactly as the declaration says they are read, with the whole record a click away — the page exists to answer "is what arrives what I meant", so a table that read a path differently from the metrics over it would answer it wrongly. The table **MUST** page through the whole dataset and order by any declared field or by the instant records arrived. A dataset may declare more fields than a table can show, so which columns are drawn **MUST** be the reader's choice, and that choice **MUST** survive leaving the page.

The page size **MUST** be the service's to decide and **MUST** be reported with the page, because the cap is an installation's setting: a reader stepping by a size of its own would walk over whatever a narrower page left behind, and nothing in the answer would say so.

A page **MUST** be read in a total order. A record sent in is told apart by the instant it arrived and its own identity; a row of a relation has neither, so the order **MUST** run over every field the dataset declares. A partial order makes a page read by offset show one row twice and skip another, on data nobody has touched — and two rows agreeing on every declared field are interchangeable to every reader of the dataset, so that is as total an order as this can see.

**Implements**:
- `cpt-insightspec-v3-flow-datasets-browse`
- `cpt-insightspec-v3-flow-datasets-remove`
- `cpt-insightspec-v3-algo-datasets-dependents`

**Touches**:
- API: `GET /v1/datasets`, `GET /v1/datasets/{name}`, `GET /v1/datasets/{name}/records`, `GET /v1/datasets/{name}/dependents`, `DELETE /v1/datasets/{name}`
- Entities: portal routes under `/portal/custom/datasets`

### One Editor for Every Definition

- [ ] `p1` - **ID**: `cpt-insightspec-v3-dod-datasets-editor`

Every definition the Custom zone holds — a dataset, a metric, a widget, a dashboard — **MUST** be creatable and editable by hand in the portal. Three of them have no such surface today, so a reader who wants a small change asks the assistant for one and hopes.

The editor **MUST** be one editor over four descriptions, not four editors: what differs between the kinds is the shape of the document, and the act of editing is the same. The editor **MUST** hold a document, offer the fields that kind admits, add and remove entries of a list, choose the variant of an entry where a kind has variants — a widget by its type, a dashboard item by what it carries — and offer the names of definitions a field refers to, rather than asking them to be typed from memory.

Beside the fields **MUST** sit the same document as text, holding what would be sent, so that one can be pasted whole or carried from another installation. The two **MUST** show one document rather than two copies: whichever was edited last is believed, text that does not parse leaves the fields as they stand and blocks sending until it does, and a property the fields do not know **MUST** be sent rather than dropped, so a mistyped one is refused by name instead of disappearing. One action sends what both show.

A property the definition is made with and cannot afterwards change **MUST** be shown and **MUST NOT** be offered: a control that takes an edit only to have the service refuse it is a worse way to say so than not offering it. What a dataset is over is such a property.

A refusal **MUST** be shown where it belongs: a violation naming a place in the document against that place, in the fields and in the text alike, and one naming the document as a whole above it. A form that draws an input of its own outside the description — the definition's name is one — **MUST** place a refusal about it there too, rather than above the form with the offending input unmarked. Nothing about a kind's shape **MUST** be inferred from an example or a stored body; each description says what its kind admits.

**Implements**:
- `cpt-insightspec-v3-flow-datasets-create`
- `cpt-insightspec-v3-algo-datasets-validate-declaration`

**Touches**:
- API: `PUT /v1/datasets/{name}`, `PUT /v1/metrics/{name}`, `PUT /v1/widgets/{name}`, `PUT /v1/dashboards/{name}`
- Entities: the Custom zone's definition editor and one description per kind

### Refusals Say Where They Belong

- [ ] `p2` - **ID**: `cpt-insightspec-v3-dod-datasets-violations`

A refusal to store a definition **MUST** carry the places in the document it is about, each with a stable reason and a sentence naming what would have been admissible, rather than one sentence about the body as a whole — that is what lets an editor put a message beside the field that caused it.

A dataset declaration **MUST** answer this way from the start. Where a kind's checks already know the place they failed at, they **MUST** answer the same way; where a check only knows that the body as a whole is wrong, it **MUST** say so plainly and the editor **MUST** show it above the document rather than inventing a field for it.

**Implements**:
- `cpt-insightspec-v3-algo-datasets-validate-declaration`
- `cpt-insightspec-v3-flow-datasets-author-metric`

**Touches**:
- API: `PUT /v1/{datasets,metrics,widgets,dashboards}/{name}`

### Existing Streams and Definitions Are Not Carried Over

- [ ] `p1` - **ID**: `cpt-insightspec-v3-dod-datasets-no-carryover`

Nothing migrates. The service **MUST NOT** read, move, rename or adopt a table it did not create as a dataset, and **MUST NOT** rewrite a stored metric. A stand that already holds stream tables and custom definitions keeps them where they are, unreachable: every custom metric stored before this change names a table and is therefore refused, and every stream table sits outside the datasets database and is never addressed. Datasets are declared fresh and records are sent again through the ordinary ingest path.

The service **MUST** stop creating an ingest-schema landing table in the warehouse database, so that after this change it owns nothing outside the datasets database.

**Implements**:
- `cpt-insightspec-v3-state-datasets-lifecycle`

**Touches**:
- DB: the datasets database in ClickHouse
- Entities: the `migrate` subcommand, the landing-table migration

## 6. Acceptance Criteria

- [ ] A record sent to a dataset that does not exist is refused and no table appears, whatever token it carries
- [ ] A dataset created in the portal is listed in the catalogue, in the rail and in the assistant's context, and its table exists before the first record
- [ ] The create form offers only what a declaration admits: the types, the roles, one main date, and a row identity chosen from the fields declared
- [ ] Editing the form updates the text beside it, and text that parses updates the form; text that does not parse leaves the form alone and blocks sending
- [ ] A declaration pasted as text, carrying a property the form does not know, is sent as pasted and refused by name rather than silently stripped
- [ ] A declaration refused for several reasons shows each one against the field it belongs to, not as one message above the form
- [ ] A metric, a widget and a dashboard can each be created and changed by hand in the portal, through the same editor a dataset uses
- [ ] Changing a widget's type changes the fields the editor offers, and a dashboard item offers what its kind of item carries
- [ ] A field that refers to another definition offers the names that exist rather than expecting one to be typed
- [ ] A create whose table step fails leaves the name claimed and nothing readable, and the request repeated succeeds
- [ ] A create and a removal of one dataset, issued together, end in one of the two outcomes and never in a ready dataset whose table was dropped
- [ ] A metric stored while its dataset is being removed either lands before the removal's dependency check, which then refuses, or is refused itself
- [ ] Two removals issued together drop the table once, and the second reports the removal already under way rather than dropping anything
- [ ] An attempt that finishes after losing its operation writes nothing, and the table it had provisioned is left pointing at no dataset
- [ ] A drop belonging to a lapsed attempt cannot delete the table a later attempt provisioned, because the two tables are named apart
- [ ] A dataset mid-create is absent from the catalogue, from its own page, from the assistant's context and from MCP, refuses records, and no metric can be stored against it
- [ ] A dataset's records are read a page at a time and ordered by any declared field, the records missing that field sorting last rather than filling the first page; a field the dataset does not declare is refused naming the ones it does
- [ ] A page asked to be wider than the installation allows is refused rather than quietly cut down, and every page says the size that was applied
- [ ] Shortening the configured lease lets an abandoned create be taken over sooner, and the three values this feature adds are read from configuration rather than compiled in
- [ ] A removed dataset leaves a record of who removed it and when
- [ ] The ingest token cannot declare or remove a dataset
- [ ] A dataset whose name is held by a table of another shape is refused, and no table outside the datasets database is ever created, read or dropped
- [ ] A declaration with several problems is refused once, with every problem and its field path
- [ ] A metric groups by a numeric field and is stored; the same field summed as a string field is refused
- [ ] A metric body naming a table is refused; the same metric rewritten onto a dataset is stored and runs
- [ ] A dataset declared over a relation the warehouse already builds is listed, described and read exactly like one records are sent into, and a metric over it runs
- [ ] Declaring one over a relation creates nothing, and removing it leaves the relation where it was; no path in the service can drop or alter it
- [ ] A relation the warehouse does not have is refused against the name that asked for it, and a column it does not hold against the field that named it
- [ ] A relation whose engine is not known to count each row once is refused when it is declared, naming a view over it as the way through
- [ ] A record sent into a dataset over a relation is refused as the wrong kind of dataset, not as one that is absent
- [ ] Replacing a declaration so that what the dataset is over would change is refused — the mode and the relation alike; removing it and declaring it anew is how it is done
- [ ] A column is refused when its own type is not one the declared type can read; the same column declared a string is accepted, because every value has a text form
- [ ] Two pages of a relation, read one after the other on data nobody has touched, hold no row in common and leave none out
- [ ] A metric filters by a field it also sums, over a relation whose column carries that field's name
- [ ] The catalogue says what each dataset is over without it being opened, and names the relation where it reads one
- [ ] Editing a dataset shows what it is over and does not offer to change it
- [ ] A field of a dataset over a relation reads a column and one over a stream reads a path; the wrong one is refused against the key that carries it
- [ ] A dataset over a relation declares no row identity, and its page shows rows with no arrival instant and no identity of their own
- [ ] Two identical records, re-sent, count once in every metric over a dataset that declares a row identity, and twice over one that does not
- [ ] Two different records missing the same identity field are counted separately, not collapsed into one, even when that field declares a substitute for absent values
- [ ] A metric ordering by the name of an aggregate it computes is stored and runs
- [ ] A card whose metric has no clock of its own follows the board's window through the dataset's default clock, and a dataset with no clock at all shows the card as covering all time
- [ ] Removing a dataset a metric reads is refused naming the metric; after the metric is removed the dataset and its table are gone, and the request repeated is not found
- [ ] A removal whose drop fails leaves the dataset unreachable and its name unclaimable, and the request repeated completes it
- [ ] A replacement that drops a field a metric reads, retypes it, or moves the clock a metric inherits, is refused naming the metric and the reason; adding a field is accepted
- [ ] The assistant's system prompt names every dataset with its declared fields and no warehouse table, and a proposal over a table is refused and repaired
- [ ] An MCP client lists and describes datasets and cannot create one
- [ ] On a stand that already held stream tables and custom metrics, nothing is moved, renamed or rewritten: the tables stay where they were, the stored metrics stay as they were, and every one of them is refused because it names a table
- [ ] After this change the service creates no table outside the datasets database, including the landing table earlier releases created on every deploy
- [ ] A dataset declared fresh and sent the same records twice holds them once when it declares a row identity, so an interrupted load is finished by sending it again

## 7. Testing

**Feature**: `cpt-insightspec-v3-feature-datasets`

The risks are a silent mismatch between what a declaration says and what a run reads, a write path that still creates tables on its own, and a migration that changes what a stored metric means. Unit tests drive the handlers, the validator and the compiler with an in-memory definitions store and the ClickHouse mock the service already tests against; the portal is exercised with the request mocks the Custom zone's tests use; one stand check confirms the admin gate end to end.

- [ ] 1. **Unknown dataset refuses a record** — Reliability · rust-unit — post a record naming a dataset that does not exist → not found, and no create statement reaches ClickHouse.
  **Requirements**: `cpt-insightspec-v3-fr-ingest-into-dataset`.
  **Covers**: `cpt-insightspec-v3-dod-datasets-ingest`.
  **Test**: Not implemented.
- [ ] 2. **Every declaration violation is reported together** — Reliability · rust-unit — submit a declaration with an unknown type, two default clocks and an identity over an undeclared field → one refusal listing all three with field paths, nothing stored.
  **Requirements**: `cpt-insightspec-v3-fr-create-dataset`.
  **Covers**: `cpt-insightspec-v3-dod-datasets-store`.
  **Test**: Not implemented.
- [ ] 3. **Type decides admissibility, and outputs are named apart from fields** — Reliability · rust-unit — store a metric grouping by an int field declared as a measurable and ordering by the name of an aggregate it computes → accepted; sum a string field, and order by a name the metric does not produce → each refused naming what would be admissible.
  **Requirements**: `cpt-insightspec-v3-fr-metrics-over-datasets`.
  **Covers**: `cpt-insightspec-v3-dod-datasets-metrics`.
  **Test**: Not implemented.
- [ ] 4. **A metric naming a table is refused** — Reliability · rust-unit — put a metric body with `table` → refused with the dataset catalogue as the admissible set; the same body with `dataset` is stored.
  **Requirements**: `cpt-insightspec-v3-fr-metrics-over-datasets`.
  **Covers**: `cpt-insightspec-v3-dod-datasets-metrics`.
  **Test**: Not implemented.
- [ ] 5. **Compiled SQL reads through the declaration** — Reliability · rust-unit — compile a metric over a dataset whose field is declared as a nested key with an absent value → the statement extracts that key, substitutes the absent value, takes database and table from the dataset, and binds every caller value.
  **Requirements**: `cpt-insightspec-v3-fr-metrics-over-datasets`.
  **Covers**: `cpt-insightspec-v3-dod-datasets-metrics`.
  **Test**: Not implemented.
- [ ] 6. **Duplicates collapse only on a complete, raw identity** — Reliability · rust-unit — run over records where two share a complete identity, two share a receipt instant, and two are missing an identity field that declares a substitute → the shared pair counts once with the later record winning and the id breaking the tie, and the pair missing the key counts twice because identity reads the raw value.
  **Requirements**: `cpt-insightspec-v3-fr-metrics-over-datasets`, `cpt-insightspec-v3-nfr-reliability`.
  **Covers**: `cpt-insightspec-v3-dod-datasets-dedup`.
  **Test**: Not implemented.
- [ ] 7. **A create that cannot provision holds its name and repeats** — Reliability · rust-unit — create a dataset with the table step failing → the name is claimed, the dataset is not readable and no ready declaration is stored; repeat once the lease lapses with it succeeding → the dataset is ready and names the table its own attempt provisioned.
  **Requirements**: `cpt-insightspec-v3-fr-create-dataset`, `cpt-insightspec-v3-nfr-reliability`.
  **Covers**: `cpt-insightspec-v3-dod-datasets-recovery`, `cpt-insightspec-v3-dod-datasets-database`.
  **Test**: Not implemented.
- [ ] 8. **Removal is refused while referenced and recovers after a failed drop** — Reliability · rust-unit — delete a dataset a metric reads → conflict naming it; delete an unreferenced one whose drop fails → it is removing, unreachable and unclaimable, and the repeated request drops the table and deletes the declaration.
  **Requirements**: `cpt-insightspec-v3-fr-remove-dataset`.
  **Covers**: `cpt-insightspec-v3-dod-datasets-recovery`.
  **Test**: Not implemented.
- [ ] 9. **A breaking replacement is refused by revalidating dependents** — Reliability · rust-unit — replace a declaration dropping a field a metric reads, then retyping one, then moving the default clock a metric inherits → each refused naming the metric and the reason; adding a field is accepted.
  **Requirements**: `cpt-insightspec-v3-fr-create-dataset`, `cpt-insightspec-v3-nfr-reliability`.
  **Covers**: `cpt-insightspec-v3-dod-datasets-store`.
  **Test**: Not implemented.
- [ ] 10. **Nothing outside the datasets database is touched, and nothing is dropped by name** — Security · rust-unit — create, read and remove datasets whose names match warehouse relations, and point a removal at a table no declaration names → every statement names the datasets database and the physical table the declaration records, and the unclaimed table is never dropped.
  **Requirements**: `cpt-insightspec-v3-fr-remove-dataset`, `cpt-insightspec-v3-nfr-security`.
  **Covers**: `cpt-insightspec-v3-dod-datasets-database`.
  **Test**: Not implemented.
- [ ] 11. **Only administrators create and remove datasets, and the ingest token is not one** — Security · stand-api — call PUT and DELETE on a dataset as a signed-in non-administrator, with no credential, and with the ingest token → each refused; as an administrator → accepted.
  **Requirements**: `cpt-insightspec-v3-fr-create-dataset`, `cpt-insightspec-v3-fr-remove-dataset`, `cpt-insightspec-v3-nfr-security`.
  **Covers**: `cpt-insightspec-v3-dod-datasets-store`.
  **Test**: Not implemented.
- [ ] 12. **The assistant sees declarations, not the warehouse** — Security · rust-unit — build the chat context over declared datasets and warehouse relations → the prompt names every dataset with its fields and no warehouse table; a proposal over a table is handed back for repair; a create carrying a dataset is refused.
  **Requirements**: `cpt-insightspec-v3-fr-assistant-reads-datasets`.
  **Covers**: `cpt-insightspec-v3-dod-datasets-assistant`.
  **Test**: Not implemented.
- [ ] 14. **An inherited clock drives the board's window** — Versatility · fe-component — render a card whose metric reports a clock inherited from its dataset, and one whose metric reports none → the first sends the picked window and draws buckets, the second is marked all time and sends none.
  **Requirements**: `cpt-insightspec-v3-fr-metrics-over-datasets`, `cpt-insightspec-v3-nfr-versatility`.
  **Covers**: `cpt-insightspec-v3-dod-datasets-effective-clock`.
  **Test**: Not implemented.
- [ ] 15. **The catalogue renders every dataset surface** — Versatility · fe-component — open the Datasets catalogue, a dataset page and its remove action with mocked responses → list with search and total, fields and records and dependents on the page, the dependents shown when removal is refused.
- [ ] 15a. **The records table reads a declaration the way the service does** — Correctness · fe-component — draw a record against fields whose paths hold an escaped dot and which declare a substitute for an absent value → each cell holds what a metric over the same declaration would read.
- [ ] 15b. **Paging follows the size the service reports** — Reliability · fe-component — answer with a page smaller than the client's own assumption, then step forward → the next request's offset is the reported size, so no record between the two pages is skipped.
  **Requirements**: `cpt-insightspec-v3-fr-view-dataset`, `cpt-insightspec-v3-nfr-versatility`.
  **Covers**: `cpt-insightspec-v3-dod-datasets-portal`.
  **Test**: Not implemented.
- [ ] 16. **Ingest adds no warehouse round trip** — Efficiency · rust-unit — post a record into an existing dataset → exactly one insert statement reaches ClickHouse and the dataset lookup is answered by the definitions store.
  **Requirements**: `cpt-insightspec-v3-fr-ingest-into-dataset`, `cpt-insightspec-v3-nfr-efficiency`.
  **Covers**: `cpt-insightspec-v3-dod-datasets-ingest`.
  **Test**: Not implemented.
- [ ] 17. **Concurrent writers of one dataset are serialized** — Reliability · rust-unit — run a replacement against a removal, then a metric write against a removal, against a store that records lock order → each pair ends in one coherent outcome, never a ready dataset whose table was dropped and never a stored metric over a removed dataset.
  **Requirements**: `cpt-insightspec-v3-fr-create-dataset`, `cpt-insightspec-v3-fr-remove-dataset`, `cpt-insightspec-v3-nfr-reliability`.
  **Covers**: `cpt-insightspec-v3-dod-datasets-recovery`.
  **Test**: Not implemented.
- [ ] 20. **One owner per operation, and a stale attempt writes nothing** — Reliability · rust-unit — take a create, then attempt a second create before and after the lease lapses → refused, then taken over; let the first attempt finish afterwards → it reports stale and stores no declaration; issue two removals together → one drops the table and the other reports the removal under way.
  **Requirements**: `cpt-insightspec-v3-fr-create-dataset`, `cpt-insightspec-v3-fr-remove-dataset`, `cpt-insightspec-v3-nfr-reliability`.
  **Covers**: `cpt-insightspec-v3-dod-datasets-recovery`.
  **Test**: Not implemented.
- [ ] 21. **A dataset that is not ready is written to by nothing** — Reliability · rust-unit — with a dataset claimed by an unfinished create and another mid-removal, post a record, run a metric and store a metric against each → every one is refused, and each statement that does run names the physical table the declaration records.
  **Requirements**: `cpt-insightspec-v3-fr-ingest-into-dataset`, `cpt-insightspec-v3-nfr-reliability`.
  **Covers**: `cpt-insightspec-v3-dod-datasets-ready-gate`.
  **Test**: Not implemented.
- [ ] 22. **MCP lists and describes datasets and cannot create one** — Versatility · rust-unit — call each MCP tool against declared datasets → listing and describing answer from declarations with no storage detail, the table-listing tool is gone, and no tool creates a dataset.
  **Requirements**: `cpt-insightspec-v3-fr-assistant-reads-datasets`, `cpt-insightspec-v3-nfr-versatility`.
  **Covers**: `cpt-insightspec-v3-dod-datasets-assistant`.
  **Test**: Not implemented.
- [ ] 23. **The lease is configurable and the three states are visible** — Efficiency · rust-unit — take an operation with a shortened lease → it may be taken over sooner; drive a refused take, a stale finish and a failed drop → each is logged distinguishably, and a removal records who asked for it.
  **Requirements**: `cpt-insightspec-v3-nfr-efficiency`.
  **Covers**: `cpt-insightspec-v3-dod-datasets-operations`.
  **Test**: Not implemented.
- [ ] 24. **A dataset that is not ready is shown by nothing** — Reliability · rust-unit — with the same two datasets, read the catalogue, one dataset, the record preview and the assistant's context → each omits both, and one dataset answers not found rather than a partial declaration.
  **Requirements**: `cpt-insightspec-v3-fr-view-dataset`, `cpt-insightspec-v3-fr-assistant-reads-datasets`.
  **Covers**: `cpt-insightspec-v3-dod-datasets-ready-gate`.
  **Test**: Not implemented.
- [ ] 25. **The editor builds a declaration and shows where it is wrong** — Versatility · fe-component — add fields, mark one as the main date, pick a row identity from them, then submit a declaration the service refuses for two different fields → the editor offers only admissible types and roles, sends what was built, and shows each violation against its own field.
  **Requirements**: `cpt-insightspec-v3-fr-create-dataset`, `cpt-insightspec-v3-fr-author-by-hand`, `cpt-insightspec-v3-nfr-versatility`.
  **Covers**: `cpt-insightspec-v3-dod-datasets-editor`.
  **Test**: Not implemented.
- [ ] 26. **Fields and text stay one document** — Reliability · fe-component — edit the fields and read the text, paste a document that parses and read the fields, type text that does not parse, then paste one carrying an unknown property → the text follows the fields, the fields follow text that parses, broken text leaves the fields untouched and blocks sending, and the unknown property reaches the request.
  **Requirements**: `cpt-insightspec-v3-fr-create-dataset`, `cpt-insightspec-v3-nfr-reliability`.
  **Covers**: `cpt-insightspec-v3-dod-datasets-editor`.
  **Test**: Not implemented.
- [ ] 27. **One editor serves every kind, and a variant changes what it offers** — Versatility · fe-component — open the editor on a stored metric, widget and dashboard in turn, then change the widget's type and the kind of a dashboard item → each kind offers its own document, the offered fields follow the chosen variant, a field referring to another definition offers the names that exist, and a refusal naming no place is shown above the document.
  **Requirements**: `cpt-insightspec-v3-fr-author-by-hand`, `cpt-insightspec-v3-fr-create-metrics`, `cpt-insightspec-v3-fr-create-widgets`, `cpt-insightspec-v3-fr-create-dashboards`, `cpt-insightspec-v3-nfr-versatility`.
  **Covers**: `cpt-insightspec-v3-dod-datasets-editor`.
  **Test**: Not implemented.

**Performance** — n/a: the feature states no latency or throughput obligation of its own, and the two costs it adds — a payload parsed per record on every run, and a collapsing read over a dataset that declares an identity — are the costs the deferred column materialization exists to remove. Naming a budget here would invent one.
